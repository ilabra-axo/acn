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
use crate::terms::{ConditionClass, GovernedParty, Terms};

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
                Ok(h) if h == tag => match Terms::parse(doc) {
                    Ok(t) => {
                        terms.insert(h, t);
                    }
                    Err(e) => {
                        acc.push(p.id(), Code::Malformed, format!("terms: {e}"));
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
        let price = root.price.amount;
        let mint = root.payment.as_ref().map(|p| p.mint.clone());

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
        let mut evidence: BTreeMap<(String, String), Vec<(String, String)>> = BTreeMap::new();
        for p in self.members.iter() {
            match p.kind() {
                K_PROGRESS => self.progress(acc, &run, &snapshots, p, &mut commitment),
                K_ATTEST_MECH | K_ATTEST_GROUND | K_INTERP_REPORT => {
                    self.attestation(acc, &run, &snapshots, p, &mut evidence)
                }
                _ => {}
            }
        }

        // Condition status for the current report.
        let terms_now = run.terms.get(&st.terms).cloned().unwrap_or(root);
        if let Some(report) = &st.report {
            for cond in &terms_now.conditions {
                let ev = evidence.get(&(report.clone(), cond.id.clone()));
                let status = match cond.class {
                    ConditionClass::Mechanical | ConditionClass::Grounding => match ev {
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
                };
                commitment.conditions.insert(cond.id.clone(), status);
            }
        }

        // V-16: rejections contradicted by mechanical evidence.
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
                let mechanical = t
                    .and_then(|t| t.condition(cond))
                    .is_some_and(|c| c.class == ConditionClass::Mechanical);
                let ev = evidence.get(&(report.clone(), cond.to_string()));
                if mechanical
                    && ev.is_some_and(|v| {
                        v.iter().any(|(r, _)| r == "pass") && !v.iter().any(|(r, _)| r == "fail")
                    })
                {
                    acc.push(
                        id,
                        Code::ContradictedRejection,
                        format!("condition `{cond}` has a passing mechanical attestation"),
                    );
                }
            }
        }

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
        evidence: &mut BTreeMap<(String, String), Vec<(String, String)>>,
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
            acc.push(
                p.id(),
                Code::Malformed,
                format!("unknown condition `{cond}`"),
            );
            return;
        };
        let kind_class = match p.kind() {
            K_ATTEST_MECH => ConditionClass::Mechanical,
            K_ATTEST_GROUND => ConditionClass::Grounding,
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
        if !t.verifier_counts(&p.ev.pubkey, condition.class) {
            acc.push(
                p.id(),
                Code::UnlistedVerifier,
                "signer is not a listed verifier for this class",
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
                if k.at() > terms.deadline {
                    flags.push((Code::Late, "REPORT after the terms deadline".into()));
                }
                next.phase = Reported;
                next.pending = None;
                next.report = Some(k.id().to_string());
                next.report_at = Some(k.at());
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
                for gap in k.content["gaps"].as_array().into_iter().flatten() {
                    let cond = gap["condition"].as_str().unwrap_or_default();
                    if terms.condition(cond).is_none() && cond != "artifact_unavailable" {
                        return Err((
                            Code::Malformed,
                            format!("gap names unknown condition `{cond}`"),
                        ));
                    }
                }
                if late_review(terms, st, k) {
                    flags.push((Code::Late, "REJECT after the review window".into()));
                }
                if n && st.rework_count < terms.rework_limit {
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
