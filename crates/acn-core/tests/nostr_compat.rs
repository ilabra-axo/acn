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
        &serde_json::json!({"v": "acn-terms/0.5"}),
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

/// Sealed deliverables interoperate with rust-nostr's NIP-44 v2, both ways.
#[test]
fn sealed_payloads_are_nip44_v2() {
    use base64::Engine as _;
    use nostr::nips::nip44::v2::{decrypt_to_bytes, encrypt_to_bytes, ConversationKey};
    let k = [0x6bu8; 32];
    let text = "A 500-word technical summary. ".repeat(20);
    let ours = acn_core::sealed::seal(&k, &[3; 32], text.as_bytes()).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(&ours).unwrap();
    assert_eq!(decrypt_to_bytes(&ConversationKey::new(k), &bytes).unwrap(), text.as_bytes());
    let theirs = encrypt_to_bytes(&ConversationKey::new(k), text.as_bytes()).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(theirs);
    assert_eq!(acn_core::sealed::open(&k, &b64).unwrap(), text.as_bytes());
}
