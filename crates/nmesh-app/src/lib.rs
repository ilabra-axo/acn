//! nmesh application layer.
//!
//! Owns a [`Session`] plus the state a chat UI needs (contacts, per-conversation history,
//! unread counts, agent consent) and exposes it as serializable view models. Every structured
//! message is validated and assigned a renderer *here*, so all UIs make the same decision.
//!
//! The Tauri shell (`apps/desktop/src-tauri`) is a thin wrapper around this crate.
//!
//! ACN commitments (docs/Core.md): signed acts are kept per conversation and every
//! [`CommitmentView`] is a projection of `acn_core::fold` (INV-12). The app only signs acts
//! the user asked for; legality is always decided by the validator.
//!
//! Wallet (docs/Core.md §8): a CDK Cashu wallet per mint, keyed by the user's Nostr key. Priced
//! hash-lock templates lock the price to the performer before the REQUEST is signed; the token
//! travels NIP-44-encrypted in `content.token`. Refunds after the locktime are user-initiated.

mod commitments;
mod store;
mod view;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nmesh_contracts::{builtin, ContractRegistry, TrustPolicy};
use nmesh_core::{
    Body, EventId, IdentityKind, Inbound, InboxOptions, PublicKey, Session, SessionConfig,
    Timestamp, ToBech32,
};
use nmesh_proto::manifest::RendererManifest;
use nmesh_proto::{Action, ContractRef};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};

use std::sync::atomic::{AtomicI64, Ordering};

use acn_core::draft::{Draft, Template};
use acn_core::kinds::*;
use acn_sim::{Hint, HintAction};
use nmesh_core::Keys;
use nmesh_proto::command::{bind_args, parse_invocation, SlashCommandDefinition};

pub use acn_sim::{Hint as AcnHint, HintAction as AcnHintAction};
/// The ACN wire version this app speaks (shown in the UI).
pub const ACN_VERSION: &str = acn_core::kinds::VERSION;
pub use acn_wallet::WalletView;
pub use commitments::{BindingView, CommitmentView};
use nmesh_wallet::CashuWallet;
pub use view::{
    AcnPeerView, AcnToolView, ArgView, BodyView, CommandView, ContactView, Discovery, Identity,
    MessageView, RenderContext, RendererView,
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Core(#[from] nmesh_core::CoreError),
    #[error("invalid public key: {0}")]
    BadKey(String),
    #[error("{0}")]
    Invalid(String),
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub relays: Vec<String>,
    pub key_file: PathBuf,
    pub data_dir: PathBuf,
    pub display_name: Option<String>,
    /// How much DM history to fetch from relays on start.
    pub history: Duration,
    /// Renderer target this UI implements (`web`, `android`, `ar`, `tui`).
    pub target: String,
    /// Cashu mint of the built-in wallet. `None`: no wallet; priced requests fail.
    pub mint: Option<String>,
    /// Which settlement triggers this requester accepts (local policy, not protocol).
    pub settlement_policy: SettlementPolicy,
    /// Blossom servers for terms too large to inline (Core §7), and to fetch referenced terms
    /// from besides the signer's own servers.
    pub blossom: Vec<String>,
}

/// Requester-side settlement policy (Binding API policy filters, docs/binding-api.md). Local
/// configuration: the protocol only checks that the terms are a valid combination.
#[derive(Debug, Clone, Default)]
pub struct SettlementPolicy {
    /// Refuse terms that hand the release to a settlement verifier.
    pub refuse_verifier: bool,
    /// Refuse contingent settlement (pay against an attested, encrypted delivery).
    pub refuse_contingent: bool,
    /// When non-empty, accept the verifier and contingent triggers only with one of these
    /// settlement verifiers.
    pub trusted_verifiers: Vec<String>,
}

/// Does a `K_SVC` declare `cap` for `mint` (Core.md §8.6)? Entries are `{url, capabilities}`.
pub fn mint_has_capability(svc: &Value, mint: &str, cap: &str) -> bool {
    svc["mints"].as_array().into_iter().flatten().any(|m| {
        m["url"].as_str().map(|u| u.trim_end_matches('/')) == Some(mint.trim_end_matches('/'))
            && m["capabilities"]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x == cap))
    })
}

impl SettlementPolicy {
    fn check(&self, terms: &Value) -> Result<(), AppError> {
        let trigger = terms["settlement_trigger"].as_str().unwrap_or("requester");
        if trigger == "requester" {
            return Ok(());
        }
        if (trigger == "verifier" && self.refuse_verifier)
            || (trigger == "contingent" && self.refuse_contingent)
        {
            return Err(AppError::Invalid(format!(
                "your settlement policy refuses the {trigger} trigger"
            )));
        }
        let v = terms["settlement_verifier"].as_str().unwrap_or_default();
        if !self.trusted_verifiers.is_empty() && !self.trusted_verifiers.iter().any(|t| t == v) {
            return Err(AppError::Invalid(format!(
                "settlement verifier {v} is not in your trusted set"
            )));
        }
        Ok(())
    }
}

impl AppConfig {
    pub fn new(relays: Vec<String>, data_dir: PathBuf) -> Self {
        Self {
            relays,
            key_file: data_dir.join("me.nsec"),
            data_dir,
            display_name: None,
            history: Duration::from_secs(30 * 24 * 60 * 60),
            target: "web".into(),
            mint: None,
            settlement_policy: SettlementPolicy::default(),
            blossom: vec![],
        }
    }
}

/// Pushed to the UI as things change.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "short-lived UI events; boxing buys nothing"
)]
pub enum AppEvent {
    Message { message: MessageView },
    Contact { contact: ContactView },
    Commitment { commitment: CommitmentView },
    Wallet { wallet: WalletView },
}

struct State {
    contacts: BTreeMap<String, ContactView>,
    messages: HashMap<String, Vec<MessageView>>,
    seen: HashSet<String>,
    /// Agents whose data-handling disclosure the user accepted.
    accepted: HashSet<String>,
    /// Conversation the user is looking at (no unread increments for it).
    active: Option<String>,
    /// ACN acts per conversation (counterparty hex), keyed by event id.
    acts: HashMap<String, BTreeMap<String, acn_core::Event>>,
    /// Raw slash command definitions per contact (for binding args into ACN terms).
    commands: HashMap<String, Vec<SlashCommandDefinition>>,
    /// Public ACN binding events per contact (fold input for V-8).
    bindings: HashMap<String, Vec<acn_core::Event>>,
    /// `K_SVC` content per contact (terms templates).
    svc: HashMap<String, Value>,
    /// commitment id -> conversation.
    owner: HashMap<String, String>,
    /// commitment id -> payment preimage `s` (hex) for conditional commitments we requested.
    secrets: HashMap<String, String>,
    /// commitment id -> our requester seed for held-out instances (hex).
    seeds: HashMap<String, String>,
    /// Terms documents referenced by hash rather than inlined (Core §7), as fetched.
    docs: BTreeMap<String, Value>,
    /// Delivery keys recovered from the mint (A10), persisted.
    keys: BTreeMap<String, String>,
    /// Hashes being fetched or found missing (not retried on every act).
    docs_tried: HashSet<String>,
    /// ACN acts created before this are ignored ("Reset demo").
    demo_epoch: u64,
    /// Commitments the reset cleared that the time cutoff alone can't exclude (same second).
    cleared: HashSet<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoardView {
    /// Open commitments, then the newest `closed_limit` closed ones; each newest first.
    pub items: Vec<CommitmentView>,
    pub closed_total: usize,
    /// Board time (real time + presentation clock offset).
    pub now: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClockView {
    pub now: u64,
    pub offset: i64,
}

fn hex32(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

struct Inner {
    session: Session,
    registry: ContractRegistry,
    trust: TrustPolicy,
    installed: Vec<RendererManifest>,
    target: String,
    store: store::Store,
    state: Mutex<State>,
    events: mpsc::Sender<AppEvent>,
    clock_offset: AtomicI64,
    wallet: Option<CashuWallet>,
    policy: SettlementPolicy,
    blossom: Option<nmesh_blossom::BlossomClient>,
    /// Bounds concurrent fetches of referenced documents.
    fetches: tokio::sync::Semaphore,
}

#[derive(Clone)]
pub struct App {
    inner: Arc<Inner>,
}

impl App {
    pub async fn start(config: AppConfig) -> Result<(Self, mpsc::Receiver<AppEvent>), AppError> {
        let keys = nmesh_core::load_or_create_keys(&config.key_file)?;
        let session = Session::connect(
            keys,
            SessionConfig {
                relays: config.relays.clone(),
                kind: IdentityKind::Human,
                display_name: config.display_name.clone(),
                about: None,
                announce: true,
            },
        )
        .await?;

        let wallet = match &config.mint {
            Some(url) => Some(
                CashuWallet::open(
                    url,
                    &session.keys().secret_key().secret_bytes(),
                    Some(&config.data_dir.join("wallet")),
                )
                .await
                .map_err(|e| AppError::Invalid(format!("wallet: {e}")))?,
            ),
            None => None,
        };
        let store = store::Store::new(&config.data_dir);
        let stored = store.load();
        let mut contacts = BTreeMap::new();
        let mut accepted = HashSet::new();
        for c in &stored.contacts {
            if let Ok(pk) = PublicKey::parse(&c.pubkey) {
                let mut v = ContactView::new(pk);
                v.is_agent = c.is_agent;
                // A classification is only trusted if discovery established it; anything else
                // is re-checked before the first send (`ensure_consent`).
                if c.is_agent || c.resolved {
                    v.discovery = view::Discovery::Resolved;
                }
                v.consented = c.accepted || (c.resolved && !c.is_agent);
                if c.accepted {
                    accepted.insert(v.pubkey.clone());
                }
                contacts.insert(v.pubkey.clone(), v);
            }
        }

        let (tx, rx) = mpsc::channel(512);
        let session_keys = session.keys().clone();
        let app = Self {
            inner: Arc::new(Inner {
                trust: TrustPolicy::new(session.public_key()),
                session,
                registry: ContractRegistry::with_builtins(),
                installed: builtin::renderers(),
                target: config.target.clone(),
                store,
                state: Mutex::new(State {
                    contacts,
                    messages: HashMap::new(),
                    seen: HashSet::new(),
                    accepted,
                    active: None,
                    acts: HashMap::new(),
                    commands: HashMap::new(),
                    bindings: HashMap::new(),
                    svc: HashMap::new(),
                    owner: HashMap::new(),
                    secrets: stored.secrets.clone(),
                    seeds: stored.seeds.clone(),
                    docs: BTreeMap::new(),
                    keys: stored.keys.clone(),
                    docs_tried: HashSet::new(),
                    demo_epoch: stored.demo_epoch,
                    cleared: stored.cleared.iter().cloned().collect(),
                }),
                events: tx,
                clock_offset: AtomicI64::new(0),
                wallet,
                policy: config.settlement_policy.clone(),
                blossom: (!config.blossom.is_empty()).then(|| {
                    nmesh_blossom::BlossomClient::new(config.blossom.clone(), session_keys.clone())
                }),
                fetches: tokio::sync::Semaphore::new(1),
            }),
        };

        // Refresh profiles of known contacts in the background.
        let known: Vec<String> = app
            .inner
            .state
            .lock()
            .await
            .contacts
            .keys()
            .cloned()
            .collect();
        for pk in known {
            app.spawn_refresh(pk);
        }

        let mut inbox = app
            .inner
            .session
            .inbox_with(InboxOptions {
                include_own: true,
                lookback: config.history,
            })
            .await?;
        let this = app.clone();
        tokio::spawn(async move {
            while let Some(m) = inbox.recv().await {
                this.ingest(m).await;
            }
        });
        // Contingent deliveries whose performer redeemed but never sent `CLAIM` (A10).
        if app.inner.wallet.is_some() {
            let this = app.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(20)).await;
                    this.recover_delivery_keys().await;
                }
            });
        }

        Ok((app, rx))
    }

    fn ctx(&self) -> RenderContext<'_> {
        RenderContext {
            registry: &self.inner.registry,
            installed: &self.inner.installed,
            trust: &self.inner.trust,
            target: &self.inner.target,
        }
    }

    fn me(&self) -> PublicKey {
        self.inner.session.public_key()
    }

    pub fn identity(&self) -> Identity {
        let me = self.me();
        Identity {
            pubkey: me.to_hex(),
            npub: me.to_bech32().unwrap_or_default(),
            name: None,
            relays: self
                .inner
                .session
                .relays()
                .iter()
                .map(|r| r.to_string())
                .collect(),
        }
    }

    /// Contacts ordered by most recent activity.
    pub async fn contacts(&self) -> Vec<ContactView> {
        let st = self.inner.state.lock().await;
        let mut v: Vec<_> = st.contacts.values().cloned().collect();
        v.sort_by(|a, b| b.last_at.cmp(&a.last_at).then(a.name.cmp(&b.name)));
        v
    }

    /// History for a conversation; also marks it as the active one and clears unread.
    pub async fn open(&self, pubkey: &str) -> Result<Vec<MessageView>, AppError> {
        let pk = parse_pk(pubkey)?.to_hex();
        let mut st = self.inner.state.lock().await;
        st.active = Some(pk.clone());
        let mut is_agent = false;
        if let Some(c) = st.contacts.get_mut(&pk) {
            c.unread = 0;
            is_agent = c.is_agent;
        }
        let history = st.messages.get(&pk).cloned().unwrap_or_default();
        drop(st);
        // Agents change their commands and terms templates (e.g. a restarted demo cast); re-read
        // the advertisement so the composer and Director don't show a stale copy.
        if is_agent {
            self.spawn_refresh(pk);
        }
        Ok(history)
    }

    /// Add (or re-fetch) a contact by npub / hex / nprofile.
    pub async fn add_contact(&self, input: &str) -> Result<ContactView, AppError> {
        let pk = parse_pk(input)?;
        if pk == self.me() {
            return Err(AppError::Invalid("that's you".into()));
        }
        let key = pk.to_hex();
        {
            let mut st = self.inner.state.lock().await;
            st.contacts
                .entry(key.clone())
                .or_insert_with(|| ContactView::new(pk));
        }
        self.persist().await;
        self.refresh(&key).await
    }

    /// A card image the user asked to load (A16: images never load on their own). The payload
    /// must pin its sha256; the native fetch is bounded and public-addresses-only, and the image
    /// is returned as a `data:` URL only if the hash matches, so the sender learns at most that
    /// the user clicked.
    pub async fn fetch_image(&self, url: &str, sha256: &str) -> Result<String, AppError> {
        use base64::Engine;
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::Invalid(
                "the image has no valid sha256 pin".into(),
            ));
        }
        let policy = nmesh_blossom::FetchPolicy {
            max_bytes: 5 * 1024 * 1024,
            ..nmesh_blossom::FetchPolicy::default()
        };
        let bytes = nmesh_blossom::fetch_url(url, sha256, &policy)
            .await
            .map_err(|e| AppError::Invalid(format!("image: {e}")))?;
        let mime = match bytes.as_slice() {
            [0x89, b'P', b'N', b'G', ..] => "image/png",
            [0xff, 0xd8, 0xff, ..] => "image/jpeg",
            [b'G', b'I', b'F', b'8', ..] => "image/gif",
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
            _ => {
                return Err(AppError::Invalid(
                    "image: not a PNG, JPEG, GIF or WebP".into(),
                ))
            }
        };
        Ok(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }

    /// Record that the user has seen and accepted an agent's data-handling disclosure.
    pub async fn accept_agent(&self, pubkey: &str) -> Result<ContactView, AppError> {
        let key = parse_pk(pubkey)?.to_hex();
        let c = {
            let mut st = self.inner.state.lock().await;
            st.accepted.insert(key.clone());
            let c = st
                .contacts
                .get_mut(&key)
                .ok_or_else(|| AppError::Invalid("unknown contact".into()))?;
            c.consented = true;
            c.clone()
        };
        self.persist().await;
        // Terms referenced by acts that arrived before the user accepted.
        self.fetch_referenced(&key);
        Ok(c)
    }

    pub async fn send_text(&self, pubkey: &str, text: &str) -> Result<MessageView, AppError> {
        let pk = parse_pk(pubkey)?;
        self.ensure_consent(&pk).await?;
        let text = text.trim();
        if text.is_empty() {
            return Err(AppError::Invalid("empty message".into()));
        }
        let id = self.inner.session.send_text(pk, text).await?;
        Ok(self
            .record_outgoing(pk, id, Body::Text(text.to_string()))
            .await)
    }

    /// Emit an action on a structured message (button press / form submit). Renderers never
    /// send directly; they call this through the host.
    pub async fn send_action(
        &self,
        pubkey: &str,
        target: &str,
        action_id: Option<String>,
        values: Option<Value>,
        label: Option<String>,
    ) -> Result<MessageView, AppError> {
        let pk = parse_pk(pubkey)?;
        self.ensure_consent(&pk).await?;
        // The action answers a message this peer sent us, in this conversation (A11): a stale
        // UI can't send one message's form values to another peer.
        {
            let st = self.inner.state.lock().await;
            let owned = st
                .messages
                .get(&pk.to_hex())
                .is_some_and(|l| l.iter().any(|m| m.id == target && !m.mine));
            if !owned {
                return Err(AppError::Invalid(
                    "this action belongs to a message from another conversation".into(),
                ));
            }
        }
        let target =
            EventId::parse(target).map_err(|e| AppError::Invalid(format!("bad target id: {e}")))?;

        let mut payload = serde_json::Map::new();
        if let Some(a) = &action_id {
            payload.insert("action_id".into(), Value::String(a.clone()));
        }
        if let Some(v) = values {
            payload.insert("values".into(), v);
        }
        let payload = Value::Object(payload);
        let contract = self
            .inner
            .registry
            .pinned_ref(builtin::FORM_SUBMIT)
            .unwrap_or_else(|| ContractRef::new(builtin::FORM_SUBMIT));
        self.inner
            .registry
            .validate(&contract, &payload)
            .map_err(|e| AppError::Invalid(e.to_string()))?;

        let alt = label
            .or(action_id)
            .unwrap_or_else(|| "Submitted form".to_string());
        let action = Action {
            target,
            contract,
            payload,
            alt,
        };
        let id = self.inner.session.send_action(pk, &action).await?;
        Ok(self.record_outgoing(pk, id, Body::Action(action)).await)
    }

    pub async fn shutdown(&self) {
        self.inner.session.shutdown().await;
    }

    // --- ACN commitments ---

    /// Presentation clock: `status_at` (lapses, countdowns) uses real time + this offset.
    /// Signing always uses real time.
    pub fn clock(&self) -> ClockView {
        ClockView {
            now: self.now(),
            offset: self.inner.clock_offset.load(Ordering::Relaxed),
        }
    }

    pub async fn set_clock_offset(&self, offset: i64) -> ClockView {
        self.inner
            .clock_offset
            .store(offset.max(0), Ordering::Relaxed);
        self.refresh_commitments().await;
        self.clock()
    }

    fn now(&self) -> u64 {
        (Timestamp::now().as_secs() as i64 + self.inner.clock_offset.load(Ordering::Relaxed)) as u64
    }

    /// Find the ACN demo cast (`nmesh-acn-sim`) on our relays and add Atlas and Rogue as
    /// contacts. Empty if the simulator isn't running against these relays.
    pub async fn discover_demo(&self) -> Result<Vec<ContactView>, AppError> {
        let found = self
            .inner
            .session
            .find_acn_services(&["acn-sim-atlas", "acn-sim-rogue"], Duration::from_secs(4))
            .await?;
        let mut out = Vec::new();
        for pk in found {
            out.push(self.add_contact(&pk.to_hex()).await?);
        }
        Ok(out)
    }

    /// Board (queue view): commitments across all conversations, in every role. Open ones are
    /// all returned; closed ones newest first, at most `closed_limit` (`closed_total` tells the
    /// UI how many more there are).
    pub async fn board(&self, closed_limit: usize) -> BoardView {
        let st = self.inner.state.lock().await;
        let mut all: Vec<CommitmentView> = st
            .acts
            .keys()
            .flat_map(|conv| self.project_locked(&st, conv))
            .collect();
        drop(st);
        all.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let (closed, mut items): (Vec<_>, Vec<_>) = all
            .into_iter()
            .partition(|c| c.lane == acn_view::Lane::Closed);
        let closed_total = closed.len();
        items.extend(closed.into_iter().take(closed_limit));
        BoardView {
            items,
            closed_total,
            now: self.now(),
        }
    }

    /// All commitments in a conversation, oldest first.
    pub async fn commitments(&self, pubkey: &str) -> Result<Vec<CommitmentView>, AppError> {
        let conv = parse_pk(pubkey)?.to_hex();
        let st = self.inner.state.lock().await;
        Ok(self.project_locked(&st, &conv))
    }

    fn project_locked(&self, st: &State, conv: &str) -> Vec<CommitmentView> {
        match st.acts.get(conv) {
            Some(acts) => commitments::project(
                conv,
                &self.me().to_hex(),
                acts,
                st.bindings.get(conv).map(Vec::as_slice).unwrap_or_default(),
                &commitments::Known {
                    documents: &st.docs,
                    keys: &st.keys,
                },
                &self.ctx(),
                self.now(),
            ),
            None => vec![],
        }
    }

    /// Request a tool as an ACN commitment. `text` is a slash command (`/status`,
    /// `/negotiate a logo`); arguments are bound with the agent's NIP-CD definition. If the
    /// agent's `K_SVC` publishes a terms template for the tool, it is used (profile, conditions,
    /// verifiers, arbiter, governance, price); otherwise a free T0 call is requested.
    pub async fn request(&self, pubkey: &str, text: &str) -> Result<CommitmentView, AppError> {
        let pk = parse_pk(pubkey)?;
        self.ensure_consent(&pk).await?;
        let key = pk.to_hex();
        let inv = parse_invocation(text.trim())
            .ok_or_else(|| AppError::Invalid("a commitment request must be a /command".into()))?;
        let (defs, acn, svc) = {
            let st = self.inner.state.lock().await;
            (
                st.commands.get(&key).cloned().unwrap_or_default(),
                st.contacts.get(&key).and_then(|c| c.acn.clone()),
                st.svc.get(&key).cloned(),
            )
        };
        let acn = acn.ok_or_else(|| {
            AppError::Invalid("this contact doesn't offer ACN commitments".into())
        })?;
        let def = defs
            .iter()
            .find(|d| d.command == inv.command)
            .ok_or_else(|| AppError::Invalid(format!("unknown command /{}", inv.command)))?;
        let input = Value::Object(
            bind_args(&def.args, &inv.rest)
                .map_err(|e| AppError::Invalid(format!("/{}: {e}", def.command)))?,
        );

        let template: Option<Template> = svc
            .as_ref()
            .and_then(|s| s["tools"].as_array())
            .and_then(|tools| tools.iter().find(|t| t["name"] == def.command.as_str()))
            .and_then(|t| serde_json::from_value(t["terms_template"].clone()).ok());
        let now = Timestamp::now().as_secs();
        let mut secret = None;
        // Held-out instances (CR-003 §2.3): our seed's hash goes in the terms; the seed itself
        // goes privately to the runner, who reveals it when it attests.
        let mut seed: Option<(String, String)> = None;
        let (profile, terms) = match &template {
            Some(t) => {
                let hashlock = t.hashlock.then(|| {
                    let s = Keys::generate().secret_key().secret_bytes();
                    let h = acn_core::jcs::sha256_hex(&s);
                    secret = Some(hex32(&s));
                    h
                });
                let seed_hash = t.requester_seed_to.as_ref().map(|to| {
                    let s = Keys::generate().secret_key().secret_bytes();
                    seed = Some((hex32(&s), to.clone()));
                    acn_core::jcs::sha256_hex(&s)
                });
                let terms = acn_core::draft::instantiate(
                    t,
                    &def.command,
                    input,
                    now,
                    hashlock.as_deref(),
                    seed_hash.as_deref(),
                )
                .map_err(AppError::Invalid)?;
                (t.profile.clone(), terms)
            }
            None => (
                "T0".to_string(),
                acn_core::draft::tool_call_terms(
                    &def.command,
                    input,
                    now + acn.deadline_secs,
                    acn.review_window_secs,
                    acn.min_price_sat,
                ),
            ),
        };
        self.send_request(&pk, svc.as_ref(), &profile, terms, secret, seed)
            .await
    }

    /// Check terms before they're sent: settlement policy, then the linter (CR-003 §3: errors
    /// block sending; warnings go on the card).
    fn check_terms(&self, terms: &Value) -> Result<(), AppError> {
        self.inner.policy.check(terms)?;
        let t = acn_core::terms::Terms::check(terms).map_err(|(code, e)| {
            AppError::Invalid(format!("the terms aren't valid ({}): {e}", code.as_str()))
        })?;
        let errors: Vec<String> = acn_author::lint(&t, &acn_author::LintContext::default())
            .into_iter()
            .filter(|f| f.level == acn_author::Level::Error)
            .map(|f| format!("{} {}: {}", f.rule, f.item.unwrap_or_default(), f.message))
            .collect();
        if !errors.is_empty() {
            return Err(AppError::Invalid(format!(
                "the terms can't be sent: {}",
                errors.join("; ")
            )));
        }
        Ok(())
    }

    /// The linter's findings on a terms document (for an editor; `binding` = about to accept).
    pub fn lint_terms(
        &self,
        terms: &Value,
        binding: bool,
    ) -> Result<Vec<acn_author::Finding>, AppError> {
        let t = acn_core::terms::Terms::check(terms)
            .map_err(|(code, e)| AppError::Invalid(format!("{}: {e}", code.as_str())))?;
        Ok(acn_author::lint(
            &t,
            &acn_author::LintContext {
                binding,
                judge_price_threshold: acn_view::JUDGE_PRICE_THRESHOLD,
                ..Default::default()
            },
        ))
    }

    /// Put a terms document on Blossom (JCS bytes); returns its hash and the server URL hint.
    async fn store_terms(&self, terms: &Value) -> Result<(String, String), AppError> {
        let client = self.inner.blossom.as_ref().ok_or_else(|| {
            AppError::Invalid("these terms exceed the 16 KiB inline limit: configure a Blossom server (NMESH_BLOSSOM) to send them by reference".into())
        })?;
        let bytes =
            acn_core::jcs::canonical(terms).map_err(|e| AppError::Invalid(e.to_string()))?;
        let (hash, servers) = client
            .upload(bytes.as_bytes(), "application/json")
            .await
            .map_err(|e| AppError::Invalid(format!("Blossom: {e}")))?;
        self.inner
            .state
            .lock()
            .await
            .docs
            .insert(hash.clone(), terms.clone());
        Ok((hash, servers.first().cloned().unwrap_or_default()))
    }

    /// Send a `REQUEST` with terms we drafted ourselves (e.g. with [`App::draft_request`] and
    /// edits), rather than an agent's terms offer. A price with a `payment` mint gets a fresh
    /// hashlock and the token is locked as for an offer; held-out instances aren't supported here
    /// (the seed needs a runner the offer names).
    pub async fn request_terms(
        &self,
        pubkey: &str,
        profile: &str,
        mut terms: Value,
    ) -> Result<CommitmentView, AppError> {
        let pk = parse_pk(pubkey)?;
        self.ensure_consent(&pk).await?;
        let key = pk.to_hex();
        let svc = self.inner.state.lock().await.svc.get(&key).cloned();
        if !matches!(profile, "T0" | "T1" | "N") {
            return Err(AppError::Invalid(format!("unknown profile {profile}")));
        }
        let parsed = acn_core::terms::Terms::check(&terms).map_err(|(code, e)| {
            AppError::Invalid(format!("the terms aren't valid ({}): {e}", code.as_str()))
        })?;
        if parsed.has_instances() {
            return Err(AppError::Invalid("held-out instances need the runner named by the agent's terms offer: request them with its /command".into()));
        }
        let mut secret = None;
        if parsed.price.amount > 0 {
            let pay = terms
                .get_mut("payment")
                .filter(|p| p.is_object())
                .ok_or_else(|| {
                    AppError::Invalid("priced terms need `payment` with the mint you pay at".into())
                })?;
            let s = Keys::generate().secret_key().secret_bytes();
            pay["hashlock"] = serde_json::json!(acn_core::jcs::sha256_hex(&s));
            pay["locktime"] = serde_json::json!(parsed.deadline + parsed.review_window + 24 * 3600);
            secret = Some(hex32(&s));
        }
        self.send_request(&pk, svc.as_ref(), profile, terms, secret, None)
            .await
    }

    /// Validate, lock payment, escrow the preimage and seed, and send a `REQUEST`.
    async fn send_request(
        &self,
        pk: &PublicKey,
        svc: Option<&Value>,
        profile: &str,
        terms: Value,
        secret: Option<String>,
        seed: Option<(String, String)>,
    ) -> Result<CommitmentView, AppError> {
        let key = pk.to_hex();
        let pk = *pk;
        self.check_terms(&terms)?;
        // Rail capability (Core.md §8.6): contingent needs a mint that reveals the preimage to
        // the payer, as declared in the agent's K_SVC.
        if terms["settlement_trigger"] == "contingent" {
            let mint = terms["payment"]["mint"].as_str().unwrap_or_default();
            let capable =
                svc.is_some_and(|s| mint_has_capability(s, mint, "reveals_preimage_to_payer"));
            if !capable {
                return Err(AppError::Invalid(format!(
                    "rail_incapable: mint {mint} is not declared to reveal preimages to payers"
                )));
            }
        }
        // Lock the price to the performer (Core.md §8.2 steps 1-2).
        let token = match (&secret, terms.get("payment").filter(|p| p.is_object())) {
            (Some(_), Some(pay)) if terms["price"]["amount"].as_u64().unwrap_or(0) > 0 => {
                let mint = pay["mint"].as_str().unwrap_or_default();
                if mint.starts_with("sim:") {
                    None
                } else {
                    let w = self.wallet_for(mint)?;
                    ensure_rail(w, mint, false).await?;
                    let amount = terms["price"]["amount"].as_u64().unwrap_or(0);
                    let tok = w
                        .lock_htlc(
                            amount,
                            pay["hashlock"].as_str().unwrap_or_default(),
                            &key,
                            pay["locktime"].as_u64().unwrap_or(0),
                            None,
                        )
                        .await
                        .map_err(|e| AppError::Invalid(format!("wallet: {e}")))?;
                    Some(tok)
                }
            }
            _ => None,
        };
        let ct = match &token {
            Some(t) => Some(self.inner.session.nip44_encrypt(&pk, t)?),
            None => None,
        };
        // Inline when small enough; otherwise on Blossom, referenced by hash (Core §7).
        let inline = acn_core::jcs::canonical(&terms)
            .map(|c| c.len())
            .unwrap_or(usize::MAX);
        let mut draft =
            if inline + ct.as_ref().map_or(0, |c| c.len()) < acn_core::INLINE_LIMIT - 512 {
                Draft::request(&key, profile, &terms, ct.as_deref())
                    .map_err(|e| AppError::Invalid(e.to_string()))?
            } else {
                let (hash, url) = self.store_terms(&terms).await?;
                Draft::request_ref(&key, profile, &hash, &url, ct.as_deref())
            };
        // Escrow the preimage with its holder: the arbiter (Core.md §8.4) or the settlement
        // verifier (verifier trigger, §8.2).
        let holder = terms["settlement_verifier"]
            .as_str()
            .or(terms["arbiter"].as_str());
        if let (Some(s), Some(holder)) = (&secret, holder) {
            let holder = PublicKey::parse(holder).map_err(|e| AppError::BadKey(e.to_string()))?;
            let ct = self.inner.session.nip44_encrypt(&holder, s)?;
            draft = draft.tag(vec!["escrow".into(), ct, holder.to_hex()]);
        }
        if let Some((s, to)) = &seed {
            let runner = PublicKey::parse(to).map_err(|e| AppError::BadKey(e.to_string()))?;
            // When we run the checks ourselves, the seed stays here (see `held_out`).
            if runner != self.inner.session.public_key() {
                let ct = self.inner.session.nip44_encrypt(&runner, s)?;
                draft = draft.tag(vec!["seed".into(), ct, runner.to_hex()]);
            }
        }
        let act = self.inner.session.sign_act(&draft, None)?;
        let id = act.id.clone();
        if secret.is_some() || seed.is_some() {
            let mut st = self.inner.state.lock().await;
            if let Some(s) = secret {
                st.secrets.insert(id.clone(), s);
            }
            if let Some((s, _)) = seed {
                st.seeds.insert(id.clone(), s);
            }
            drop(st);
            // Without the saved preimage we could never pay this commitment: don't send it. A
            // lock already made is tagged below and stays refundable after its locktime.
            if let Err(e) = self.persist_checked().await {
                if let (Some(t), Some(w)) = (&token, &self.inner.wallet) {
                    let _ = w.tag(t, &id);
                    self.emit_wallet().await;
                }
                return Err(e);
            }
        }
        if let (Some(t), Some(w)) = (&token, &self.inner.wallet) {
            let _ = w.tag(t, &id);
            self.emit_wallet().await;
        }
        self.inner.session.send_act(pk, &act).await?;
        self.on_act(&key, act).await;
        self.commitment(&key, &id)
            .await
            .ok_or_else(|| AppError::Invalid("commitment not created".into()))
    }

    /// Propose other terms (profile N): a `COUNTER` on a commitment we requested, while it is
    /// being negotiated. The terms go through the same checks as a request's; large ones go to
    /// Blossom by reference.
    pub async fn counter(
        &self,
        pubkey: &str,
        commitment: &str,
        terms: Value,
        reason: &str,
    ) -> Result<CommitmentView, AppError> {
        self.act(
            pubkey,
            commitment,
            HintAction::Counter {
                terms,
                reason: reason.to_string(),
            },
        )
        .await
    }

    /// Fetch terms documents the conversation's acts reference but don't inline (Core §7): from
    /// the tag's URL hint, the agent's Blossom servers, then ours; each checked against its hash.
    /// Only for conversations the user has accepted (A13): a stranger's act can't make the app
    /// contact a server. Hints and advertised servers are other parties' URLs, so they are
    /// fetched under the bounded, public-addresses-only policy; only our configured servers may
    /// be local. One fetch runs at a time.
    fn fetch_referenced(&self, conv: &str) {
        let this = self.clone();
        let conv = conv.to_string();
        tokio::spawn(async move {
            let allowed = {
                let st = this.inner.state.lock().await;
                st.contacts
                    .get(&conv)
                    .is_some_and(|c| !c.needs_consent() && c.discovery != view::Discovery::Pending)
            };
            if !allowed {
                tracing::debug!(%conv, "not fetching referenced terms for a contact not yet accepted");
                return;
            }
            let _permit = this.inner.fetches.acquire().await;
            let events = this.fold_input(&conv).await;
            let wanted: Vec<(String, Option<String>)> = {
                let mut st = this.inner.state.lock().await;
                let st = &mut *st;
                let (docs, tried) = (&st.docs, &mut st.docs_tried);
                acn_core::draft::referenced_terms(&events)
                    .into_iter()
                    .filter(|(h, _)| !docs.contains_key(h) && tried.insert(h.clone()))
                    .collect()
            };
            if wanted.is_empty() {
                return;
            }
            let agent_servers: Vec<String> = {
                let st = this.inner.state.lock().await;
                st.svc
                    .get(&conv)
                    .and_then(|s| s["blossom"].as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            };
            let ours = this
                .inner
                .blossom
                .as_ref()
                .map(|b| b.servers.clone())
                .unwrap_or_default();
            let policy = nmesh_blossom::FetchPolicy {
                max_bytes: acn_core::INLINE_LIMIT * 16,
                ..nmesh_blossom::FetchPolicy::trusting(&ours)
            };
            let mut got = false;
            for (hash, hint) in wanted {
                let servers: Vec<String> = hint
                    .into_iter()
                    .chain(agent_servers.clone())
                    .chain(ours.clone())
                    .collect();
                let doc = nmesh_blossom::fetch_from(&servers, &hash, &policy)
                    .await
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .filter(|v| acn_core::jcs::hash(v).ok().as_deref() == Some(hash.as_str()));
                match doc {
                    Some(v) => {
                        this.inner.state.lock().await.docs.insert(hash, v);
                        got = true;
                    }
                    None => {
                        tracing::warn!(%hash, "referenced terms not retrievable; the commitment stays pending");
                        this.inner.state.lock().await.docs_tried.remove(&hash);
                    }
                }
            }
            if got {
                let views = {
                    let st = this.inner.state.lock().await;
                    this.project_locked(&st, &conv)
                };
                for v in views {
                    let _ = this
                        .inner
                        .events
                        .send(AppEvent::Commitment { commitment: v })
                        .await;
                }
            }
        });
    }

    /// Close a reported commitment as satisfied. Reveals the preimage when the terms carry a
    /// hashlock (INV-6: closing is paying).
    pub async fn satisfy(
        &self,
        pubkey: &str,
        commitment: &str,
    ) -> Result<CommitmentView, AppError> {
        self.act(pubkey, commitment, HintAction::Satisfy).await
    }

    /// Reject a reported result, naming the failed condition (defaults to the first one).
    pub async fn reject(
        &self,
        pubkey: &str,
        commitment: &str,
        condition: Option<String>,
        note: &str,
    ) -> Result<CommitmentView, AppError> {
        let condition = match condition.filter(|c| !c.is_empty()) {
            Some(c) => c,
            None => self
                .commitments(pubkey)
                .await?
                .into_iter()
                .find(|c| c.id == commitment)
                .and_then(|c| c.projection.conditions.first().map(|x| x.id.clone()))
                .unwrap_or_else(|| acn_core::draft::RESULT_CONDITION.into()),
        };
        let note = if note.trim().is_empty() {
            "Not what I asked for"
        } else {
            note.trim()
        };
        self.act(
            pubkey,
            commitment,
            HintAction::Reject {
                condition,
                note: note.into(),
            },
        )
        .await
    }

    pub async fn withdraw(
        &self,
        pubkey: &str,
        commitment: &str,
    ) -> Result<CommitmentView, AppError> {
        self.act(pubkey, commitment, HintAction::Withdraw).await
    }

    pub async fn accept_counter(
        &self,
        pubkey: &str,
        commitment: &str,
    ) -> Result<CommitmentView, AppError> {
        self.act(pubkey, commitment, HintAction::AcceptCounter)
            .await
    }

    pub async fn dispute(
        &self,
        pubkey: &str,
        commitment: &str,
        reason: &str,
    ) -> Result<CommitmentView, AppError> {
        let reason = if reason.trim().is_empty() {
            "I dispute this outcome"
        } else {
            reason.trim()
        };
        self.act(
            pubkey,
            commitment,
            HintAction::Dispute {
                reason: reason.into(),
            },
        )
        .await
    }

    fn wallet_for(&self, mint: &str) -> Result<&CashuWallet, AppError> {
        let w = self.inner.wallet.as_ref().ok_or_else(|| {
            AppError::Invalid(format!(
                "this request costs sats at {mint}, but no wallet is configured (set NMESH_MINT)"
            ))
        })?;
        if w.mint_url() != mint.trim_end_matches('/') {
            return Err(AppError::Invalid(format!(
                "the agent wants payment at {mint}, but your wallet uses {}",
                w.mint_url()
            )));
        }
        Ok(w)
    }

    /// The built-in wallet (balance, locks, history), refreshed from the mint.
    pub async fn wallet(&self) -> Result<Option<WalletView>, AppError> {
        match &self.inner.wallet {
            Some(w) => Ok(Some(
                w.view()
                    .await
                    .map_err(|e| AppError::Invalid(format!("wallet: {e}")))?,
            )),
            None => Ok(None),
        }
    }

    async fn emit_wallet(&self) {
        if let Ok(Some(wallet)) = self.wallet().await {
            let _ = self.inner.events.send(AppEvent::Wallet { wallet }).await;
        }
    }

    /// Recover delivery keys from the mint (Core §8.2): for every contingent commitment we
    /// funded that has no `CLAIM`, ask the mint whether the `FUND` token was spent; if so its
    /// witness carries `k`, which opens the sealed `REPORT` (checked against its hashlock by
    /// the projection). Runs in the background; returns how many keys were recovered.
    pub async fn recover_delivery_keys(&self) -> usize {
        let Some(w) = self.inner.wallet.as_ref() else {
            return 0;
        };
        let me = self.me().to_hex();
        let (convs, docs) = {
            let st = self.inner.state.lock().await;
            (st.acts.keys().cloned().collect::<Vec<_>>(), st.docs.clone())
        };
        let mut found = 0;
        for conv in convs {
            let events = self.fold_input(&conv).await;
            let out = acn_core::fold(&events, &docs);
            let mut got = false;
            for cid in out.commitments.keys() {
                if self.inner.state.lock().await.keys.contains_key(cid) {
                    continue;
                }
                let Some(acn_wallet::settlement::Advice::LearnKey { .. }) =
                    acn_wallet::settlement::advise_with(&events, &docs, cid, &me, self.now())
                else {
                    continue;
                };
                // The FUND act carries the token encrypted to the performer; ours is in the wallet.
                let Some(token) = w.token_for(cid) else {
                    continue;
                };
                match w.revealed_preimage(&token).await {
                    Ok(Some(k)) => {
                        tracing::info!(commitment = %cid, "delivery key recovered from the mint (no CLAIM)");
                        self.inner.state.lock().await.keys.insert(cid.clone(), k);
                        got = true;
                        found += 1;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::debug!(commitment = %cid, error = %e, "key recovery: mint check failed")
                    }
                }
            }
            if got {
                self.persist().await;
                let views = {
                    let st = self.inner.state.lock().await;
                    self.project_locked(&st, &conv)
                };
                for v in views {
                    let _ = self
                        .inner
                        .events
                        .send(AppEvent::Commitment { commitment: v })
                        .await;
                }
            }
        }
        found
    }

    /// Mint test sats (the demo mint's Lightning backend is fake and pays itself).
    pub async fn top_up(&self, amount: u64) -> Result<WalletView, AppError> {
        let w = self
            .inner
            .wallet
            .as_ref()
            .ok_or_else(|| AppError::Invalid("no wallet configured (set NMESH_MINT)".into()))?;
        w.top_up(amount)
            .await
            .map_err(|e| AppError::Invalid(format!("top up: {e}")))?;
        self.emit_wallet().await;
        Ok(self.wallet().await?.expect("wallet exists"))
    }

    /// Reclaim the locked payment of a commitment that closed unsatisfied, once its locktime has
    /// passed (NUT-14 refund path). The mint checks the locktime against its own clock, so the
    /// presentation clock cannot fast-forward this.
    ///
    /// `commitment` may also be a wallet lock id. A lock whose commitment isn't in our history
    /// (its request was never published, or "reset demo" hid it) is refundable from the wallet's
    /// own record after its locktime (A18): recovering funds never depends on history.
    pub async fn refund(&self, pubkey: &str, commitment: &str) -> Result<WalletView, AppError> {
        let conv = parse_pk(pubkey)?.to_hex();
        let w = self
            .inner
            .wallet
            .as_ref()
            .ok_or_else(|| AppError::Invalid("no wallet configured".into()))?;
        let events = self.fold_input(&conv).await;
        let me = self.me().to_hex();
        let docs = self.inner.state.lock().await.docs.clone();
        let token = w
            .token_for(commitment)
            .or_else(|| w.token_of_lock(commitment))
            .ok_or_else(|| AppError::Invalid("no locked token for this commitment".into()))?;
        let known = acn_core::fold(&events, &docs)
            .commitments
            .contains_key(commitment);
        let locktime = if known {
            match acn_wallet::settlement::advise_with(&events, &docs, commitment, &me, self.now()) {
                Some(acn_wallet::settlement::Advice::Refund { locktime, .. }) => locktime,
                _ => {
                    return Err(AppError::Invalid(
                        "nothing to refund: the commitment is open or was satisfied".into(),
                    ))
                }
            }
        } else {
            let id = nmesh_wallet::lock_id(&token);
            let view = w
                .view()
                .await
                .map_err(|e| AppError::Invalid(format!("wallet: {e}")))?;
            let lock = view
                .locks
                .iter()
                .find(|l| l.id == id)
                .ok_or_else(|| AppError::Invalid("no such lock".into()))?;
            if lock.refund != me || lock.state != acn_wallet::LockState::Locked {
                return Err(AppError::Invalid(
                    "this lock isn't refundable by you".into(),
                ));
            }
            lock.locktime
        };
        if Timestamp::now().as_secs() <= locktime {
            return Err(AppError::Invalid(format!(
                "refundable after the locktime ({locktime}, unix time); the mint enforces it"
            )));
        }
        w.refund(&token)
            .await
            .map_err(|e| AppError::Invalid(format!("refund: {e}")))?;
        self.emit_wallet().await;
        Ok(self.wallet().await?.expect("wallet exists"))
    }

    /// Clear the demo: forget every commitment signed so far and reset the presentation clock.
    ///
    /// Signed acts can't be deleted from relays, so this sets a cutoff instead: acts created
    /// before now are ignored from here on, also after a restart (history refetch). Contacts,
    /// consent and bindings stay. The wallet stays as well: it holds real ecash, and locks from
    /// earlier commitments remain refundable from the wallet panel.
    pub async fn reset_demo(&self) {
        {
            let mut st = self.inner.state.lock().await;
            let epoch = Timestamp::now().as_secs();
            st.demo_epoch = epoch;
            let boundary: Vec<String> = st
                .acts
                .values()
                .flat_map(|m| m.values())
                .filter(|e| e.created_at + 1 >= epoch)
                .map(commitments::commitment_of)
                .collect();
            st.cleared.extend(boundary);
            st.acts.clear();
            st.owner.clear();
            for list in st.messages.values_mut() {
                list.retain(|m| !matches!(m.body, BodyView::Commitment { .. }));
            }
        }
        self.inner.clock_offset.store(0, Ordering::Relaxed);
        self.persist().await;
    }

    /// Contingent settlement (Core.md §8.2): pay for an attested, encrypted delivery. Locks the
    /// price to the REPORT's hashlock and signs `FUND`; the performer's redemption then reveals
    /// the key (by `CLAIM`, and through the mint).
    pub async fn fund(&self, pubkey: &str, commitment: &str) -> Result<CommitmentView, AppError> {
        let pk = parse_pk(pubkey)?;
        let conv = pk.to_hex();
        let events = self.fold_input(&conv).await;
        let docs = self.inner.state.lock().await.docs.clone();
        let out = acn_core::fold(&events, &docs);
        let c = out
            .commitments
            .get(commitment)
            .ok_or_else(|| AppError::Invalid("unknown commitment".into()))?;
        let (Some(st), Some(terms)) = (&c.state, &c.terms) else {
            return Err(AppError::Invalid("terms unavailable".into()));
        };
        let (acn_core::Phase::Offered, Some(h)) = (st.phase, st.offer_hashlock.as_deref()) else {
            return Err(AppError::Invalid(
                "nothing to fund: no encrypted delivery is offered".into(),
            ));
        };
        let mint = terms
            .payment
            .as_ref()
            .map(|p| p.mint.clone())
            .unwrap_or_default();
        let w = self.wallet_for(&mint)?;
        ensure_rail(w, &mint, true).await?;
        let amount = terms.price.amount;
        // The performer must claim before this; then the requester can refund. Real time: the
        // mint enforces it, the presentation clock doesn't apply.
        let locktime = Timestamp::now().as_secs() + terms.review_window + 24 * 3600;
        let tok = w
            .lock_htlc(amount, h, &c.performer, locktime, Some(commitment))
            .await
            .map_err(|e| AppError::Invalid(format!("wallet: {e}")))?;
        let ct = self.inner.session.nip44_encrypt(&pk, &tok)?;
        let parties = [c.requester.as_str(), c.performer.as_str()];
        let draft = Draft::act(K_FUND, &c.id, &st.head, &parties).content(serde_json::json!({
            "hashlock": h, "amount": amount, "mint": mint, "locktime": locktime, "token": ct
        }));
        let act = self.inner.session.sign_act(&draft, Some(st.head_at))?;
        self.inner.session.send_act(pk, &act).await?;
        self.on_act(&conv, act).await;
        self.emit_wallet().await;
        self.commitment(&conv, commitment)
            .await
            .ok_or_else(|| AppError::Invalid("unknown commitment".into()))
    }

    /// The scripted next step for the requester (presenter guidance), if any.
    pub async fn hint(&self, pubkey: &str, commitment: &str) -> Result<Option<Hint>, AppError> {
        let conv = parse_pk(pubkey)?.to_hex();
        let events = self.fold_input(&conv).await;
        let docs = self.inner.state.lock().await.docs.clone();
        Ok(acn_sim::requester_hint_with(
            &events,
            &docs,
            commitment,
            self.now(),
        ))
    }

    /// Perform the scripted next step (presenter "Next").
    pub async fn run_hint(
        &self,
        pubkey: &str,
        commitment: &str,
    ) -> Result<Option<CommitmentView>, AppError> {
        let Some(hint) = self.hint(pubkey, commitment).await? else {
            return Ok(None);
        };
        match hint.action {
            HintAction::Wait | HintAction::Done => Ok(None),
            HintAction::AdvanceClock { secs } => {
                let off = self.inner.clock_offset.load(Ordering::Relaxed) + secs as i64;
                self.set_clock_offset(off).await;
                Ok(self
                    .commitment(&parse_pk(pubkey)?.to_hex(), commitment)
                    .await)
            }
            // FUND locks real money: the wallet path, not the generic act path.
            HintAction::Fund => self.fund(pubkey, commitment).await.map(Some),
            action => self.act(pubkey, commitment, action).await.map(Some),
        }
    }

    async fn fold_input(&self, conv: &str) -> Vec<acn_core::Event> {
        let st = self.inner.state.lock().await;
        st.acts
            .get(conv)
            .into_iter()
            .flat_map(|m| m.values())
            .chain(st.bindings.get(conv).into_iter().flatten())
            .cloned()
            .collect()
    }

    /// Draft terms for an intent with the requester drafter (CR-003 §3): it matches the terms
    /// templates published by the agents you've added, fills their slots from `cfg` (it never
    /// invents a key), and leaves what it can't close `open` for the performer to close.
    pub async fn draft_request(
        &self,
        intent: &acn_author::Intent,
        cfg: &acn_author::RequesterConfig,
    ) -> acn_author::RequestDraft {
        let templates: Vec<acn_author::TemplateCandidate> = {
            let st = self.inner.state.lock().await;
            let mut seen = HashSet::new();
            st.bindings
                .values()
                .flatten()
                .filter(|e| {
                    e.kind == acn_core::kinds::K_TERMS_TEMPLATE && seen.insert(e.id.clone())
                })
                .filter_map(|e| {
                    Some(acn_author::TemplateCandidate {
                        event: Some(e.id.clone()),
                        content: serde_json::from_str(&e.content).ok()?,
                    })
                })
                .collect()
        };
        acn_author::draft_request(intent, &templates, cfg)
    }

    /// What a requester-side runner needs to attest an item with held-out instances (CR-003
    /// §2.3): our seed for the commitment, the performer's seed its REPORT revealed, and the
    /// pool, which we wrote and keep (it never goes on the wire). Check it against the item's
    /// terms with `HeldOut::reveal_for` before running anything.
    pub async fn held_out(
        &self,
        pubkey: &str,
        commitment: &str,
        pool: Vec<Value>,
    ) -> Result<acn_core::instances::HeldOut, AppError> {
        let conv = parse_pk(pubkey)?.to_hex();
        let requester_seed = self
            .inner
            .state
            .lock()
            .await
            .seeds
            .get(commitment)
            .cloned()
            .ok_or_else(|| {
                AppError::Invalid("no requester seed kept for that commitment".into())
            })?;
        let events = self.fold_input(&conv).await;
        let docs = self.inner.state.lock().await.docs.clone();
        let out = acn_core::fold(&events, &docs);
        let performer_seed = out
            .commitments
            .get(commitment)
            .and_then(|c| c.state.as_ref())
            .and_then(|s| s.performer_seed.clone())
            .ok_or_else(|| {
                AppError::Invalid(
                    "the performer hasn't revealed its seed yet (no counted REPORT)".into(),
                )
            })?;
        Ok(acn_core::instances::HeldOut {
            pool,
            requester_seed,
            performer_seed,
            commitment: commitment.to_string(),
        })
    }

    /// Sign and send a requester act built through the act schemas.
    async fn act(
        &self,
        pubkey: &str,
        commitment: &str,
        action: HintAction,
    ) -> Result<CommitmentView, AppError> {
        let pk = parse_pk(pubkey)?;
        let conv = pk.to_hex();
        let events = self.fold_input(&conv).await;
        let (preimage, docs) = {
            let st = self.inner.state.lock().await;
            (st.secrets.get(commitment).cloned(), st.docs.clone())
        };
        if let HintAction::Counter { terms, .. } = &action {
            self.check_terms(terms)?;
        }
        let (mut draft, not_before) =
            acn_sim::requester_draft_with(&events, &docs, commitment, &action, preimage.as_deref())
                .ok_or_else(|| AppError::Invalid("that act isn't possible right now".into()))?;
        // Too large to inline: the counter's terms go to Blossom, referenced by hash (Core §7).
        if matches!(action, HintAction::Counter { .. })
            && draft.content.len() > acn_core::INLINE_LIMIT - 512
        {
            if let HintAction::Counter { terms, .. } = &action {
                let (_, url) = self.store_terms(terms).await?;
                draft = draft
                    .with_terms_by_reference(&url)
                    .map(|(d, _, _)| d)
                    .ok_or_else(|| AppError::Invalid("counter without terms".into()))?;
            }
        }
        let act = self.inner.session.sign_act(&draft, Some(not_before))?;
        let act_id = act.id.clone();
        self.inner.session.send_act(pk, &act).await?;
        self.on_act(&conv, act).await;
        let view = self
            .commitment(&conv, commitment)
            .await
            .ok_or_else(|| AppError::Invalid("unknown commitment".into()))?;
        // Surface validator decisions on our own act (e.g. stale after a concurrent act).
        if let Some(a) = view
            .projection
            .anomalies
            .iter()
            .find(|a| a.event == act_id && a.severity == acn_core::Severity::Reject)
        {
            return Err(AppError::Invalid(format!("{}: {}", a.code, a.detail)));
        }
        Ok(view)
    }

    async fn commitment(&self, conv: &str, id: &str) -> Option<CommitmentView> {
        let st = self.inner.state.lock().await;
        self.project_locked(&st, conv)
            .into_iter()
            .find(|c| c.id == id)
    }

    /// Re-emit every commitment of the active conversation (e.g. after a clock change).
    async fn refresh_commitments(&self) {
        let views = {
            let st = self.inner.state.lock().await;
            match &st.active {
                Some(conv) => self.project_locked(&st, conv),
                None => vec![],
            }
        };
        for v in views {
            let _ = self
                .inner
                .events
                .send(AppEvent::Commitment { commitment: v })
                .await;
        }
    }

    /// Store an ACN act and push the updated projection. REQUESTs also get a thread entry.
    /// Acts are grouped by commitment, so attestations or rulings from third parties (verifier,
    /// arbiter, gate) land in the conversation where the commitment lives.
    async fn on_act(&self, conv: &str, act: acn_core::Event) {
        let me = self.me().to_hex();
        let cid = commitments::commitment_of(&act);
        let is_request = act.kind == K_REQUEST;
        let mine = act.pubkey == me;
        let at = act.created_at;
        let kind = act.kind;
        let valid = act.verify().is_ok();
        let (conv, fresh) = {
            let mut st = self.inner.state.lock().await;
            if at < st.demo_epoch || st.cleared.contains(&cid) {
                return;
            }
            let conv = if is_request {
                let performer = act
                    .tags
                    .iter()
                    .find(|t| t.first().map(String::as_str) == Some("p"))
                    .and_then(|t| t.get(1).cloned());
                let conv = if mine {
                    performer.unwrap_or_else(|| conv.to_string())
                } else {
                    conv.to_string()
                };
                st.owner.insert(cid.clone(), conv.clone());
                conv
            } else {
                st.owner
                    .get(&cid)
                    .cloned()
                    .unwrap_or_else(|| conv.to_string())
            };
            let fresh = commitments::admit(st.acts.entry(conv.clone()).or_default(), act);
            (conv, fresh)
        };
        if !fresh {
            return;
        }
        if valid && matches!(kind, K_REQUEST | K_COUNTER) {
            self.fetch_referenced(&conv);
        }
        if is_request {
            let view = MessageView {
                id: cid.clone(),
                conversation: conv.clone(),
                sender: if mine { me.clone() } else { conv.clone() },
                mine,
                created_at: at,
                body: BodyView::Commitment {
                    commitment: cid.clone(),
                },
            };
            if let Some(new) = self.insert(view.clone()).await {
                self.spawn_refresh(new);
            }
            let _ = self
                .inner
                .events
                .send(AppEvent::Message { message: view })
                .await;
        } else if !mine {
            self.touch(&conv, at, format!("⚖ {}", name(kind).to_lowercase()))
                .await;
        }
        if let Some(view) = self.commitment(&conv, &cid).await {
            // A closing act on a paid commitment: the performer redeems shortly after; let the
            // wallet view catch up with the mint (NUT-07).
            if view.settlement.is_some()
                && self.inner.wallet.is_some()
                && matches!(kind, K_SATISFY | K_RULING)
            {
                let this = self.clone();
                tokio::spawn(async move {
                    for secs in [2, 5] {
                        tokio::time::sleep(Duration::from_secs(secs)).await;
                        this.emit_wallet().await;
                    }
                });
            }
            let _ = self
                .inner
                .events
                .send(AppEvent::Commitment { commitment: view })
                .await;
        }
    }

    /// Activity from the other side that isn't a thread message (e.g. a REPORT).
    async fn touch(&self, conv: &str, at: u64, summary: String) {
        let contact = {
            let mut st = self.inner.state.lock().await;
            let active = st.active.as_deref() == Some(conv);
            let Some(c) = st.contacts.get_mut(conv) else {
                return;
            };
            if c.last_at.is_none_or(|t| at >= t) {
                c.last_at = Some(at);
                c.last_message = Some(summary);
            }
            if !active {
                c.unread += 1;
            }
            c.clone()
        };
        let _ = self.inner.events.send(AppEvent::Contact { contact }).await;
    }

    // --- internals ---

    /// The consent gate (ADR 0004). Fails closed: a contact whose kind isn't known yet is
    /// discovered first, and one that can't be discovered can't be messaged.
    async fn ensure_consent(&self, pk: &PublicKey) -> Result<(), AppError> {
        let key = pk.to_hex();
        let pending = {
            let st = self.inner.state.lock().await;
            st.contacts
                .get(&key)
                .is_none_or(|c| c.discovery == view::Discovery::Pending)
        };
        let contact = if pending {
            self.refresh(&key).await.map_err(|e| {
                AppError::Invalid(format!(
                    "couldn't check whether this contact is an agent ({e}); try again"
                ))
            })?
        } else {
            let st = self.inner.state.lock().await;
            st.contacts.get(&key).cloned().expect("checked above")
        };
        if contact.needs_consent() {
            return Err(AppError::Invalid(if contact.is_agent {
                "review and accept this agent's data handling first".into()
            } else {
                "no profile found for this key: review and accept before contacting it".into()
            }));
        }
        Ok(())
    }

    async fn record_outgoing(&self, to: PublicKey, id: EventId, body: Body) -> MessageView {
        let m = Inbound {
            id,
            sender: self.me(),
            created_at: Timestamp::now(),
            recipients: vec![to],
            body,
        };
        let view = self.ctx().message(&self.me(), &m);
        self.insert(view.clone()).await;
        view
    }

    async fn ingest(&self, m: Inbound) {
        if let Body::Act(act) = &m.body {
            // Grouping by commitment happens in `on_act`; this is only the fallback.
            let conv = m.counterparty(&self.me()).to_hex();
            self.on_act(&conv, (**act).clone()).await;
            return;
        }
        let view = self.ctx().message(&self.me(), &m);
        if let Some(new_contact) = self.insert(view.clone()).await {
            self.spawn_refresh(new_contact);
        }
        let _ = self
            .inner
            .events
            .send(AppEvent::Message { message: view })
            .await;
    }

    /// Insert a message; returns the counterparty key if it is a new contact.
    async fn insert(&self, view: MessageView) -> Option<String> {
        let mut st = self.inner.state.lock().await;
        if !st.seen.insert(view.id.clone()) {
            return None;
        }
        let conv = view.conversation.clone();
        let mut new_contact = None;
        let active = st.active.as_deref() == Some(conv.as_str());
        let contact = st.contacts.entry(conv.clone()).or_insert_with(|| {
            new_contact = Some(conv.clone());
            ContactView::new(PublicKey::parse(&conv).expect("valid hex from view"))
        });
        if contact.last_at.is_none_or(|t| view.created_at >= t) {
            contact.last_at = Some(view.created_at);
            contact.last_message = Some(view.body.summary());
        }
        if !view.mine && !active {
            contact.unread += 1;
        }
        let contact = contact.clone();

        let list = st.messages.entry(conv).or_default();
        let pos = list.partition_point(|m| m.created_at <= view.created_at);
        list.insert(pos, view);
        drop(st);

        let _ = self.inner.events.send(AppEvent::Contact { contact }).await;
        if new_contact.is_some() {
            self.persist().await;
        }
        new_contact
    }

    fn spawn_refresh(&self, pubkey: String) {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.refresh(&pubkey).await {
                tracing::debug!(error = %e, %pubkey, "profile refresh failed");
            }
        });
    }

    async fn refresh(&self, pubkey: &str) -> Result<ContactView, AppError> {
        let pk = parse_pk(pubkey)?;
        let info = self.inner.session.peer_info(pk).await?;
        let contact = {
            let mut st = self.inner.state.lock().await;
            st.commands.insert(pk.to_hex(), info.commands.clone());
            st.bindings
                .insert(pk.to_hex(), info.acn_binding_events.clone());
            if let Some(svc) = &info.acn_svc {
                st.svc.insert(pk.to_hex(), svc.clone());
            }
            let accepted = st.accepted.contains(&pk.to_hex());
            let c = st
                .contacts
                .entry(pk.to_hex())
                .or_insert_with(|| ContactView::new(pk));
            c.apply_peer_info(info, accepted);
            c.clone()
        };
        self.persist().await;
        let _ = self
            .inner
            .events
            .send(AppEvent::Contact {
                contact: contact.clone(),
            })
            .await;
        Ok(contact)
    }

    async fn persist(&self) {
        let _ = self.persist_checked().await;
    }

    /// Save, and say if it failed: callers that must not go on without the saved state (a
    /// payment preimage, a held-out seed) stop before publishing anything (A18).
    async fn persist_checked(&self) -> Result<(), AppError> {
        let st = self.inner.state.lock().await;
        let data = store::Stored {
            contacts: st
                .contacts
                .values()
                .map(|c| store::StoredContact {
                    pubkey: c.pubkey.clone(),
                    is_agent: c.is_agent,
                    accepted: st.accepted.contains(&c.pubkey),
                    resolved: c.discovery == view::Discovery::Resolved,
                })
                .collect(),
            secrets: st.secrets.clone(),
            seeds: st.seeds.clone(),
            keys: st.keys.clone(),
            demo_epoch: st.demo_epoch,
            cleared: st.cleared.iter().cloned().collect(),
        };
        drop(st);
        self.inner.store.save(&data).map_err(|e| {
            AppError::Invalid(format!(
                "couldn't save the payment secret or seed ({e}); nothing was sent"
            ))
        })
    }
}

/// Observed, not just declared (Core §8.6): before locking money at a mint, check that the mint
/// itself (NUT-06 info) supports hash-time locks (NUT-10/11/14) and DLEQ (NUT-12), and for
/// contingent the state check (NUT-07) that reveals the preimage to the payer.
async fn ensure_rail(w: &CashuWallet, mint: &str, contingent: bool) -> Result<(), AppError> {
    let nuts = w
        .observed_nuts()
        .await
        .map_err(|e| AppError::Invalid(format!("rail_incapable: {e}")))?;
    let mut need = vec![10u8, 11, 12, 14];
    if contingent {
        need.push(7);
    }
    match need.iter().find(|n| !nuts.contains(n)) {
        Some(n) => Err(AppError::Invalid(format!(
            "rail_incapable: mint {mint} doesn't support NUT-{n:02}"
        ))),
        None => Ok(()),
    }
}

fn parse_pk(s: &str) -> Result<PublicKey, AppError> {
    let s = s.trim();
    let s = s.strip_prefix("nostr:").unwrap_or(s);
    PublicKey::parse(s).map_err(|e| AppError::BadKey(e.to_string()))
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settlement_policy_filters_the_verifier_trigger() {
        let v = json!({"settlement_trigger": "verifier", "settlement_verifier": "aa"});
        let plain = json!({});
        assert!(SettlementPolicy::default().check(&v).is_ok());
        let refuse = SettlementPolicy {
            refuse_verifier: true,
            ..Default::default()
        };
        assert!(refuse.check(&v).is_err());
        assert!(
            refuse.check(&plain).is_ok(),
            "the default trigger is never filtered"
        );
        let trusted = SettlementPolicy {
            trusted_verifiers: vec!["bb".into()],
            ..Default::default()
        };
        assert!(trusted.check(&v).is_err());
        let trusted = SettlementPolicy {
            trusted_verifiers: vec!["aa".into()],
            ..Default::default()
        };
        assert!(trusted.check(&v).is_ok());

        let k = json!({"settlement_trigger": "contingent", "settlement_verifier": "aa"});
        assert!(SettlementPolicy::default().check(&k).is_ok());
        let refuse = SettlementPolicy {
            refuse_contingent: true,
            ..Default::default()
        };
        assert!(refuse.check(&k).is_err() && refuse.check(&v).is_ok());
        assert!(
            trusted.check(&k).is_ok(),
            "the trusted set covers contingent too"
        );
    }

    /// CR-002 S-10, as a binding check: contingent needs `reveals_preimage_to_payer`.
    #[test]
    fn rail_capability_is_read_from_the_service_advertisement() {
        let svc = json!({"mints": [
            {"url": "http://127.0.0.1:3338", "capabilities": ["hashlock", "reveals_preimage_to_payer"]},
            {"url": "https://other", "capabilities": ["hashlock"]}
        ]});
        assert!(mint_has_capability(
            &svc,
            "http://127.0.0.1:3338/",
            "reveals_preimage_to_payer"
        ));
        assert!(!mint_has_capability(
            &svc,
            "https://other",
            "reveals_preimage_to_payer"
        ));
        assert!(!mint_has_capability(
            &json!({"mints": ["https://legacy"]}),
            "https://legacy",
            "hashlock"
        ));
    }
}
