//! The reference stack behind the Binding API's interfaces: an `nmesh_core::Session` is the
//! [`Signer`] and [`Inbox`] (private acts over NIP-17, Core §5.5), `nmesh_wallet::CashuWallet` the
//! [`Wallet`], and Blossom the [`Evidence`]. [`run`] is the performer loop both reference
//! bindings use (the MCP shim and the CLI shim); [`announce`] publishes what a client needs to
//! know before the first request: the operator binding, the service advertisement and the
//! Blossom server list, all pinned so they survive a relay restart.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
    /// `sha256("acn-binding derived secret" ‖ secret key ‖ label)`: a different value per
    /// label, recoverable after a restart from the key alone, and revealing nothing about it.
    fn derive_secret(&self, label: &str) -> Result<[u8; 32], String> {
        use sha2::{Digest, Sha256};
        Ok(Sha256::new()
            .chain_update(b"acn-binding derived secret")
            .chain_update(self.0.keys().secret_key().secret_bytes())
            .chain_update(label.as_bytes())
            .finalize()
            .into())
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
    fn fetch<'a>(
        &'a self,
        hash: &'a str,
        hints: &'a [String],
    ) -> BoxFut<'a, Result<Vec<u8>, String>> {
        Box::pin(async move {
            let servers: Vec<String> = hints
                .iter()
                .cloned()
                .chain(self.0.servers.clone())
                .collect();
            // Hints come from the sender: only our own servers may be non-public addresses.
            let policy = nmesh_blossom::FetchPolicy::trusting(&self.0.servers);
            nmesh_blossom::fetch_from(&servers, hash, &policy)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// Issue (if missing) and acknowledge an operator binding for the session's key (Core §4),
/// scoped to the performer's kinds and `max_sat`; pinned either way. Returns the binding events
/// (bindings, acknowledgements, revocations): the authority context the performer folds with
/// every commitment ([`Performer::add_context`]).
///
/// A binding the operator revoked is not silently replaced: the revocation is returned as
/// context, so the performer stops signing, and an operator must issue a new binding on purpose.
pub async fn ensure_binding(
    session: &Session,
    operator: &Keys,
    max_sat: u64,
    days: u64,
) -> anyhow::Result<Vec<acn_core::Event>> {
    let me = session.public_key();
    let (_, existing) = session.acn_public(me, Duration::from_secs(3)).await?;
    let now = Timestamp::now().as_secs();
    let status = acn_core::binding_status(&existing, &me.to_hex(), K_REPORT, max_sat, None, now);
    let binding: Vec<_> = existing
        .into_iter()
        .filter(|e| matches!(e.kind, K_BIND | K_BIND_ACK | K_BIND_REVOKE))
        .collect();
    match status {
        acn_core::BindingStatus::Bound { .. } => {
            session.pin_acts(&binding);
            return Ok(binding);
        }
        acn_core::BindingStatus::Revoked { binding: id } => {
            tracing::warn!(binding = %id, "binding: the operator revoked this agent's binding; not re-issuing it");
            session.pin_acts(&binding);
            return Ok(binding);
        }
        _ => {}
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
    Ok(vec![bind, ack])
}

/// Fetch the agent's binding events (and any revocation) from the relays into the performer's
/// authority context.
pub async fn refresh_context(session: &Session, performer: &Performer) {
    match session
        .acn_public(session.public_key(), Duration::from_secs(3))
        .await
    {
        Ok((_, events)) => performer.add_context(events),
        Err(e) => tracing::warn!(error = %e, "binding: could not refresh the operator binding"),
    }
}

/// The default execution journal location: next to the agent's key file
/// (`agent.nsec` → `agent.acn-journal.json`).
pub fn default_journal_path(key_file: &Path) -> PathBuf {
    key_file.with_extension("acn-journal.json")
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

/// How [`run_with`] admits and replays.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Transport senders whose acts and messages are handled at all; `None` admits anyone. The
    /// same rule applies to history and live traffic. (Hosts should also give the performer the
    /// matching [`Performer::with_admission`] for the requester key inside the acts.)
    pub allow: Option<HashSet<PublicKey>>,
    /// Act on ACN acts. When off, acts are ignored entirely, history included.
    pub acn: bool,
    /// A plain message created more than this before startup is history: never handed to
    /// `other`. Newer ones are handed over at most once (by rumor id, in the journal).
    pub fresh_within: Duration,
    /// How often to re-send undelivered acts and retry due redemptions.
    pub retry_every: Duration,
    /// How often to re-fetch the operator binding (to see revocations).
    pub context_every: Duration,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            allow: None,
            acn: true,
            fresh_within: Duration::from_secs(120),
            retry_every: Duration::from_secs(5),
            context_every: Duration::from_secs(60),
        }
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// [`run_with`] with the default options: anyone admitted, ACN on.
pub async fn run(
    session: &Session,
    performer: Arc<Performer>,
    other: impl FnMut(nmesh_core::Inbound),
) -> anyhow::Result<()> {
    run_with(session, performer, RunOptions::default(), other).await
}

/// Serve as a performer until the inbox closes. History is replayed into the store first (terms
/// sent by reference are fetched), then every admitted commitment is driven, then live ACN acts
/// are acted on as they arrive. `other` gets admitted plain messages (chat, actions) that are
/// not history, each at most once.
///
/// Repeated work is prevented by the performer's journal, not by the 1.5 s pause that ends the
/// replay (which only saves a round of re-folding when history is quick to arrive).
pub async fn run_with(
    session: &Session,
    performer: Arc<Performer>,
    opts: RunOptions,
    mut other: impl FnMut(nmesh_core::Inbound),
) -> anyhow::Result<()> {
    let me = session.public_key();
    let started = Timestamp::now().as_secs();
    let mut inbox = session
        .inbox_with(InboxOptions {
            include_own: true,
            ..Default::default()
        })
        .await?;
    let admitted = |pk: &PublicKey| opts.allow.as_ref().is_none_or(|a| a.contains(pk));
    let fresh = |m: &nmesh_core::Inbound| {
        let at = m.created_at.as_secs();
        if at.saturating_add(opts.fresh_within.as_secs()) < started {
            tracing::debug!(id = %m.id, "binding: historical message not replayed");
            return false;
        }
        match performer
            .journal()
            .first_seen(&m.id.to_hex(), at, Timestamp::now().as_secs())
        {
            Ok(first) => first,
            Err(e) => {
                tracing::warn!(error = %e, "binding: message not handled (journal unavailable)");
                false
            }
        }
    };
    if opts.acn {
        refresh_context(session, &performer).await;
    }
    let settle = Duration::from_millis(1500);
    let mut early = Vec::new();
    while let Ok(Some(m)) = tokio::time::timeout(settle, inbox.recv()).await {
        match m.body {
            Body::Act(e) if opts.acn && (m.sender == me || admitted(&m.sender)) => {
                performer.ingest(*e).await;
            }
            Body::Act(_) => {}
            _ if m.sender != me && admitted(&m.sender) => early.push(m),
            _ => {}
        }
    }
    let _maintenance = opts.acn.then(|| {
        let (s, p) = (session.clone(), performer.clone());
        let (retry_every, context_every) = (opts.retry_every, opts.context_every);
        AbortOnDrop(tokio::spawn(async move {
            let mut retry = tokio::time::interval(retry_every);
            let mut context = tokio::time::interval(context_every);
            context.tick().await;
            loop {
                tokio::select! {
                    _ = retry.tick() => { p.retry_due().await; }
                    _ = context.tick() => refresh_context(&s, &p).await,
                }
            }
        }))
    });
    if opts.acn {
        performer.drive_all().await;
    }
    for m in early {
        if fresh(&m) {
            other(m);
        }
    }
    while let Some(m) = inbox.recv().await {
        match m.body {
            Body::Act(_) if !opts.acn => {}
            Body::Act(e) if m.sender == me => {
                performer.ingest(*e).await;
            }
            Body::Act(e) if admitted(&m.sender) => {
                let p = performer.clone();
                tokio::spawn(async move {
                    p.on_event(*e).await;
                });
            }
            Body::Act(_) => {
                tracing::debug!(sender = %m.sender, "binding: act from a sender not allowed");
            }
            _ if m.sender != me && admitted(&m.sender) && fresh(&m) => other(m),
            _ => {}
        }
    }
    Ok(())
}
