//! End-to-end: nmesh-app (what the desktop UI drives) ↔ mock relay ↔ agent bridge ↔ echo MCP.

use std::path::PathBuf;
use std::time::Duration;

use acn_core::Phase;
use nmesh_agent_bridge::config::{
    AcnConfig, AgentConfig, AgentSection, McpServerConfig, McpTransport,
};
use nmesh_agent_bridge::Bridge;
use nmesh_app::{App, AppConfig, AppEvent, BodyView, CommitmentView, MessageView, RendererView};
use nmesh_proto::manifest::Hosting;
use nostr_relay_builder::MockRelay;
use serde_json::json;
use tokio::sync::mpsc::Receiver;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nmesh-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Wait for the next message event matching `pred`.
async fn next_message(
    rx: &mut Receiver<AppEvent>,
    pred: impl Fn(&MessageView) -> bool,
) -> MessageView {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match rx.recv().await.expect("events closed") {
                AppEvent::Message { message } if pred(&message) => return message,
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for message")
}

/// Wait for a commitment update matching `pred`.
async fn next_commitment(
    rx: &mut Receiver<AppEvent>,
    pred: impl Fn(&CommitmentView) -> bool,
) -> CommitmentView {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match rx.recv().await.expect("events closed") {
                AppEvent::Commitment { commitment } if pred(&commitment) => return commitment,
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for commitment")
}

#[tokio::test]
async fn desktop_app_talks_to_mcp_agent() -> anyhow::Result<()> {
    let relay = MockRelay::run().await?;
    let url = relay.url().await.to_string();
    let dir = temp_dir("agent-flow");

    let bridge = Bridge::start(AgentConfig {
        agent: AgentSection {
            id: "echo".into(),
            name: "Echo agent".into(),
            about: "test".into(),
            key_file: dir.join("agent.nsec"),
            hosting: Hosting::SelfHosted,
            operator: "tests".into(),
            retention: "none".into(),
            forwards_to: vec![],
            allow: vec![],
        },
        relays: vec![url.clone()],
        servers: vec![McpServerConfig {
            name: "echo".into(),
            transport: McpTransport::Stdio {
                command: env!("CARGO_BIN_EXE_nmesh-echo-mcp").to_string(),
                args: vec![],
                env: Default::default(),
            },
        }],
        acn: AcnConfig {
            operator_key_file: Some(dir.join("operator.nsec")),
            ..Default::default()
        },
    })
    .await?;

    let agent_pk = bridge.session().public_key().to_hex();
    tokio::spawn(async move { bridge.run().await });

    let (app, mut events) = App::start(AppConfig::new(vec![url], dir.join("app"))).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Discovery: profile + manifest + slash commands.

    let contact = app.add_contact(&agent_pk).await?;

    assert!(contact.is_agent);
    assert_eq!(contact.agent.as_ref().unwrap().hosting, Hosting::SelfHosted);
    assert!(contact.agent.as_ref().unwrap().data_handling.sees_plaintext);
    let cmds: Vec<_> = contact
        .commands
        .iter()
        .map(|c| c.command.as_str())
        .collect();
    assert_eq!(cmds, ["card", "echo", "form", "status"]);

    // Consent gate: can't message an agent before accepting its data handling.
    assert!(!contact.consented);
    assert!(app.send_text(&agent_pk, "/echo hi").await.is_err());
    app.accept_agent(&agent_pk).await?;

    // Text tool.
    app.send_text(&agent_pk, "/echo hello there").await?;
    let m = next_message(&mut events, |m| !m.mine).await;
    assert!(matches!(&m.body, BodyView::Text { text } if text == "hello there"));

    // Structured tool → validated card with an installed renderer.
    app.send_text(&agent_pk, "/form").await?;

    let form = next_message(&mut events, |m| !m.mine).await;
    let BodyView::Structured {
        contract,
        valid,
        renderer,
        ..
    } = &form.body
    else {
        panic!("expected structured, got {:?}", form.body);
    };
    assert_eq!(contract, "nmesh.form/v1");
    assert!(valid);
    assert!(matches!(renderer, RendererView::Installed { id } if id == "nmesh.form"));

    // Form submit → action → agent routes to /card with named args.
    app.send_action(
        &agent_pk,
        &form.id,
        Some("/card".into()),
        Some(json!({"title": "From a form", "body": "submitted in the UI"})),
        Some("Create card".into()),
    )
    .await?;
    let card = next_message(&mut events, |m| !m.mine).await;

    let BodyView::Structured {
        contract, payload, ..
    } = &card.body
    else {
        panic!("expected card, got {:?}", card.body);
    };
    assert_eq!(contract, "nmesh.card/v1");
    assert_eq!(payload["title"], "From a form");

    // History holds our sends + replies exactly once (self-copies de-duplicated).
    tokio::time::sleep(Duration::from_millis(500)).await;
    let history = app.open(&agent_pk).await?;
    assert_eq!(history.iter().filter(|m| m.mine).count(), 3);
    assert_eq!(history.iter().filter(|m| !m.mine).count(), 3);

    // --- ACN (docs/Core.md): the agent advertises K_SVC and is bound to an operator key. ---
    let acn = contact.acn.clone().expect("agent offers ACN");
    assert_eq!(acn.profiles, ["T0"]);
    assert!(acn.binding.bound, "binding: {}", acn.binding.detail);

    // T0 request → signed REPORT (table contract) → SATISFY.
    let c = app.request(&agent_pk, "/status").await?;
    assert_eq!(c.phase, Some(Phase::Requested));
    assert_eq!(c.tool.as_deref(), Some("status"));
    let reported = next_commitment(&mut events, |v| {
        v.id == c.id && v.phase == Some(Phase::Reported)
    })
    .await;
    assert!(reported.awaiting_me && reported.allowed.satisfy);
    assert!(reported.performer_binding.bound);
    assert!(reported.anomalies.is_empty(), "{:?}", reported.anomalies);
    let Some(BodyView::Structured {
        contract, valid, ..
    }) = &reported.result
    else {
        panic!("expected structured result, got {:?}", reported.result);
    };
    assert_eq!((contract.as_str(), *valid), ("nmesh.table/v1", true));
    let done = app.satisfy(&agent_pk, &c.id).await?;
    assert_eq!(done.phase, Some(Phase::Satisfied));
    assert!(done.terminal);
    let acts: Vec<_> = done.chain.iter().map(|a| a.act).collect();
    assert_eq!(acts, ["REQUEST", "REPORT", "SATISFY"]);

    // Positional args bound via the NIP-CD definition; REJECT closes a T0 as Rejected.
    let c2 = app.request(&agent_pk, "/echo hello acn").await?;
    let r2 = next_commitment(&mut events, |v| {
        v.id == c2.id && v.phase == Some(Phase::Reported)
    })
    .await;
    assert!(matches!(&r2.result, Some(BodyView::Text { text }) if text == "hello acn"));
    let rejected = app.reject(&agent_pk, &c2.id, None, "wrong words").await?;
    assert_eq!(rejected.phase, Some(Phase::Rejected));

    // The thread shows one entry per commitment, not per act.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let thread = app.open(&agent_pk).await?;
    let entries = thread
        .iter()
        .filter(|m| matches!(m.body, BodyView::Commitment { .. }))
        .count();
    assert_eq!(entries, 2);
    assert_eq!(app.commitments(&agent_pk).await?.len(), 2);

    app.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
