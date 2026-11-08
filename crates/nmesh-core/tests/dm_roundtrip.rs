use std::time::Duration;

use nmesh_contracts::{builtin, ContractRegistry};
use nmesh_core::{Body, Keys, Session, SessionConfig};
use nmesh_proto::StructuredMessage;
use nostr_relay_builder::MockRelay;
use serde_json::json;

async fn recv(rx: &mut tokio::sync::mpsc::Receiver<nmesh_core::Inbound>) -> nmesh_core::Inbound {
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("timed out waiting for message")
        .expect("inbox closed")
}

#[tokio::test]
async fn text_and_structured_dm_roundtrip() -> anyhow::Result<()> {
    let relay = MockRelay::run().await?;
    let url = relay.url().await.to_string();
    let cfg = || SessionConfig {
        relays: vec![url.clone()],
        ..Default::default()
    };

    let alice = Session::connect(Keys::generate(), cfg()).await?;
    let bob = Session::connect(Keys::generate(), cfg()).await?;
    let mut bob_inbox = bob.inbox().await?;
    // Give the subscription a moment to register on the relay.
    tokio::time::sleep(Duration::from_millis(200)).await;

    alice.send_text(bob.public_key(), "hello bob").await?;
    let m = recv(&mut bob_inbox).await;
    assert_eq!(m.sender, alice.public_key());
    assert_eq!(m.body, Body::Text("hello bob".into()));

    let registry = ContractRegistry::with_builtins();
    let card = StructuredMessage::new(
        registry.pinned_ref(builtin::CARD).unwrap(),
        json!({"title": "Status", "body": "All good"}),
        "Status: All good",
    );
    registry.validate_message(&card)?;
    alice.send_structured(bob.public_key(), &card).await?;

    let m = recv(&mut bob_inbox).await;
    let Body::Structured(got) = m.body else {
        panic!("expected structured, got {:?}", m.body);
    };
    assert_eq!(got, card);
    registry.validate_message(&got)?;

    alice.shutdown().await;
    bob.shutdown().await;
    Ok(())
}
