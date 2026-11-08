//! The deterministic fold (spec §5.4, §6, §10).
//!
//! Determinism (INV-2): every collection is a `BTreeMap`/`BTreeSet` keyed by event id, the
//! effective chain is built from content only (no arrival order, no clock), and the anomaly list
//! is sorted. `tests/permutations.rs` checks this property.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

use crate::act::{self, class_rank, is_advancing, Parsed};
use crate::anomaly::{Anomaly, Code};
use crate::binding::{BindingStatus, Bindings};
use crate::event::Event;
use crate::jcs;
use crate::kinds::*;
use crate::terms::{
    Assurance, ConditionClass, Disqualifier, DisqualifierStatus, GovernedParty, SettlementTrigger,
    Terms,
};

/// (REPORT id, condition or disqualifier id) -> counted verdicts with their attestation ids.
type EvidenceMap = BTreeMap<(String, String), Vec<(String, String)>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Profile {
    T0,
    T1,
    N,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Requested,
    Countered,
    Promised,
    Reported,
    Reworking,
    Satisfied,
    Rejected,
    Disputed,
    Declined,
    Withdrawn,
    Revoked,
    /// Payment released by the designated trigger (`RELEASE` or `CLAIM`; CR-002). Terminal.
    Settled,
    /// Contingent: encrypted deliverable reported, awaiting attestation and funding.
    Offered,
    /// Contingent: payment locked to the REPORT's hashlock, awaiting the performer's claim.
    Funded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pending {
    /// Counter terms awaiting ACCEPT.
    pub terms: String,
    /// The countering party.
    pub by: String,
}

/// Commitment state as a record (spec §6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct State {
    pub phase: Phase,
    /// Terms hash in force.
    pub terms: String,
    pub pending: Option<Pending>,
    pub promised: bool,
    pub rework_count: u32,
    pub report: Option<String>,
    pub report_at: Option<u64>,
    pub rejected_at: Option<u64>,
    pub ruled: bool,
    /// After `Settled`: the requester's one recorded judgement (`SATISFY`/`REJECT`) was made.
    pub judged: bool,
    /// Contingent: the current REPORT's ciphertext hash and hashlock `H = sha256(k)`.
    pub ciphertext: Option<String>,
    pub offer_hashlock: Option<String>,
    /// Contingent: locktime of the FUND payment (the claim deadline).
    pub fund_locktime: Option<u64>,
    pub head: String,
    pub head_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ConditionStatus {
    Pass,
    Fail,
    Unverified,
    /// Interpretive conditions: judge verdicts inform, the requester decides (INV-8).
    Advisory {
        verdicts: Vec<String>,
    },
    /// Judged by the requester's SATISFY/REJECT.
    Human,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Commitment {
    pub id: String,
    pub requester: String,
    pub performer: String,
    pub profile: Profile,
    pub parent: Option<String>,
    /// `None` while the terms document is unavailable (`pending_terms`).
    pub state: Option<State>,
    /// Terms in force.
    pub terms: Option<Terms>,
    /// Effective chain from the root, in causal order.
    pub chain: Vec<String>,
    /// Kinds along the chain (parallel to `chain`), for projections.
    pub chain_kinds: Vec<u16>,
    pub progress: Vec<String>,
    /// Status of each condition against the current REPORT.
    pub conditions: BTreeMap<String, ConditionStatus>,
    pub performer_binding: BindingStatus,
    /// Content of the current REPORT (inline result, contract, artifacts).
    pub report: Option<Value>,
    /// Overall assurance of the terms in force: the weakest condition's label (spec §7).
    /// `None` while the terms are unavailable.
    pub assurance: Option<Assurance>,
    /// When every condition of the current REPORT passed: the latest of each condition's
    /// earliest counted passing attestation. Starts the contingent `fund_window`.
    pub passed_at: Option<u64>,
    /// Status of each disqualifier against the current REPORT (CR-003 §2.2).
    pub disqualifiers: BTreeMap<String, DisqualifierStatus>,
}

impl Commitment {
    /// A counted `hit` on any disqualifier of the current REPORT: the commitment failed (breach).
    pub fn disqualified(&self) -> bool {
        self.disqualifiers
            .values()
            .any(|s| *s == DisqualifierStatus::Hit)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Output {
    pub commitments: BTreeMap<String, Commitment>,
    pub anomalies: Vec<Anomaly>,
}

impl Output {
    pub fn anomalies_for<'a>(&'a self, event: &'a str) -> impl Iterator<Item = &'a Anomaly> + 'a {
        self.anomalies.iter().filter(move |a| a.event == event)
    }
    pub fn has(&self, event: &str, code: Code) -> bool {
        self.anomalies_for(event).any(|a| a.code == code)
    }
}

#[derive(Default)]
struct Acc(BTreeSet<Anomaly>);

impl Acc {
    fn push(&mut self, event: &str, code: Code, detail: impl Into<String>) {
        self.0.insert(Anomaly::new(event, code, detail));
    }
}

type Flags = Vec<(Code, String)>;
type Step = Result<(State, Flags), (Code, String)>;
/// A sibling that passed validation: the act, the state it produces, and its flags.
type Candidate<'p, 'e> = (&'p Parsed<'e>, State, Flags);

/// Validate and fold an event set (plus content-addressed documents keyed by JCS sha256).
pub fn fold(events: &[Event], documents: &BTreeMap<String, Value>) -> Output {
    let mut acc = Acc::default();

    // V-1, de-duplication (two valid copies of an id have identical content; keep the
    // lexicographically smallest signature so the choice is order-independent).
    let mut valid: BTreeMap<&str, &Event> = BTreeMap::new();
    for e in events {
        if !BLOCK.contains(&e.kind) && e.kind != K_SVC {
            continue;
        }
        match e.verify() {
            Ok(()) => {
                valid
                    .entry(e.id.as_str())
                    .and_modify(|cur| {
                        if e.sig < cur.sig {
                            *cur = e
                        }
                    })
                    .or_insert(e);
            }
            Err(err) => acc.push(&e.id, Code::BadSig, err.to_string()),
        }
    }

    // V-2, V-3.
    let mut parsed: Vec<Parsed<'_>> = Vec::new();
    for e in valid.values() {
        match act::parse(e) {
            Ok(Some(p)) => parsed.push(p),
            Ok(None) => {}
            Err((code, d)) => acc.push(&e.id, code, d),
        }
    }

    let bindings = Bindings::build(&parsed);

    // Ownership: event id -> commitment id.
    let mut owner: BTreeMap<&str, &str> = BTreeMap::new();
    let mut requests: BTreeMap<&str, &Parsed<'_>> = BTreeMap::new();
    let mut members: BTreeMap<&str, Vec<&Parsed<'_>>> = BTreeMap::new();
    for p in &parsed {
        if p.kind() == K_REQUEST {
            requests.insert(p.id(), p);
            owner.insert(p.id(), p.id());
        } else if let Some(c) = &p.c {
            owner.insert(p.id(), c.as_str());
            members.entry(c.as_str()).or_default().push(p);
        }
    }

    let mut commitments = BTreeMap::new();
    for (cid, list) in &members {
        if !requests.contains_key(cid) {
            for p in list {
                acc.push(
                    p.id(),
                    Code::PendingRef,
                    format!("commitment root {cid} not available"),
                );
            }
        }
    }
    for (cid, req) in &requests {
        let list = members.get(cid).map(Vec::as_slice).unwrap_or(&[]);
        let ctx = Ctx {
            req,
            members: list,
            documents,
            bindings: &bindings,
            owner: &owner,
        };
        if let Some(c) = ctx.run(&mut acc) {
            commitments.insert(cid.to_string(), c);
        }
    }

    Output {
        commitments,
        anomalies: acc.0.into_iter().collect(),
    }
}

struct Ctx<'a, 'e> {
    req: &'a Parsed<'e>,
    members: &'a [&'a Parsed<'e>],
    documents: &'a BTreeMap<String, Value>,
    bindings: &'a Bindings,
    owner: &'a BTreeMap<&'a str, &'a str>,
}

struct Run<'a> {
    requester: String,
    performer: String,
    profile: Profile,
    /// hash -> terms (inline from REQUEST/COUNTER, or supplied documents).
    terms: BTreeMap<String, Terms>,
    endorsements: BTreeMap<&'a str, Vec<(&'a str, u64)>>,
}

impl Run<'_> {
    fn parties(&self, by: &str) -> bool {
        by == self.requester || by == self.performer
    }
}

impl<'a, 'e> Ctx<'a, 'e> {
    fn run(&self, acc: &mut Acc) -> Option<Commitment> {
        let req = self.req;
        let requester = req.ev.pubkey.clone();
        let performer = req.ev.values("p")[0].to_string();
        let profile = match req.ev.single("profile") {
            Ok("T0") => Profile::T0,
            Ok("T1") => Profile::T1,
            _ => Profile::N,
        };
        let parent = req.ev.optional("parent").ok().flatten().map(str::to_owned);
        let root_terms = req.terms.clone().expect("checked in parse");

        // Collect terms: inline (REQUEST/COUNTER) must hash to their own `terms` tag (V-9).
        let mut terms: BTreeMap<String, Terms> = BTreeMap::new();
        let mut inline_bad: BTreeSet<&str> = BTreeSet::new();
        for p in std::iter::once(&req).chain(self.members.iter()) {
            if !matches!(p.kind(), K_REQUEST | K_COUNTER) {
                continue;
            }
            let Some(doc) = p.content.get("terms") else {
                continue;
            };
            let tag = p.terms.as_deref().unwrap_or_default();
            match jcs::hash(doc) {
                Ok(h) if h == tag => match Terms::check(doc) {
                    Ok(t) => {
                        terms.insert(h, t);
                    }
                    Err((code, e)) => {
                        acc.push(p.id(), code, format!("terms: {e}"));
                        inline_bad.insert(p.id());
                    }
                },
                _ => {
                    acc.push(
                        p.id(),
                        Code::TermsMismatch,
                        "inline terms do not hash to the `terms` tag",
                    );
                    inline_bad.insert(p.id());
                }
            }
        }
        for (h, doc) in self.documents {
            if terms.contains_key(h) {
                continue;
            }
            if jcs::hash(doc).ok().as_deref() == Some(h.as_str()) {
                if let Ok(t) = Terms::parse(doc) {
                    terms.insert(h.clone(), t);
                }
            }
        }

        if inline_bad.contains(req.id()) {
            for p in self.members {
                acc.push(p.id(), Code::PendingRef, "commitment root is invalid");
            }
            return None;
        }

        let mut endorsements: BTreeMap<&str, Vec<(&str, u64)>> = BTreeMap::new();
        for p in self.members.iter().filter(|p| p.kind() == K_ENDORSE) {
            let target = p.ev.e_marked("endorses")[0];
            endorsements
                .entry(target)
                .or_default()
                .push((p.ev.pubkey.as_str(), p.at()));
        }

        let run = Run {
            requester,
            performer,
            profile,
            terms,
            endorsements,
        };

        let mut commitment = Commitment {
            id: req.id().to_string(),
            requester: run.requester.clone(),
            performer: run.performer.clone(),
            profile,
            parent,
            state: None,
            terms: None,
            chain: vec![req.id().to_string()],
            chain_kinds: vec![K_REQUEST],
            progress: vec![],
            conditions: BTreeMap::new(),
            assurance: None,
            passed_at: None,
            disqualifiers: BTreeMap::new(),
            performer_binding: BindingStatus::Unbound,
            report: None,
        };

        let Some(root) = run.terms.get(&root_terms).cloned() else {
            for p in self.members {
                acc.push(
                    p.id(),
                    Code::PendingTerms,
                    format!("terms {root_terms} not available"),
                );
            }
            acc.push(
                req.id(),
                Code::PendingTerms,
                format!("terms {root_terms} not available"),
            );
            return Some(commitment);
        };
        // CR-002: only the valid settlement combinations make a commitment.
        let combo = root.settlement_combo().and_then(|()| {
            if root.settlement_trigger == SettlementTrigger::Contingent && profile == Profile::T0 {
                Err((
                    Code::InvalidSettlementCombo,
                    "contingent needs a conditional profile (T1 or N)".into(),
                ))
            } else {
                Ok(())
            }
        });
        if let Err((code, e)) = combo {
            acc.push(req.id(), code, e);
            for p in self.members {
                acc.push(p.id(), Code::PendingRef, "commitment root is invalid");
            }
            return None;
        }
        let price = root.price.amount;
        let mint = root.payment.as_ref().map(|p| p.mint.clone());

        // V-17.1: terms that list the performer as a runner of a judged condition.
        for p in std::iter::once(req).chain(
            self.members
                .iter()
                .copied()
                .filter(|p| p.kind() == K_COUNTER),
        ) {
            let Some(t) = p.terms.as_ref().and_then(|h| run.terms.get(h)) else {
                continue;
            };
            for c in t
                .conditions
                .iter()
                .filter(|c| c.class == ConditionClass::Judged)
            {
                if c.judged().is_ok_and(|j| j.runners.contains(&run.performer)) {
                    acc.push(
                        p.id(),
                        Code::SelfJudged,
                        format!("terms list the performer as a runner of `{}`", c.id),
                    );
                }
            }
        }

        // Linkage (V-4) for advancing acts; build the children index.
        let mut children: BTreeMap<&str, Vec<&Parsed<'e>>> = BTreeMap::new();
        let mut assigned: BTreeSet<&str> = BTreeSet::new();
        for p in self.members.iter().filter(|p| is_advancing(p.kind())) {
            if inline_bad.contains(p.id()) {
                assigned.insert(p.id());
                continue;
            }
            let prev = p.prev.as_deref().expect("checked in parse");
            match self.owner.get(prev) {
                Some(o) if *o == req.id() => children.entry(prev).or_default().push(p),
                Some(_) => {
                    acc.push(
                        p.id(),
                        Code::ForeignPrev,
                        "prev belongs to another commitment",
                    );
                    assigned.insert(p.id());
                }
                None => {
                    acc.push(
                        p.id(),
                        Code::PendingRef,
                        format!("prev {prev} not available"),
                    );
                    assigned.insert(p.id());
                }
            }
        }

        // Walk the effective chain (spec §5.4).
        let mut st = State {
            phase: Phase::Requested,
            terms: root_terms.clone(),
            pending: None,
            promised: false,
            rework_count: 0,
            report: None,
            report_at: None,
            rejected_at: None,
            ruled: false,
            judged: false,
            ciphertext: None,
            offer_hashlock: None,
            fund_locktime: None,
            head: req.id().to_string(),
            head_at: req.at(),
        };
        let mut snapshots: BTreeMap<String, State> = BTreeMap::new();
        snapshots.insert(st.head.clone(), st.clone());
        let mut first_performer_act: Option<&str> = None;

        while let Some(kids) = children.get(st.head.as_str()) {
            let current_terms = run
                .terms
                .get(&st.terms)
                .cloned()
                .unwrap_or_else(|| root.clone());
            let mut cands: Vec<Candidate<'_, 'e>> = Vec::new();
            let mut awaiting: Vec<&Parsed<'e>> = Vec::new();
            for k in kids {
                match self.step(&run, &current_terms, &st, k, price, mint.as_deref()) {
                    Ok((next, flags)) => {
                        if needs_endorsement(&run, &current_terms, k)
                            && !endorsed(&run, &current_terms, k)
                        {
                            awaiting.push(k);
                        } else {
                            cands.push((k, next, flags));
                        }
                    }
                    Err((code, d)) => {
                        acc.push(k.id(), code, d);
                        assigned.insert(k.id());
                    }
                }
            }

            // V-13 equivocation among valid siblings from the same signer.
            let mut by_signer: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for (k, _, _) in &cands {
                by_signer
                    .entry(k.ev.pubkey.as_str())
                    .or_default()
                    .push(k.id());
            }
            for k in &awaiting {
                by_signer
                    .entry(k.ev.pubkey.as_str())
                    .or_default()
                    .push(k.id());
            }
            for ids in by_signer.values().filter(|v| v.len() > 1) {
                for id in ids {
                    acc.push(
                        id,
                        Code::Equivocation,
                        format!("{} sibling acts by the same signer", ids.len()),
                    );
                }
            }

            let winner = cands
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    (class_rank(a.0.kind()), a.0.id()).cmp(&(class_rank(b.0.kind()), b.0.id()))
                })
                .map(|(i, _)| i);
            let Some(wi) = winner else {
                for k in awaiting {
                    acc.push(
                        k.id(),
                        Code::PendingEndorsement,
                        "awaiting governance endorsement",
                    );
                    assigned.insert(k.id());
                }
                break;
            };
            let (w, next, flags) = cands.swap_remove(wi);
            for (k, _, _) in &cands {
                acc.push(
                    k.id(),
                    Code::Stale,
                    format!("lost sibling resolution to {}", w.id()),
                );
                assigned.insert(k.id());
            }
            let window = current_terms.governance.as_ref().map_or(0, |g| g.window);
            for k in awaiting {
                if w.at() > k.at().saturating_add(window) {
                    acc.push(k.id(), Code::Unendorsed, "endorsement window elapsed");
                } else {
                    acc.push(k.id(), Code::Stale, format!("chain advanced to {}", w.id()));
                }
                assigned.insert(k.id());
            }
            for (code, d) in flags {
                acc.push(w.id(), code, d);
            }
            if w.ev.pubkey == run.performer && first_performer_act.is_none() {
                first_performer_act = Some(w.id());
            }
            if w.kind() == K_REPORT {
                commitment.report = Some(w.content.clone()).map(|mut c| {
                    c["artifacts"] = Value::from(w.ev.values("x"));
                    c
                });
            }
            assigned.insert(w.id());
            commitment.chain.push(w.id().to_string());
            commitment.chain_kinds.push(w.kind());
            st = next;
            snapshots.insert(st.head.clone(), st.clone());
        }

        for p in self.members.iter().filter(|p| is_advancing(p.kind())) {
            if !assigned.contains(p.id()) {
                acc.push(p.id(), Code::Stale, "prev is not on the effective chain");
            }
        }

        // Binding of the performer (V-8 flag at commitment level).
        commitment.performer_binding =
            self.bindings
                .status(&run.performer, K_REPORT, price, mint.as_deref(), st.head_at);
        if let (Some(first), BindingStatus::Unbound) =
            (first_performer_act, &commitment.performer_binding)
        {
            acc.push(
                first,
                Code::Unbound,
                "performer key has no acknowledged operator binding",
            );
        }

        // Annotating acts.
        let mut evidence: EvidenceMap = BTreeMap::new();
        for p in self.members.iter() {
            match p.kind() {
                K_PROGRESS => self.progress(acc, &run, &snapshots, p, &mut commitment),
                K_ATTEST_MECH | K_ATTEST_GROUND | K_ATTEST_JUDGED | K_INTERP_REPORT => {
                    self.attestation(acc, &run, &snapshots, p, &mut evidence)
                }
                K_ESCROW_ACK => {
                    let holder = run.terms.values().any(|t| {
                        t.settlement_verifier.as_deref() == Some(p.ev.pubkey.as_str())
                            || t.arbiter.as_deref() == Some(p.ev.pubkey.as_str())
                    });
                    if !holder {
                        acc.push(
                            p.id(),
                            Code::Unauthorized,
                            "ESCROW_ACK must be signed by the settlement verifier or arbiter",
                        );
                    }
                }
                _ => {}
            }
        }

        // Disqualifier hits are breach, recorded on the performer's REPORT (CR-003 §2.2).
        for ((report, id), list) in &evidence {
            let disq = snapshots
                .get(report)
                .and_then(|s| run.terms.get(&s.terms))
                .is_some_and(|t| t.disqualifier(id).is_some());
            if disq && list.iter().any(|(r, _)| r == "hit") {
                acc.push(
                    report,
                    Code::Disqualified,
                    format!("disqualifier `{id}` attested as hit"),
                );
            }
        }

        // Condition status for the current report.
        let terms_now = run.terms.get(&st.terms).cloned().unwrap_or(root);
        if let Some(report) = &st.report {
            for d in &terms_now.disqualifiers {
                let ev = evidence.get(&(report.clone(), d.id.clone()));
                commitment
                    .disqualifiers
                    .insert(d.id.clone(), disqualifier_status(ev));
            }
            for cond in &terms_now.conditions {
                let ev = evidence.get(&(report.clone(), cond.id.clone()));
                commitment
                    .conditions
                    .insert(cond.id.clone(), condition_status(cond.class, ev));
            }
            // passed_at: each condition's earliest counted passing attestation; the latest of those.
            let at = |id: &str| self.members.iter().find(|p| p.id() == id).map(|p| p.at());
            let firsts: Option<Vec<u64>> = terms_now
                .conditions
                .iter()
                .map(|cond| {
                    let ev = evidence.get(&(report.clone(), cond.id.clone()))?;
                    if commitment.conditions.get(&cond.id) != Some(&ConditionStatus::Pass) {
                        return None;
                    }
                    ev.iter()
                        .filter(|(r, _)| r == "pass")
                        .filter_map(|(_, id)| at(id))
                        .min()
                })
                .collect();
            commitment.passed_at = firsts
                .filter(|f| !f.is_empty())
                .and_then(|f| f.into_iter().max());
        }

        // V-16: rejections contradicted by mechanical evidence, or overruling a counted judged
        // pass (D-28). Per gap, whatever else the REJECT names; no exemption for judges that
        // disagreed among themselves, since the aggregation is part of the agreed terms.
        for (i, id) in commitment.chain.iter().enumerate() {
            if commitment.chain_kinds[i] != K_REJECT || i == 0 {
                continue;
            }
            let before = &snapshots[&commitment.chain[i - 1]];
            let Some(report) = &before.report else {
                continue;
            };
            let Some(rej) = self.members.iter().find(|p| p.id() == id) else {
                continue;
            };
            let t = run.terms.get(&before.terms);
            for gap in rej.content["gaps"].as_array().into_iter().flatten() {
                let cond = gap["condition"].as_str().unwrap_or_default();
                let class = t.and_then(|t| t.condition(cond)).map(|c| c.class);
                let ev = evidence.get(&(report.clone(), cond.to_string()));
                let passed = ev.is_some_and(|v| {
                    v.iter().any(|(r, _)| r == "pass") && !v.iter().any(|(r, _)| r == "fail")
                });
                if !passed {
                    continue;
                }
                // Judged evidence only enters `evidence` once V-17 counts it.
                match class {
                    Some(ConditionClass::Mechanical) => acc.push(
                        id,
                        Code::ContradictedRejection,
                        format!("condition `{cond}` has a passing mechanical attestation"),
                    ),
                    Some(ConditionClass::Judged) => acc.push(
                        id,
                        Code::OverruledJudgement,
                        format!("condition `{cond}` has a counted passing judged attestation"),
                    ),
                    _ => {}
                }
            }
        }

        commitment.assurance = terms_now.assurance();
        commitment.terms = Some(terms_now);
        commitment.state = Some(st);
        Some(commitment)
    }

    fn progress(
        &self,
        acc: &mut Acc,
        run: &Run,
        snaps: &BTreeMap<String, State>,
        p: &Parsed,
        c: &mut Commitment,
    ) {
        let prev = p.prev.as_deref().unwrap_or_default();
        let Some(at) = snaps.get(prev) else {
            let code = if self.owner.contains_key(prev) {
                Code::Stale
            } else {
                Code::PendingRef
            };
            acc.push(
                p.id(),
                code,
                "PROGRESS must reference an act on the effective chain",
            );
            return;
        };
        if p.ev.pubkey != run.performer {
            acc.push(
                p.id(),
                Code::Unauthorized,
                "only the performer reports progress",
            );
        } else if !matches!(at.phase, Phase::Promised | Phase::Reworking) {
            acc.push(
                p.id(),
                Code::IllegalTransition,
                format!("PROGRESS not legal from {:?}", at.phase),
            );
        } else if p.at() < at.head_at {
            acc.push(p.id(), Code::TimeRegress, "created_at precedes prev");
        } else {
            c.progress.push(p.id().to_string());
        }
    }

    fn attestation(
        &self,
        acc: &mut Acc,
        run: &Run,
        snaps: &BTreeMap<String, State>,
        p: &Parsed,
        evidence: &mut EvidenceMap,
    ) {
        let report = p.ev.e_marked("report")[0];
        let cond = p.ev.single("cond").unwrap_or_default();
        let Some(snap) = snaps
            .get(report)
            .filter(|s| s.report.as_deref() == Some(report))
        else {
            let code = if self.owner.contains_key(report) {
                Code::Stale
            } else {
                Code::PendingRef
            };
            acc.push(
                p.id(),
                code,
                "attestation must reference a REPORT on the effective chain",
            );
            return;
        };
        let Some(t) = run.terms.get(&snap.terms) else {
            return;
        };
        let Some(condition) = t.condition(cond) else {
            match t.disqualifier(cond) {
                Some(d) => {
                    self.disqualifier_attestation(acc, run, snaps, (snap, t), d, p, evidence)
                }
                None => acc.push(
                    p.id(),
                    Code::Malformed,
                    format!("unknown condition or disqualifier `{cond}`"),
                ),
            }
            return;
        };
        let kind_class = match p.kind() {
            K_ATTEST_MECH => ConditionClass::Mechanical,
            K_ATTEST_GROUND => ConditionClass::Grounding,
            K_ATTEST_JUDGED => ConditionClass::Judged,
            _ => ConditionClass::Interpretive,
        };
        if kind_class != condition.class {
            acc.push(
                p.id(),
                Code::WrongEvidenceClass,
                format!(
                    "{} cannot satisfy a {:?} condition",
                    name(p.kind()),
                    condition.class
                ),
            );
            return;
        }
        if condition.class == ConditionClass::Judged {
            let Some(verdict) = self.judged_evidence(acc, run, condition, report, p, false) else {
                return;
            };
            let list = evidence
                .entry((report.to_string(), cond.to_string()))
                .or_default();
            list.push((verdict.to_string(), p.id().to_string()));
            list.sort();
            return;
        }
        if !t.verifier_counts(&p.ev.pubkey, condition.class) {
            acc.push(
                p.id(),
                Code::UnlistedVerifier,
                "signer is not a listed verifier for this class",
            );
            return;
        }
        if !contingent_bound(acc, t, snap, p) {
            return;
        }
        if matches!(p.kind(), K_ATTEST_MECH | K_ATTEST_GROUND)
            && !matches!(p.str_field("result"), Some("pass" | "fail"))
        {
            acc.push(
                p.id(),
                Code::Malformed,
                format!("`{cond}` is a condition: its attestation's result is `pass` or `fail`"),
            );
            return;
        }
        let verdict = p
            .str_field("result")
            .or_else(|| p.str_field("verdict"))
            .unwrap_or_default()
            .to_string();
        let list = evidence
            .entry((report.to_string(), cond.to_string()))
            .or_default();
        list.push((verdict, p.id().to_string()));
        list.sort();
    }

    /// An attestation on a disqualifier (CR-003 §2.2): only the keys it names count, it must say
    /// `hit` or `clear`, and it must come before the act that settles the REPORT (commitments are
    /// atomic: nothing is checked after settlement).
    #[allow(clippy::too_many_arguments)]
    fn disqualifier_attestation(
        &self,
        acc: &mut Acc,
        run: &Run,
        snaps: &BTreeMap<String, State>,
        (snap, t): (&State, &Terms),
        d: &Disqualifier,
        p: &Parsed,
        evidence: &mut EvidenceMap,
    ) {
        let report = snap.report.as_deref().unwrap_or_default();
        let kind_class = match p.kind() {
            K_ATTEST_MECH => ConditionClass::Mechanical,
            K_ATTEST_GROUND => ConditionClass::Grounding,
            K_ATTEST_JUDGED => ConditionClass::Judged,
            _ => ConditionClass::Interpretive,
        };
        if kind_class != d.class {
            acc.push(
                p.id(),
                Code::WrongEvidenceClass,
                format!(
                    "{} cannot attest a {:?} disqualifier",
                    name(p.kind()),
                    d.class
                ),
            );
            return;
        }
        let signer = p.ev.pubkey.as_str();
        let listed = d.runners().iter().any(|r| r == signer);
        if signer == run.performer && d.class == ConditionClass::Judged {
            // V-17.1 reports it (`self_judged`).
        } else if !listed || signer == run.performer {
            acc.push(
                p.id(),
                Code::UnsupportedDisqualification,
                format!("signer is not named in disqualifier `{}`", d.id),
            );
            return;
        }
        // Checked before settlement only: the act that settles this REPORT closes it.
        let closed = snaps
            .values()
            .filter(|s| s.report.as_deref() == Some(report))
            .filter(|s| matches!(s.phase, Phase::Satisfied | Phase::Settled))
            .map(|s| s.head_at)
            .min();
        if closed.is_some_and(|c| p.at() > c) {
            acc.push(
                p.id(),
                Code::Late,
                format!(
                    "disqualifier `{}` attested after the commitment settled; disqualifiers are checked before settlement",
                    d.id
                ),
            );
            return;
        }
        if !contingent_bound(acc, t, snap, p) {
            return;
        }
        let verdict = if d.class == ConditionClass::Judged {
            match self.judged_evidence(acc, run, &d.as_condition(), report, p, true) {
                Some(v) => v,
                None => return,
            }
        } else {
            match p.str_field("result") {
                Some("hit") => "hit",
                Some("clear") => "clear",
                _ => {
                    acc.push(
                        p.id(),
                        Code::Malformed,
                        "a disqualifier attestation's result is `hit` or `clear`",
                    );
                    return;
                }
            }
        };
        let list = evidence
            .entry((report.to_string(), d.id.clone()))
            .or_default();
        list.push((verdict.to_string(), p.id().to_string()));
        list.sort();
    }

    /// V-17 for one `K_ATTEST_JUDGED`. Returns the verdict it counts as (`pass` / `fail`; for a
    /// disqualifier `hit` / `clear`), or `None` if it doesn't count. Every failed check records
    /// its own anomaly. For a judged disqualifier (CR-003) the judges score whether the act
    /// happened: at or above θ is a `hit`, and so is a detected injection.
    fn judged_evidence(
        &self,
        acc: &mut Acc,
        run: &Run,
        condition: &crate::terms::Condition,
        report: &str,
        p: &Parsed,
        disqualifier: bool,
    ) -> Option<&'static str> {
        // Terms parse only with consistent judged fields (Terms::parse).
        let j = condition.judged().ok()?;
        let signer = p.ev.pubkey.as_str();
        let mut counts = true;

        // 1. A listed runner, never the performer.
        if signer == run.performer {
            acc.push(
                p.id(),
                Code::SelfJudged,
                "the performer cannot run its own judges",
            );
            counts = false;
        } else if !j.runners.iter().any(|r| r == signer) {
            acc.push(
                p.id(),
                Code::UnlistedVerifier,
                "signer is not a listed runner of this judged condition",
            );
            counts = false;
        }

        // 2-3. Fingerprints match the terms exactly (as a multiset); run count = repeats.
        let fp = |v: &Value| -> Option<String> {
            let o: serde_json::Map<String, Value> = [
                "model_id",
                "model_version",
                "prompt_template_hash",
                "harness_hash",
                "sampling",
            ]
            .iter()
            .map(|k| Some((k.to_string(), v.get(*k)?.clone())))
            .collect::<Option<_>>()?;
            jcs::hash(&Value::Object(o)).ok()
        };
        let mut want: Vec<String> = j
            .judges
            .iter()
            .filter_map(|f| serde_json::to_value(f).ok().as_ref().and_then(fp))
            .collect();
        let entries = p.content["judges"].as_array().cloned().unwrap_or_default();
        let mut got: Vec<String> = entries.iter().filter_map(fp).collect();
        want.sort();
        got.sort();
        if want != got {
            acc.push(
                p.id(),
                Code::JudgeMismatch,
                "judge fingerprints differ from the terms",
            );
            counts = false;
        }
        if entries
            .iter()
            .any(|e| e["runs"].as_array().map_or(0, Vec::len) != j.repeats as usize)
        {
            acc.push(
                p.id(),
                Code::JudgeMismatch,
                format!("every judge must run exactly {} time(s)", j.repeats),
            );
            counts = false;
        }

        // 4. The rubric used is the committed one (public since INV-14).
        let rubric = p.str_field("rubric_text_hash").unwrap_or_default();
        let inline_ok = p
            .str_field("rubric_text")
            .is_none_or(|text| jcs::sha256_hex(text.as_bytes()) == j.rubric_hash);
        if rubric != j.rubric_hash || !inline_ok {
            acc.push(
                p.id(),
                Code::RubricMismatch,
                "revealed rubric does not match rubric_hash",
            );
            counts = false;
        }
        if !counts {
            return None;
        }

        // Recompute the aggregate; the attestation's own numbers must agree.
        let medians: Vec<f64> = entries
            .iter()
            .filter_map(|e| e["median"].as_f64())
            .collect();
        let aggregate = match j.aggregation {
            crate::terms::Aggregation::All => medians.iter().copied().fold(f64::INFINITY, f64::min),
            crate::terms::Aggregation::Mean => medians.iter().sum::<f64>() / medians.len() as f64,
            crate::terms::Aggregation::Median => act::median(&medians),
        };
        let injected = p.content["injection_detected"] == true;
        let pass = aggregate >= j.threshold && !injected;
        let hit = aggregate >= j.threshold || injected;
        let verdict = match (disqualifier, pass, hit) {
            (false, true, _) => "pass",
            (false, false, _) => "fail",
            (true, _, true) => "hit",
            (true, _, false) => "clear",
        };
        let declared = p.content["aggregate_score"].as_f64().unwrap_or(f64::NAN);
        if (declared - aggregate).abs() > 1e-9 || p.str_field("result") != Some(verdict) {
            acc.push(
                p.id(),
                Code::Malformed,
                format!("declared aggregate/result disagree with the recomputed {aggregate}"),
            );
            return None;
        }

        // 5. Injection fails the condition and is recorded on the performer (its REPORT).
        if injected {
            acc.push(
                report,
                Code::InjectionDetected,
                format!(
                    "judge runner {} detected text addressed to the judge",
                    &signer[..8.min(signer.len())]
                ),
            );
        }
        // 6. Threshold under the declared aggregation (a low score on a disqualifier is `clear`).
        if aggregate < j.threshold && !disqualifier {
            acc.push(
                p.id(),
                Code::BelowThreshold,
                format!("aggregate {aggregate} < threshold {}", j.threshold),
            );
        }
        Some(verdict)
    }

    /// Under the verifier trigger, nothing becomes promised before the escrow is acknowledged.
    fn require_escrow(&self, t: &Terms) -> Result<(), (Code, String)> {
        if t.settlement_trigger == crate::terms::SettlementTrigger::Verifier
            && !self.escrow_acked(t)
        {
            return Err((
                Code::MissingEscrowAck,
                "no ESCROW_ACK from the settlement_verifier for this hashlock".into(),
            ));
        }
        Ok(())
    }

    /// REJECT gaps must name conditions (or disqualifiers) of the terms.
    fn check_gaps(&self, terms: &Terms, k: &Parsed) -> Result<(), (Code, String)> {
        for gap in k.content["gaps"].as_array().into_iter().flatten() {
            let cond = gap["condition"].as_str().unwrap_or_default();
            if terms.condition(cond).is_none()
                && terms.disqualifier(cond).is_none()
                && cond != "artifact_unavailable"
            {
                return Err((
                    Code::Malformed,
                    format!("gap names unknown condition `{cond}`"),
                ));
            }
        }
        Ok(())
    }

    /// CR-002: an `ESCROW_ACK` from the settlement verifier for the hashlock of `t`.
    fn escrow_acked(&self, t: &Terms) -> bool {
        let (Some(v), Some(h)) = (t.settlement_verifier.as_deref(), t.hashlock()) else {
            return false;
        };
        self.members.iter().any(|p| {
            p.kind() == K_ESCROW_ACK
                && p.ev.pubkey == v
                && p.str_field("hashlock")
                    .is_some_and(|x| x.eq_ignore_ascii_case(h))
        })
    }

    /// Condition statuses against the REPORT at `st` (for verifier-trigger legality). Uses the
    /// same evidence rules as the fold's own pass; anomalies are reported there, not here.
    fn conditions_at(
        &self,
        run: &Run,
        terms: &Terms,
        st: &State,
    ) -> BTreeMap<String, ConditionStatus> {
        self.verdicts_at(run, terms, st).0
    }

    /// Disqualifiers with a counted `hit` against the REPORT at `st`.
    fn hits_at(&self, run: &Run, terms: &Terms, st: &State) -> BTreeSet<String> {
        self.verdicts_at(run, terms, st)
            .1
            .into_iter()
            .filter(|(_, s)| *s == DisqualifierStatus::Hit)
            .map(|(id, _)| id)
            .collect()
    }

    fn verdicts_at(
        &self,
        run: &Run,
        terms: &Terms,
        st: &State,
    ) -> (
        BTreeMap<String, ConditionStatus>,
        BTreeMap<String, DisqualifierStatus>,
    ) {
        let Some(report) = st.report.clone() else {
            return Default::default();
        };
        let snaps = BTreeMap::from([(report.clone(), st.clone())]);
        let mut scratch = Acc::default();
        let mut evidence = BTreeMap::new();
        for p in self.members.iter().filter(|p| {
            matches!(
                p.kind(),
                K_ATTEST_MECH | K_ATTEST_GROUND | K_ATTEST_JUDGED | K_INTERP_REPORT
            )
        }) {
            self.attestation(&mut scratch, run, &snaps, p, &mut evidence);
        }
        let conditions = terms
            .conditions
            .iter()
            .map(|c| {
                let ev = evidence.get(&(report.clone(), c.id.clone()));
                (c.id.clone(), condition_status(c.class, ev))
            })
            .collect();
        let disqualifiers = terms
            .disqualifiers
            .iter()
            .map(|d| {
                let ev = evidence.get(&(report.clone(), d.id.clone()));
                (d.id.clone(), disqualifier_status(ev))
            })
            .collect();
        (conditions, disqualifiers)
    }

    /// A REJECT against the disqualifiers: whether any has a counted hit (the commitment then
    /// fails outright, whatever the gaps name: CR-003 §2.2), and a flag for each gap naming a
    /// disqualifier without one (a pretextual disqualification, recorded on the requester).
    fn reject_disqualifiers(
        &self,
        run: &Run,
        terms: &Terms,
        st: &State,
        k: &Parsed,
        flags: &mut Flags,
    ) -> bool {
        let named: Vec<&str> = k.content["gaps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|g| g["condition"].as_str())
            .filter(|c| terms.disqualifier(c).is_some())
            .collect();
        if named.is_empty() && terms.disqualifiers.is_empty() {
            return false;
        }
        let hits = self.hits_at(run, terms, st);
        for d in named.into_iter().filter(|d| !hits.contains(*d)) {
            flags.push((
                Code::UnsupportedDisqualification,
                format!("REJECT names disqualifier `{d}`, which has no counted hit"),
            ));
        }
        !hits.is_empty()
    }

    /// V-15, V-5, V-6, V-9, V-10, V-8 for one candidate act at the current head.
    fn step(
        &self,
        run: &Run,
        terms: &Terms,
        st: &State,
        k: &Parsed,
        price: u64,
        mint: Option<&str>,
    ) -> Step {
        let kind = k.kind();
        let by = k.ev.pubkey.as_str();
        let n = run.profile == Profile::N;
        let is_req = by == run.requester;
        let is_perf = by == run.performer;
        let verifier = terms.settlement_trigger == SettlementTrigger::Verifier;
        let contingent = terms.settlement_trigger == SettlementTrigger::Contingent;
        let mut next = st.clone();
        next.head = k.id().to_string();
        next.head_at = k.at();
        let mut flags = Vec::new();

        if k.at() < st.head_at {
            return Err((Code::TimeRegress, "created_at precedes prev".into()));
        }

        let unauthorized = |who: &str| {
            Err((
                Code::Unauthorized,
                format!("{} must be signed by the {who}", name(kind)),
            ))
        };
        let illegal = || {
            Err((
                Code::IllegalTransition,
                format!(
                    "{} not legal from {:?} in profile {:?}",
                    name(kind),
                    st.phase,
                    run.profile
                ),
            ))
        };
        use Phase::*;
        match kind {
            K_COUNTER => {
                if !run.parties(by) {
                    return unauthorized("requester or performer");
                }
                if !n || !matches!(st.phase, Requested | Countered | Promised) {
                    return illegal();
                }
                let h = k.terms.clone().unwrap_or_default();
                if !run.terms.contains_key(&h) {
                    return Err((
                        Code::PendingTerms,
                        format!("counter terms {h} not available"),
                    ));
                }
                run.terms[&h].settlement_combo()?;
                next.pending = Some(Pending {
                    terms: h,
                    by: by.into(),
                });
                if !st.promised {
                    next.phase = Countered;
                }
            }
            K_ACCEPT => {
                if !run.parties(by) {
                    return unauthorized("requester or performer");
                }
                let Some(pending) = st
                    .pending
                    .as_ref()
                    .filter(|_| n && matches!(st.phase, Countered | Promised))
                else {
                    return illegal();
                };
                if pending.by == by {
                    return unauthorized("party that did not make the counter");
                }
                if k.terms.as_deref() != Some(pending.terms.as_str()) {
                    return Err((
                        Code::TermsMismatch,
                        "ACCEPT terms differ from the pending counter".into(),
                    ));
                }
                if let Some(t) = run.terms.get(&pending.terms) {
                    require_closed(t)?;
                    self.require_escrow(t)?;
                }
                next.terms = pending.terms.clone();
                next.pending = None;
                next.promised = true;
                next.phase = Promised;
            }
            K_PROMISE => {
                if !is_perf {
                    return unauthorized("performer");
                }
                if !n || st.phase != Requested {
                    return illegal();
                }
                if k.terms.as_deref() != Some(st.terms.as_str()) {
                    return Err((
                        Code::TermsMismatch,
                        "PROMISE terms differ from the terms in force".into(),
                    ));
                }
                require_closed(terms)?;
                self.require_escrow(terms)?;
                next.promised = true;
                next.phase = Promised;
            }
            K_DECLINE => {
                if !is_perf {
                    return unauthorized("performer");
                }
                if !matches!(st.phase, Requested | Countered) {
                    return illegal();
                }
                next.phase = Declined;
            }
            K_WITHDRAW => {
                if !is_req {
                    return unauthorized("requester");
                }
                if !matches!(st.phase, Requested | Countered | Promised) {
                    return illegal();
                }
                next.phase = Withdrawn;
            }
            K_REVOKE => {
                if !is_perf {
                    return unauthorized("performer");
                }
                if !n || !matches!(st.phase, Promised | Reworking) {
                    return illegal();
                }
                next.phase = Revoked;
            }
            K_REPORT => {
                if !is_perf {
                    return unauthorized("performer");
                }
                let legal = if n {
                    matches!(st.phase, Promised | Reworking)
                } else {
                    st.phase == Requested
                };
                if !legal {
                    return illegal();
                }
                // T1: a REPORT from Requested is the implicit promise.
                if st.phase == Requested {
                    require_closed(terms)?;
                    self.require_escrow(terms)?;
                }
                if k.at() > terms.deadline {
                    flags.push((Code::Late, "REPORT after the terms deadline".into()));
                }
                next.phase = Reported;
                next.pending = None;
                next.report = Some(k.id().to_string());
                next.report_at = Some(k.at());
                next.fund_locktime = None;
                if contingent {
                    // The deliverable is encrypted under k; the REPORT names C and H = sha256(k).
                    let hex64 = |f: &str| {
                        k.str_field(f)
                            .filter(|h| is_sha256_hex(h))
                            .map(str::to_ascii_lowercase)
                    };
                    let (Some(c), Some(h)) = (hex64("ciphertext"), hex64("hashlock")) else {
                        return Err((
                            Code::Malformed,
                            "a contingent REPORT needs `ciphertext` and `hashlock` (sha256 hex)"
                                .into(),
                        ));
                    };
                    next.phase = Offered;
                    next.ciphertext = Some(c);
                    next.offer_hashlock = Some(h);
                }
            }
            K_SATISFY | K_REJECT if st.phase == Settled => {
                // After release: one recorded judgement, no settlement effect (CR-002).
                if !is_req {
                    return unauthorized("requester");
                }
                if st.judged {
                    return illegal();
                }
                if kind == K_REJECT {
                    self.check_gaps(terms, k)?;
                    self.reject_disqualifiers(run, terms, st, k, &mut flags);
                }
                next.judged = true;
            }
            K_SATISFY | K_REJECT
                if (verifier && st.phase == Reported) || (contingent && st.phase == Offered) =>
            {
                if !is_req {
                    return unauthorized("requester");
                }
                if kind == K_SATISFY {
                    return Err((
                        Code::IllegalTransition,
                        "the designated trigger settles (RELEASE or CLAIM); SATISFY is a judgement after it".into(),
                    ));
                }
                // REJECT for rework only against a failing attestation or a disqualifier hit, so
                // a requester can't pre-empt the settlement of work that passes.
                self.check_gaps(terms, k)?;
                let failing = self
                    .conditions_at(run, terms, st)
                    .values()
                    .any(|s| *s == ConditionStatus::Fail);
                let hit = !self.hits_at(run, terms, st).is_empty();
                if !failing && !hit {
                    return Err((
                        Code::IllegalTransition,
                        "before settlement, REJECT needs a failing attestation or a disqualifier hit".into(),
                    ));
                }
                if late_review(terms, st, k) {
                    flags.push((Code::Late, "REJECT after the review window".into()));
                }
                let breach = self.reject_disqualifiers(run, terms, st, k, &mut flags);
                if n && !breach && st.rework_count < terms.rework_limit {
                    next.rework_count += 1;
                    next.phase = Reworking;
                } else {
                    next.phase = Rejected;
                    next.rejected_at = Some(k.at());
                }
            }
            K_FUND => {
                if !is_req {
                    return unauthorized("requester");
                }
                if !contingent || st.phase != Offered {
                    return illegal();
                }
                let pay_mint = terms.payment.as_ref().map(|p| p.mint.as_str());
                let bound = k
                    .str_field("hashlock")
                    .is_some_and(|h| Some(h.to_ascii_lowercase()) == st.offer_hashlock)
                    && k.content["amount"]
                        .as_u64()
                        .is_some_and(|a| a >= terms.price.amount)
                    && k.str_field("mint") == pay_mint;
                if !bound {
                    return Err((
                        Code::FundMismatch,
                        "FUND must lock the price at the terms' mint to the REPORT's hashlock"
                            .into(),
                    ));
                }
                let all_pass = self
                    .conditions_at(run, terms, st)
                    .values()
                    .all(|s| *s == ConditionStatus::Pass);
                if !all_pass {
                    flags.push((
                        Code::PrematureFund,
                        "FUND before every condition passed".into(),
                    ));
                }
                next.phase = Funded;
                next.fund_locktime = k.content["locktime"].as_u64();
            }
            K_CLAIM => {
                if !is_perf {
                    return unauthorized("performer");
                }
                if !contingent || st.phase != Funded {
                    return illegal();
                }
                let ok = k
                    .str_field("key")
                    .and_then(|key| hex::decode(key).ok())
                    .is_some_and(|b| Some(jcs::sha256_hex(&b)) == st.offer_hashlock);
                if !ok {
                    return Err((
                        Code::BadKey,
                        "sha256(key) does not match the REPORT's hashlock".into(),
                    ));
                }
                next.phase = Settled;
            }
            K_RELEASE => {
                if !verifier {
                    return illegal();
                }
                if terms.settlement_verifier.as_deref() != Some(by) {
                    return Err((
                        Code::BadRelease,
                        "RELEASE must be signed by the settlement_verifier".into(),
                    ));
                }
                if st.phase != Reported {
                    return illegal();
                }
                if let Err((_, why)) = check_preimage(terms, k) {
                    return Err((Code::BadRelease, why));
                }
                let statuses = self.conditions_at(run, terms, st);
                if let Some((c, s)) = statuses.iter().find(|(_, s)| **s != ConditionStatus::Pass) {
                    return Err((
                        Code::BadRelease,
                        format!("condition `{c}` has not passed ({s:?})"),
                    ));
                }
                if let Some(d) = self.hits_at(run, terms, st).into_iter().next() {
                    return Err((
                        Code::BadRelease,
                        format!("disqualifier `{d}` was attested as hit"),
                    ));
                }
                next.phase = Settled;
            }
            K_SATISFY => {
                if !is_req {
                    return unauthorized("requester");
                }
                if st.phase != Reported {
                    return illegal();
                }
                check_preimage(terms, k)?;
                if late_review(terms, st, k) {
                    flags.push((Code::Late, "SATISFY after the review window".into()));
                }
                next.phase = Satisfied;
            }
            K_REJECT => {
                if !is_req {
                    return unauthorized("requester");
                }
                if st.phase != Reported {
                    return illegal();
                }
                self.check_gaps(terms, k)?;
                if late_review(terms, st, k) {
                    flags.push((Code::Late, "REJECT after the review window".into()));
                }
                // With a disqualifier hit, a REJECT fails the commitment: breach, no rework.
                let breach = self.reject_disqualifiers(run, terms, st, k, &mut flags);
                if n && !breach && st.rework_count < terms.rework_limit {
                    next.rework_count += 1;
                    next.phase = Reworking;
                } else {
                    next.phase = Rejected;
                    next.rejected_at = Some(k.at());
                }
            }
            K_DISPUTE => {
                if !run.parties(by) {
                    return unauthorized("requester or performer");
                }
                if !n || terms.arbiter.is_none() {
                    return Err((
                        Code::IllegalTransition,
                        "DISPUTE requires an arbiter in the terms".into(),
                    ));
                }
                let disputable_rejection = st.phase == Rejected
                    && !st.ruled
                    && st
                        .rejected_at
                        .is_some_and(|r| k.at() <= r.saturating_add(terms.review_window));
                if st.phase != Reported && !disputable_rejection {
                    return illegal();
                }
                next.phase = Disputed;
            }
            K_RULING => {
                if terms.arbiter.as_deref() != Some(by) {
                    return unauthorized("arbiter named in the terms");
                }
                if !n || st.phase != Disputed {
                    return illegal();
                }
                next.ruled = true;
                if k.str_field("decision") == Some("performer") {
                    check_preimage(terms, k)?;
                    next.phase = Satisfied;
                } else {
                    next.phase = Rejected;
                    next.rejected_at = Some(k.at());
                }
            }
            _ => return illegal(),
        }

        // V-8 for performer-signed acts.
        if is_perf {
            match self.bindings.status(by, kind, price, mint, k.at()) {
                BindingStatus::OutOfScope { reason } => return Err((Code::OutOfScope, reason)),
                BindingStatus::Revoked { binding } => {
                    return Err((Code::Revoked, format!("binding {binding} revoked")))
                }
                _ => {}
            }
        }
        Ok((next, flags))
    }
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// CR-003 §2.4: underspecified requests are allowed; underspecified promises are not.
fn require_closed(t: &Terms) -> Result<(), (Code, String)> {
    match t.unclosed() {
        Some(why) => Err((Code::UnclosedCondition, why)),
        None => Ok(()),
    }
}

/// Contingent (V-21): an attestation names the REPORT's ciphertext, hashlock and plaintext hash.
/// Records `attestation_mismatch` and returns false otherwise.
fn contingent_bound(acc: &mut Acc, t: &Terms, snap: &State, p: &Parsed) -> bool {
    if t.settlement_trigger != SettlementTrigger::Contingent {
        return true;
    }
    let same =
        |f: &str, want: &Option<String>| p.str_field(f).map(str::to_ascii_lowercase) == *want;
    let plaintext = p.str_field("plaintext").is_some_and(is_sha256_hex);
    if !same("ciphertext", &snap.ciphertext)
        || !same("hashlock", &snap.offer_hashlock)
        || !plaintext
    {
        acc.push(
            p.id(),
            Code::AttestationMismatch,
            "a contingent attestation must name the REPORT's ciphertext and hashlock, and the plaintext hash",
        );
        return false;
    }
    true
}

fn disqualifier_status(ev: Option<&Vec<(String, String)>>) -> DisqualifierStatus {
    match ev {
        Some(v) if v.iter().any(|(r, _)| r == "hit") => DisqualifierStatus::Hit,
        Some(v) if v.iter().any(|(r, _)| r == "clear") => DisqualifierStatus::Clear,
        _ => DisqualifierStatus::Unchecked,
    }
}

fn condition_status(class: ConditionClass, ev: Option<&Vec<(String, String)>>) -> ConditionStatus {
    match class {
        // Nothing can attest an open condition (its class is unknown); terms with one are never
        // promised (closure rule).
        ConditionClass::Open => ConditionStatus::Unverified,
        ConditionClass::Mechanical | ConditionClass::Grounding | ConditionClass::Judged => match ev
        {
            Some(v) if v.iter().any(|(r, _)| r == "fail") => ConditionStatus::Fail,
            Some(v) if v.iter().any(|(r, _)| r == "pass") => ConditionStatus::Pass,
            _ => ConditionStatus::Unverified,
        },
        ConditionClass::Interpretive => ConditionStatus::Advisory {
            verdicts: ev
                .map(|v| v.iter().map(|(r, _)| r.clone()).collect())
                .unwrap_or_default(),
        },
        ConditionClass::Human => ConditionStatus::Human,
    }
}

fn check_preimage(terms: &Terms, k: &Parsed) -> Result<(), (Code, String)> {
    let Some(h) = terms.hashlock() else {
        return Ok(());
    };
    let Some(s) = k.str_field("preimage") else {
        return Err((
            Code::Malformed,
            "preimage required: terms carry a hashlock".into(),
        ));
    };
    match hex::decode(s) {
        Ok(bytes) if bytes.len() == 32 && jcs::sha256_hex(&bytes) == h.to_ascii_lowercase() => {
            Ok(())
        }
        _ => Err((
            Code::BadPreimage,
            "sha256(preimage) does not match the hashlock".into(),
        )),
    }
}

fn late_review(terms: &Terms, st: &State, k: &Parsed) -> bool {
    st.report_at
        .is_some_and(|r| k.at() > r.saturating_add(terms.review_window))
}

fn governed(run: &Run, terms: &Terms, k: &Parsed) -> bool {
    let Some(g) = &terms.governance else {
        return false;
    };
    let party = match g.party {
        GovernedParty::Performer => &run.performer,
        GovernedParty::Requester => &run.requester,
    };
    g.kinds.contains(&k.kind()) && &k.ev.pubkey == party
}

fn needs_endorsement(run: &Run, terms: &Terms, k: &Parsed) -> bool {
    governed(run, terms, k)
}

fn endorsed(run: &Run, terms: &Terms, k: &Parsed) -> bool {
    let Some(g) = &terms.governance else {
        return true;
    };
    run.endorsements.get(k.id()).is_some_and(|list| {
        list.iter().any(|(signer, at)| {
            g.keys.iter().any(|key| key == signer) && *at <= k.at().saturating_add(g.window)
        })
    })
}
