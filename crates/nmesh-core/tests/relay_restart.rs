//! A restarted in-memory relay has lost everything; a session republishes what it pinned
//! (profile, inbox relays, service advertisements, bindings, templates) verbatim when the relay
//! comes back, so ids stay the same.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use nmesh_core::{IdentityKind, Keys, Session, SessionConfig};
use nostr_relay_builder::prelude::*;
use nostr_sdk::Client;

fn relay_on(port: u16) -> LocalRelay {
    LocalRelay::new(
        RelayBuilder::default()
            .addr(IpAddr::V4(Ipv4Addr::LOCALHOST))
            .port(port),
    )
}

async fn fetch(url: &str, filter: Filter) -> anyhow::Result<Events> {
    let c = Client::default();
    c.add_relay(url).await?;
    c.connect().await;
    c.wait_for_connection(Duration::from_secs(5)).await;
    let ev = c.fetch_events(filter, Duration::from_secs(3)).await?;
    c.shutdown().await;
    Ok(ev)
}

#[tokio::test]
async fn pinned_events_survive_a_relay_restart() -> anyhow::Result<()> {
    let port = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    let url = format!("ws://127.0.0.1:{port}");
    let first = relay_on(port);
    first.run().await?;

    let s = Session::connect(
        Keys::generate(),
        SessionConfig {
            relays: vec![url.clone()],
            kind: IdentityKind::Agent,
            display_name: Some("Atlas".into()),
            about: None,
            announce: true,
        },
    )
    .await?;
    let pinned = s
        .publish_pinned_builder(EventBuilder::new(Kind::Custom(7727), "{\"name\":\"t\"}"))
        .await?;
    // Not pinned: a restart loses it.
    let plain = s
        .client()
        .send_event_builder(EventBuilder::new(Kind::Custom(7727), "plain"))
        .await?;
    let me = s.public_key();
    let mine = || Filter::new().author(me);
    assert_eq!(
        fetch(&url, mine()).await?.len(),
        4,
        "profile, inbox relays, two events"
    );

    first.shutdown();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let second = relay_on(port);
    second.run().await?;
    assert!(
        fetch(&url, mine()).await?.is_empty(),
        "the new relay starts empty"
    );

    // The pool retries every ~10 s (plus jitter, growing after failures).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let got = loop {
        let got = fetch(&url, mine()).await?;
        if got.len() >= 3 || tokio::time::Instant::now() > deadline {
            break got;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    let ids: Vec<EventId> = got.iter().map(|e| e.id).collect();
    assert!(
        ids.contains(&pinned),
        "the pinned event is back, with its id"
    );
    assert!(!ids.contains(&plain), "only pinned events are republished");
    assert!(
        got.iter().any(|e| e.kind == Kind::Metadata),
        "the profile is back"
    );
    assert_eq!(got.len(), 3);
    s.shutdown().await;
    second.shutdown();
    Ok(())
}
