//! `acn-binding`: the ACN Binding API (open decision D-10, proposal in
//! docs/harness-binding-API.md). Any agent or harness joins ACN by implementing these
//! interfaces; [`Performer`] runs a performer's lifecycle on them with the reference validator.
//!
//! | Interface | Required | Here |
//! | --- | --- | --- |
//! | [`Signer`] | always | key custody, signing, NIP-44 decryption |
//! | [`Inbox`] | always | delivery of acts to the counterparty (the host feeds inbound events to [`Performer::on_event`]) |
//! | [`Executor`] | performers | quote and run |
//! | [`Evidence`] | performers | artifacts and derivation records by hash |
//! | [`Wallet`] | when value moves | Cashu: verify, redeem, lock, refund |
//! | [`Gate`] | optional | pre-signing policy |
//! | [`Verifier`] | optional | attestations |
//! | [`TermsAuthor`] | optional | drafting and critique ([`ReferenceAuthor`] wraps `acn-author`) |
//!
//! The traits are object-safe; asynchronous operations return a boxed future, so a binding can
//! hold `Arc<dyn Executor>` and the like. Nothing here does I/O itself.

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use acn_core::anomaly::{Anomaly, Severity};
use acn_core::draft::Draft;
use acn_core::kinds::*;
use acn_core::{fold, Event, Phase, Profile, Terms};
use serde::Serialize;
use serde_json::{json, Value};

pub use acn_wallet::Verified;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The conformance levels (docs/harness-binding-API.md §2), as declared in `K_SVC`
/// `binding_level`. A binding may claim only a level whose suite it passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Level {
    L0,
    L1,
    L2,
    L3,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::L0 => "L0",
            Level::L1 => "L1",
            Level::L2 => "L2",
            Level::L3 => "L3",
        }
    }
}

// ---------- the interfaces ----------

/// Key custody and signing. Enforces the binding's scope locally.
pub trait Signer: Send + Sync {
    /// x-only public key, hex.
    fn pubkey(&self) -> String;
    /// Sign with `created_at ≥ not_before` (V-15).
    fn sign(&self, draft: &Draft, not_before: Option<u64>) -> Result<Event, String>;
    /// NIP-44 decrypt a payload sent to this key by `from` (tokens, seeds, escrowed secrets).
    fn decrypt(&self, from: &str, ciphertext: &str) -> Result<String, String>;
}

/// Delivery of signed acts. Inbound events reach the binding through [`Performer::on_event`].
pub trait Inbox: Send + Sync {
    /// Send an act to a counterparty (privately, Core §5.5, or publicly; the transport decides).
    fn send<'a>(&'a self, to: &'a str, act: &'a Event) -> BoxFut<'a, Result<(), String>>;
}

/// The answer to a request.
#[derive(Debug, Clone, PartialEq)]
pub enum Quote {
    Accept {
        p_on_time: f64,
    },
    /// Better-specified terms, through an ordinary `COUNTER` (rule 3). Profile N only.
    Counter {
        terms: Value,
        reason: String,
    },
    Decline {
        reason: String,
        note: String,
    },
}

/// What the executor is asked to do.
#[derive(Debug, Clone)]
pub struct Job {
    pub commitment: String,
    pub terms: Terms,
    pub tool: Option<String>,
    pub input: Value,
}

/// An artifact the work produced; stored by [`Evidence`] and listed in the `REPORT`'s `x` tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// e.g. `output`, `log`, a file name.
    pub role: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// What the work returned.
#[derive(Debug, Clone, Default)]
pub struct Output {
    /// Small inline result (≤ 16 KiB, else it becomes an artifact).
    pub result: Option<Value>,
    pub contract: Option<String>,
    pub artifacts: Vec<Artifact>,
    /// Derivation record `tools[]` entries: tool, service and model identifiers with versions.
    pub tools: Vec<Value>,
    /// Hash of the execution log, if the executor kept one.
    pub log: Option<Vec<u8>>,
}

/// Maps a commitment to work.
pub trait Executor: Send + Sync {
    fn quote(&self, terms: &Terms, profile: Profile) -> Quote;
    fn run(&self, job: Job) -> BoxFut<'_, Result<Output, String>>;
}

/// Stores artifacts by hash (Blossom) and replicates them.
pub trait Evidence: Send + Sync {
    /// Store `bytes`; returns their sha256 (hex).
    fn put(&self, bytes: Vec<u8>, media_type: String) -> BoxFut<'_, Result<String, String>>;
    /// Where the blobs can be fetched (published as the binding's server list).
    fn servers(&self) -> Vec<String>;
}

/// Cashu operations.
pub trait Wallet: Send + Sync {
    /// The mint this wallet uses.
    fn mint(&self) -> String;
    /// The mint's rail capabilities as declared (Core §8.6).
    fn capabilities(&self) -> Vec<String>;
    /// DLEQ, lock shape and spent state (Core §8.2 step 3).
    fn verify<'a>(&'a self, token: &'a str) -> BoxFut<'a, Result<Verified, String>>;
    fn redeem<'a>(
        &'a self,
        token: &'a str,
        preimage: &'a str,
        commitment: &'a str,
    ) -> BoxFut<'a, Result<u64, String>>;
}

/// A gate's decision on an act draft.
#[derive(Debug, Clone, PartialEq)]
pub enum GateDecision {
    Allow,
    Deny { reason: String },
}

/// Pre-signing policy (rule 1: never sign what the gate denied or hasn't decided).
pub trait Gate: Send + Sync {
    fn evaluate(&self, draft: &Draft, commitment: &acn_core::Commitment) -> GateDecision;
}

/// Produces attestations (L3).
pub trait Verifier: Send + Sync {
    /// The attestation draft for `item` (a condition or disqualifier id) on `report`, if any.
    fn attest<'a>(
        &'a self,
        report: &'a Event,
        terms: &'a Terms,
        item: &'a str,
    ) -> BoxFut<'a, Result<Option<Draft>, String>>;
}

/// Drafts and critiques terms. Off-protocol: its output is terms like any other.
pub trait TermsAuthor: Send + Sync {
    fn draft(
        &self,
        intent: &acn_author::Intent,
        templates: &[acn_author::TemplateCandidate],
    ) -> acn_author::RequestDraft;
    fn critique(&self, terms: &Terms) -> Vec<acn_author::Finding>;
}

/// The reference Terms Author: `acn-author`'s drafter and linter.
#[derive(Debug, Clone, Default)]
pub struct ReferenceAuthor {
    pub config: acn_author::RequesterConfig,
    pub lint: acn_author::LintContext,
}

impl TermsAuthor for ReferenceAuthor {
    fn draft(
        &self,
        intent: &acn_author::Intent,
        templates: &[acn_author::TemplateCandidate],
    ) -> acn_author::RequestDraft {
        acn_author::draft_request(intent, templates, &self.config)
    }
    fn critique(&self, terms: &Terms) -> Vec<acn_author::Finding> {
        acn_author::lint(terms, &self.lint)
    }
}

// ---------- the performer driver ----------

/// Acts per commitment, in arrival order (the fold is order-independent anyway).
#[derive(Default)]
pub struct Store {
    by_commitment: Mutex<BTreeMap<String, BTreeMap<String, Event>>>,
}

impl Store {
    /// Add an event; returns its commitment id.
    pub fn add(&self, e: Event) -> String {
        let c = if e.kind == K_REQUEST {
            e.id.clone()
        } else {
            e.tags
                .iter()
                .find(|t| t.first().map(String::as_str) == Some("c"))
                .and_then(|t| t.get(1).cloned())
                .unwrap_or_default()
        };
        self.by_commitment
            .lock()
            .unwrap()
            .entry(c.clone())
            .or_default()
            .insert(e.id.clone(), e);
        c
    }
    pub fn events(&self, c: &str) -> Vec<Event> {
        self.by_commitment
            .lock()
            .unwrap()
            .get(c)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }
    pub fn ids(&self) -> Vec<String> {
        self.by_commitment.lock().unwrap().keys().cloned().collect()
    }
}

/// What an inbound event did (`Inbox.on_event` in the proposal).
#[derive(Debug, Clone, Default)]
pub struct StateChange {
    pub commitment: String,
    pub from: Option<Phase>,
    pub to: Option<Phase>,
    /// Anomalies the validator found on this event.
    pub anomalies: Vec<Anomaly>,
    /// Acts the binding signed and sent in response.
    pub sent: Vec<Event>,
    /// Sats redeemed in response (the requester's SATISFY revealed the preimage).
    pub redeemed: u64,
}

/// Inline results larger than this become an artifact (Core §6.2: ≤ 16 KiB inline).
const INLINE_MAX: usize = 16 * 1024;

/// A performer: its interfaces and the lifecycle that runs on them. Profiles T0 and T1 under
/// the requester trigger (conformance level L0); anything else is declined.
pub struct Performer {
    pub signer: Arc<dyn Signer>,
    pub inbox: Arc<dyn Inbox>,
    pub executor: Arc<dyn Executor>,
    pub evidence: Option<Arc<dyn Evidence>>,
    pub wallet: Option<Arc<dyn Wallet>>,
    pub gate: Option<Arc<dyn Gate>>,
    /// The advertised minimum price (`K_SVC` `price.min`): cheaper requests are declined.
    pub min_price_sat: u64,
    pub store: Store,
    redeemed: Mutex<HashSet<String>>,
    /// Commitments the driver is acting on (an act is in flight).
    busy: Mutex<HashSet<String>>,
}

impl Performer {
    pub fn new(
        signer: Arc<dyn Signer>,
        inbox: Arc<dyn Inbox>,
        executor: Arc<dyn Executor>,
    ) -> Self {
        Self {
            signer,
            inbox,
            executor,
            evidence: None,
            wallet: None,
            gate: None,
            min_price_sat: 0,
            store: Store::default(),
            redeemed: Mutex::new(HashSet::new()),
            busy: Mutex::new(HashSet::new()),
        }
    }

    pub fn with_evidence(mut self, e: Arc<dyn Evidence>) -> Self {
        self.evidence = Some(e);
        self
    }

    pub fn with_wallet(mut self, w: Arc<dyn Wallet>) -> Self {
        self.wallet = Some(w);
        self
    }

    pub fn with_min_price(mut self, sat: u64) -> Self {
        self.min_price_sat = sat;
        self
    }

    pub fn with_gate(mut self, g: Arc<dyn Gate>) -> Self {
        self.gate = Some(g);
        self
    }

    /// The level these interfaces can reach (the suite decides whether it is met): L0 needs
    /// Evidence, and a Wallet for priced calls.
    pub fn level(&self) -> Option<Level> {
        self.evidence.as_ref().map(|_| Level::L0)
    }

    /// Record an event without acting (history replay).
    pub fn ingest(&self, e: Event) -> String {
        self.store.add(e)
    }

    /// Fold an inbound event and make the performer's move, if it has one.
    pub async fn on_event(&self, e: Event) -> StateChange {
        let id = e.id.clone();
        let c = self.store.add(e);
        let before = self.phase(&c);
        let mut change = self.drive(&c).await;
        change.from = before;
        let out = fold(&self.store.events(&c), &BTreeMap::new());
        change.anomalies = out
            .anomalies
            .into_iter()
            .filter(|a| a.event == id)
            .collect();
        change
    }

    fn phase(&self, c: &str) -> Option<Phase> {
        let out = fold(&self.store.events(c), &BTreeMap::new());
        out.commitments
            .get(c)
            .and_then(|x| x.state.as_ref())
            .map(|s| s.phase)
    }

    /// Re-evaluate commitment `c` and act. Safe to call repeatedly: it acts only on a fresh
    /// head, and never twice on the same one.
    pub async fn drive(&self, c: &str) -> StateChange {
        let mut change = StateChange {
            commitment: c.to_string(),
            ..Default::default()
        };
        if !self.busy.lock().unwrap().insert(c.to_string()) {
            // Another drive is acting on `c`; it re-folds before it finishes.
            return change;
        }
        // Re-fold while events arrive during a move (the other caller returned early above).
        for _ in 0..8 {
            let before = self.store.events(c).len();
            let ours = change.sent.len();
            if let Err(e) = self.drive_inner(c, &mut change).await {
                tracing::warn!(commitment = %c, error = %e, "binding: no move");
            }
            let arrived = self.store.events(c).len() - before - (change.sent.len() - ours);
            if arrived == 0 {
                break;
            }
        }
        self.busy.lock().unwrap().remove(c);
        change.to = self.phase(c);
        change
    }

    async fn drive_inner(&self, c: &str, change: &mut StateChange) -> Result<(), String> {
        let me = self.signer.pubkey();
        let events = self.store.events(c);
        let out = fold(&events, &BTreeMap::new());
        let Some(cm) = out.commitments.get(c) else {
            return Ok(());
        };
        let (Some(st), Some(terms)) = (&cm.state, &cm.terms) else {
            return Ok(());
        };
        if cm.performer != me {
            return Ok(());
        }
        match st.phase {
            Phase::Requested => {
                let draft = self.answer(c, cm, st, terms, &events).await;
                self.sign_and_send(c, cm, &draft, st.head_at, change).await
            }
            Phase::Satisfied => {
                change.redeemed = self.redeem(c, &events).await?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The move on a fresh REQUEST: decline, or do the work and REPORT (T0/T1).
    async fn answer(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        st: &acn_core::State,
        terms: &Terms,
        events: &[Event],
    ) -> Draft {
        let parties = [cm.requester.as_str(), cm.performer.as_str()];
        let decline = |reason: &str, note: String| {
            Draft::closing(K_DECLINE, c, &st.head, &parties, reason)
                .content(json!({ "reason": reason, "note": note }))
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Some((reason, note)) = self.refusal(terms, cm.profile, now) {
            return decline(reason, note);
        }
        match self.executor.quote(terms, cm.profile) {
            Quote::Accept { .. } => {}
            Quote::Decline { reason, note } => return decline(&reason, note),
            Quote::Counter { .. } => {
                return decline(
                    "profile_unsupported",
                    "this binding answers T0/T1 calls; counters need profile N".into(),
                )
            }
        }
        if terms.price.amount > 0 {
            if let Err(why) = self.check_token(c, cm, terms, events).await {
                return decline("payment_invalid", why);
            }
        }
        let job = Job {
            commitment: c.to_string(),
            terms: terms.clone(),
            tool: terms.deliverable.tool.clone(),
            input: terms.deliverable.input.clone().unwrap_or_else(|| json!({})),
        };
        let started = now;
        let output = match self.executor.run(job).await {
            Ok(o) => o,
            Err(e) => return decline("tool_error", e),
        };
        match self.report(c, st, &parties, terms, output, started).await {
            Ok(d) => d,
            Err(e) => decline("evidence_unavailable", e),
        }
    }

    /// What L0 declines before running anything (docs/harness-binding-API.md §1).
    fn refusal(&self, terms: &Terms, profile: Profile, now: u64) -> Option<(&'static str, String)> {
        if !terms.settlement_trigger.is_requester() {
            return Some((
                "trigger_unsupported",
                "this binding settles through the requester only".into(),
            ));
        }
        if profile == Profile::N {
            return Some((
                "profile_unsupported",
                "this binding offers T0 and T1".into(),
            ));
        }
        if terms.price.amount < self.min_price_sat {
            return Some((
                "price_too_low",
                format!("this binding's price is {} sat", self.min_price_sat),
            ));
        }
        if terms.price.amount > 0 {
            let Some(w) = &self.wallet else {
                return Some((
                    "payment_unsupported",
                    "this binding has no wallet: free calls only".into(),
                ));
            };
            let pay = terms.payment.as_ref();
            if pay.is_none_or(|p| p.mint.trim_end_matches('/') != w.mint().trim_end_matches('/')) {
                return Some((
                    "mint_unsupported",
                    format!("this binding is paid at {}", w.mint()),
                ));
            }
            if profile != Profile::T1 || pay.is_none_or(|p| p.hashlock.is_none()) {
                return Some((
                    "payment_unsupported",
                    "paid calls are T1 with a hash-locked token".into(),
                ));
            }
        }
        if let Some(why) = terms.unclosed() {
            return Some(("terms_unclosed", why));
        }
        if terms.has_instances() {
            return Some((
                "instances_unsupported",
                "held-out instances need a performer seed, which an L0 binding doesn't keep".into(),
            ));
        }
        if !terms.disqualifiers.is_empty() {
            return Some((
                "disqualifiers_unsupported",
                "an L0 binding doesn't take terms with disqualifiers".into(),
            ));
        }
        if now > terms.deadline {
            return Some(("expired", "the request's deadline has passed".into()));
        }
        None
    }

    /// The requester's token, NIP-44 to us in the REQUEST: decrypt, verify with the mint, check
    /// it against the terms (amount, hashlock, receiver, locktime, unspent).
    async fn check_token(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        terms: &Terms,
        events: &[Event],
    ) -> Result<String, String> {
        let w = self.wallet.as_ref().ok_or("no wallet")?;
        let ct =
            acn_wallet::settlement::request_token(events, c).ok_or("no token in the request")?;
        let token = self
            .signer
            .decrypt(&cm.requester, &ct)
            .map_err(|_| "the token is not encrypted to this binding".to_string())?;
        let v = w.verify(&token).await?;
        acn_wallet::settlement::check_payment(terms, &v, &self.signer.pubkey())?;
        Ok(token)
    }

    /// REPORT with evidence: artifacts to Blossom, a derivation record, the inline result.
    async fn report(
        &self,
        c: &str,
        st: &acn_core::State,
        parties: &[&str],
        terms: &Terms,
        mut output: Output,
        started: u64,
    ) -> Result<Draft, String> {
        if let Some(r) = &output.result {
            let bytes = serde_json::to_vec(r).unwrap_or_default();
            if bytes.len() > INLINE_MAX {
                output.artifacts.push(Artifact {
                    role: "result".into(),
                    media_type: "application/json".into(),
                    bytes,
                });
                output.result = Some(json!({"note": "the result is an artifact (over 16 KiB)"}));
            }
        }
        let Some(ev) = &self.evidence else {
            // No Evidence store: the inline result is the whole report (and the binding
            // claims no conformance level, see `level`).
            if output.result.is_none() && !output.artifacts.is_empty() {
                return Err(
                    "the work produced only artifacts, and this binding has no Evidence store"
                        .into(),
                );
            }
            return Ok(Draft::report(
                c,
                &st.head,
                parties,
                output.result,
                output.contract.as_deref(),
                &[],
            ));
        };
        let mut outputs = Vec::new();
        for a in output.artifacts {
            let h = ev.put(a.bytes, a.media_type.clone()).await?;
            outputs.push(json!({"hash": h, "role": a.role, "type": a.media_type}));
        }
        if let Some(r) = &output.result {
            let h = acn_core::jcs::hash(r).map_err(|e| e.to_string())?;
            outputs.push(json!({"hash": h, "role": "inline_result"}));
        }
        let input = terms.deliverable.input.clone().unwrap_or_else(|| json!({}));
        let mut record = json!({
            "inputs": [{"hash": acn_core::jcs::hash(&input).map_err(|e| e.to_string())?, "role": "input"}],
            "outputs": outputs,
            "tools": output.tools,
            "subcommitments": [],
            "started": started,
            "ended": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        });
        if let Some(log) = output.log {
            record["log"] = json!(ev.put(log, "text/plain".into()).await?);
        }
        let canonical = acn_core::jcs::canonical(&record).map_err(|e| e.to_string())?;
        let derivation = ev
            .put(canonical.into_bytes(), "application/json".into())
            .await?;
        let artifacts: Vec<String> = record["outputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|o| o["role"] != "inline_result")
            .filter_map(|o| o["hash"].as_str().map(str::to_owned))
            .collect();
        let refs: Vec<&str> = artifacts.iter().map(String::as_str).collect();
        Ok(Draft::report(
            c,
            &st.head,
            parties,
            output.result,
            output.contract.as_deref(),
            &refs,
        )
        .field("derivation", json!(derivation)))
    }

    /// Rule 1 (gate) and rule 5 (fold before signing): sign only what the validator accepts.
    async fn sign_and_send(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        draft: &Draft,
        not_before: u64,
        change: &mut StateChange,
    ) -> Result<(), String> {
        if let Some(g) = &self.gate {
            if let GateDecision::Deny { reason } = g.evaluate(draft, cm) {
                return Err(format!("gate denied: {reason}"));
            }
        }
        let act = self.signer.sign(draft, Some(not_before))?;
        let mut events = self.store.events(c);
        events.push(act.clone());
        let out = fold(&events, &BTreeMap::new());
        if let Some(a) = out
            .anomalies
            .iter()
            .find(|a| a.event == act.id && a.code.severity() == Severity::Reject)
        {
            return Err(format!(
                "refused locally: the validator would reject it ({})",
                a.code.as_str()
            ));
        }
        self.store.add(act.clone());
        self.inbox.send(&cm.requester, &act).await?;
        tracing::info!(commitment = %c, act = acn_core::kinds::name(act.kind), "binding: sent");
        change.sent.push(act);
        Ok(())
    }

    /// Satisfied with a preimage: redeem the requester's token.
    async fn redeem(&self, c: &str, events: &[Event]) -> Result<u64, String> {
        let Some(w) = &self.wallet else { return Ok(0) };
        if self.redeemed.lock().unwrap().contains(c) {
            return Ok(0);
        }
        let now = u64::MAX / 2;
        let Some(acn_wallet::settlement::Advice::Redeem { preimage, .. }) =
            acn_wallet::settlement::advise(events, c, &self.signer.pubkey(), now)
        else {
            return Ok(0);
        };
        let out = fold(events, &BTreeMap::new());
        let requester = out
            .commitments
            .get(c)
            .map(|x| x.requester.clone())
            .unwrap_or_default();
        let ct = acn_wallet::settlement::request_token(events, c).ok_or("no token")?;
        let token = self.signer.decrypt(&requester, &ct)?;
        self.redeemed.lock().unwrap().insert(c.to_string());
        let got = w.redeem(&token, &preimage, c).await?;
        tracing::info!(commitment = %c, sat = got, "binding: redeemed");
        Ok(got)
    }
}

/// The terms offer for a call (`K_SVC` `terms_template`, Core §7): T0 when free; T1 with a
/// hash-locked token at `mint` when priced.
pub fn call_offer(
    tool: &str,
    price_sat: u64,
    mint: Option<&str>,
    deadline_secs: u64,
    review_window_secs: u64,
) -> Value {
    let mut terms = acn_core::draft::tool_call_terms(tool, json!({}), 0, 0, price_sat);
    let paid = price_sat > 0 && mint.is_some();
    if paid {
        terms["payment"] = json!({"mint": mint, "keysets": []});
    }
    serde_json::to_value(acn_core::draft::Template {
        profile: if paid { "T1" } else { "T0" }.into(),
        deadline_secs,
        review_window_secs,
        terms,
        hashlock: paid,
        locktime_margin_secs: 24 * 60 * 60,
        requester_seed_to: None,
    })
    .unwrap_or_default()
}

/// `K_SVC` content for a binding: its tools (each with its terms offer, if any), profiles,
/// mints with their capabilities, Blossom servers and conformance level.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ServiceAd {
    pub tools: Vec<Value>,
    pub profiles: Vec<String>,
    pub min_price_sat: u64,
    /// `(url, capabilities)`.
    pub mints: Vec<(String, Vec<String>)>,
    pub blossom: Vec<String>,
    pub binding_level: Option<Level>,
    pub data_handling: Value,
    pub deadline_secs: u64,
    pub review_window_secs: u64,
}

impl ServiceAd {
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "v": VERSION,
            "tools": self.tools,
            "profiles": self.profiles,
            "price": {"min": self.min_price_sat, "unit": "sat"},
            "mints": self.mints.iter().map(|(u, c)| json!({"url": u, "capabilities": c})).collect::<Vec<_>>(),
            "transport": "nostr",
            "terms_defaults": {"deadline_secs": self.deadline_secs, "review_window_secs": self.review_window_secs},
            "silence_threshold_sat": 0,
            "data_handling": self.data_handling,
        });
        if !self.blossom.is_empty() {
            v["blossom"] = json!(self.blossom);
        }
        if let Some(l) = self.binding_level {
            v["binding_level"] = json!(l.as_str());
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acn_core::draft::public_key;

    struct Key([u8; 32]);
    impl Signer for Key {
        fn pubkey(&self) -> String {
            public_key(&self.0).unwrap()
        }
        fn sign(&self, d: &Draft, nb: Option<u64>) -> Result<Event, String> {
            d.sign(&self.0, nb.unwrap_or(0) + 1)
                .map_err(|e| e.to_string())
        }
        fn decrypt(&self, _: &str, _: &str) -> Result<String, String> {
            Err("no".into())
        }
    }
    #[derive(Default)]
    struct Sent(Mutex<Vec<Event>>);
    impl Inbox for Sent {
        fn send<'a>(&'a self, _: &'a str, act: &'a Event) -> BoxFut<'a, Result<(), String>> {
            self.0.lock().unwrap().push(act.clone());
            Box::pin(async { Ok(()) })
        }
    }
    struct Echo;
    impl Executor for Echo {
        fn quote(&self, _: &Terms, _: Profile) -> Quote {
            Quote::Accept { p_on_time: 0.9 }
        }
        fn run(&self, job: Job) -> BoxFut<'_, Result<Output, String>> {
            Box::pin(async move {
                Ok(Output {
                    result: Some(json!({"echo": job.input})),
                    artifacts: vec![Artifact {
                        role: "output".into(),
                        media_type: "text/plain".into(),
                        bytes: b"done".to_vec(),
                    }],
                    tools: vec![json!({"tool": "echo", "version": "1"})],
                    ..Default::default()
                })
            })
        }
    }
    #[derive(Default)]
    struct Mem(Mutex<BTreeMap<String, Vec<u8>>>);
    impl Evidence for Mem {
        fn put(&self, b: Vec<u8>, _: String) -> BoxFut<'_, Result<String, String>> {
            let h = acn_core::jcs::sha256_hex(&b);
            self.0.lock().unwrap().insert(h.clone(), b);
            Box::pin(async move { Ok(h) })
        }
        fn servers(&self) -> Vec<String> {
            vec!["mem:".into()]
        }
    }

    fn request(perf: &str, f: impl FnOnce(&mut Value)) -> Event {
        let mut t =
            acn_core::draft::tool_call_terms("echo", json!({"text": "hi"}), u64::MAX / 4, 3600, 0);
        f(&mut t);
        Draft::request(perf, "T0", &t, None)
            .unwrap()
            .sign(&[1; 32], 100)
            .unwrap()
    }

    #[tokio::test]
    async fn t0_report_carries_artifacts_and_a_derivation_record() {
        let sent = Arc::new(Sent::default());
        let ev = Arc::new(Mem::default());
        let p = Performer::new(Arc::new(Key([2; 32])), sent.clone(), Arc::new(Echo))
            .with_evidence(ev.clone());
        assert_eq!(p.level(), Some(Level::L0));
        let ch = p
            .on_event(request(&public_key(&[2; 32]).unwrap(), |_| {}))
            .await;
        assert_eq!(ch.to, Some(Phase::Reported), "{ch:?}");
        let r = &ch.sent[0];
        let c: Value = serde_json::from_str(&r.content).unwrap();
        let d = c["derivation"].as_str().unwrap();
        let record: Value = serde_json::from_slice(&ev.0.lock().unwrap()[d]).unwrap();
        assert_eq!(record["tools"][0]["tool"], "echo");
        let x: Vec<&str> = r
            .tags
            .iter()
            .filter(|t| t[0] == "x")
            .map(|t| t[1].as_str())
            .collect();
        assert_eq!(x, [acn_core::jcs::sha256_hex(b"done")]);
        // Acting again on the same head does nothing.
        assert!(p.drive(&ch.commitment).await.sent.is_empty());
    }

    #[tokio::test]
    async fn declines_what_l0_cannot_meet() {
        let perf = public_key(&[2; 32]).unwrap();
        let p = Performer::new(
            Arc::new(Key([2; 32])),
            Arc::new(Sent::default()),
            Arc::new(Echo),
        )
        .with_evidence(Arc::new(Mem::default()));
        for (f, reason) in [
            (
                Box::new(|t: &mut Value| t["price"]["amount"] = json!(5))
                    as Box<dyn FnOnce(&mut Value)>,
                "payment_unsupported",
            ),
            (
                Box::new(|t: &mut Value| {
                    t["conditions"][0] = json!({"id": "c1", "class": "open", "description": "x"})
                }),
                "terms_unclosed",
            ),
            (
                Box::new(|t: &mut Value| t["deadline"] = json!(1)),
                "expired",
            ),
        ] {
            let ch = p.on_event(request(&perf, f)).await;
            let c: Value = serde_json::from_str(&ch.sent[0].content).unwrap();
            assert_eq!(
                (ch.sent[0].kind, c["reason"].as_str()),
                (K_DECLINE, Some(reason))
            );
        }
    }
}
