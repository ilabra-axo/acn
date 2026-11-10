//! The reference stack behind the Binding API's interfaces: an `nmesh_core::Session` is the
//! [`Signer`] and [`Inbox`] (private acts over NIP-17, Core §5.5), `nmesh_wallet::CashuWallet` the
//! [`Wallet`], and Blossom the [`Evidence`]. [`run`] is the performer loop both reference
//! bindings use (the MCP shim and the CLI shim); [`announce`] publishes what a client needs to
//! know before the first request: the operator binding, the service advertisement and the
//! Blossom server list, all pinned so they survive a relay restart.

use std::sync::Arc;
use std::time::Duration;

use acn_binding::{BoxFut, Evidence, Inbox, Performer, ServiceAd, Signer, Verified, Wallet};
use acn_core::draft::Draft;
use acn_core::kinds::*;
use nmesh_core::{Body, InboxOptions, Keys, PublicKey, Session, Timestamp};

/// A session as Signer and Inbox.
#[derive(Clone)]
pub struct SessionBinding(pub Session);

impl Signer for SessionBinding {
    fn pubkey(&self) -> String {
        self.0.public_key().to_hex()
    }
    fn sign(&self, draft: &Draft, not_before: Option<u64>) -> Result<acn_core::Event, String> {
        self.0
            .sign_act(draft, not_before)
            .map_err(|e| e.to_string())
    }
    fn decrypt(&self, from: &str, ciphertext: &str) -> Result<String, String> {
        let pk = PublicKey::parse(from).map_err(|e| e.to_string())?;
        nostr_sdk::nips::nip44::decrypt(self.0.keys().secret_key(), &pk, ciphertext)
            .map_err(|e| e.to_string())
    }
}

impl Inbox for SessionBinding {
    fn send<'a>(&'a self, to: &'a str, act: &'a acn_core::Event) -> BoxFut<'a, Result<(), String>> {
        Box::pin(async move {
            let pk = PublicKey::parse(to).map_err(|e| e.to_string())?;
            self.0
                .send_act(pk, act)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    }
}

/// The CDK wallet at one mint, with that mint's declared capabilities.
pub struct MintWallet {
    pub wallet: nmesh_wallet::CashuWallet,
    pub capabilities: Vec<String>,
}

/// What the reference test mint (`nmesh-mint`, CDK) declares (Core §8.6).
pub const TEST_MINT_CAPABILITIES: &[&str] = &[
    "hashlock",
    "timelock_refund",
    "p2pk",
    "reveals_preimage_to_payer",
];

impl Wallet for MintWallet {
    fn mint(&self) -> String {
        self.wallet.mint_url().to_string()
    }
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    fn verify<'a>(&'a self, token: &'a str) -> BoxFut<'a, Result<Verified, String>> {
        Box::pin(async move { self.wallet.inspect(token).await.map_err(|e| e.to_string()) })
    }
    fn redeem<'a>(
        &'a self,
        token: &'a str,
        preimage: &'a str,
        commitment: &'a str,
    ) -> BoxFut<'a, Result<u64, String>> {
        Box::pin(async move {
            self.wallet
                .redeem(token, preimage, Some(commitment))
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// Blossom servers as Evidence: every artifact goes to all of them (Core §9: replicate).
pub struct BlossomEvidence(pub nmesh_blossom::BlossomClient);

impl Evidence for BlossomEvidence {
    fn put(&self, bytes: Vec<u8>, media_type: String) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move {
            self.0
                .upload(&bytes, &media_type)
                .await
                .map(|(h, _)| h)
                .map_err(|e| e.to_string())
        })
    }
    fn servers(&self) -> Vec<String> {
        self.0.servers.clone()
    }
}

/// Issue (if missing) and acknowledge an operator binding for the session's key (Core §4),
/// scoped to the performer's kinds and `max_sat`; pinned either way.
pub async fn ensure_binding(
    session: &Session,
    operator: &Keys,
    max_sat: u64,
    days: u64,
) -> anyhow::Result<()> {
    let me = session.public_key();
    let (_, existing) = session.acn_public(me, Duration::from_secs(3)).await?;
    let now = Timestamp::now().as_secs();
    let status = acn_core::binding_status(&existing, &me.to_hex(), K_REPORT, max_sat, None, now);
    if matches!(status, acn_core::BindingStatus::Bound { .. }) {
        let binding: Vec<_> = existing
            .into_iter()
            .filter(|e| matches!(e.kind, K_BIND | K_BIND_ACK | K_BIND_REVOKE))
            .collect();
        session.pin_acts(&binding);
        return Ok(());
    }
    let kinds = [
        K_COUNTER, K_ACCEPT, K_PROMISE, K_DECLINE, K_REVOKE, K_PROGRESS, K_REPORT, K_DISPUTE,
    ]
    .map(|k| k.to_string())
    .join(",");
    let max = max_sat.to_string();
    let aux = Keys::generate().secret_key().secret_bytes();
    let bind = Draft::bind(
        &me.to_hex(),
        now + days * 24 * 60 * 60,
        &[("kinds", kinds.as_str()), ("max_sat", max.as_str())],
    )
    .sign_with_aux(&operator.secret_key().secret_bytes(), now, &aux)?;
    session.publish_act_pinned(&bind).await?;
    let ack = session.sign_act(&Draft::bind_ack(&bind.id), Some(now))?;
    session.publish_act_pinned(&ack).await?;
    tracing::info!(operator = %operator.public_key().to_hex(), "binding: operator binding published");
    Ok(())
}

/// Publish the service advertisement (`K_SVC`, `d` = `id`) and, if the binding stores evidence,
/// its Blossom server list (BUD-03, kind 10063). Both pinned.
pub async fn announce(session: &Session, id: &str, ad: &ServiceAd) -> anyhow::Result<()> {
    let d = Draft::new(K_SVC)
        .tag(vec!["d".into(), id.into()])
        .tag(vec!["alt".into(), "ACN service advertisement".into()])
        .content(ad.to_json());
    let ev = session.sign_act(&d, None)?;
    session.publish_act_pinned(&ev).await?;
    if !ad.blossom.is_empty() {
        session
            .publish_pinned_builder(nmesh_blossom::server_list(&ad.blossom))
            .await?;
    }
    Ok(())
}

/// Serve as a performer until the inbox closes: replay history first (so a request already
/// answered isn't run twice), then act on every inbound ACN act. `other` gets everything else
/// (chat messages, actions), if the host has a use for it.
pub async fn run(
    session: &Session,
    performer: Arc<Performer>,
    mut other: impl FnMut(nmesh_core::Inbound),
) -> anyhow::Result<()> {
    let me = session.public_key();
    let mut inbox = session
        .inbox_with(InboxOptions {
            include_own: true,
            ..Default::default()
        })
        .await?;
    let settle = Duration::from_millis(1500);
    let mut early = Vec::new();
    while let Ok(Some(m)) = tokio::time::timeout(settle, inbox.recv()).await {
        match m.body {
            Body::Act(e) => {
                performer.ingest(*e);
            }
            _ if m.sender != me => early.push(m),
            _ => {}
        }
    }
    for c in performer.store.ids() {
        performer.drive(&c).await;
    }
    for m in early {
        other(m);
    }
    while let Some(m) = inbox.recv().await {
        match m.body {
            Body::Act(e) if m.sender == me => {
                performer.ingest(*e);
            }
            Body::Act(e) => {
                let p = performer.clone();
                tokio::spawn(async move {
                    p.on_event(*e).await;
                });
            }
            _ if m.sender != me => other(m),
            _ => {}
        }
    }
    Ok(())
}
