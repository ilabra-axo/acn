//! acn-core's pure-Rust event code must agree with rust-nostr in both directions.

use acn_core::draft::Draft;
use acn_core::kinds::K_REQUEST;
use acn_core::Event;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, SecretKey, Tag};

#[test]
fn verifies_events_signed_by_rust_nostr() {
    let keys = Keys::generate();
    let d = Draft::request(
        &"aa".repeat(32),
        "T0",
        &serde_json::json!({"v": "acn-terms/0.3"}),
        None,
    )
    .unwrap();
    let tags: Vec<Tag> = d
        .tags
        .iter()
        .map(|t| Tag::parse(t.clone()).unwrap())
        .collect();
    let ev = EventBuilder::new(Kind::Custom(K_REQUEST), &d.content)
        .tags(tags)
        .sign_with_keys(&keys)
        .unwrap();
    let ours = Event::from_json(&ev.as_json()).unwrap();
    ours.verify()
        .expect("acn-core verifies rust-nostr signature");
}

#[test]
fn rust_nostr_verifies_our_events() {
    let secret = [3u8; 32];
    let d = Draft::new(K_REQUEST).content(serde_json::json!({"note": "héllo \"quoted\"\n"}));
    let ours = d.sign(&secret, 1_700_000_000).unwrap();
    let theirs = nostr::Event::from_json(ours.to_json()).unwrap();
    theirs
        .verify()
        .expect("rust-nostr verifies acn-core signature");
    let sk = SecretKey::from_slice(&secret).unwrap();
    assert_eq!(theirs.pubkey, Keys::new(sk).public_key());
}
