//! Count the public events per author and kind on a relay: is the cast (or an agent) visible?
//! `cargo run -p nmesh-core --example relay_dump -- ws://127.0.0.1:7777`
use nostr_sdk::prelude::*;
use std::collections::BTreeMap;
use std::time::Duration;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or("ws://127.0.0.1:7777".into());
    let c = Client::default();
    c.add_relay(&url).await?;
    c.connect().await;
    c.wait_for_connection(Duration::from_secs(5)).await;
    let ev = c
        .fetch_events(
            Filter::new().kinds([
                Kind::Metadata,
                Kind::Custom(10050),
                Kind::Custom(37700),
                Kind::Custom(7727),
                Kind::Custom(7730),
                Kind::Custom(7731),
                Kind::Custom(31992),
                Kind::Custom(34932),
            ]),
            Duration::from_secs(3),
        )
        .await?;
    let mut m: BTreeMap<(String, u16), usize> = BTreeMap::new();
    for e in ev.iter() {
        *m.entry((e.pubkey.to_bech32()?[..14].to_string(), e.kind.as_u16()))
            .or_default() += 1;
    }
    for ((a, k), n) in m {
        println!("{a} kind {k}: {n}");
    }
    Ok(())
}
