//! The reference bindings pass the L0 suite: a real relay, a real (fake-Lightning) CDK mint and
//! a Blossom server, all local. `acn-mcp-shim` (the agent bridge with the echo MCP server) runs
//! priced, so T1 is exercised, and free, so T0 is.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use acn_conformance::{l0, Env, Executions, Target};
use nmesh_agent_bridge::config::{
    AcnConfig, AgentConfig, AgentSection, Disclosure, McpServerConfig, McpTransport,
};
use nmesh_agent_bridge::Bridge;
use nmesh_proto::manifest::Hosting;
use nostr_relay_builder::prelude::*;
use serde_json::json;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("acn-l0-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The echo MCP server binary from `nmesh-agent-bridge` (built here if this test runs first).
fn echo_mcp() -> String {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let bin = dir.join("nmesh-echo-mcp");
    if !bin.exists() {
        let ok = std::process::Command::new(env!("CARGO"))
            .args([
                "build",
                "-q",
                "-p",
                "nmesh-agent-bridge",
                "--bin",
                "nmesh-echo-mcp",
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "building nmesh-echo-mcp");
    }
    bin.to_string_lossy().into_owned()
}

struct Infra {
    relay: LocalRelay,
    url: String,
    mint: nmesh_mint::RunningMint,
    blossom: nmesh_blossom::RunningServer,
}

async fn infra() -> anyhow::Result<Infra> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let relay = LocalRelay::new(RelayBuilder::default().rate_limit(RateLimit {
        max_reqs: 500,
        notes_per_minute: 6_000,
    }));
    relay.run().await?;
    let url = relay.url().await.to_string();
    let mint = nmesh_mint::start(nmesh_mint::MintConfig {
        listen: "127.0.0.1:0".parse()?,
        url: None,
        db: None,
        mnemonic: None,
    })
    .await?;
    let blossom = nmesh_blossom::serve("127.0.0.1:0".parse()?, None).await?;
    Ok(Infra {
        relay,
        url,
        mint,
        blossom,
    })
}

/// Every MCP tool call the echo server makes is a line here.
fn call_log(dir: &Path) -> PathBuf {
    dir.join("calls.log")
}

fn executions(dir: &Path) -> Executions {
    let log = call_log(dir);
    Executions(Arc::new(move || {
        std::fs::read_to_string(&log)
            .map(|s| s.lines().count() as u64)
            .unwrap_or(0)
    }))
}

async fn shim(i: &Infra, dir: &std::path::Path, price_sat: u64) -> anyhow::Result<String> {
    let bridge = Bridge::start(AgentConfig {
        agent: AgentSection {
            id: "echo".into(),
            name: "Echo agent".into(),
            about: "conformance".into(),
            key_file: dir.join("agent.nsec"),
            hosting: Hosting::SelfHosted,
            operator: "tests".into(),
            retention: "none".into(),
            forwards_to: vec![],
            allow: vec![],
        },
        relays: vec![i.url.clone()],
        servers: vec![McpServerConfig {
            name: "echo".into(),
            transport: McpTransport::Stdio {
                command: echo_mcp(),
                args: vec![],
                env: [(
                    "NMESH_ECHO_CALL_LOG".to_string(),
                    call_log(dir).to_string_lossy().into_owned(),
                )]
                .into(),
            },
        }],
        acn: AcnConfig {
            operator_key_file: Some(dir.join("operator.nsec")),
            price_sat,
            mint: (price_sat > 0).then(|| i.mint.url.clone()),
            blossom: vec![i.blossom.url.clone()],
            // L0 publishes evidence: opt in explicitly (the default is private).
            evidence: Disclosure::Public,
            ..Default::default()
        },
    })
    .await?;
    let pk = bridge.session().public_key().to_hex();
    tokio::spawn(async move { bridge.run().await });
    tokio::time::sleep(Duration::from_millis(2000)).await;
    Ok(pk)
}

#[tokio::test]
async fn mcp_shim_passes_l0_priced_and_free() -> anyhow::Result<()> {
    let i = infra().await?;
    let env = Env {
        relays: vec![i.url.clone()],
        mint: Some(i.mint.url.clone()),
        blossom: Some(i.blossom.url.clone()),
    };
    for (name, price) in [("priced", 5), ("free", 0)] {
        let dir = temp_dir(name);
        let agent = shim(&i, &dir, price).await?;
        let report = l0(
            &env,
            &Target {
                agent,
                tool: "echo".into(),
                input: json!({"text": "conformance"}),
                executions: Some(executions(&dir)),
            },
        )
        .await?;
        println!("{}", report.summary());
        assert!(report.passed(), "{name}:\n{}", report.summary());
        let names: Vec<&str> = report.cases.iter().map(|c| c.name.as_str()).collect();
        // The execution counter was observed, not skipped.
        assert!(
            report
                .cases
                .iter()
                .filter(|c| c.name.starts_with("decline-"))
                .all(|c| c.detail.contains("0 execution(s)")),
            "{}",
            report.summary()
        );
        if price > 0 {
            assert!(
                names.contains(&"t1-paid-call")
                    && names.contains(&"t1-terms-by-reference")
                    && names.contains(&"free-call-below-price-declined"),
                "{names:?}"
            );
        } else {
            assert!(names.contains(&"t0-call"), "{names:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    i.blossom.stop();
    i.mint.stop();
    i.relay.shutdown();
    Ok(())
}
