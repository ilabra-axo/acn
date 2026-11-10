//! Startup and replay: history is reconstructed, never re-executed, and the same authorization
//! applies to it as to live traffic (A01, A02); paid tools can't be had for free through the
//! direct command path. MCP executions are counted by the echo server's call log.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acn_core::draft::Draft;
use acn_core::kinds::*;
use nmesh_agent_bridge::config::{
    AcnConfig, AgentConfig, AgentSection, McpServerConfig, McpTransport,
};
use nmesh_agent_bridge::Bridge;
use nmesh_core::{Body, IdentityKind, Keys, PublicKey, Session, SessionConfig, Timestamp};
use nmesh_proto::manifest::Hosting;
use nostr_relay_builder::MockRelay;
use serde_json::json;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nmesh-startup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn calls(log: &Path) -> usize {
    std::fs::read_to_string(log)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

fn config(url: &str, dir: &Path, acn: AcnConfig, allow: Vec<String>) -> AgentConfig {
    AgentConfig {
        agent: AgentSection {
            id: "echo".into(),
            name: "Echo agent".into(),
            about: "test".into(),
            key_file: dir.join("agent.nsec"),
            hosting: Hosting::SelfHosted,
            operator: "tests".into(),
            retention: "none".into(),
            forwards_to: vec![],
            allow,
        },
        relays: vec![url.to_string()],
        servers: vec![McpServerConfig {
            name: "echo".into(),
            transport: McpTransport::Stdio {
                command: env!("CARGO_BIN_EXE_nmesh-echo-mcp").to_string(),
                args: vec![],
                env: [(
                    "NMESH_ECHO_CALL_LOG".to_string(),
                    dir.join("calls.log").to_string_lossy().into_owned(),
                )]
                .into(),
            },
        }],
        acn,
    }
}

/// Start a bridge and serve in the background; returns it (to stop it) and its key.
async fn start(cfg: AgentConfig) -> (Arc<Bridge>, tokio::task::JoinHandle<()>, PublicKey) {
    let bridge = Arc::new(Bridge::start(cfg).await.unwrap());
    let pk = bridge.session().public_key();
    let b = bridge.clone();
    let task = tokio::spawn(async move {
        let _ = b.run().await;
    });
    (bridge, task, pk)
}

async fn stop(bridge: Arc<Bridge>, task: tokio::task::JoinHandle<()>) {
    task.abort();
    let _ = task.await;
    bridge.session().shutdown().await;
    drop(bridge);
    tokio::time::sleep(Duration::from_millis(300)).await;
}

struct Requester {
    session: Session,
    acts: Arc<Mutex<Vec<acn_core::Event>>>,
    texts: Arc<Mutex<Vec<String>>>,
}

async fn requester(url: &str) -> Requester {
    let session = Session::connect(
        Keys::generate(),
        SessionConfig {
            relays: vec![url.to_string()],
            kind: IdentityKind::Human,
            display_name: None,
            about: None,
            announce: false,
        },
    )
    .await
    .unwrap();
    let (acts, texts) = (Arc::new(Mutex::new(vec![])), Arc::new(Mutex::new(vec![])));
    let mut inbox = session.inbox().await.unwrap();
    let (a, t) = (acts.clone(), texts.clone());
    tokio::spawn(async move {
        while let Some(m) = inbox.recv().await {
            match m.body {
                Body::Act(e) => a.lock().unwrap().push(*e),
                Body::Text(s) => t.lock().unwrap().push(s),
                _ => {}
            }
        }
    });
    Requester {
        session,
        acts,
        texts,
    }
}

impl Requester {
    async fn request(&self, agent: PublicKey, text: &str) -> String {
        let now = Timestamp::now().as_secs();
        let terms =
            acn_core::draft::tool_call_terms("echo", json!({"text": text}), now + 600, 3600, 0);
        let act = self
            .session
            .sign_act(
                &Draft::request(&agent.to_hex(), "T0", &terms, None).unwrap(),
                None,
            )
            .unwrap();
        self.session.send_act(agent, &act).await.unwrap();
        act.id
    }

    async fn wait<T>(&self, mut f: impl FnMut(&Self) -> Option<T>) -> Option<T> {
        for _ in 0..150 {
            if let Some(v) = f(self) {
                return Some(v);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    fn answered(&self, c: &str, kind: u16) -> bool {
        self.acts.lock().unwrap().iter().any(|e| {
            e.kind == kind
                && e.tags
                    .iter()
                    .any(|t| t.len() > 1 && t[0] == "c" && t[1] == c)
        })
    }
}

fn agent_key(dir: &Path) -> PublicKey {
    nmesh_core::load_or_create_keys(&dir.join("agent.nsec"))
        .unwrap()
        .public_key()
}

/// A01: a historical request from a sender outside `agent.allow`, or to a bridge with ACN off, is
/// not executed at startup.
#[tokio::test]
async fn startup_applies_allowlist_and_acn_switch() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let relay = MockRelay::run().await?;
    let url = relay.url().await.to_string();
    let r = requester(&url).await;
    for (name, acn, allow) in [
        (
            "not-allowed",
            AcnConfig::default(),
            vec![Keys::generate().public_key().to_hex()],
        ),
        (
            "acn-off",
            AcnConfig {
                enabled: false,
                ..Default::default()
            },
            vec![],
        ),
    ] {
        let dir = temp_dir(name);
        let agent = agent_key(&dir);
        // History: the request is on the relay before the bridge starts.
        let c = r.request(agent, "from history").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (bridge, task, _) = start(config(&url, &dir, acn, allow)).await;
        tokio::time::sleep(Duration::from_millis(4000)).await;
        assert_eq!(
            calls(&dir.join("calls.log")),
            0,
            "{name}: executed at startup"
        );
        assert!(!r.answered(&c, K_REPORT), "{name}: reported");
        stop(bridge, task).await;
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}

/// A02: after a restart, neither the ACN request nor the direct `/echo` command from history runs
/// again.
#[tokio::test]
async fn restart_does_not_repeat_work() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let relay = MockRelay::run().await?;
    let url = relay.url().await.to_string();
    let dir = temp_dir("restart");
    let log = dir.join("calls.log");
    let r = requester(&url).await;
    let (bridge, task, agent) = start(config(&url, &dir, AcnConfig::default(), vec![])).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let c = r.request(agent, "once").await;
    r.session.send_text(agent, "/echo direct once").await?;
    r.wait(|r| r.answered(&c, K_REPORT).then_some(()))
        .await
        .expect("REPORT");
    r.wait(|r| {
        r.texts
            .lock()
            .unwrap()
            .iter()
            .any(|t| t == "direct once")
            .then_some(())
    })
    .await
    .expect("direct reply");
    assert_eq!(calls(&log), 2);
    stop(bridge, task).await;

    let (bridge, task, _) = start(config(&url, &dir, AcnConfig::default(), vec![])).await;
    tokio::time::sleep(Duration::from_millis(5000)).await;
    assert_eq!(calls(&log), 2, "the restart executed history again");
    // Still serving live traffic.
    r.session.send_text(agent, "/echo live").await?;
    r.wait(|r| {
        r.texts
            .lock()
            .unwrap()
            .iter()
            .any(|t| t == "live")
            .then_some(())
    })
    .await
    .expect("live reply");
    assert_eq!(calls(&log), 3);
    stop(bridge, task).await;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// A priced bridge doesn't run its tools for free through `/command` DMs or form actions.
#[tokio::test]
async fn paid_tools_refuse_direct_commands() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let relay = MockRelay::run().await?;
    let url = relay.url().await.to_string();
    let mint = nmesh_mint::start(nmesh_mint::MintConfig {
        listen: "127.0.0.1:0".parse()?,
        url: None,
        db: None,
        mnemonic: None,
    })
    .await?;
    let dir = temp_dir("paid-direct");
    let r = requester(&url).await;
    let acn = AcnConfig {
        price_sat: 5,
        mint: Some(mint.url.clone()),
        ..Default::default()
    };
    let (bridge, task, agent) = start(config(&url, &dir, acn, vec![])).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    r.session.send_text(agent, "/echo for free").await?;
    let reply = r
        .wait(|r| r.texts.lock().unwrap().first().cloned())
        .await
        .expect("a reply");
    assert!(reply.contains("commitment"), "{reply}");
    assert_eq!(calls(&dir.join("calls.log")), 0, "ran a paid tool for free");
    stop(bridge, task).await;
    mint.stop();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
