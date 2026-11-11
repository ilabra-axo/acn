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
//! hold `Arc<dyn Executor>` and the like. Nothing here does network I/O itself; the optional
//! durable [`Journal`] writes one local JSON file.
//!
//! Safety properties of the driver (docs/DESIGN-ALIGNMENT-AUDIT.md A01–A04, A08, A16):
//!
//! - **Validated history:** only acts whose id and signature verify enter the store; an invalid
//!   copy is quarantined and never displaces a verified act ([`Store`]).
//! - **Authority:** operator bindings and revocations ([`CONTEXT_KINDS`]) are folded with every
//!   commitment, and a revoked or out-of-scope binding stops work before it starts and before
//!   anything is signed (`out_of_scope`, `binding_revoked`).
//! - **Admission:** an optional [`Admission`] policy applies to every route into the driver.
//! - **Effects at most once:** intent is journaled before the executor runs; signed acts are
//!   journaled before they are sent and enter the store only once delivered; a failed send is
//!   retried with the same act; redemption is recorded only on success ([`journal`]).
//! - **Applied, not merely unrejected:** a candidate act is folded before it is sent and must
//!   land on the commitment's chain; governance an L0 binding can't satisfy is declined.
//! - **Evidence:** nothing is uploaded unless [`Disclosure::Public`], and then only after the
//!   REPORT referencing it has passed the gate and the validator.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use acn_core::anomaly::{Anomaly, Code, Severity};
use acn_core::draft::Draft;
use acn_core::kinds::*;
use acn_core::terms::GovernedParty;
use acn_core::{fold, BindingStatus, Event, Phase, Profile, Terms};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub mod journal;

pub use acn_wallet::Verified;
pub use journal::{Entry, Journal, Redemption, Step};

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
    /// A secret derived from this key for `label` (the per-commitment performer seed for
    /// held-out instances, Core §7): deterministic, so nothing has to be stored, and revealed
    /// only by the act the spec names. A signer that can't derive secrets returns an error, and
    /// the driver declines terms that need one.
    fn derive_secret(&self, label: &str) -> Result<[u8; 32], String> {
        let _ = label;
        Err("this signer derives no secrets".into())
    }
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
    /// On rework (profile N): the gaps of the `REJECT` being answered (`{condition, note}`).
    pub feedback: Vec<Value>,
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
    /// Fetch a blob by hash: from `hints` (e.g. a terms tag's URL), then the own servers. The
    /// caller checks the hash.
    fn fetch<'a>(
        &'a self,
        hash: &'a str,
        hints: &'a [String],
    ) -> BoxFut<'a, Result<Vec<u8>, String>>;
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
    /// Allowed by policy (an endorsement, if one is needed, says `automated`).
    Allow,
    /// Allowed after a person signed off (an endorsement says `human`).
    AllowedByHuman,
    Deny {
        reason: String,
    },
}

/// Pre-signing policy (rule 1: never sign what the gate denied or hasn't decided). `evaluate`
/// may wait, e.g. for a person's sign-off; a gate that times out denies. A gate that holds a
/// governance key (L2) also endorses the acts it lets through (`K_ENDORSE`, Core §9), which is
/// what terms with `governance` require (V-14).
pub trait Gate: Send + Sync {
    fn evaluate<'a>(
        &'a self,
        draft: &'a Draft,
        commitment: &'a acn_core::Commitment,
    ) -> BoxFut<'a, GateDecision>;
    /// The governance key this gate endorses with (x-only hex), if it endorses at all.
    fn endorser(&self) -> Option<String> {
        None
    }
    /// The hash of the policy it applies (the endorsement's `policy` tag).
    fn policy_hash(&self) -> String {
        String::new()
    }
    /// A signed endorsement of `act`, after the gate allowed it (`human` if a person signed
    /// off).
    fn endorse(&self, act: &Event, human: bool) -> Result<Event, String> {
        let _ = (act, human);
        Err("this gate does not endorse".into())
    }
}

/// A Signer for a bare key (a governance key, an arbiter's): signs drafts, nothing else.
pub struct KeySigner([u8; 32]);

impl KeySigner {
    pub fn new(secret: [u8; 32]) -> Self {
        Self(secret)
    }
}

impl Signer for KeySigner {
    fn pubkey(&self) -> String {
        acn_core::draft::public_key(&self.0).unwrap_or_default()
    }
    fn sign(&self, draft: &Draft, not_before: Option<u64>) -> Result<Event, String> {
        draft
            .sign(&self.0, now_secs().max(not_before.unwrap_or(0)))
            .map_err(|e| e.to_string())
    }
    fn decrypt(&self, _: &str, _: &str) -> Result<String, String> {
        Err("this key decrypts nothing".into())
    }
}

/// What the reference gate lets through.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GatePolicy {
    /// The highest price it endorses a commitment at (sat); `None`: any.
    #[serde(default)]
    pub max_sat: Option<u64>,
    /// Tools it endorses work on; empty: any.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Act kinds that need a person's sign-off (through [`PolicyGate::with_approver`]).
    #[serde(default)]
    pub human_kinds: Vec<u16>,
}

/// A person's sign-off on a draft: `true` approves. Asked for [`GatePolicy::human_kinds`].
pub type Approver = Arc<dyn Fn(Draft, acn_core::Commitment) -> BoxFut<'static, bool> + Send + Sync>;

/// The reference gate (L2): a policy, a governance key that endorses what the policy allows,
/// and optionally a person who signs off on some act kinds (within `approval_timeout`, else
/// denied). Refusals (`DECLINE`, `REVOKE`) are always allowed: a gate can stop work, never force
/// it.
pub struct PolicyGate {
    pub signer: Arc<dyn Signer>,
    pub policy: GatePolicy,
    approver: Option<Approver>,
    approval_timeout: std::time::Duration,
}

impl PolicyGate {
    pub fn new(signer: Arc<dyn Signer>, policy: GatePolicy) -> Self {
        Self {
            signer,
            policy,
            approver: None,
            approval_timeout: std::time::Duration::from_secs(300),
        }
    }

    pub fn with_approver(mut self, a: Approver, timeout: std::time::Duration) -> Self {
        self.approver = Some(a);
        self.approval_timeout = timeout;
        self
    }
}

impl Gate for PolicyGate {
    fn evaluate<'a>(
        &'a self,
        draft: &'a Draft,
        cm: &'a acn_core::Commitment,
    ) -> BoxFut<'a, GateDecision> {
        Box::pin(async move {
            if matches!(draft.kind, K_DECLINE | K_REVOKE) {
                return GateDecision::Allow;
            }
            if let Some(t) = &cm.terms {
                if self.policy.max_sat.is_some_and(|m| t.price.amount > m) {
                    return GateDecision::Deny {
                        reason: format!(
                            "the gate's policy endorses up to {} sat",
                            self.policy.max_sat.unwrap_or(0)
                        ),
                    };
                }
                if !self.policy.tools.is_empty()
                    && !t
                        .deliverable
                        .tool
                        .as_ref()
                        .is_some_and(|x| self.policy.tools.contains(x))
                {
                    return GateDecision::Deny {
                        reason: "the gate's policy doesn't cover this tool".into(),
                    };
                }
            }
            if !self.policy.human_kinds.contains(&draft.kind) {
                return GateDecision::Allow;
            }
            let Some(a) = &self.approver else {
                return GateDecision::Deny {
                    reason: "this act needs a person's sign-off and nobody is set to give it"
                        .into(),
                };
            };
            match tokio::time::timeout(self.approval_timeout, a(draft.clone(), cm.clone())).await {
                Ok(true) => GateDecision::AllowedByHuman,
                Ok(false) => GateDecision::Deny {
                    reason: "not approved".into(),
                },
                Err(_) => GateDecision::Deny {
                    reason: "no sign-off in time".into(),
                },
            }
        })
    }
    fn endorser(&self) -> Option<String> {
        Some(self.signer.pubkey())
    }
    fn policy_hash(&self) -> String {
        acn_core::jcs::hash(&serde_json::to_value(&self.policy).unwrap_or_default())
            .unwrap_or_default()
    }
    fn endorse(&self, act: &Event, human: bool) -> Result<Event, String> {
        let c = if act.kind == K_REQUEST {
            act.id.clone()
        } else {
            act.values("c")
                .first()
                .map(|c| c.to_string())
                .unwrap_or_default()
        };
        self.signer.sign(
            &Draft::endorse(&c, &act.id, &self.policy_hash(), human),
            Some(act.created_at),
        )
    }
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

/// Standalone events folded as context with every commitment: operator bindings, their
/// acknowledgements and revocations (V-8), and terms templates (V-28).
pub const CONTEXT_KINDS: [u16; 4] = [K_BIND, K_BIND_ACK, K_BIND_REVOKE, K_TERMS_TEMPLATE];

#[derive(Default)]
struct Acts {
    /// Authoritative: acts whose id and signature verify (V-1), per commitment, by id.
    verified: BTreeMap<String, BTreeMap<String, Event>>,
    /// Input that failed V-1, kept apart so it can never displace a verified act.
    quarantine: BTreeMap<String, BTreeMap<String, Event>>,
    /// Verified [`CONTEXT_KINDS`] events, by id.
    context: BTreeMap<String, Event>,
}

/// Acts per commitment. Only events whose id and signature verify enter the authoritative,
/// id-indexed store; a copy that fails verification is quarantined and can never replace a
/// verified act with the same id (a verified copy does replace a quarantined one).
#[derive(Default)]
pub struct Store {
    acts: Mutex<Acts>,
}

fn commitment_of(e: &Event) -> String {
    if e.kind == K_REQUEST {
        e.id.clone()
    } else {
        e.tags
            .iter()
            .find(|t| t.first().map(String::as_str) == Some("c"))
            .and_then(|t| t.get(1).cloned())
            .unwrap_or_default()
    }
}

impl Store {
    /// Add an event; returns its commitment id (empty for context events).
    pub fn add(&self, e: Event) -> String {
        let c = commitment_of(&e);
        let ok = e.verify().is_ok();
        let mut a = self.acts.lock().unwrap();
        if ok && CONTEXT_KINDS.contains(&e.kind) {
            a.context.entry(e.id.clone()).or_insert(e);
            return String::new();
        }
        if ok {
            if let Some(q) = a.quarantine.get_mut(&c) {
                q.remove(&e.id);
            }
            a.verified
                .entry(c.clone())
                .or_default()
                .entry(e.id.clone())
                .and_modify(|cur| {
                    // Two valid copies differ at most in the signature: keep the smaller, as the
                    // fold does, so the choice doesn't depend on arrival order.
                    if e.sig < cur.sig {
                        *cur = e.clone();
                    }
                })
                .or_insert(e);
        } else {
            if a.verified.values().any(|m| m.contains_key(&e.id)) || a.context.contains_key(&e.id) {
                tracing::warn!(id = %e.id, "binding: ignored an invalid copy of a verified act");
            }
            a.quarantine
                .entry(c.clone())
                .or_default()
                .insert(e.id.clone(), e);
        }
        c
    }

    /// The verified acts of `c`, plus quarantined input whose id no verified act has (the fold
    /// reports it as `bad_sig`; it can't hide or replace anything).
    pub fn events(&self, c: &str) -> Vec<Event> {
        let a = self.acts.lock().unwrap();
        let mut v: Vec<Event> = a
            .verified
            .get(c)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        if let Some(q) = a.quarantine.get(c) {
            v.extend(
                q.values()
                    .filter(|e| {
                        !a.verified.values().any(|m| m.contains_key(&e.id))
                            && !a.context.contains_key(&e.id)
                    })
                    .cloned(),
            );
        }
        v
    }

    /// Whether `c` has the verified act `id`.
    pub fn has(&self, c: &str, id: &str) -> bool {
        self.acts
            .lock()
            .unwrap()
            .verified
            .get(c)
            .is_some_and(|m| m.contains_key(id))
    }

    /// Input that failed verification, for `c`.
    pub fn quarantined(&self, c: &str) -> Vec<Event> {
        self.acts
            .lock()
            .unwrap()
            .quarantine
            .get(c)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    /// The context folded with every commitment ([`CONTEXT_KINDS`]).
    pub fn context(&self) -> Vec<Event> {
        self.acts
            .lock()
            .unwrap()
            .context
            .values()
            .cloned()
            .collect()
    }

    /// Commitments with at least one verified act.
    pub fn ids(&self) -> Vec<String> {
        self.acts
            .lock()
            .unwrap()
            .verified
            .keys()
            .filter(|c| !c.is_empty())
            .cloned()
            .collect()
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

/// Redemption attempts before the binding stops retrying and asks for reconciliation.
const MAX_REDEEM_ATTEMPTS: u32 = 12;

/// Who may see the evidence of a REPORT (artifacts, logs, derivation record).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disclosure {
    /// Nothing is uploaded: the inline result in the (privately delivered) REPORT is the whole
    /// report, and the binding claims no conformance level. The default.
    #[default]
    Private,
    /// Artifacts and the derivation record are uploaded through [`Evidence`] (Blossom: public,
    /// unencrypted), after the act that references them has passed the gate and the validator.
    Public,
}

/// Decides whether a requester (x-only hex) may have work done at all. A requester it refuses
/// gets no act, not even a DECLINE (the same as a sender the host ignores).
pub type Admission = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// What the driver is about to send: an act draft and the blobs it references by hash (uploaded
/// only once the act may be sent).
struct Move {
    draft: Draft,
    blobs: Vec<(Vec<u8>, String)>,
    /// The terms the act commits its signer to, when not the terms in force (a `COUNTER`'s or
    /// an `ACCEPT`'s): what V-8 is checked against.
    terms: Option<Terms>,
}

impl Move {
    fn act(draft: Draft) -> Self {
        Self {
            draft,
            blobs: vec![],
            terms: None,
        }
    }
    fn committing(draft: Draft, terms: Terms) -> Self {
        Self {
            terms: Some(terms),
            ..Self::act(draft)
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A performer: its interfaces and the lifecycle that runs on them, under the requester
/// trigger. Profiles T0 and T1 (conformance level L0); with [`Performer::with_negotiation`] also
/// profile N (L1): counters that close open conditions, accepting the requester's counters,
/// promises, rework, disputes of contradicted rejections, held-out instances and disqualifiers.
/// Anything else is declined.
pub struct Performer {
    pub signer: Arc<dyn Signer>,
    pub inbox: Arc<dyn Inbox>,
    pub executor: Arc<dyn Executor>,
    pub evidence: Option<Arc<dyn Evidence>>,
    pub disclosure: Disclosure,
    pub wallet: Option<Arc<dyn Wallet>>,
    pub gate: Option<Arc<dyn Gate>>,
    /// The advertised minimum price (`K_SVC` `price.min`): cheaper requests are declined.
    pub min_price_sat: u64,
    pub store: Store,
    journal: Journal,
    admission: Option<Admission>,
    /// First redemption retry delay (doubling, capped at an hour).
    retry_base_secs: u64,
    /// Terms documents referenced by hash rather than inlined (Core §7).
    docs: Mutex<BTreeMap<String, Value>>,
    /// Commitments the driver is acting on, each with a dirty flag: set when another event for
    /// it arrives meanwhile, so the active drive folds again before it stops (none is lost).
    busy: Mutex<BTreeMap<String, bool>>,
    /// Profile N (L1).
    negotiate: bool,
    /// Time this binding needs, after the review window, to redeem before the requester's
    /// refund path opens (Core §8.2 timing constraint).
    redeem_margin_secs: u64,
    /// The performer's own terms templates, for closing open conditions (CR-003 §2.4).
    author: acn_author::Capabilities,
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
            disclosure: Disclosure::default(),
            wallet: None,
            gate: None,
            min_price_sat: 0,
            store: Store::default(),
            journal: Journal::in_memory(),
            admission: None,
            retry_base_secs: 5,
            docs: Mutex::new(BTreeMap::new()),
            busy: Mutex::new(BTreeMap::new()),
            negotiate: false,
            redeem_margin_secs: 3600,
            author: acn_author::Capabilities::default(),
        }
    }

    /// Serve profile N too (conformance level L1).
    pub fn with_negotiation(mut self, on: bool) -> Self {
        self.negotiate = on;
        self
    }

    /// How long after `deadline + review_window` this binding needs to redeem a token (Core
    /// §8.2): paid terms whose locktime leaves less are declined `locktime_too_early`.
    pub fn with_redeem_margin(mut self, secs: u64) -> Self {
        self.redeem_margin_secs = secs;
        self
    }

    /// Templates (and slot values, verifiers) the binding closes open conditions from, through
    /// `acn_author::draft_counter`. Without one that fits, an open condition is closed as a
    /// `human` condition: the requester decides whether it is met.
    pub fn with_templates(mut self, caps: acn_author::Capabilities) -> Self {
        self.author = caps;
        self
    }

    /// Evidence: fetches terms sent by reference, and (with [`Disclosure::Public`]) stores
    /// artifacts.
    pub fn with_evidence(mut self, e: Arc<dyn Evidence>) -> Self {
        self.evidence = Some(e);
        self
    }

    pub fn with_disclosure(mut self, d: Disclosure) -> Self {
        self.disclosure = d;
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

    /// A durable execution journal (see [`journal`]). Without one, the journal lives in memory
    /// and protects against re-execution within the process only.
    pub fn with_journal(mut self, j: Journal) -> Self {
        self.journal = j;
        self
    }

    /// Who may have work done (see [`Admission`]). Applies to every route into the driver:
    /// history replay and live events alike.
    pub fn with_admission(mut self, a: Admission) -> Self {
        self.admission = Some(a);
        self
    }

    pub fn with_retry_backoff(mut self, base_secs: u64) -> Self {
        self.retry_base_secs = base_secs;
        self
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// The level these interfaces can reach (the suite decides whether it is met): L0 needs
    /// Evidence with public disclosure (the suite fetches the artifacts), and a Wallet for priced
    /// calls; L1 adds negotiation. Whether an operator binding is in force is the host's to add.
    pub fn level(&self) -> Option<Level> {
        let evidence = self.evidence.is_some() && self.disclosure == Disclosure::Public;
        let endorses = self.gate.as_ref().and_then(|g| g.endorser()).is_some();
        match (evidence, self.negotiate, endorses) {
            (true, true, true) => Some(Level::L2),
            (true, true, false) => Some(Level::L1),
            (true, false, _) => Some(Level::L0),
            _ => None,
        }
    }

    /// The governance this binding can meet, for `K_SVC` (`governance`): its gate's key and
    /// policy hash, if the gate endorses.
    pub fn governance(&self) -> Option<Value> {
        let g = self.gate.as_ref()?;
        Some(json!({"keys": [g.endorser()?], "policy": g.policy_hash()}))
    }

    /// Whether this binding's gate can endorse acts governed by `terms` (performer-governed
    /// terms listing its key).
    fn can_meet_governance(&self, terms: &Terms) -> bool {
        match &terms.governance {
            Some(g) if g.party == GovernedParty::Performer => self
                .gate
                .as_ref()
                .and_then(|x| x.endorser())
                .is_some_and(|k| g.keys.contains(&k)),
            _ => true,
        }
    }

    /// The profiles this binding serves, for `K_SVC`.
    pub fn profiles(&self) -> Vec<String> {
        let mut v = vec!["T0".to_string()];
        if self.wallet.is_some() {
            v.push("T1".into());
        }
        if self.negotiate {
            v.push("N".into());
        }
        v
    }

    /// Record an event without acting (history replay); terms it references are fetched.
    pub async fn ingest(&self, e: Event) -> String {
        self.resolve_terms(&e).await;
        self.store.add(e)
    }

    /// Operator binding / revocation events (and templates) to fold with every commitment.
    pub fn add_context(&self, events: impl IntoIterator<Item = Event>) {
        for e in events {
            if CONTEXT_KINDS.contains(&e.kind) {
                self.store.add(e);
            }
        }
    }

    /// Fold an inbound event and make the performer's move, if it has one.
    pub async fn on_event(&self, e: Event) -> StateChange {
        let id = e.id.clone();
        let invalid = e.verify().err();
        self.resolve_terms(&e).await;
        let c = self.store.add(e);
        let before = self.phase(&c);
        let mut change = self.drive(&c).await;
        change.from = before;
        let out = fold(&self.events(&c), &self.docs());
        change.anomalies = out
            .anomalies
            .into_iter()
            .filter(|a| a.event == id)
            .collect();
        if let Some(err) = invalid {
            // Anything else filed under this id describes a verified copy, not this input
            // (which, quarantined behind it, the fold may never have seen).
            change.anomalies.retain(|a| a.code == Code::BadSig);
            if change.anomalies.is_empty() {
                change
                    .anomalies
                    .push(Anomaly::new(id, Code::BadSig, err.to_string()));
            }
        }
        change
    }

    /// Drive every known commitment (after a history replay).
    pub async fn drive_all(&self) -> Vec<StateChange> {
        let mut v = Vec::new();
        for c in self.store.ids() {
            v.push(self.drive(&c).await);
        }
        v
    }

    /// Re-send prepared acts and retry redemptions that are due. Hosts call this periodically.
    pub async fn retry_due(&self) -> Vec<StateChange> {
        let mut v = Vec::new();
        for c in self.journal.due(now_secs()) {
            v.push(self.drive(&c).await);
        }
        v
    }

    /// Terms by reference: fetch each through Evidence and keep it if it hashes right.
    async fn resolve_terms(&self, e: &Event) {
        let Some(ev) = &self.evidence else { return };
        for (hash, hint) in acn_core::draft::referenced_terms(std::slice::from_ref(e)) {
            if self.docs.lock().unwrap().contains_key(&hash) {
                continue;
            }
            let hints: Vec<String> = hint.into_iter().collect();
            let doc = ev
                .fetch(&hash, &hints)
                .await
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .filter(|v| acn_core::jcs::hash(v).ok().as_deref() == Some(hash.as_str()));
            match doc {
                Some(v) => {
                    self.docs.lock().unwrap().insert(hash, v);
                }
                None => tracing::warn!(%hash, "binding: referenced terms not retrievable"),
            }
        }
    }

    fn docs(&self) -> BTreeMap<String, Value> {
        self.docs.lock().unwrap().clone()
    }

    /// The acts of `c` with the authority context: what every fold here sees.
    fn events(&self, c: &str) -> Vec<Event> {
        let mut v = self.store.events(c);
        v.extend(self.store.context());
        v
    }

    fn phase(&self, c: &str) -> Option<Phase> {
        let out = fold(&self.events(c), &self.docs());
        out.commitments
            .get(c)
            .and_then(|x| x.state.as_ref())
            .map(|s| s.phase)
    }

    fn admits(&self, requester: &str) -> bool {
        self.admission.as_ref().is_none_or(|a| a(requester))
    }

    /// V-8 locally, with the same binding events a validator sees: may this key sign an act of
    /// `kind` committing to `terms` at `at`? No binding at all is allowed (the validator flags
    /// it `unbound`); a binding that is revoked, expired or out of scope is not. A refusal
    /// (`DECLINE`, `REVOKE`) commits to no value, so only the binding being in force and
    /// covering the kind is checked: a request above scope can always be declined, while a
    /// revoked or expired binding can sign nothing at all.
    fn authority(&self, kind: u16, terms: &Terms, at: u64) -> Result<(), (&'static str, String)> {
        let (sat, mint) = if matches!(kind, K_DECLINE | K_REVOKE) {
            (0, None)
        } else {
            (
                terms.price.amount,
                terms.payment.as_ref().map(|p| p.mint.as_str()),
            )
        };
        match acn_core::binding_status(
            &self.store.context(),
            &self.signer.pubkey(),
            kind,
            sat,
            mint,
            at,
        ) {
            BindingStatus::Bound { .. } | BindingStatus::Unbound => Ok(()),
            BindingStatus::OutOfScope { reason } => Err((
                "out_of_scope",
                format!("this binding's operator scope doesn't cover it: {reason}"),
            )),
            BindingStatus::Revoked { binding } => Err((
                "binding_revoked",
                format!("this binding's operator binding {binding} is revoked"),
            )),
        }
    }

    /// Re-evaluate commitment `c` and act. Safe to call repeatedly: it acts only on a fresh
    /// head, and never twice on the same one (the journal says what was done, across restarts).
    pub async fn drive(&self, c: &str) -> StateChange {
        let mut change = StateChange {
            commitment: c.to_string(),
            ..Default::default()
        };
        if c.is_empty() {
            return change;
        }
        {
            let mut busy = self.busy.lock().unwrap();
            if let Some(dirty) = busy.get_mut(c) {
                // Another drive is acting on `c`: it folds again before it stops.
                *dirty = true;
                return change;
            }
            busy.insert(c.to_string(), false);
        }
        // Fold again while the head moves: others' acts arrived (dirty), or ours did (a PROMISE
        // is followed by the work and its REPORT). Bounded; a later event drives again.
        for round in 0.. {
            let ours = change.sent.len();
            if let Err(e) = self.drive_inner(c, &mut change).await {
                tracing::warn!(commitment = %c, error = %e, "binding: no move");
            }
            let mut busy = self.busy.lock().unwrap();
            let dirty = busy.get(c).copied().unwrap_or(false);
            if round >= 16 || (!dirty && change.sent.len() == ours) {
                busy.remove(c);
                break;
            }
            busy.insert(c.to_string(), false);
        }
        change.to = self.phase(c);
        change
    }

    async fn drive_inner(&self, c: &str, change: &mut StateChange) -> Result<(), String> {
        self.flush(c, change).await?;
        let me = self.signer.pubkey();
        let events = self.events(c);
        let out = fold(&events, &self.docs());
        let Some(cm) = out.commitments.get(c) else {
            return Ok(());
        };
        let (Some(st), Some(terms)) = (&cm.state, &cm.terms) else {
            return Ok(());
        };
        if cm.performer != me {
            return Ok(());
        }
        if st.phase == Phase::Satisfied {
            change.redeemed = self.redeem(c, &events).await?;
            return Ok(());
        }
        if matches!(st.phase, Phase::Requested | Phase::Countered) && !self.admits(&cm.requester) {
            tracing::debug!(commitment = %c, requester = %cm.requester, "binding: requester not admitted");
            return Ok(());
        }
        // One move per head: the journal says whether this head was answered (across restarts).
        let key = Journal::key(c, &st.head);
        let first = match self.journal.entry(&key) {
            // Intent without an outcome: the work may have run. Never run it again
            // automatically; say so (a refusal: DECLINE before a promise, REVOKE after).
            Some(e) if e.step == Step::Executing => Move::act(self.refuse_draft(
                c,
                cm,
                st,
                "execution_uncertain",
                "an earlier run of this request was interrupted and may or may not have \
                 completed; it is not run again automatically",
            )),
            // An act is in flight (`flush` delivers it), was sent, or the binding halted.
            Some(_) => return Ok(()),
            None => match self
                .next_move(c, cm, st, terms, &out, &events, &key)
                .await?
            {
                Some(m) => m,
                None => return Ok(()),
            },
        };
        self.commit(c, cm, st, terms, &key, first, change).await
    }

    /// The performer's move on head `st.head`, if it has one.
    #[allow(clippy::too_many_arguments)]
    async fn next_move(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        st: &acn_core::State,
        terms: &Terms,
        out: &acn_core::Output,
        events: &[Event],
        key: &str,
    ) -> Result<Option<Move>, String> {
        let requester_counter = st
            .pending
            .as_ref()
            .filter(|p| p.by == cm.requester)
            .map(|p| p.terms.clone());
        match st.phase {
            Phase::Requested if cm.profile != Profile::N => {
                self.answer(c, cm, st, terms, events, key).await.map(Some)
            }
            Phase::Requested => self.negotiate(c, cm, st, terms, None, events).await,
            Phase::Countered => match requester_counter {
                Some(h) => self.negotiate(c, cm, st, terms, Some(&h), events).await,
                // Our own counter: the requester's move.
                None => Ok(None),
            },
            Phase::Promised => {
                if let Some(h) = &requester_counter {
                    // A counter while promised: accept it if this binding can meet it; otherwise
                    // the work goes on under the terms in force.
                    if let Some(m) = self.negotiate(c, cm, st, terms, Some(h), events).await? {
                        if m.draft.kind == K_ACCEPT {
                            return Ok(Some(m));
                        }
                    }
                }
                self.work(c, cm, st, terms, events, key, vec![])
                    .await
                    .map(Some)
            }
            Phase::Reworking => {
                let gaps = events
                    .iter()
                    .find(|e| e.id == st.head)
                    .and_then(|e| serde_json::from_str::<Value>(&e.content).ok())
                    .and_then(|v| v["gaps"].as_array().cloned())
                    .unwrap_or_default();
                self.work(c, cm, st, terms, events, key, gaps)
                    .await
                    .map(Some)
            }
            Phase::Rejected => Ok(self.dispute_move(c, cm, st, terms, out)),
            _ => Ok(None),
        }
    }

    /// Profile N, on the `REQUEST` (`pending` `None`) or on the requester's `COUNTER` (its
    /// terms hash): `PROMISE` / `ACCEPT` terms this binding can meet, `COUNTER` closing open
    /// conditions, or `DECLINE`.
    async fn negotiate(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        st: &acn_core::State,
        terms: &Terms,
        pending: Option<&str>,
        events: &[Event],
    ) -> Result<Option<Move>, String> {
        let parties = [cm.requester.as_str(), cm.performer.as_str()];
        let decline = |reason: &str, note: String| {
            Ok(Some(Move::act(self.refuse_draft(c, cm, st, reason, note))))
        };
        let candidate = match pending {
            None => terms.clone(),
            Some(h) => match self.terms_doc(events, h) {
                Some(t) => t,
                None => return Ok(None), // the counter's terms aren't available yet
            },
        };
        let now = now_secs();
        if let Err((reason, note)) = self.authority(K_PROMISE, &candidate, now) {
            return decline(reason, note);
        }
        if let Some((reason, note)) = self.refusal(&candidate, Profile::N, now) {
            return decline(reason, note);
        }
        // Open conditions: the performer drafts what it will be held to (CR-003 §2.4).
        if candidate.unclosed().is_some() {
            let Some((closed, reason)) = self.close_open(&candidate) else {
                return decline(
                    "terms_unclosed",
                    "this binding can't close the open conditions".into(),
                );
            };
            return self.counter_move(c, cm, st, closed, &reason, now);
        }
        match self.executor.quote(&candidate, Profile::N) {
            Quote::Accept { p_on_time } => {
                let paid = candidate.price.amount > 0;
                if paid {
                    if let Err(why) = self.check_token(c, cm, &candidate, events).await {
                        return decline("payment_invalid", why);
                    }
                }
                let mut d = match pending {
                    None => Draft::promise(c, &st.head, &parties, &st.terms, p_on_time, paid),
                    Some(h) => Draft::accept(c, &st.head, &parties, h),
                };
                if candidate.has_instances() {
                    match self.performer_seed(c) {
                        Ok(seed) => {
                            d = d.field(
                                "performer_seed_hash",
                                json!(acn_core::jcs::sha256_hex(&seed)),
                            )
                        }
                        Err(e) => return decline("instances_unsupported", e),
                    }
                }
                Ok(Some(Move::committing(d, candidate)))
            }
            Quote::Decline { reason, note } => decline(&reason, note),
            Quote::Counter { terms: t, reason } => self.counter_move(c, cm, st, t, &reason, now),
        }
    }

    /// Our `COUNTER` with `terms`, if this binding could meet them; else a `DECLINE`.
    fn counter_move(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        st: &acn_core::State,
        terms: Value,
        reason: &str,
        now: u64,
    ) -> Result<Option<Move>, String> {
        let parties = [cm.requester.as_str(), cm.performer.as_str()];
        let parsed = match Terms::parse(&terms) {
            Ok(t) if t.unclosed().is_none() => t,
            Ok(_) | Err(_) => {
                return Ok(Some(Move::act(self.refuse_draft(
                    c,
                    cm,
                    st,
                    "terms_unclosed",
                    "this binding's counter would not be closed terms",
                ))))
            }
        };
        if let Some((r, note)) = self.refusal(&parsed, Profile::N, now) {
            return Ok(Some(Move::act(self.refuse_draft(c, cm, st, r, note))));
        }
        let mut d =
            Draft::counter(c, &st.head, &parties, &terms, reason).map_err(|e| e.to_string())?;
        if parsed.has_instances() {
            let seed = self.performer_seed(c)?;
            d = d.field(
                "performer_seed_hash",
                json!(acn_core::jcs::sha256_hex(&seed)),
            );
        }
        Ok(Some(Move::committing(d, parsed)))
    }

    /// Close every open condition: from the binding's own templates (`acn-author`), or else as a
    /// `human` condition (the requester decides whether it is met), with one line per change.
    fn close_open(&self, terms: &Terms) -> Option<(Value, String)> {
        if let Some(d) = acn_author::draft_counter(terms, &self.author) {
            return Some((d.terms, d.reason));
        }
        let mut v = serde_json::to_value(terms).ok()?;
        let obj = v.as_object_mut()?;
        // The conditions change, so a template reference would no longer re-instantiate (V-28).
        obj.remove("template_ref");
        let mut lines = Vec::new();
        for cond in obj.get_mut("conditions")?.as_array_mut()? {
            if cond["class"] == "open" {
                let id = cond["id"].as_str().unwrap_or_default().to_string();
                let o = cond.as_object_mut()?;
                o.retain(|k, _| matches!(k.as_str(), "id" | "description"));
                o.insert("class".into(), json!("human"));
                lines.push(format!(
                    "{id}: closed as a human check: you decide whether it is met"
                ));
            }
        }
        Some((v, lines.join("; ")))
    }

    /// Do the work and draft the `REPORT` (with the performer seed, for held-out instances). The
    /// intent is journaled before the executor runs; if it can't be, nothing runs.
    #[allow(clippy::too_many_arguments)]
    async fn work(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        st: &acn_core::State,
        terms: &Terms,
        events: &[Event],
        key: &str,
        feedback: Vec<Value>,
    ) -> Result<Move, String> {
        let parties = [cm.requester.as_str(), cm.performer.as_str()];
        let refuse =
            |reason: &str, note: String| Move::act(self.refuse_draft(c, cm, st, reason, note));
        let now = now_secs();
        if let Err((reason, note)) = self.authority(K_REPORT, terms, now) {
            return Ok(refuse(reason, note));
        }
        // Profile N: the token is checked against the terms in force before every run (they may
        // be a counter's, agreed after the REQUEST's token was locked).
        if cm.profile == Profile::N && terms.price.amount > 0 {
            if let Err(why) = self.check_token(c, cm, terms, events).await {
                return Ok(refuse("payment_invalid", why));
            }
        }
        let seed = if terms.has_instances() {
            match self.performer_seed(c) {
                Ok(s) => Some(s),
                Err(e) => return Ok(refuse("instances_unsupported", e)),
            }
        } else {
            None
        };
        self.journal.put(
            key,
            Entry {
                commitment: c.to_string(),
                terms: st.terms.clone(),
                to: cm.requester.clone(),
                step: Step::Executing,
                act: None,
                extra: vec![],
                executed: true,
                at: now,
            },
        )?;
        let job = Job {
            commitment: c.to_string(),
            terms: terms.clone(),
            tool: terms.deliverable.tool.clone(),
            input: terms.deliverable.input.clone().unwrap_or_else(|| json!({})),
            feedback,
        };
        let output = match self.executor.run(job).await {
            Ok(o) => o,
            Err(e) => return Ok(refuse("tool_error", e)),
        };
        let mut mv = match self.report(c, st, &parties, terms, output, now) {
            Ok(m) => m,
            Err(e) => return Ok(refuse("evidence_unavailable", e)),
        };
        if let Some(s) = seed {
            mv.draft = mv.draft.field("performer_seed", json!(hex::encode(s)));
        }
        Ok(mv)
    }

    /// Dispute a rejection the evidence contradicts (V-16: `contradicted_rejection`, or
    /// `overruled_judgement`), when the terms name an arbiter. Other rejections stand.
    fn dispute_move(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        st: &acn_core::State,
        terms: &Terms,
        out: &acn_core::Output,
    ) -> Option<Move> {
        if !self.negotiate || terms.arbiter.is_none() || st.ruled {
            return None;
        }
        let contradicted = out.has(&st.head, Code::ContradictedRejection)
            || out.has(&st.head, Code::OverruledJudgement);
        if !contradicted {
            return None;
        }
        let parties = [cm.requester.as_str(), cm.performer.as_str()];
        Some(Move::act(Draft::dispute(
            c,
            &st.head,
            &parties,
            "the rejection is contradicted by the evidence the terms name",
        )))
    }

    /// The per-commitment performer seed (held-out instances): derived from our key.
    fn performer_seed(&self, c: &str) -> Result<[u8; 32], String> {
        self.signer
            .derive_secret(&format!("acn-binding performer seed/{c}"))
    }

    /// Terms with hash `h`: inline in a `REQUEST` / `COUNTER` among `events`, or a fetched
    /// document.
    fn terms_doc(&self, events: &[Event], h: &str) -> Option<Terms> {
        if let Some(v) = self.docs.lock().unwrap().get(h) {
            return Terms::parse(v).ok();
        }
        events
            .iter()
            .filter(|e| matches!(e.kind, K_REQUEST | K_COUNTER))
            .filter(|e| {
                e.tags.iter().any(|t| {
                    t.first().map(String::as_str) == Some("terms")
                        && t.get(1).map(String::as_str) == Some(h)
                })
            })
            .find_map(|e| {
                let v: Value = serde_json::from_str(&e.content).ok()?;
                let t = v.get("terms")?;
                (acn_core::jcs::hash(t).ok()? == h)
                    .then(|| Terms::parse(t).ok())
                    .flatten()
            })
    }

    /// The journal's view of `c` into the store: a prepared act is (re)sent as signed; a sent one
    /// missing from history is restored; an interrupted run whose act history shows is completed.
    async fn flush(&self, c: &str, change: &mut StateChange) -> Result<(), String> {
        let me = self.signer.pubkey();
        for (key, e) in self.journal.entries_for(c) {
            match (&e.step, &e.act) {
                (Step::Prepared, Some(act)) => {
                    self.deliver(&key, e.clone(), act.clone(), change).await?;
                }
                (Step::Sent, Some(act)) if !self.store.has(c, &act.id) => {
                    self.store.add(act.clone());
                    for x in &e.extra {
                        self.store.add(x.clone());
                    }
                }
                (Step::Executing, None) => {
                    // Reconcile: our act answering this head (the key's), from history.
                    let head = key.split_once(':').map(|(_, h)| h).unwrap_or_default();
                    let events = self.events(c);
                    let ours = events.iter().find(|x| {
                        x.pubkey == me && x.kind != K_REQUEST && x.e_marked("prev").contains(&head)
                    });
                    if let Some(act) = ours {
                        tracing::info!(commitment = %c, "binding: interrupted run reconciled from history");
                        self.journal.put(
                            &key,
                            Entry {
                                step: Step::Sent,
                                act: Some(act.clone()),
                                ..e
                            },
                        )?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A refusal from the current phase: `DECLINE` before a promise, `REVOKE` after (both
    /// commit to no value, so they are always in scope, V-8).
    fn refuse_draft(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        st: &acn_core::State,
        reason: &str,
        note: impl Into<String>,
    ) -> Draft {
        if matches!(st.phase, Phase::Promised | Phase::Reworking) {
            let parties = [cm.requester.as_str(), cm.performer.as_str()];
            Draft::closing(K_REVOKE, c, &st.head, &parties, reason)
                .content(json!({ "reason": reason, "note": note.into() }))
        } else {
            self.decline_draft(c, cm, st, reason, note)
        }
    }

    fn decline_draft(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        st: &acn_core::State,
        reason: &str,
        note: impl Into<String>,
    ) -> Draft {
        let parties = [cm.requester.as_str(), cm.performer.as_str()];
        Draft::closing(K_DECLINE, c, &st.head, &parties, reason)
            .content(json!({ "reason": reason, "note": note.into() }))
    }

    /// The move on a fresh T0/T1 `REQUEST`: decline, or do the work and REPORT.
    async fn answer(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        st: &acn_core::State,
        terms: &Terms,
        events: &[Event],
        key: &str,
    ) -> Result<Move, String> {
        let decline =
            |reason: &str, note: String| Ok(Move::act(self.decline_draft(c, cm, st, reason, note)));
        let now = now_secs();
        if let Err((reason, note)) = self.authority(K_REPORT, terms, now) {
            return decline(reason, note);
        }
        if let Some((reason, note)) = self.refusal(terms, cm.profile, now) {
            return decline(reason, note);
        }
        match self.executor.quote(terms, cm.profile) {
            Quote::Accept { .. } => {}
            Quote::Decline { reason, note } => return decline(&reason, note),
            Quote::Counter { .. } => {
                return decline(
                    "profile_unsupported",
                    "this request is T0/T1; counters need profile N".into(),
                )
            }
        }
        if terms.price.amount > 0 {
            if let Err(why) = self.check_token(c, cm, terms, events).await {
                return decline("payment_invalid", why);
            }
        }
        self.work(c, cm, st, terms, events, key, vec![]).await
    }

    /// What this binding declines before running anything (docs/harness-binding-API.md §1):
    /// what L0 can't meet, less what L1 adds when negotiating.
    fn refusal(&self, terms: &Terms, profile: Profile, now: u64) -> Option<(&'static str, String)> {
        let n = profile == Profile::N;
        if !terms.settlement_trigger.is_requester() {
            return Some((
                "trigger_unsupported",
                "this binding settles through the requester only".into(),
            ));
        }
        if n && !self.negotiate {
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
            if profile == Profile::T0 || pay.is_none_or(|p| p.hashlock.is_none()) {
                return Some((
                    "payment_unsupported",
                    "paid calls are T1 or N with a hash-locked token".into(),
                ));
            }
            // Core §8.2: the refund path must not open before this binding can redeem.
            let need = terms
                .deadline
                .saturating_add(terms.review_window)
                .saturating_add(self.redeem_margin_secs);
            match pay.and_then(|p| p.locktime) {
                Some(l) if l > need => {}
                l => {
                    return Some((
                        "locktime_too_early",
                        format!(
                        "the payment's locktime ({}) must be after the deadline plus the review \
                             window plus {} s to redeem ({need})",
                        l.map_or("none".to_string(), |l| l.to_string()),
                        self.redeem_margin_secs
                    ),
                    ))
                }
            }
        }
        // Profile N closes open conditions by counter; T0/T1 can't.
        if !n {
            if let Some(why) = terms.unclosed() {
                return Some(("terms_unclosed", why));
            }
        }
        if terms.has_instances() && !n {
            return Some((
                "instances_unsupported",
                "held-out instances need a performer seed committed before delivery: profile N"
                    .into(),
            ));
        }
        if !terms.disqualifiers.is_empty() && !self.negotiate {
            return Some((
                "disqualifiers_unsupported",
                "an L0 binding doesn't take terms with disqualifiers".into(),
            ));
        }
        if !self.can_meet_governance(terms) {
            // Its acts would wait for an endorsement from a key its gate doesn't hold.
            return Some((
                "governance_unsupported",
                "the terms require performer acts endorsed by a key this binding's gate doesn't \
                 hold"
                    .into(),
            ));
        }
        if now > terms.deadline {
            return Some(("expired", "the request's deadline has passed".into()));
        }
        None
    }

    /// The requester's token, NIP-44 to us in the REQUEST: decrypt, verify with the mint, check
    /// it against the terms (amount, hashlock, receiver, locktime, refund key, unspent).
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
        acn_wallet::settlement::check_payment_from(
            terms,
            &v,
            &self.signer.pubkey(),
            &cm.requester,
        )?;
        Ok(token)
    }

    /// The REPORT draft. With public disclosure: artifacts, log and the derivation record by
    /// hash, computed locally (uploaded only once the REPORT may be sent). Otherwise the inline
    /// result only.
    fn report(
        &self,
        c: &str,
        st: &acn_core::State,
        parties: &[&str],
        terms: &Terms,
        mut output: Output,
        started: u64,
    ) -> Result<Move, String> {
        if self.level().is_none() {
            // Private (or no Evidence store): the inline result is the whole report.
            let Some(r) = output.result else {
                return Err(
                    "the work produced only artifacts, and this binding publishes no evidence"
                        .into(),
                );
            };
            let size = serde_json::to_vec(&r).map(|b| b.len()).unwrap_or(0);
            if size > INLINE_MAX {
                return Err(format!(
                    "the result is {size} bytes, over the 16 KiB inline limit, and this binding \
                     publishes no evidence"
                ));
            }
            return Ok(Move {
                draft: Draft::report(
                    c,
                    &st.head,
                    parties,
                    Some(r),
                    output.contract.as_deref(),
                    &[],
                ),
                blobs: vec![],
                terms: None,
            });
        }
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
        let mut blobs = Vec::new();
        let mut outputs = Vec::new();
        for a in output.artifacts {
            let h = acn_core::jcs::sha256_hex(&a.bytes);
            outputs.push(json!({"hash": h, "role": a.role, "type": a.media_type}));
            blobs.push((a.bytes, a.media_type));
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
            "ended": now_secs(),
        });
        if let Some(log) = output.log {
            record["log"] = json!(acn_core::jcs::sha256_hex(&log));
            blobs.push((log, "text/plain".into()));
        }
        let canonical = acn_core::jcs::canonical(&record).map_err(|e| e.to_string())?;
        let derivation = acn_core::jcs::sha256_hex(canonical.as_bytes());
        blobs.push((canonical.into_bytes(), "application/json".into()));
        let artifacts: Vec<String> = record["outputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|o| o["role"] != "inline_result")
            .filter_map(|o| o["hash"].as_str().map(str::to_owned))
            .collect();
        let refs: Vec<&str> = artifacts.iter().map(String::as_str).collect();
        Ok(Move {
            draft: Draft::report(
                c,
                &st.head,
                parties,
                output.result,
                output.contract.as_deref(),
                &refs,
            )
            .field("derivation", json!(derivation)),
            blobs,
            terms: None,
        })
    }

    /// Send `first`; if it is a REPORT that may not be sent, a DECLINE saying why. A move that
    /// can't be sent at all halts the commitment (journaled; nothing more is signed for it).
    #[allow(clippy::too_many_arguments)]
    async fn commit(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        st: &acn_core::State,
        terms: &Terms,
        key: &str,
        first: Move,
        change: &mut StateChange,
    ) -> Result<(), String> {
        let mut mv = first;
        loop {
            match self.prepare(c, cm, terms, &mv, st.head_at).await {
                Ok((act, extra)) => {
                    let entry = Entry {
                        commitment: c.to_string(),
                        terms: st.terms.clone(),
                        to: cm.requester.clone(),
                        step: Step::Prepared,
                        act: Some(act.clone()),
                        extra,
                        executed: self.journal.entry(key).is_some_and(|e| e.executed),
                        at: now_secs(),
                    };
                    // Journaled before it leaves: a restart re-sends this act, never re-signs.
                    self.journal.put(key, entry.clone())?;
                    return self.deliver(key, entry, act, change).await;
                }
                // Any move but a refusal that can't go out (a REPORT the gate or the validator
                // won't pass, a PROMISE the gate denies) is replaced by a refusal saying why.
                Err((reason, note)) if !matches!(mv.draft.kind, K_DECLINE | K_REVOKE) => {
                    tracing::warn!(commitment = %c, act = acn_core::kinds::name(mv.draft.kind), %reason, %note, "binding: act withheld");
                    mv = Move::act(self.refuse_draft(c, cm, st, reason, note));
                }
                Err((reason, note)) => {
                    let executed = self.journal.entry(key).is_some_and(|e| e.executed);
                    self.journal.put(
                        key,
                        Entry {
                            commitment: c.to_string(),
                            terms: st.terms.clone(),
                            to: cm.requester.clone(),
                            step: Step::Halted {
                                reason: format!("{reason}: {note}"),
                            },
                            act: None,
                            extra: vec![],
                            executed,
                            at: now_secs(),
                        },
                    )?;
                    return Err(format!("halted, nothing sent ({reason}: {note})"));
                }
            }
        }
    }

    /// Rule 1 (gate), V-8 (local authority) and rule 5 (the validator applies it), then the
    /// evidence uploads: the act that may be sent, or the reason code for why not. Signing is
    /// local; an act that fails here is discarded and never leaves the process.
    async fn prepare(
        &self,
        c: &str,
        cm: &acn_core::Commitment,
        terms: &Terms,
        mv: &Move,
        not_before: u64,
    ) -> Result<(Event, Vec<Event>), (&'static str, String)> {
        let human = match &self.gate {
            Some(g) => match g.evaluate(&mv.draft, cm).await {
                GateDecision::Deny { reason } => return Err(("gate_denied", reason)),
                GateDecision::AllowedByHuman => true,
                GateDecision::Allow => false,
            },
            None => false,
        };
        self.authority(
            mv.draft.kind,
            mv.terms.as_ref().unwrap_or(terms),
            now_secs().max(not_before),
        )?;
        let act = self
            .signer
            .sign(&mv.draft, Some(not_before))
            .map_err(|e| ("signer_refused", e))?;
        // V-14 against the terms in force at the act's prev, as the validator checks it.
        let governed = terms.governance.as_ref().is_some_and(|g| {
            g.party == GovernedParty::Performer && g.kinds.contains(&mv.draft.kind)
        });
        let mut extra = Vec::new();
        if governed {
            let Some(g) = self
                .gate
                .as_ref()
                .filter(|_| self.can_meet_governance(terms))
            else {
                return Err((
                    "governance_unsupported",
                    "the act needs an endorsement this binding's gate can't give".into(),
                ));
            };
            extra.push(g.endorse(&act, human).map_err(|e| ("gate_denied", e))?);
        }
        self.applied(c, &act, &extra)?;
        if let Some(ev) = self.evidence.as_ref().filter(|_| !mv.blobs.is_empty()) {
            for (bytes, media_type) in &mv.blobs {
                let want = acn_core::jcs::sha256_hex(bytes);
                match ev.put(bytes.clone(), media_type.clone()).await {
                    Ok(h) if h == want => {}
                    Ok(h) => {
                        return Err((
                            "evidence_unavailable",
                            format!("the evidence store returned {h} for a blob hashing to {want}"),
                        ))
                    }
                    Err(e) => return Err(("evidence_unavailable", e)),
                }
            }
        }
        Ok((act, extra))
    }

    /// Fold the candidate (with its endorsement, if governed) with what we know: it must be
    /// applied (on the effective chain), not merely free of rejections. A governed act that
    /// would wait for an endorsement, a stale or out-of-scope one, isn't.
    fn applied(&self, c: &str, act: &Event, extra: &[Event]) -> Result<(), (&'static str, String)> {
        let mut events = self.events(c);
        events.push(act.clone());
        events.extend(extra.iter().cloned());
        let out = fold(&events, &self.docs());
        if let Some(a) = out
            .anomalies
            .iter()
            .find(|a| a.event == act.id && a.code.severity() != Severity::Flag)
        {
            let reason = match a.code {
                Code::PendingEndorsement | Code::Unendorsed => "governance_unsupported",
                Code::OutOfScope => "out_of_scope",
                Code::Revoked => "binding_revoked",
                _ => "refused_locally",
            };
            return Err((
                reason,
                format!(
                    "the validator would not apply it ({}: {})",
                    a.code.as_str(),
                    a.detail
                ),
            ));
        }
        if !out
            .commitments
            .get(c)
            .is_some_and(|x| x.chain.contains(&act.id))
        {
            return Err((
                "refused_locally",
                "the validator would not put it on the commitment's chain".into(),
            ));
        }
        Ok(())
    }

    /// Send a journaled act. It enters the store only once delivered; on failure it stays
    /// `Prepared` and the next drive (or [`Performer::retry_due`]) re-sends the same event.
    async fn deliver(
        &self,
        key: &str,
        entry: Entry,
        act: Event,
        change: &mut StateChange,
    ) -> Result<(), String> {
        // The act, then what goes with it (its endorsement). All or nothing: a failure leaves
        // the entry `Prepared`, and the same events are sent again.
        for e in std::iter::once(&act).chain(&entry.extra) {
            self.inbox
                .send(&entry.to, e)
                .await
                .map_err(|e| format!("send failed, will retry the same act: {e}"))?;
        }
        self.store.add(act.clone());
        for e in &entry.extra {
            self.store.add(e.clone());
        }
        if let Err(e) = self.journal.put(
            key,
            Entry {
                step: Step::Sent,
                ..entry.clone()
            },
        ) {
            // Delivered either way; a `Prepared` entry only means one redundant re-send.
            tracing::warn!(error = %e, "binding: journal not updated after send");
        }
        tracing::info!(commitment = %entry.commitment, act = acn_core::kinds::name(act.kind), "binding: sent");
        change.sent.push(act);
        Ok(())
    }

    /// Satisfied with a preimage: redeem the requester's token. Recorded as redeemed only once
    /// the wallet says so; failures are retried with backoff; an "already spent" answer (ours
    /// from before, or the requester's refund) is left for reconciliation, not guessed at.
    async fn redeem(&self, c: &str, events: &[Event]) -> Result<u64, String> {
        let Some(w) = &self.wallet else { return Ok(0) };
        let now = now_secs();
        let attempts = match self.journal.redemption(c) {
            Some(Redemption::Redeemed { .. }) | Some(Redemption::Reconcile { .. }) => return Ok(0),
            Some(Redemption::Pending { next_at, .. }) if now < next_at => return Ok(0),
            Some(Redemption::Pending { attempts, .. }) => attempts,
            None => 0,
        };
        let Some(acn_wallet::settlement::Advice::Redeem { preimage, .. }) =
            acn_wallet::settlement::advise_with(
                events,
                &self.docs(),
                c,
                &self.signer.pubkey(),
                u64::MAX / 2,
            )
        else {
            return Ok(0);
        };
        let out = fold(events, &self.docs());
        let requester = out
            .commitments
            .get(c)
            .map(|x| x.requester.clone())
            .unwrap_or_default();
        let ct = acn_wallet::settlement::request_token(events, c).ok_or("no token")?;
        let token = self.signer.decrypt(&requester, &ct)?;
        match w.redeem(&token, &preimage, c).await {
            Ok(got) => {
                self.journal
                    .set_redemption(c, Redemption::Redeemed { sat: got, at: now })?;
                tracing::info!(commitment = %c, sat = got, "binding: redeemed");
                Ok(got)
            }
            Err(e) if e.to_ascii_lowercase().contains("spent") => {
                tracing::warn!(commitment = %c, error = %e, "binding: token already spent; needs reconciliation");
                self.journal.set_redemption(
                    c,
                    Redemption::Reconcile {
                        error: e.clone(),
                        at: now,
                    },
                )?;
                Err(e)
            }
            Err(e) => {
                let attempts = attempts + 1;
                let r = if attempts >= MAX_REDEEM_ATTEMPTS {
                    tracing::warn!(commitment = %c, error = %e, "binding: redemption keeps failing; needs reconciliation");
                    Redemption::Reconcile {
                        error: format!("gave up after {attempts} attempts: {e}"),
                        at: now,
                    }
                } else {
                    let delay = self
                        .retry_base_secs
                        .saturating_mul(1 << (attempts - 1).min(10))
                        .min(3600);
                    Redemption::Pending {
                        attempts,
                        next_at: now + delay,
                        last_error: e.clone(),
                    }
                };
                self.journal.set_redemption(c, r)?;
                Err(e)
            }
        }
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
    /// The governance the binding can meet: `{keys, policy}` of its gate (L2).
    pub governance: Option<Value>,
    pub data_handling: Value,
    pub deadline_secs: u64,
    pub review_window_secs: u64,
}

impl ServiceAd {
    pub fn with_governance(mut self, g: Option<Value>) -> Self {
        self.governance = g;
        self
    }

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
        if let Some(g) = &self.governance {
            v["governance"] = g.clone();
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
        /// Test "ciphertexts" are the plaintext.
        fn decrypt(&self, _: &str, ct: &str) -> Result<String, String> {
            Ok(ct.to_string())
        }
        fn derive_secret(&self, label: &str) -> Result<[u8; 32], String> {
            let mut k = self.0;
            for (i, b) in label.bytes().enumerate() {
                k[i % 32] ^= b;
            }
            Ok(k)
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
        fn fetch<'a>(&'a self, h: &'a str, _: &'a [String]) -> BoxFut<'a, Result<Vec<u8>, String>> {
            let b = self.0.lock().unwrap().get(h).cloned();
            Box::pin(async move { b.ok_or_else(|| "missing".to_string()) })
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
            .with_evidence(ev.clone())
            .with_disclosure(Disclosure::Public);
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

    /// Echo, counting the work it does.
    #[derive(Default)]
    struct Counting(std::sync::atomic::AtomicUsize);
    impl Counting {
        fn n(&self) -> usize {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
    }
    impl Executor for Counting {
        fn quote(&self, t: &Terms, p: Profile) -> Quote {
            Echo.quote(t, p)
        }
        fn run(&self, job: Job) -> BoxFut<'_, Result<Output, String>> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Echo.run(job)
        }
    }
    impl Executor for Arc<Counting> {
        fn quote(&self, t: &Terms, p: Profile) -> Quote {
            (**self).quote(t, p)
        }
        fn run(&self, job: Job) -> BoxFut<'_, Result<Output, String>> {
            (**self).run(job)
        }
    }

    /// Evidence counting uploads.
    #[derive(Default)]
    struct Uploads(Mem, std::sync::atomic::AtomicUsize);
    impl Evidence for Uploads {
        fn put(&self, b: Vec<u8>, t: String) -> BoxFut<'_, Result<String, String>> {
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.0.put(b, t)
        }
        fn servers(&self) -> Vec<String> {
            self.0.servers()
        }
        fn fetch<'a>(&'a self, h: &'a str, s: &'a [String]) -> BoxFut<'a, Result<Vec<u8>, String>> {
            self.0.fetch(h, s)
        }
    }

    /// An inbox whose first `fail` sends fail; records every attempt.
    #[derive(Default)]
    struct Flaky {
        fail: Mutex<usize>,
        attempts: Mutex<Vec<String>>,
        delivered: Mutex<Vec<Event>>,
    }
    impl Inbox for Flaky {
        fn send<'a>(&'a self, _: &'a str, act: &'a Event) -> BoxFut<'a, Result<(), String>> {
            self.attempts.lock().unwrap().push(act.id.clone());
            let mut f = self.fail.lock().unwrap();
            let r = if *f > 0 {
                *f -= 1;
                Err("relay unreachable".to_string())
            } else {
                self.delivered.lock().unwrap().push(act.clone());
                Ok(())
            };
            Box::pin(async move { r })
        }
    }

    /// A wallet at `mint:`: a token is the JSON of its [`Verified`] facts; `redeem` fails while
    /// `errors` has entries.
    #[derive(Default)]
    struct FakeWallet {
        errors: Mutex<Vec<String>>,
        redeems: Mutex<usize>,
    }
    impl Wallet for FakeWallet {
        fn mint(&self) -> String {
            "mint:".into()
        }
        fn capabilities(&self) -> Vec<String> {
            vec!["hashlock".into()]
        }
        fn verify<'a>(&'a self, token: &'a str) -> BoxFut<'a, Result<Verified, String>> {
            Box::pin(async move { serde_json::from_str(token).map_err(|e| e.to_string()) })
        }
        fn redeem<'a>(
            &'a self,
            t: &'a str,
            _: &'a str,
            _: &'a str,
        ) -> BoxFut<'a, Result<u64, String>> {
            *self.redeems.lock().unwrap() += 1;
            let e = self.errors.lock().unwrap().pop();
            let amount = serde_json::from_str::<Verified>(t)
                .map(|v| v.amount)
                .unwrap_or(0);
            Box::pin(async move { e.map_or(Ok(amount), Err) })
        }
    }

    struct DenyReports;
    impl Gate for DenyReports {
        fn evaluate<'a>(
            &'a self,
            d: &'a Draft,
            _: &'a acn_core::Commitment,
        ) -> BoxFut<'a, GateDecision> {
            let r = if d.kind == K_REPORT {
                GateDecision::Deny {
                    reason: "no reports today".into(),
                }
            } else {
                GateDecision::Allow
            };
            Box::pin(async move { r })
        }
    }

    const PREIMAGE: [u8; 32] = [0x11; 32];

    /// A paid T1 request for `sat` at `mint:` from key 1 to key 2, with a matching token.
    fn paid_request(sat: u64) -> Event {
        let me = public_key(&[2; 32]).unwrap();
        let requester = public_key(&[1; 32]).unwrap();
        let h = acn_core::jcs::sha256_hex(&PREIMAGE);
        let offer: acn_core::draft::Template =
            serde_json::from_value(call_offer("echo", sat, Some("mint:"), 600, 3600)).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let terms = acn_core::draft::instantiate(
            &offer,
            "echo",
            json!({"text": "hi"}),
            now,
            Some(&h),
            None,
        )
        .unwrap();
        let token = serde_json::to_string(&Verified {
            amount: sat,
            hashlock: h,
            receiver: me.clone(),
            locktime: terms["payment"]["locktime"].as_u64().unwrap(),
            unspent: true,
            receivers: vec![me.clone()],
            n_sigs: 1,
            refund: vec![requester],
            n_sigs_refund: 1,
            sig_flag: acn_wallet::SIG_INPUTS.into(),
            keysets: vec![],
        })
        .unwrap();
        Draft::request(&me, "T1", &terms, Some(&token))
            .unwrap()
            .sign(&[1; 32], 100)
            .unwrap()
    }

    /// An operator (key 9) binding of key 2 with `max_sat`, acknowledged; optionally revoked.
    fn binding(max_sat: u64, revoked: bool) -> Vec<Event> {
        let me = public_key(&[2; 32]).unwrap();
        let kinds = [K_DECLINE, K_REPORT, K_PROGRESS]
            .map(|k| k.to_string())
            .join(",");
        let max = max_sat.to_string();
        let bind = Draft::bind(&me, u64::MAX / 4, &[("kinds", &kinds), ("max_sat", &max)])
            .sign(&[9; 32], 10)
            .unwrap();
        let ack = Draft::bind_ack(&bind.id).sign(&[2; 32], 11).unwrap();
        let mut v = vec![bind.clone(), ack];
        if revoked {
            v.push(Draft::bind_revoke(&bind.id).sign(&[9; 32], 12).unwrap());
        }
        v
    }

    fn reason_of(e: &Event) -> String {
        serde_json::from_str::<Value>(&e.content).unwrap()["reason"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    // A03: a corrupt copy of a verified act (same id) must not replace it.
    #[tokio::test]
    async fn corrupt_copy_does_not_replace_a_verified_report() {
        let work = Arc::new(Counting::default());
        let p = Performer::new(
            Arc::new(Key([2; 32])),
            Arc::new(Sent::default()),
            Arc::new(work.clone()),
        );
        let ch = p
            .on_event(request(&public_key(&[2; 32]).unwrap(), |_| {}))
            .await;
        assert_eq!((ch.to, work.n()), (Some(Phase::Reported), 1));
        let mut corrupt = ch.sent[0].clone();
        corrupt.content = json!({"result": "forged"}).to_string();
        let ch2 = p.on_event(corrupt).await;
        assert_eq!(ch2.to, Some(Phase::Reported), "{ch2:?}");
        assert_eq!(ch2.anomalies[0].code, Code::BadSig);
        assert!(ch2.sent.is_empty());
        assert_eq!(work.n(), 1, "the corrupt copy made the work run again");
        let kept = p.store.events(&ch.commitment);
        assert!(
            kept.iter().any(|e| e == &ch.sent[0]),
            "the verified REPORT was replaced"
        );
    }

    // A04: the operator's max_sat holds even above the advertised price.
    #[tokio::test]
    async fn request_above_operator_scope_is_not_worked() {
        let work = Arc::new(Counting::default());
        let sent = Arc::new(Sent::default());
        let p = Performer::new(Arc::new(Key([2; 32])), sent.clone(), Arc::new(work.clone()))
            .with_wallet(Arc::new(FakeWallet::default()))
            .with_min_price(5);
        for e in binding(5, false) {
            p.on_event(e).await;
        }
        let ch = p.on_event(paid_request(10)).await;
        assert_eq!(work.n(), 0, "worked a request above the operator's max_sat");
        assert!(sent.0.lock().unwrap().iter().all(|e| e.kind != K_REPORT));
        // A DECLINE commits to no value: it is sent, says why, and the fold applies it cleanly.
        assert_eq!(ch.sent.len(), 1, "{ch:?}");
        let d = &ch.sent[0];
        assert_eq!((d.kind, reason_of(d).as_str()), (K_DECLINE, "out_of_scope"));
        assert_eq!(ch.to, Some(Phase::Declined));
        let mut events = p.store.events(&ch.commitment);
        events.extend(binding(5, false));
        let out = fold(&events, &BTreeMap::new());
        assert!(out.commitments[&ch.commitment].chain.contains(&d.id));
        assert!(
            out.anomalies_for(&d.id).next().is_none(),
            "{:?}",
            out.anomalies
        );
        assert!(p
            .journal()
            .entries_for(&ch.commitment)
            .iter()
            .all(|(_, e)| e.step == Step::Sent));
        // Another mint than the request's payment is out of scope the same way.
        let mint_scoped = Performer::new(
            Arc::new(Key([2; 32])),
            Arc::new(Sent::default()),
            Arc::new(work.clone()),
        )
        .with_wallet(Arc::new(FakeWallet::default()))
        .with_min_price(5);
        let me = public_key(&[2; 32]).unwrap();
        let bind = Draft::bind(
            &me,
            u64::MAX / 4,
            &[
                ("kinds", &format!("{K_DECLINE},{K_REPORT}")),
                ("max_sat", "100"),
                ("mint", "https://other.mint"),
            ],
        )
        .sign(&[9; 32], 10)
        .unwrap();
        let ack = Draft::bind_ack(&bind.id).sign(&[2; 32], 11).unwrap();
        mint_scoped.add_context([bind, ack]);
        let m = mint_scoped.on_event(paid_request(5)).await;
        assert_eq!(work.n(), 0, "worked a request paid at a mint outside scope");
        assert_eq!(
            (m.sent[0].kind, reason_of(&m.sent[0]).as_str()),
            (K_DECLINE, "out_of_scope")
        );
        // Within scope, the same binding works.
        let ok = p.on_event(paid_request(5)).await;
        assert_eq!((ok.to, work.n()), (Some(Phase::Reported), 1), "{ok:?}");
    }

    // A04: a revoked binding stops work.
    #[tokio::test]
    async fn revoked_binding_stops_work() {
        let work = Arc::new(Counting::default());
        let sent = Arc::new(Sent::default());
        let p = Performer::new(Arc::new(Key([2; 32])), sent.clone(), Arc::new(work.clone()));
        for e in binding(100, true) {
            p.on_event(e).await;
        }
        let ch = p
            .on_event(request(&public_key(&[2; 32]).unwrap(), |_| {}))
            .await;
        assert_eq!(work.n(), 0, "worked under a revoked binding");
        // A revoked key can sign nothing, not even a DECLINE: silence, halted in the journal.
        assert!(sent.0.lock().unwrap().is_empty());
        let entries = p.journal().entries_for(&ch.commitment);
        assert!(
            matches!(&entries[..], [(_, e)] if matches!(&e.step, Step::Halted { reason } if reason.starts_with("binding_revoked"))),
            "{entries:?}"
        );
    }

    // §3: governance an L0 binding can't satisfy is declined before any work.
    #[tokio::test]
    async fn governance_is_declined_before_work() {
        let work = Arc::new(Counting::default());
        let p = Performer::new(
            Arc::new(Key([2; 32])),
            Arc::new(Sent::default()),
            Arc::new(work.clone()),
        );
        let gov = public_key(&[7; 32]).unwrap();
        let ch = p
            .on_event(request(&public_key(&[2; 32]).unwrap(), |t| {
                t["governance"] = json!({"keys": [gov], "kinds": [K_REPORT], "window": 60})
            }))
            .await;
        assert_eq!(
            work.n(),
            0,
            "worked terms that need an endorsement it can't get"
        );
        assert_eq!(ch.sent.len(), 1, "{ch:?}");
        assert_eq!(
            (ch.sent[0].kind, reason_of(&ch.sent[0]).as_str()),
            (K_DECLINE, "governance_unsupported")
        );
    }

    // A08: a failed send is retried with the same signed act, not forgotten or redone.
    #[tokio::test]
    async fn failed_send_is_retried_with_the_same_act() {
        let work = Arc::new(Counting::default());
        let inbox = Arc::new(Flaky::default());
        *inbox.fail.lock().unwrap() = 1;
        let p = Performer::new(
            Arc::new(Key([2; 32])),
            inbox.clone(),
            Arc::new(work.clone()),
        );
        let ch = p
            .on_event(request(&public_key(&[2; 32]).unwrap(), |_| {}))
            .await;
        assert!(ch.sent.is_empty());
        assert_eq!(
            ch.to,
            Some(Phase::Requested),
            "an unsent act entered the store"
        );
        let again = p.drive(&ch.commitment).await;
        assert_eq!(again.sent.len(), 1, "{again:?}");
        assert_eq!(again.to, Some(Phase::Reported));
        let attempts = inbox.attempts.lock().unwrap().clone();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0], attempts[1], "the retry signed a different act");
        assert_eq!(work.n(), 1, "the retry redid the work");
    }

    // A08: a failed redemption is not recorded as done.
    #[tokio::test]
    async fn failed_redemption_is_retried() {
        let wallet = Arc::new(FakeWallet::default());
        wallet
            .errors
            .lock()
            .unwrap()
            .push("mint unreachable".into());
        let p = Performer::new(
            Arc::new(Key([2; 32])),
            Arc::new(Sent::default()),
            Arc::new(Echo),
        )
        .with_wallet(wallet.clone())
        .with_min_price(5)
        .with_retry_backoff(0);
        let ch = p.on_event(paid_request(5)).await;
        assert_eq!(ch.to, Some(Phase::Reported), "{ch:?}");
        let (me, requester) = (public_key(&[2; 32]).unwrap(), public_key(&[1; 32]).unwrap());
        let pre: String = PREIMAGE.iter().map(|b| format!("{b:02x}")).collect();
        let sat = Draft::satisfy(
            &ch.commitment,
            &ch.sent[0].id,
            &[&requester, &me],
            Some(&pre),
        )
        .sign(&[1; 32], 200)
        .unwrap();
        let s = p.on_event(sat).await;
        assert_eq!((s.to, s.redeemed), (Some(Phase::Satisfied), 0));
        let r = p.drive(&ch.commitment).await;
        assert_eq!(r.redeemed, 5, "the failed redemption was never retried");
        assert_eq!(*wallet.redeems.lock().unwrap(), 2);
        assert_eq!(p.drive(&ch.commitment).await.redeemed, 0, "redeemed twice");
    }

    // A16: nothing is uploaded before the gate has allowed the REPORT.
    #[tokio::test]
    async fn gate_denial_uploads_nothing() {
        let ev = Arc::new(Uploads::default());
        let p = Performer::new(
            Arc::new(Key([2; 32])),
            Arc::new(Sent::default()),
            Arc::new(Echo),
        )
        .with_evidence(ev.clone())
        .with_disclosure(Disclosure::Public)
        .with_gate(Arc::new(DenyReports));
        let ch = p
            .on_event(request(&public_key(&[2; 32]).unwrap(), |_| {}))
            .await;
        assert!(ch.sent.iter().all(|e| e.kind != K_REPORT));
        assert_eq!(
            ev.1.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "uploaded evidence the gate denied"
        );
        // The work ran, so the requester is told why there's no REPORT.
        assert_eq!(
            (ch.sent[0].kind, reason_of(&ch.sent[0]).as_str()),
            (K_DECLINE, "gate_denied")
        );
    }

    // A16: private disclosure (the default) uploads nothing and claims no level.
    #[tokio::test]
    async fn private_disclosure_uploads_nothing() {
        let ev = Arc::new(Uploads::default());
        let p = Performer::new(
            Arc::new(Key([2; 32])),
            Arc::new(Sent::default()),
            Arc::new(Echo),
        )
        .with_evidence(ev.clone());
        assert_eq!((p.disclosure, p.level()), (Disclosure::Private, None));
        let ch = p
            .on_event(request(&public_key(&[2; 32]).unwrap(), |_| {}))
            .await;
        assert_eq!(ch.to, Some(Phase::Reported), "{ch:?}");
        let r = &ch.sent[0];
        assert!(r.tags.iter().all(|t| t[0] != "x"), "artifact references");
        let content: Value = serde_json::from_str(&r.content).unwrap();
        assert!(content.get("derivation").is_none());
        assert_eq!(content["result"]["echo"]["text"], "hi");
        assert_eq!(ev.1.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    fn journal_path(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("acn-binding-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("journal.json")
    }

    // A02: a restart whose history lacks the REPORT (not arrived yet, or lost) doesn't redo the
    // work: the journal restores the act.
    #[tokio::test]
    async fn restart_does_not_reexecute() {
        let path = journal_path("restart");
        let work = Arc::new(Counting::default());
        let req = request(&public_key(&[2; 32]).unwrap(), |_| {});
        let first = Performer::new(
            Arc::new(Key([2; 32])),
            Arc::new(Sent::default()),
            Arc::new(work.clone()),
        )
        .with_journal(Journal::open(&path).unwrap());
        let ch = first.on_event(req.clone()).await;
        assert_eq!((ch.to, work.n()), (Some(Phase::Reported), 1));
        drop(first);
        // Restart: only the REQUEST has been replayed so far.
        let sent = Arc::new(Sent::default());
        let second = Performer::new(Arc::new(Key([2; 32])), sent.clone(), Arc::new(work.clone()))
            .with_journal(Journal::open(&path).unwrap());
        second.ingest(req).await;
        let all = second.drive_all().await;
        assert_eq!(work.n(), 1, "the restart ran the work again");
        assert_eq!(all[0].to, Some(Phase::Reported), "{all:?}");
        assert!(sent.0.lock().unwrap().is_empty(), "re-sent a delivered act");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    // A02: intent without an outcome is not re-run: declined `execution_uncertain`, unless our
    // act turns up in history.
    #[tokio::test]
    async fn interrupted_run_is_not_rerun() {
        let path = journal_path("interrupted");
        let req = request(&public_key(&[2; 32]).unwrap(), |_| {});
        let j = Journal::open(&path).unwrap();
        let c = req.id.clone();
        let terms = fold(std::slice::from_ref(&req), &BTreeMap::new()).commitments[&c]
            .state
            .clone()
            .unwrap()
            .terms;
        // Keyed by the head being answered: the REQUEST itself.
        j.put(
            &Journal::key(&c, &c),
            Entry {
                commitment: c.clone(),
                terms,
                to: public_key(&[1; 32]).unwrap(),
                step: Step::Executing,
                act: None,
                extra: vec![],
                executed: true,
                at: 1,
            },
        )
        .unwrap();
        drop(j);
        let work = Arc::new(Counting::default());
        let p = Performer::new(
            Arc::new(Key([2; 32])),
            Arc::new(Sent::default()),
            Arc::new(work.clone()),
        )
        .with_journal(Journal::open(&path).unwrap());
        let ch = p.on_event(req).await;
        assert_eq!(work.n(), 0, "re-ran an interrupted execution");
        assert_eq!(
            (ch.sent[0].kind, reason_of(&ch.sent[0]).as_str()),
            (K_DECLINE, "execution_uncertain")
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    // A01 (driver side): a requester the admission policy refuses gets nothing done.
    #[tokio::test]
    async fn admission_applies_to_replayed_history() {
        let work = Arc::new(Counting::default());
        let sent = Arc::new(Sent::default());
        let p = Performer::new(Arc::new(Key([2; 32])), sent.clone(), Arc::new(work.clone()))
            .with_admission(Arc::new(|_: &str| false));
        p.ingest(request(&public_key(&[2; 32]).unwrap(), |_| {}))
            .await;
        p.drive_all().await;
        assert_eq!(work.n(), 0);
        assert!(sent.0.lock().unwrap().is_empty());
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

    // ---------- profile N (L1) ----------

    const REQ: [u8; 32] = [1; 32];
    const PERF: [u8; 32] = [2; 32];
    const ARB: [u8; 32] = [3; 32];

    fn me() -> String {
        public_key(&PERF).unwrap()
    }
    fn requester() -> String {
        public_key(&REQ).unwrap()
    }

    /// A free profile-N request from key 1 to key 2.
    fn n_request(f: impl FnOnce(&mut Value)) -> Event {
        let mut t =
            acn_core::draft::tool_call_terms("echo", json!({"text": "hi"}), u64::MAX / 4, 3600, 0);
        t["rework_limit"] = json!(1);
        f(&mut t);
        Draft::request(&me(), "N", &t, None)
            .unwrap()
            .sign(&REQ, 100)
            .unwrap()
    }

    fn n_performer(work: Arc<Counting>, sent: Arc<Sent>) -> Performer {
        Performer::new(Arc::new(Key(PERF)), sent, Arc::new(work)).with_negotiation(true)
    }

    /// The requester's (or `key`'s) next act on `c`, signed after the head.
    fn act(
        p: &Performer,
        key: &[u8; 32],
        c: &str,
        d: impl FnOnce(&str, &[&str]) -> Draft,
    ) -> Event {
        let out = fold(&p.events(c), &p.docs());
        let st = out.commitments[c].state.clone().unwrap();
        let parties = [requester(), me()];
        let parties: Vec<&str> = parties.iter().map(String::as_str).collect();
        d(&st.head, &parties).sign(key, st.head_at + 1).unwrap()
    }

    fn kinds(ch: &StateChange) -> Vec<u16> {
        ch.sent.iter().map(|e| e.kind).collect()
    }

    fn clean(p: &Performer, c: &str) {
        let out = fold(&p.events(c), &p.docs());
        let mine: Vec<&Anomaly> = out
            .anomalies
            .iter()
            .filter(|a| a.code != Code::Unbound) // no operator binding in these unit tests
            .filter(|a| {
                p.events(c)
                    .iter()
                    .any(|e| e.id == a.event && e.pubkey == me())
            })
            .collect();
        assert!(mine.is_empty(), "anomalies on the binding's acts: {mine:?}");
    }

    #[tokio::test]
    async fn n_promise_then_work_then_satisfy() {
        let work = Arc::new(Counting::default());
        let p = n_performer(work.clone(), Arc::new(Sent::default()));
        assert_eq!(p.profiles(), ["T0", "N"]);
        let ch = p.on_event(n_request(|_| {})).await;
        assert_eq!(kinds(&ch), [K_PROMISE, K_REPORT], "{ch:?}");
        assert_eq!((ch.to, work.n()), (Some(Phase::Reported), 1));
        let c = ch.commitment.clone();
        let sat = act(&p, &REQ, &c, |h, pa| Draft::satisfy(&c, h, pa, None));
        assert_eq!(p.on_event(sat).await.to, Some(Phase::Satisfied));
        clean(&p, &c);
    }

    #[tokio::test]
    async fn n_open_condition_is_closed_by_counter_then_worked_after_accept() {
        let work = Arc::new(Counting::default());
        let p = n_performer(work.clone(), Arc::new(Sent::default()));
        let req = n_request(|t| {
            t["conditions"] = json!([{"id": "c1", "class": "open", "description": "Make it fast"}]);
        });
        let ch = p.on_event(req).await;
        assert_eq!(kinds(&ch), [K_COUNTER], "{ch:?}");
        assert_eq!(work.n(), 0, "worked before the terms were agreed");
        let body: Value = serde_json::from_str(&ch.sent[0].content).unwrap();
        assert_eq!(body["terms"]["conditions"][0]["class"], "human");
        assert!(body["reason"].as_str().unwrap().contains("c1"));
        let c = ch.commitment.clone();
        let h = ch.sent[0].values("terms")[0].to_string();
        let accept = act(&p, &REQ, &c, |head, pa| Draft::accept(&c, head, pa, &h));
        let ch = p.on_event(accept).await;
        assert_eq!(kinds(&ch), [K_REPORT], "{ch:?}");
        assert_eq!(work.n(), 1);
        clean(&p, &c);
    }

    #[tokio::test]
    async fn n_requester_counter_is_accepted_or_declined() {
        let work = Arc::new(Counting::default());
        let p = n_performer(work.clone(), Arc::new(Sent::default()));
        let req = n_request(|t| {
            t["conditions"] = json!([{"id": "c1", "class": "open", "description": "Make it fast"}]);
        });
        let ch = p.on_event(req).await;
        let c = ch.commitment.clone();
        let ours: Value = serde_json::from_str(&ch.sent[0].content).unwrap();
        // The requester counters back: closed, with a later deadline.
        let mut t = ours["terms"].clone();
        t["deadline"] = json!(t["deadline"].as_u64().unwrap() + 600);
        let counter = act(&p, &REQ, &c, |h, pa| {
            Draft::counter(&c, h, pa, &t, "more time").unwrap()
        });
        let ch = p.on_event(counter).await;
        assert_eq!(kinds(&ch), [K_ACCEPT, K_REPORT], "{ch:?}");
        let out = fold(&p.events(&c), &p.docs());
        assert_eq!(
            out.commitments[&c].state.as_ref().unwrap().terms,
            acn_core::jcs::hash(&t).unwrap()
        );
        clean(&p, &c);

        // A counter this binding can't meet (deadline gone): declined, no work.
        let p = n_performer(work.clone(), Arc::new(Sent::default()));
        let req = n_request(|t| {
            t["conditions"] = json!([{"id": "c1", "class": "open", "description": "Make it fast"}]);
        });
        let ch = p.on_event(req).await;
        let c = ch.commitment.clone();
        let ours: Value = serde_json::from_str(&ch.sent[0].content).unwrap();
        let mut t = ours["terms"].clone();
        t["deadline"] = json!(1000);
        let counter = act(&p, &REQ, &c, |h, pa| {
            Draft::counter(&c, h, pa, &t, "now").unwrap()
        });
        let before = work.n();
        let ch = p.on_event(counter).await;
        assert_eq!(kinds(&ch), [K_DECLINE], "{ch:?}");
        assert_eq!(reason_of(&ch.sent[0]), "expired");
        assert_eq!(work.n(), before);
        clean(&p, &c);
    }

    #[tokio::test]
    async fn n_rework_runs_again_with_the_rejection() {
        #[derive(Default)]
        struct Feedback(Mutex<Vec<Vec<Value>>>);
        impl Executor for Arc<Feedback> {
            fn quote(&self, t: &Terms, p: Profile) -> Quote {
                Echo.quote(t, p)
            }
            fn run(&self, job: Job) -> BoxFut<'_, Result<Output, String>> {
                self.0.lock().unwrap().push(job.feedback.clone());
                Echo.run(job)
            }
        }
        let fb = Arc::new(Feedback::default());
        let p = Performer::new(
            Arc::new(Key(PERF)),
            Arc::new(Sent::default()),
            Arc::new(fb.clone()),
        )
        .with_negotiation(true);
        let ch = p.on_event(n_request(|_| {})).await;
        let c = ch.commitment.clone();
        let cond = fold(&p.events(&c), &p.docs()).commitments[&c]
            .terms
            .as_ref()
            .unwrap()
            .conditions[0]
            .id
            .clone();
        let rej = act(&p, &REQ, &c, |h, pa| {
            Draft::reject(&c, h, pa, vec![(&cond, "shorter, please")])
        });
        let ch = p.on_event(rej).await;
        assert_eq!(kinds(&ch), [K_REPORT], "{ch:?}");
        let runs = fb.0.lock().unwrap().clone();
        assert_eq!(runs.len(), 2);
        assert!(runs[0].is_empty());
        assert_eq!(runs[1][0]["note"], "shorter, please");
        // The second REPORT answers the REJECT.
        let rej_id = p
            .events(&c)
            .iter()
            .find(|e| e.kind == K_REJECT)
            .unwrap()
            .id
            .clone();
        assert_eq!(ch.sent[0].e_marked("prev"), [rej_id.as_str()]);
        clean(&p, &c);
    }

    #[tokio::test]
    async fn n_held_out_seed_is_committed_then_revealed() {
        let work = Arc::new(Counting::default());
        let p = n_performer(work.clone(), Arc::new(Sent::default()));
        let req = n_request(|t| {
            t["rework_limit"] = json!(0);
            t["verifiers"] = json!([{"pubkey": requester(), "classes": ["mechanical"]}]);
            t["conditions"] = json!([{"id": "c1", "class": "mechanical", "description": "held-out tests pass",
                "instances": {"pool_description": "cases", "pool_rule": "curated", "pool_root": "cd".repeat(32),
                    "pool_size": 4, "sample_size": 2, "requester_seed_hash": "ef".repeat(32)}}]);
        });
        let ch = p.on_event(req).await;
        assert_eq!(kinds(&ch), [K_PROMISE, K_REPORT], "{ch:?}");
        let promise: Value = serde_json::from_str(&ch.sent[0].content).unwrap();
        let report: Value = serde_json::from_str(&ch.sent[1].content).unwrap();
        let seed = hex::decode(report["performer_seed"].as_str().unwrap()).unwrap();
        assert_eq!(
            promise["performer_seed_hash"],
            acn_core::jcs::sha256_hex(&seed)
        );
        clean(&p, &ch.commitment);
    }

    #[tokio::test]
    async fn n_contradicted_rejection_is_disputed() {
        let work = Arc::new(Counting::default());
        let p = n_performer(work.clone(), Arc::new(Sent::default()));
        let arbiter = public_key(&ARB).unwrap();
        let req = n_request(|t| {
            t["rework_limit"] = json!(0);
            t["arbiter"] = json!(arbiter);
            t["verifiers"] = json!([{"pubkey": arbiter, "classes": ["mechanical"]}]);
            t["conditions"] =
                json!([{"id": "tests", "class": "mechanical", "description": "tests pass"}]);
        });
        let ch = p.on_event(req).await;
        let c = ch.commitment.clone();
        let report = ch.sent[1].id.clone();
        let pass = Draft::attest(
            K_ATTEST_MECH,
            &c,
            &report,
            "tests",
            json!({"result": "pass", "method": "cargo test"}),
        )
        .sign(&ARB, 200)
        .unwrap();
        assert!(p.on_event(pass).await.sent.is_empty());
        let rej = act(&p, &REQ, &c, |h, pa| {
            Draft::reject(&c, h, pa, vec![("tests", "no")])
        });
        let ch = p.on_event(rej).await;
        assert_eq!(kinds(&ch), [K_DISPUTE], "{ch:?}");
        assert_eq!(ch.to, Some(Phase::Disputed));
        let ruling = act(&p, &ARB, &c, |h, pa| Draft::ruling(&c, h, pa, true, None));
        assert_eq!(p.on_event(ruling).await.to, Some(Phase::Satisfied));
        assert_eq!(work.n(), 1, "no rework at limit 0");
        clean(&p, &c);
    }

    #[tokio::test]
    async fn an_uncontradicted_rejection_stands() {
        let p = n_performer(Arc::new(Counting::default()), Arc::new(Sent::default()));
        let req = n_request(|t| {
            t["rework_limit"] = json!(0);
            t["arbiter"] = json!(public_key(&ARB).unwrap());
        });
        let ch = p.on_event(req).await;
        let c = ch.commitment.clone();
        let cond = fold(&p.events(&c), &p.docs()).commitments[&c]
            .terms
            .as_ref()
            .unwrap()
            .conditions[0]
            .id
            .clone();
        let rej = act(&p, &REQ, &c, |h, pa| {
            Draft::reject(&c, h, pa, vec![(&cond, "no")])
        });
        let ch = p.on_event(rej).await;
        assert!(ch.sent.is_empty(), "{ch:?}");
        assert_eq!(ch.to, Some(Phase::Rejected));
    }

    #[tokio::test]
    async fn paid_terms_need_a_locktime_after_the_review_window() {
        let p = Performer::new(
            Arc::new(Key(PERF)),
            Arc::new(Sent::default()),
            Arc::new(Echo),
        )
        .with_wallet(Arc::new(FakeWallet::default()))
        .with_redeem_margin(25 * 3600);
        // The offer's locktime is deadline + review + 24 h: one hour short of this margin.
        let ch = p.on_event(paid_request(10)).await;
        assert_eq!(kinds(&ch), [K_DECLINE], "{ch:?}");
        assert_eq!(reason_of(&ch.sent[0]), "locktime_too_early");
    }

    #[tokio::test]
    async fn an_interrupted_run_after_a_promise_is_revoked() {
        let work = Arc::new(Counting::default());
        let sent = Arc::new(Sent::default());
        let path = journal_path("interrupted-n");
        let req = n_request(|_| {});
        let p = n_performer(work.clone(), sent.clone()).with_journal(Journal::open(&path).unwrap());
        // Promise, then "crash" during the work: replay the PROMISE into a new performer whose
        // journal says the run started.
        let ch = p.on_event(req.clone()).await;
        let promise = ch.sent[0].clone();
        let key = Journal::key(&ch.commitment, &promise.id);
        let mut e = p.journal().entry(&key).unwrap();
        drop(p);
        e.step = Step::Executing;
        e.act = None;
        let j = Journal::open(&path).unwrap();
        j.put(&key, e).unwrap();
        drop(j);
        let p = n_performer(work.clone(), sent).with_journal(Journal::open(&path).unwrap());
        p.ingest(req).await;
        let ch = p.on_event(promise).await;
        assert_eq!(kinds(&ch), [K_REVOKE], "{ch:?}");
        assert_eq!(reason_of(&ch.sent[0]), "execution_uncertain");
        assert_eq!(work.n(), 1, "re-ran an interrupted job");
    }

    // ---------- governance (L2) ----------

    const GOV: [u8; 32] = [4; 32];

    fn gated(work: Arc<Counting>, policy: GatePolicy) -> Performer {
        n_performer(work, Arc::new(Sent::default()))
            .with_gate(Arc::new(PolicyGate::new(Arc::new(Key(GOV)), policy)))
    }

    fn governed(keys: &[String], kinds: &[u16]) -> impl FnOnce(&mut Value) {
        let g = json!({"keys": keys, "kinds": kinds, "window": 600});
        move |t: &mut Value| t["governance"] = g
    }

    #[tokio::test]
    async fn a_gate_endorses_the_governed_acts_it_allows() {
        let work = Arc::new(Counting::default());
        let p = gated(work.clone(), GatePolicy::default());
        assert_eq!(p.level(), None, "no public evidence: no level");
        let gk = public_key(&GOV).unwrap();
        assert_eq!(p.governance().unwrap()["keys"][0], gk);
        let ch = p
            .on_event(n_request(governed(
                std::slice::from_ref(&gk),
                &[K_PROMISE, K_REPORT],
            )))
            .await;
        assert_eq!(kinds(&ch), [K_PROMISE, K_REPORT], "{ch:?}");
        assert_eq!(ch.to, Some(Phase::Reported));
        let c = ch.commitment.clone();
        let ends: Vec<Event> = p
            .store
            .events(&c)
            .into_iter()
            .filter(|e| e.kind == K_ENDORSE)
            .collect();
        assert_eq!(ends.len(), 2);
        for (e, act) in ends.iter().zip(&ch.sent) {
            assert_eq!(e.pubkey, gk);
            assert!(ends
                .iter()
                .any(|x| x.e_marked("endorses") == [act.id.as_str()]));
            assert_eq!(e.values("endorser"), ["automated"]);
        }
        clean(&p, &c);
        // An independent fold with the endorsements: applied, no pending endorsement.
        let out = fold(&p.events(&c), &p.docs());
        assert!(!out
            .anomalies
            .iter()
            .any(|a| a.code == Code::PendingEndorsement));
    }

    #[tokio::test]
    async fn governance_by_another_key_is_declined() {
        let work = Arc::new(Counting::default());
        let p = gated(work.clone(), GatePolicy::default());
        let other = public_key(&[7; 32]).unwrap();
        let ch = p
            .on_event(n_request(governed(&[other], &[K_PROMISE])))
            .await;
        assert_eq!(kinds(&ch), [K_DECLINE], "{ch:?}");
        assert_eq!(reason_of(&ch.sent[0]), "governance_unsupported");
        assert_eq!(work.n(), 0);
        // Without a gate at all, the same.
        let p = n_performer(work.clone(), Arc::new(Sent::default()));
        let gk = public_key(&GOV).unwrap();
        let ch = p.on_event(n_request(governed(&[gk], &[K_PROMISE]))).await;
        assert_eq!(reason_of(&ch.sent[0]), "governance_unsupported");
    }

    #[tokio::test]
    async fn the_gate_policy_stops_work_and_refusals_pass() {
        let work = Arc::new(Counting::default());
        let p = gated(
            work.clone(),
            GatePolicy {
                tools: vec!["other".into()],
                ..Default::default()
            },
        );
        let gk = public_key(&GOV).unwrap();
        let ch = p
            .on_event(n_request(governed(&[gk], &[K_PROMISE, K_REPORT])))
            .await;
        // The PROMISE is denied; the DECLINE saying so passes the gate (refusals always do).
        assert_eq!(kinds(&ch), [K_DECLINE], "{ch:?}");
        assert_eq!(reason_of(&ch.sent[0]), "gate_denied");
        assert_eq!(work.n(), 0);
    }

    #[tokio::test]
    async fn a_person_signs_off_on_the_report() {
        let asked = Arc::new(Mutex::new(Vec::<u16>::new()));
        let log = asked.clone();
        let approver: Approver = Arc::new(move |d: Draft, _| {
            log.lock().unwrap().push(d.kind);
            Box::pin(async { true })
        });
        let gate = PolicyGate::new(
            Arc::new(Key(GOV)),
            GatePolicy {
                human_kinds: vec![K_REPORT],
                ..Default::default()
            },
        )
        .with_approver(approver, std::time::Duration::from_secs(5));
        let p = n_performer(Arc::new(Counting::default()), Arc::new(Sent::default()))
            .with_gate(Arc::new(gate));
        let gk = public_key(&GOV).unwrap();
        let ch = p
            .on_event(n_request(governed(&[gk], &[K_PROMISE, K_REPORT])))
            .await;
        assert_eq!(kinds(&ch), [K_PROMISE, K_REPORT], "{ch:?}");
        assert_eq!(*asked.lock().unwrap(), [K_REPORT]);
        let human: Vec<Event> = p
            .store
            .events(&ch.commitment)
            .into_iter()
            .filter(|e| e.kind == K_ENDORSE && e.values("endorser") == ["human"])
            .collect();
        assert_eq!(human.len(), 1);
        assert_eq!(human[0].e_marked("endorses"), [ch.sent[1].id.as_str()]);

        // A person who doesn't answer in time: denied, and the work's REPORT becomes a REVOKE.
        let slow: Approver = Arc::new(|_, _| {
            Box::pin(async {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                true
            })
        });
        let gate = PolicyGate::new(
            Arc::new(Key(GOV)),
            GatePolicy {
                human_kinds: vec![K_REPORT],
                ..Default::default()
            },
        )
        .with_approver(slow, std::time::Duration::from_millis(50));
        let p = n_performer(Arc::new(Counting::default()), Arc::new(Sent::default()))
            .with_gate(Arc::new(gate));
        let gk = public_key(&GOV).unwrap();
        let ch = p
            .on_event(n_request(governed(&[gk], &[K_PROMISE, K_REPORT])))
            .await;
        assert_eq!(kinds(&ch), [K_PROMISE, K_REVOKE], "{ch:?}");
        assert_eq!(reason_of(&ch.sent[1]), "gate_denied");
    }
}
