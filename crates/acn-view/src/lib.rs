//! ACN projections (docs/Core.md §12).
//!
//! Everything here is derived from `acn_core::fold` + `status_at`; nothing feeds back into
//! commitment semantics (INV-12). Pure and WASM-friendly, so the desktop app (via `nmesh-app`)
//! and the in-browser presenter mode (via `acn-wasm`) show exactly the same thing.

use std::collections::BTreeMap;

mod terms_card;
pub use terms_card::{
    template_records, CheckedBy, DiffKind, DiffLine, DisqualifierItem, DoneItem, InstancesView,
    PayIf, TemplateRecord, TemplateView, TermsCard, TermsVersionView, JUDGE_PRICE_THRESHOLD,
};

use acn_core::kinds::*;
use acn_core::terms::{Aggregation, Assurance, ConditionClass};
use acn_core::{
    fold, status_at, BindingStatus, Commitment, ConditionStatus, Event, Output, Party, Phase,
    Profile, Severity,
};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Requester,
    Performer,
    Arbiter,
    Verifier,
    Governance,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    /// On the effective chain.
    Applied,
    /// Annotating act (progress, attestation, endorsement) that was accepted.
    Annotation,
    /// Retained as evidence, not applied (INV-11).
    Rejected,
    /// Waiting for a dependency (terms, endorsement, reference).
    Pending,
}

#[derive(Debug, Clone, Serialize)]
pub struct Note {
    pub code: &'static str,
    pub severity: Severity,
    pub detail: String,
}

/// One signed event in the commitment, with what the validator made of it.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: String,
    pub act: &'static str,
    pub kind: u16,
    pub by: String,
    pub role: Role,
    pub mine: bool,
    pub created_at: u64,
    pub status: EntryStatus,
    pub summary: String,
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    pub id: String,
    pub act: &'static str,
    pub by: String,
    pub role: Role,
    pub verdict: String,
    /// Counted toward the condition (right class, listed verifier).
    pub counted: bool,
    pub why: Option<String>,
    /// What the attestation says, for the details panel (protocol content only: rationales
    /// travel by hash, D-19).
    pub detail: AttestationDetail,
}

/// The parts of an attestation a reader checks: how it was produced and on what.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AttestationDetail {
    pub created_at: u64,
    pub method: Option<String>,
    pub recipe: Option<String>,
    /// Grounding: the external referent.
    pub referent: Option<Value>,
    /// Judged: every judge's runs and median, the aggregate and the rubric.
    pub judges: Vec<JudgeScore>,
    pub aggregate_score: Option<f64>,
    pub rubric_text_hash: Option<String>,
    pub injection_detected: bool,
    /// Held-out instances revealed with proofs (V-27): the sampled indices.
    pub instances: Vec<u64>,
    /// Interpretive verdicts carry a rationale inline.
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JudgeScore {
    pub model_id: String,
    pub model_version: String,
    pub runs: Vec<f64>,
    pub median: Option<f64>,
}

fn attestation_detail(e: &Event, v: &Value) -> AttestationDetail {
    let st = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    AttestationDetail {
        created_at: e.created_at,
        method: st("method"),
        recipe: st("recipe"),
        referent: v.get("referent").cloned(),
        judges: v["judges"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|j| JudgeScore {
                model_id: j["model_id"].as_str().unwrap_or_default().to_string(),
                model_version: j["model_version"].as_str().unwrap_or_default().to_string(),
                runs: j["runs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_f64)
                    .collect(),
                median: j["median"].as_f64(),
            })
            .collect(),
        aggregate_score: v["aggregate_score"].as_f64(),
        rubric_text_hash: st("rubric_text_hash"),
        injection_detected: v["injection_detected"] == true,
        instances: v["instances"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["index"].as_u64())
            .collect(),
        rationale: st("rationale"),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConditionView {
    pub id: String,
    pub class: ConditionClass,
    /// Derived assurance label (spec §7); `None` for an `open` condition.
    pub assurance: Option<Assurance>,
    pub description: String,
    pub status: Option<ConditionStatus>,
    pub evidence: Vec<Evidence>,
    /// For `judged` conditions: who judges and against what (clients MUST show it, §12).
    pub judged: Option<JudgedView>,
}

/// A disqualifier against the current REPORT (CR-003 §2.2), with its own assurance label.
#[derive(Debug, Clone, Serialize)]
pub struct DisqualifierView {
    pub id: String,
    pub class: ConditionClass,
    pub assurance: Option<Assurance>,
    pub description: String,
    pub status: Option<acn_core::DisqualifierStatus>,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JudgedView {
    /// `model_id model_version` per judge.
    pub judges: Vec<String>,
    pub threshold: f64,
    pub scale_min: f64,
    pub scale_max: f64,
    pub aggregation: Aggregation,
    pub repeats: u32,
    /// Where the performer reads the rubric (INV-14), and the text when it is inline.
    pub rubric_ref: Option<String>,
    pub rubric_text: Option<String>,
    pub runners: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CounterView {
    pub by: Role,
    pub mine: bool,
    pub reason: String,
    pub terms_hash: String,
    pub deadline: u64,
    pub current_deadline: u64,
    pub price_sat: u64,
    pub current_price_sat: u64,
    pub rework_limit: u32,
    pub current_rework_limit: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementStatus {
    /// Hash-locked to the performer; refundable to the requester after `locktime`.
    Locked,
    /// Preimage revealed (SATISFY or performer-side RULING): the performer can redeem.
    Paid,
    /// Closed without satisfaction: the requester reclaims after `locktime`.
    Refundable,
    /// Contingent: an encrypted deliverable is offered; nothing is locked until FUND.
    Unfunded,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettlementView {
    pub amount_sat: u64,
    pub mint: String,
    pub simulated: bool,
    pub status: SettlementStatus,
    pub detail: String,
    pub locktime: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Allowed {
    pub satisfy: bool,
    pub reject: bool,
    pub withdraw: bool,
    pub accept_counter: bool,
    /// Propose other terms with a `COUNTER` (profile N, while negotiating).
    pub counter: bool,
    pub dispute: bool,
    /// Contingent: pay against the REPORT's hashlock (CR-002).
    pub fund: bool,
}

/// What a priced commitment's payment depends on, derived from the terms (spec §12, §13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaymentDependency {
    /// The requester's `SATISFY` (or, with an arbiter, its `RULING`). The requester sees the
    /// deliverable first, so payment rests on its good faith.
    Requester { arbiter: Option<String> },
    /// The settlement verifier's `RELEASE` once every condition passes; the requester can't stop it.
    Verifier { verifier: String },
    /// The requester's `FUND` against an offer it can't read yet; the claim reveals the key.
    Contingent { verifier: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportView {
    pub id: String,
    pub payload: Value,
    pub contract: Option<String>,
    pub note: Option<String>,
    pub artifacts: Vec<String>,
    /// The derivation record's hash (Core §9), when the performer stored one.
    pub derivation: Option<String>,
    /// 1 for the first report, 2 after one rework, ...
    pub version: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnomalyView {
    pub event: String,
    pub act: &'static str,
    pub code: &'static str,
    pub severity: Severity,
    pub detail: String,
}

/// Board column (queue view, Core.md §12). Derived from the fold and `status_at`; a board is a
/// projection, never a way to change state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    /// `Requested`: waiting for the performer to respond.
    Requested,
    /// `Countered`, or a counter pending while promised.
    Negotiating,
    /// `Promised`, `Reworking` (overdue once the deadline passes).
    InProgress,
    /// `Reported`: the requester reviews.
    Review,
    /// `Rejected` while still disputable, and `Disputed`.
    Dispute,
    /// Terminal.
    Closed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Projection {
    pub id: String,
    pub requester: String,
    pub performer: String,
    /// The local user is the requester.
    pub mine: bool,
    pub profile: Profile,
    pub phase: Option<Phase>,
    pub terminal: bool,
    pub awaiting: Option<Party>,
    pub awaiting_me: bool,
    pub lapsed: Option<Party>,
    pub deadline_in: Option<i64>,
    pub tool: Option<String>,
    pub input: Option<Value>,
    pub description: String,
    pub price_sat: u64,
    pub created_at: u64,
    pub arbiter: Option<String>,
    pub rework_count: u32,
    pub rework_limit: u32,
    /// Applied acts in causal order.
    pub chain: Vec<Entry>,
    /// Every event of the commitment, including rejected/pending ones and annotations.
    pub timeline: Vec<Entry>,
    pub report: Option<ReportView>,
    pub conditions: Vec<ConditionView>,
    /// CR-003: disqualifiers of the terms in force, with their status against the REPORT.
    pub disqualifiers: Vec<DisqualifierView>,
    /// CR-003: every terms version along the chain (REQUEST, then each COUNTER).
    pub versions: Vec<TermsVersionView>,
    /// The terms card for the version in force (spec §12, CR-003 §2.7).
    pub terms_card: Option<TermsCard>,
    /// The card for a counter awaiting ACCEPT.
    pub pending_card: Option<TermsCard>,
    /// The terms under negotiation as a document (the pending counter's, else those in force):
    /// what a counter-offer edits.
    pub negotiable_terms: Option<Value>,
    /// The pending counter against the version in force; otherwise the version in force against
    /// the one before it.
    pub terms_diff: Vec<DiffLine>,
    pub counter: Option<CounterView>,
    pub settlement: Option<SettlementView>,
    pub binding: BindingStatus,
    pub anomalies: Vec<AnomalyView>,
    pub allowed: Allowed,
    /// Overall assurance (weakest condition), computed by the validator (spec §7, §12).
    pub assurance: Option<Assurance>,
    /// Priced terms: whose act releases the payment (spec §12). `None` when free.
    pub payment_depends_on: Option<PaymentDependency>,
    // --- board ---
    pub lane: Lane,
    /// The local user's role in this commitment.
    pub my_role: Role,
    /// Whose move it is (`None` when closed).
    pub next: Option<Role>,
    /// `next == my_role`.
    pub needs_me: bool,
    /// A REPORT waits for a governance endorsement (V-14).
    pub pending_endorsement: bool,
    /// Latest event of the commitment (any status).
    pub updated_at: u64,
}

/// Project every commitment in `events` from `me`'s point of view at time `now`.
/// `events` may include public binding events (they feed V-8) and terms templates (V-28).
pub fn project(events: &[Event], me: &str, now: u64) -> Vec<Projection> {
    project_with(events, &BTreeMap::new(), me, now)
}

/// [`project`] with terms documents the events reference rather than inline (Core §7).
pub fn project_with(
    events: &[Event],
    documents: &BTreeMap<String, Value>,
    me: &str,
    now: u64,
) -> Vec<Projection> {
    let out = fold(events, documents);
    let by_id: BTreeMap<&str, &Event> = events.iter().map(|e| (e.id.as_str(), e)).collect();
    let records = template_records(&out);
    let mut list: Vec<Projection> = out
        .commitments
        .values()
        .map(|c| one(c, &out, &by_id, &records, documents, me, now))
        .collect();
    list.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    list
}

fn c_tag(e: &Event) -> Option<&str> {
    e.tags
        .iter()
        .find(|t| t.first().map(String::as_str) == Some("c"))
        .and_then(|t| t.get(1).map(String::as_str))
}

fn e_marked<'a>(e: &'a Event, marker: &str) -> Option<&'a str> {
    e.tags
        .iter()
        .find(|t| {
            t.first().map(String::as_str) == Some("e")
                && t.get(3).map(String::as_str) == Some(marker)
        })
        .and_then(|t| t.get(1).map(String::as_str))
}

fn tag<'a>(e: &'a Event, name: &str) -> Option<&'a str> {
    e.tags
        .iter()
        .find(|t| t.first().map(String::as_str) == Some(name))
        .and_then(|t| t.get(1).map(String::as_str))
}

fn content(e: &Event) -> Value {
    serde_json::from_str(&e.content).unwrap_or(Value::Null)
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn role(pk: &str, c: &Commitment) -> Role {
    if pk == c.requester {
        return Role::Requester;
    }
    if pk == c.performer {
        return Role::Performer;
    }
    let Some(t) = &c.terms else {
        return Role::Other;
    };
    if t.arbiter.as_deref() == Some(pk) {
        Role::Arbiter
    } else if t.settlement_verifier.as_deref() == Some(pk)
        || t.verifiers.iter().any(|v| v.pubkey == pk)
    {
        Role::Verifier
    } else if t
        .governance
        .as_ref()
        .is_some_and(|g| g.keys.iter().any(|k| k == pk))
    {
        Role::Governance
    } else {
        Role::Other
    }
}

fn summary(e: &Event) -> String {
    let v = content(e);
    match e.kind {
        K_REQUEST => v["terms"]["deliverable"]["description"]
            .as_str()
            .unwrap_or("Request")
            .to_string(),
        K_COUNTER => format!("Counter-offer: {}", s(&v, "reason")),
        K_ACCEPT => "Accepted the counter-offer (now a promise)".into(),
        K_PROMISE => match v["p_on_time"].as_f64() {
            Some(p) => format!("Promised · {:.0}% confident of the deadline", p * 100.0),
            None => "Promised".into(),
        },
        K_DECLINE | K_WITHDRAW | K_REVOKE => {
            let note = s(&v, "note");
            if note.is_empty() {
                s(&v, "reason").replace('_', " ")
            } else {
                note
            }
        }
        K_PROGRESS => s(&v, "note"),
        K_REPORT => {
            let note = s(&v, "note");
            if note.is_empty() {
                "Delivered the result".into()
            } else {
                note
            }
        }
        K_SATISFY => {
            if v.get("preimage").is_some() {
                "Accepted · preimage revealed, payment released".into()
            } else {
                "Accepted the result".into()
            }
        }
        K_REJECT => {
            let gaps: Vec<String> = v["gaps"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|g| format!("{}: {}", s(g, "condition"), s(g, "note")))
                .collect();
            format!("Rejected · {}", gaps.join("; "))
        }
        K_DISPUTE => format!("Disputed: {}", s(&v, "reason")),
        K_RULING => {
            let who = s(&v, "decision");
            let paid = if v.get("preimage").is_some() {
                " · releases the preimage"
            } else {
                ""
            };
            format!("Ruled for the {who}{paid}")
        }
        K_ATTEST_MECH | K_ATTEST_GROUND => {
            let method = s(&v, "method");
            let m = if method.is_empty() {
                String::new()
            } else {
                format!(" ({method})")
            };
            format!(
                "{}: {}{m}{}",
                tag(e, "cond").unwrap_or("?"),
                s(&v, "result"),
                sampled(&v)
            )
        }
        K_ATTEST_JUDGED => {
            let judges: Vec<String> = v["judges"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|j| format!("{} {}", s(j, "model_id"), j["median"]))
                .collect();
            format!(
                "{}: {} · score {} ({}) · judged opinion{}{}",
                tag(e, "cond").unwrap_or("?"),
                s(&v, "result"),
                v["aggregate_score"],
                judges.join(", "),
                if v["injection_detected"] == true {
                    " · injection detected"
                } else {
                    ""
                },
                sampled(&v)
            )
        }
        K_INTERP_REPORT => format!(
            "{}: \"{}\" (opinion, advisory)",
            tag(e, "cond").unwrap_or("?"),
            s(&v, "verdict")
        ),
        K_ENDORSE => format!("Endorsed ({} gate)", tag(e, "endorser").unwrap_or("?")),
        K_ESCROW_ACK => "Escrowed secret checked against the hashlock".to_string(),
        K_RELEASE => "Released the preimage: every condition passed".to_string(),
        K_FUND => format!("Funded {} sat against the reported hashlock", v["amount"]),
        K_CLAIM => "Claimed the payment: the key to the deliverable is revealed".to_string(),
        _ => name(e.kind).to_string(),
    }
}

/// The held-out cases an attestation revealed (V-27), for its timeline line.
fn sampled(v: &Value) -> String {
    match v["instances"].as_array() {
        Some(a) => {
            let idx: Vec<String> = a.iter().map(|x| x["index"].to_string()).collect();
            format!(" · held-out cases {} with proofs", idx.join(", "))
        }
        None => String::new(),
    }
}

fn one(
    c: &Commitment,
    out: &Output,
    by_id: &BTreeMap<&str, &Event>,
    records: &BTreeMap<String, TemplateRecord>,
    documents: &BTreeMap<String, Value>,
    me: &str,
    now: u64,
) -> Projection {
    let status = status_at(c, now);
    let mine = c.requester == me;
    let my_party = if mine {
        Party::Requester
    } else if c.performer == me {
        Party::Performer
    } else {
        Party::Arbiter
    };
    let terms = c.terms.as_ref();
    let st = c.state.as_ref();

    let members: Vec<&Event> = by_id
        .values()
        .copied()
        .filter(|e| e.id == c.id || c_tag(e) == Some(c.id.as_str()))
        .collect();
    let chain_pos: BTreeMap<&str, usize> = c
        .chain
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let anomalies_of = |id: &str| -> Vec<Note> {
        out.anomalies
            .iter()
            .filter(|a| a.event == id)
            .map(|a| Note {
                code: a.code.as_str(),
                severity: a.severity,
                detail: a.detail.clone(),
            })
            .collect()
    };

    let entry = |e: &Event| -> Entry {
        let notes = anomalies_of(&e.id);
        let status = if chain_pos.contains_key(e.id.as_str()) {
            EntryStatus::Applied
        } else if notes.iter().any(|n| n.severity == Severity::Reject) {
            EntryStatus::Rejected
        } else if notes.iter().any(|n| n.severity == Severity::Pending) {
            EntryStatus::Pending
        } else if matches!(
            e.kind,
            K_PROGRESS
                | K_ATTEST_MECH
                | K_ATTEST_GROUND
                | K_ATTEST_JUDGED
                | K_INTERP_REPORT
                | K_ENDORSE
                | K_ESCROW_ACK
        ) {
            EntryStatus::Annotation
        } else {
            EntryStatus::Rejected
        };
        Entry {
            id: e.id.clone(),
            act: name(e.kind),
            kind: e.kind,
            by: e.pubkey.clone(),
            role: role(&e.pubkey, c),
            mine: e.pubkey == me,
            created_at: e.created_at,
            status,
            summary: summary(e),
            notes,
        }
    };

    let chain: Vec<Entry> = c
        .chain
        .iter()
        .filter_map(|id| by_id.get(id.as_str()))
        .map(|e| entry(e))
        .collect();
    let mut timeline: Vec<Entry> = members.iter().map(|e| entry(e)).collect();
    timeline.sort_by(|a, b| {
        let pa = chain_pos.get(a.id.as_str()).copied().unwrap_or(usize::MAX);
        let pb = chain_pos.get(b.id.as_str()).copied().unwrap_or(usize::MAX);
        (a.created_at, pa, &a.id).cmp(&(b.created_at, pb, &b.id))
    });

    // Contingent (CR-002): the key revealed by CLAIM opens a sealed REPORT.
    let claimed_key: Option<[u8; 32]> = c.chain.iter().find_map(|id| {
        let e = by_id.get(id.as_str()).filter(|e| e.kind == K_CLAIM)?;
        hex::decode(content(e)["key"].as_str()?)
            .ok()?
            .try_into()
            .ok()
    });
    // Current report and its evidence.
    let report = st
        .and_then(|s| s.report.as_deref())
        .and_then(|r| by_id.get(r))
        .map(|e| {
            let v = content(e);
            let payload = match v.get("sealed").and_then(Value::as_str) {
                Some(sealed) => claimed_key
                    .and_then(|k| acn_core::sealed::open(&k, sealed).ok())
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_else(|| {
                        json!({
                            "title": "🔒 Encrypted delivery",
                            "body": format!("Readable once the payment is claimed and the key revealed. Ciphertext {}…", s(&v, "ciphertext").get(..16).unwrap_or_default()),
                        })
                    }),
                None => v.get("result").cloned().unwrap_or(Value::Null),
            };
            ReportView {
                id: e.id.clone(),
                payload,
                contract: v.get("contract").and_then(Value::as_str).map(str::to_owned),
                note: v.get("note").and_then(Value::as_str).map(str::to_owned),
                artifacts: e
                    .tags
                    .iter()
                    .filter(|t| t.first().map(String::as_str) == Some("x"))
                    .filter_map(|t| t.get(1).cloned())
                    .collect(),
                derivation: v
                    .get("derivation")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                version: c.chain_kinds.iter().filter(|k| **k == K_REPORT).count() as u32,
            }
        });
    let evidence_for = |id: &str| -> Vec<Evidence> {
        members
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    K_ATTEST_MECH | K_ATTEST_GROUND | K_ATTEST_JUDGED | K_INTERP_REPORT
                )
            })
            .filter(|e| tag(e, "cond") == Some(id))
            .filter(|e| {
                report
                    .as_ref()
                    .is_some_and(|r| e_marked(e, "report") == Some(r.id.as_str()))
            })
            .map(|e| {
                let notes = anomalies_of(&e.id);
                let v = content(e);
                let verdict = v
                    .get("result")
                    .or_else(|| v.get("verdict"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Evidence {
                    id: e.id.clone(),
                    act: name(e.kind),
                    by: e.pubkey.clone(),
                    role: role(&e.pubkey, c),
                    detail: attestation_detail(e, &v),
                    verdict,
                    // A below-threshold judged attestation still counts, as a fail.
                    counted: notes.iter().all(|n| n.code == "below_threshold")
                        && e.kind != K_INTERP_REPORT,
                    why: notes.first().map(|n| n.detail.clone()).or_else(|| {
                        (e.kind == K_INTERP_REPORT).then(|| {
                            "opinion: informs the requester, never satisfies a condition"
                                .to_string()
                        })
                    }),
                }
            })
            .collect()
    };
    let disqualifiers = terms
        .map(|t| {
            t.disqualifiers
                .iter()
                .map(|d| DisqualifierView {
                    id: d.id.clone(),
                    class: d.class,
                    assurance: d.class.assurance(),
                    description: d.description.clone(),
                    status: c.disqualifiers.get(&d.id).copied(),
                    evidence: evidence_for(&d.id),
                })
                .collect()
        })
        .unwrap_or_default();

    // Terms versions (CR-003 §2.4): each REQUEST / COUNTER on the chain that carried terms.
    let mut versions: Vec<TermsVersionView> = Vec::new();
    let mut docs: BTreeMap<String, acn_core::Terms> = BTreeMap::new();
    for (id, kind) in c.chain.iter().zip(&c.chain_kinds) {
        if !matches!(*kind, K_REQUEST | K_COUNTER) {
            continue;
        }
        let Some(e) = by_id.get(id.as_str()) else {
            continue;
        };
        let Some(hash) = tag(e, "terms") else {
            continue;
        };
        // Inline, or by reference (a supplied document, Core §7).
        let doc = content(e)
            .get("terms")
            .filter(|t| !t.is_null())
            .cloned()
            .or_else(|| documents.get(hash).cloned());
        if let Some(Ok(t)) = doc.map(|d| acn_core::Terms::parse(&d)) {
            docs.insert(hash.to_string(), t);
        }
        versions.push(TermsVersionView {
            version: versions.len() as u32 + 1,
            hash: hash.to_string(),
            act: name(*kind),
            drafted_by: e.pubkey.clone(),
            drafted_by_role: role(&e.pubkey, c),
            drafted_by_me: e.pubkey == me,
            prev: versions.last().map(|v| v.hash.clone()),
            in_force: st.is_some_and(|s| s.terms == hash),
            pending: st
                .and_then(|s| s.pending.as_ref())
                .is_some_and(|p| p.terms == hash),
        });
    }
    // The version in force: the last one carrying that hash (an ACCEPTed counter, or the request).
    let in_force = versions.iter().rev().find(|v| v.in_force);
    let pending_v = versions.iter().rev().find(|v| v.pending);
    let card_for = |t: &acn_core::Terms, v: &TermsVersionView| {
        terms_card::card(t, v, terms_card::template_view(t, by_id, records))
    };
    let terms_card = in_force.and_then(|v| Some(card_for(docs.get(&v.hash).or(terms)?, v)));
    let pending_card = pending_v.and_then(|v| Some(card_for(docs.get(&v.hash)?, v)));
    let negotiable_terms = pending_v
        .and_then(|v| docs.get(&v.hash))
        .or(in_force.and_then(|v| docs.get(&v.hash)))
        .or(terms)
        .and_then(|t| serde_json::to_value(t).ok());
    let terms_diff = match (pending_v, in_force) {
        (Some(p), Some(f)) => docs
            .get(&f.hash)
            .zip(docs.get(&p.hash))
            .map(|(a, b)| terms_card::diff(a, b)),
        (None, Some(f)) => f
            .prev
            .as_ref()
            .and_then(|prev| docs.get(prev))
            .zip(docs.get(&f.hash))
            .map(|(a, b)| terms_card::diff(a, b)),
        _ => None,
    }
    .unwrap_or_default();

    let conditions = terms
        .map(|t| {
            t.conditions
                .iter()
                .map(|cond| {
                    let evidence = evidence_for(&cond.id);
                    ConditionView {
                        id: cond.id.clone(),
                        class: cond.class,
                        assurance: cond.class.assurance(),
                        description: cond.description.clone(),
                        status: c.conditions.get(&cond.id).cloned(),
                        evidence,
                        judged: cond.judged().ok().map(|j| JudgedView {
                            judges: j
                                .judges
                                .iter()
                                .map(|f| format!("{} {}", f.model_id, f.model_version))
                                .collect(),
                            threshold: j.threshold,
                            scale_min: j.scale.min,
                            scale_max: j.scale.max,
                            aggregation: j.aggregation,
                            repeats: j.repeats,
                            rubric_ref: j.rubric_ref.clone(),
                            rubric_text: j.rubric_text.clone(),
                            runners: j.runners.clone(),
                        }),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Pending counter-offer.
    let counter = st.and_then(|s| s.pending.as_ref()).and_then(|p| {
        let ev = members
            .iter()
            .find(|e| e.kind == K_COUNTER && tag(e, "terms") == Some(p.terms.as_str()))?;
        let v = content(ev);
        let t = &v["terms"];
        let cur = terms?;
        Some(CounterView {
            by: role(&p.by, c),
            mine: p.by == me,
            reason: s(&v, "reason"),
            terms_hash: p.terms.clone(),
            deadline: t["deadline"].as_u64().unwrap_or(0),
            current_deadline: cur.deadline,
            price_sat: t["price"]["amount"].as_u64().unwrap_or(0),
            current_price_sat: cur.price.amount,
            rework_limit: t["rework_limit"].as_u64().unwrap_or(0) as u32,
            current_rework_limit: cur.rework_limit,
        })
    });

    let phase = st.map(|s| s.phase);
    let settlement = terms.and_then(|t| {
        let pay = t.payment.as_ref()?;
        if t.price.amount == 0 {
            return None;
        }
        let ruled = c.chain_kinds.last() == Some(&K_RULING);
        let contingent = t.settlement_trigger == acn_core::terms::SettlementTrigger::Contingent;
        let (status, detail) = match phase {
            Some(Phase::Settled) if contingent => (
                SettlementStatus::Paid,
                "The performer claimed the payment, which revealed the key to the deliverable"
                    .to_string(),
            ),
            Some(Phase::Settled) => (
                SettlementStatus::Paid,
                "The settlement verifier released the preimage; the performer redeems".to_string(),
            ),
            Some(Phase::Offered) => (
                SettlementStatus::Unfunded,
                "Encrypted delivery offered: fund it to receive the key".to_string(),
            ),
            Some(Phase::Funded) => (
                SettlementStatus::Locked,
                "Locked to the reported hashlock: claiming it reveals the key".to_string(),
            ),
            Some(Phase::Satisfied) if ruled => (
                SettlementStatus::Paid,
                "Arbiter released the preimage; the performer redeems".to_string(),
            ),
            Some(Phase::Satisfied) => (
                SettlementStatus::Paid,
                "Preimage revealed in SATISFY; the performer redeems".to_string(),
            ),
            _ if status.terminal => (
                SettlementStatus::Refundable,
                "Closed without satisfaction; the requester reclaims after the locktime"
                    .to_string(),
            ),
            _ => (
                SettlementStatus::Locked,
                "Hash-locked to the performer until the requester is satisfied".to_string(),
            ),
        };
        Some(SettlementView {
            amount_sat: t.price.amount,
            mint: pay.mint.clone(),
            simulated: pay.mint.starts_with("sim:"),
            status,
            detail,
            locktime: pay.locktime.or(st.and_then(|s| s.fund_locktime)),
        })
    });

    let is_party = mine || c.performer == me;
    let disputable =
        phase == Some(Phase::Reported) || (phase == Some(Phase::Rejected) && !status.terminal);
    let trigger = terms.map(|t| t.settlement_trigger).unwrap_or_default();
    let contingent = trigger == acn_core::terms::SettlementTrigger::Contingent;
    let verifier_trigger = trigger == acn_core::terms::SettlementTrigger::Verifier || contingent;
    let review_phase = if contingent {
        Phase::Offered
    } else {
        Phase::Reported
    };
    let judged = st.is_some_and(|s| s.judged);
    // A disqualifier hit counts as failing; clients MUST NOT offer SATISFY then (CR-003 §2.2).
    let failing = c.disqualified() || c.conditions.values().any(|s| *s == ConditionStatus::Fail);
    let after_release = phase == Some(Phase::Settled) && !judged;
    let allowed = Allowed {
        // Verifier trigger: RELEASE settles; SATISFY / REJECT after it are recorded judgements,
        // and REJECT before it needs a failing attestation (CR-002).
        satisfy: mine
            && if verifier_trigger {
                after_release
            } else {
                phase == Some(Phase::Reported) && !c.disqualified()
            },
        reject: mine
            && if verifier_trigger {
                after_release || (phase == Some(review_phase) && failing)
            } else {
                phase == Some(Phase::Reported)
            },
        withdraw: mine
            && matches!(
                phase,
                Some(Phase::Requested | Phase::Countered | Phase::Promised)
            ),
        accept_counter: is_party && counter.as_ref().is_some_and(|k| !k.mine),
        counter: is_party
            && c.profile == Profile::N
            && matches!(phase, Some(Phase::Requested | Phase::Countered)),
        dispute: is_party
            && c.profile == Profile::N
            && terms.is_some_and(|t| t.arbiter.is_some())
            && disputable,
        fund: mine && contingent && phase == Some(Phase::Offered),
    };

    let anomalies = out
        .anomalies
        .iter()
        .filter(|a| members.iter().any(|e| e.id == a.event))
        .map(|a| AnomalyView {
            event: a.event.clone(),
            act: by_id.get(a.event.as_str()).map_or("?", |e| name(e.kind)),
            code: a.code.as_str(),
            severity: a.severity,
            detail: a.detail.clone(),
        })
        .collect();

    let pending_endorsement = timeline.iter().any(|e| {
        e.status == EntryStatus::Pending && e.notes.iter().any(|n| n.code == "pending_endorsement")
    });
    let lane = match phase {
        _ if status.terminal => Lane::Closed,
        Some(Phase::Countered) => Lane::Negotiating,
        Some(Phase::Promised) if counter.is_some() => Lane::Negotiating,
        Some(Phase::Promised | Phase::Reworking) => Lane::InProgress,
        Some(Phase::Reported | Phase::Offered | Phase::Funded) => Lane::Review,
        Some(Phase::Rejected | Phase::Disputed) => Lane::Dispute,
        _ => Lane::Requested,
    };
    let next = if status.terminal {
        None
    } else if pending_endorsement {
        Some(Role::Governance)
    } else if lane == Lane::Negotiating {
        counter.as_ref().map(|k| match k.by {
            Role::Requester => Role::Performer,
            _ => Role::Requester,
        })
    } else {
        status.awaiting.map(|p| match p {
            Party::Requester => Role::Requester,
            Party::Performer => Role::Performer,
            Party::Arbiter => Role::Arbiter,
            Party::Verifier => Role::Verifier,
        })
    };
    let my_role = role(me, c);
    let updated_at = timeline.iter().map(|e| e.created_at).max().unwrap_or(0);

    let payment_depends_on = terms.filter(|t| t.price.amount > 0).map(|t| {
        use acn_core::terms::SettlementTrigger as T;
        let verifier = t.settlement_verifier.clone().unwrap_or_default();
        match t.settlement_trigger {
            T::Requester => PaymentDependency::Requester {
                arbiter: t.arbiter.clone(),
            },
            T::Verifier => PaymentDependency::Verifier { verifier },
            T::Contingent => PaymentDependency::Contingent { verifier },
        }
    });
    Projection {
        lane,
        assurance: c.assurance,
        payment_depends_on,
        needs_me: next.is_some() && next == Some(my_role),
        my_role,
        next,
        pending_endorsement,
        updated_at,
        id: c.id.clone(),
        requester: c.requester.clone(),
        performer: c.performer.clone(),
        mine,
        profile: c.profile,
        phase,
        terminal: status.terminal,
        awaiting: status.awaiting,
        awaiting_me: status.awaiting == Some(my_party),
        lapsed: status.lapsed,
        deadline_in: status.deadline_in,
        tool: terms.and_then(|t| t.deliverable.tool.clone()),
        input: terms.and_then(|t| t.deliverable.input.clone()),
        description: terms
            .map(|t| t.deliverable.description.clone())
            .unwrap_or_default(),
        price_sat: terms.map_or(0, |t| t.price.amount),
        created_at: by_id.get(c.id.as_str()).map_or(0, |e| e.created_at),
        arbiter: terms.and_then(|t| t.arbiter.clone()),
        rework_count: st.map_or(0, |s| s.rework_count),
        rework_limit: terms.map_or(0, |t| t.rework_limit),
        chain,
        timeline,
        report,
        conditions,
        disqualifiers,
        versions,
        terms_card,
        pending_card,
        negotiable_terms,
        terms_diff,
        counter,
        settlement,
        binding: c.performer_binding.clone(),
        anomalies,
        allowed,
    }
}
