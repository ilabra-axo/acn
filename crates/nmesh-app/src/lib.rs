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
pub use acn_wallet::WalletView;
pub use commitments::{BindingView, CommitmentView};
use nmesh_wallet::CashuWallet;
pub use view::{
    AcnPeerView, AcnToolView, ArgView, BodyView, CommandView, ContactView, Identity, MessageView,
    RenderContext, RendererView,
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
                v.consented = !c.is_agent || c.accepted;
                if c.accepted {
                    accepted.insert(v.pubkey.clone());
                }
                contacts.insert(v.pubkey.clone(), v);
            }
        }

        let (tx, rx) = mpsc::channel(512);
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
                    demo_epoch: stored.demo_epoch,
                    cleared: stored.cleared.iter().cloned().collect(),
                }),
                events: tx,
                clock_offset: AtomicI64::new(0),
                wallet,
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
        if let Some(c) = st.contacts.get_mut(&pk) {
            c.unread = 0;
        }
        Ok(st.messages.get(&pk).cloned().unwrap_or_default())
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
        let (profile, terms) = match &template {
            Some(t) => {
                let hashlock = t.hashlock.then(|| {
                    let s = Keys::generate().secret_key().secret_bytes();
                    let h = acn_core::jcs::sha256_hex(&s);
                    secret = Some(hex32(&s));
                    h
                });
                let terms =
                    acn_core::draft::instantiate(t, &def.command, input, now, hashlock.as_deref())
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
        // Lock the price to the performer (Core.md §8.2 steps 1-2).
        let token = match (&secret, terms.get("payment").filter(|p| p.is_object())) {
            (Some(_), Some(pay)) if terms["price"]["amount"].as_u64().unwrap_or(0) > 0 => {
                let mint = pay["mint"].as_str().unwrap_or_default();
                if mint.starts_with("sim:") {
                    None
                } else {
                    let w = self.wallet_for(mint)?;
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
        let mut draft = Draft::request(&key, &profile, &terms, ct.as_deref())
            .map_err(|e| AppError::Invalid(e.to_string()))?;
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
        let act = self.inner.session.sign_act(&draft, None)?;
        let id = act.id.clone();
        if let Some(s) = secret {
            self.inner.state.lock().await.secrets.insert(id.clone(), s);
            self.persist().await;
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
    pub async fn refund(&self, pubkey: &str, commitment: &str) -> Result<WalletView, AppError> {
        let conv = parse_pk(pubkey)?.to_hex();
        let w = self
            .inner
            .wallet
            .as_ref()
            .ok_or_else(|| AppError::Invalid("no wallet configured".into()))?;
        let events = self.fold_input(&conv).await;
        let me = self.me().to_hex();
        match acn_wallet::settlement::advise(&events, commitment, &me, self.now()) {
            Some(acn_wallet::settlement::Advice::Refund { locktime, .. }) => {
                if Timestamp::now().as_secs() <= locktime {
                    return Err(AppError::Invalid(format!(
                        "refundable after the locktime ({locktime}, unix time); the mint enforces it"
                    )));
                }
            }
            _ => {
                return Err(AppError::Invalid(
                    "nothing to refund: the commitment is open or was satisfied".into(),
                ))
            }
        }
        let token = w
            .token_for(commitment)
            .ok_or_else(|| AppError::Invalid("no locked token for this commitment".into()))?;
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

    /// The scripted next step for the requester (presenter guidance), if any.
    pub async fn hint(&self, pubkey: &str, commitment: &str) -> Result<Option<Hint>, AppError> {
        let conv = parse_pk(pubkey)?.to_hex();
        let events = self.fold_input(&conv).await;
        Ok(acn_sim::requester_hint(&events, commitment, self.now()))
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
        let preimage = self
            .inner
            .state
            .lock()
            .await
            .secrets
            .get(commitment)
            .cloned();
        let (draft, not_before) =
            acn_sim::requester_draft(&events, commitment, &action, preimage.as_deref())
                .ok_or_else(|| AppError::Invalid("that act isn't possible right now".into()))?;
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
            let fresh = st
                .acts
                .entry(conv.clone())
                .or_default()
                .insert(act.id.clone(), act)
                .is_none();
            (conv, fresh)
        };
        if !fresh {
            return;
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

    async fn ensure_consent(&self, pk: &PublicKey) -> Result<(), AppError> {
        let st = self.inner.state.lock().await;
        match st.contacts.get(&pk.to_hex()) {
            Some(c) if c.is_agent && !c.consented => Err(AppError::Invalid(
                "review and accept this agent's data handling first".into(),
            )),
            _ => Ok(()),
        }
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
        let st = self.inner.state.lock().await;
        let data = store::Stored {
            contacts: st
                .contacts
                .values()
                .map(|c| store::StoredContact {
                    pubkey: c.pubkey.clone(),
                    is_agent: c.is_agent,
                    accepted: st.accepted.contains(&c.pubkey),
                })
                .collect(),
            secrets: st.secrets.clone(),
            demo_epoch: st.demo_epoch,
            cleared: st.cleared.iter().cloned().collect(),
        };
        drop(st);
        self.inner.store.save(&data);
    }
}

fn parse_pk(s: &str) -> Result<PublicKey, AppError> {
    let s = s.trim();
    let s = s.strip_prefix("nostr:").unwrap_or(s);
    PublicKey::parse(s).map_err(|e| AppError::BadKey(e.to_string()))
}
