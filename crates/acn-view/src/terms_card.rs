//! The terms card (spec §12, CR-003 §2.7): what a party sees before it signs `PROMISE`, `ACCEPT`
//! or `SATISFY`. Four parts: **Done means**, **Checked by**, **Pay if**, **Disqualified if**;
//! plus who drafted the version, and a diff against the previous one. Derived from the terms
//! only; nothing here has semantics (INV-12).

use acn_core::terms::{Assurance, ConditionClass, SettlementTrigger, Terms};
use serde::Serialize;
use serde_json::Value;

use crate::Role;

/// One terms version along the chain: a `REQUEST` or `COUNTER` that carried terms.
#[derive(Debug, Clone, Serialize)]
pub struct TermsVersionView {
    /// 1 for the request's terms, then one per counter.
    pub version: u32,
    pub hash: String,
    pub act: &'static str,
    pub drafted_by: String,
    pub drafted_by_role: Role,
    pub drafted_by_me: bool,
    /// The version before it.
    pub prev: Option<String>,
    /// The terms in force.
    pub in_force: bool,
    /// A counter awaiting `ACCEPT`.
    pub pending: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TermsCard {
    pub version: u32,
    pub hash: String,
    pub drafted_by: String,
    pub drafted_by_role: Role,
    pub drafted_by_me: bool,
    /// Overall assurance (weakest condition); `None` while a condition is open.
    pub assurance: Option<Assurance>,
    pub done_means: Vec<DoneItem>,
    pub checked_by: Vec<CheckedBy>,
    pub pay_if: PayIf,
    pub disqualified_if: Vec<DisqualifierItem>,
    /// Why this version can't be promised or accepted yet (closure rule, V-26).
    pub unclosed: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoneItem {
    pub id: String,
    pub class: ConditionClass,
    pub assurance: Option<Assurance>,
    pub description: String,
    /// For an `open` condition: the class the requester suggests.
    pub suggested_class: Option<ConditionClass>,
    /// Where the performer reads the criterion (INV-14).
    pub check_ref: Option<String>,
    pub rubric_ref: Option<String>,
    pub rubric_text: Option<String>,
}

/// Who produces the evidence for one condition or disqualifier.
#[derive(Debug, Clone, Serialize)]
pub struct CheckedBy {
    pub item: String,
    /// Keys whose attestations count.
    pub keys: Vec<String>,
    /// `human` / `interpretive`: the requester decides.
    pub requester: bool,
    /// `judged`: each judge's pinned fingerprint, one line each.
    pub judges: Vec<String>,
    /// `judged`: threshold, scale, aggregation and repeats in one line.
    pub rule: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PayIf {
    pub price_sat: u64,
    pub trigger: SettlementTrigger,
    pub settlement_verifier: Option<String>,
    pub arbiter: Option<String>,
    pub deadline: u64,
    pub review_window: u64,
    pub rework_limit: u32,
    pub fund_window: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisqualifierItem {
    pub id: String,
    pub class: ConditionClass,
    pub assurance: Option<Assurance>,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    Condition,
    Disqualifier,
    Field,
    /// `before` / `after` are unix seconds.
    Time,
}

/// One change between two terms versions.
#[derive(Debug, Clone, Serialize)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub what: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub fn card(t: &Terms, v: &TermsVersionView) -> TermsCard {
    let s = |c: &acn_core::terms::Condition, f: &str| {
        c.extra.get(f).and_then(Value::as_str).map(str::to_owned)
    };
    let done_means = t
        .conditions
        .iter()
        .map(|c| DoneItem {
            id: c.id.clone(),
            class: c.class,
            assurance: c.class.assurance(),
            description: c.description.clone(),
            suggested_class: c
                .extra
                .get("suggested_class")
                .and_then(|x| serde_json::from_value(x.clone()).ok()),
            check_ref: s(c, "check_ref"),
            rubric_ref: s(c, "rubric_ref"),
            rubric_text: s(c, "rubric_text"),
        })
        .collect();
    let mut checked_by: Vec<CheckedBy> = t.conditions.iter().map(|c| checked(t, c, None)).collect();
    checked_by.extend(
        t.disqualifiers
            .iter()
            .map(|d| checked(t, &d.as_condition(), Some(d.runners()))),
    );
    TermsCard {
        version: v.version,
        hash: v.hash.clone(),
        drafted_by: v.drafted_by.clone(),
        drafted_by_role: v.drafted_by_role,
        drafted_by_me: v.drafted_by_me,
        assurance: t.assurance(),
        done_means,
        checked_by,
        pay_if: PayIf {
            price_sat: t.price.amount,
            trigger: t.settlement_trigger,
            settlement_verifier: t.settlement_verifier.clone(),
            arbiter: t.arbiter.clone(),
            deadline: t.deadline,
            review_window: t.review_window,
            rework_limit: t.rework_limit,
            fund_window: t.fund_window,
        },
        disqualified_if: t
            .disqualifiers
            .iter()
            .map(|d| DisqualifierItem {
                id: d.id.clone(),
                class: d.class,
                assurance: d.class.assurance(),
                description: d.description.clone(),
            })
            .collect(),
        unclosed: t.unclosed(),
    }
}

fn checked(t: &Terms, c: &acn_core::terms::Condition, runners: Option<Vec<String>>) -> CheckedBy {
    let mut out = CheckedBy {
        item: c.id.clone(),
        keys: vec![],
        requester: false,
        judges: vec![],
        rule: None,
    };
    if let Ok(j) = c.judged() {
        out.keys = j.runners.clone();
        out.judges = j
            .judges
            .iter()
            .map(|f| {
                format!(
                    "{} {} · prompt {}… · harness {}… · sampling {}",
                    f.model_id,
                    f.model_version,
                    f.prompt_template_hash.get(..8).unwrap_or_default(),
                    f.harness_hash.get(..8).unwrap_or_default(),
                    f.sampling
                )
            })
            .collect();
        let agg = match j.aggregation {
            acn_core::terms::Aggregation::All => "every judge",
            acn_core::terms::Aggregation::Mean => "mean of the judges",
            acn_core::terms::Aggregation::Median => "median of the judges",
        };
        out.rule = Some(format!(
            "{agg} ≥ {} on {}–{}, median of {} run(s) each",
            j.threshold, j.scale.min, j.scale.max, j.repeats
        ));
        return out;
    }
    match (runners, c.class) {
        (Some(r), _) => out.keys = r,
        (None, ConditionClass::Human | ConditionClass::Interpretive) => out.requester = true,
        (None, ConditionClass::Open) => {}
        (None, class) => {
            out.keys = t
                .verifiers
                .iter()
                .filter(|v| v.classes.contains(&class))
                .map(|v| v.pubkey.clone())
                .collect()
        }
    }
    out
}

/// What changed from `old` to `new`, in reading order.
pub fn diff(old: &Terms, new: &Terms) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let line = |kind, what: &str, before: Option<String>, after: Option<String>| DiffLine {
        kind,
        what: what.into(),
        before,
        after,
    };
    let show =
        |c: &acn_core::terms::Condition| format!("{}: {}", class_name(c.class), c.description);
    // Conditions and disqualifiers, by id, in the order they appear (old first, then added).
    let mut items =
        |kind, a: Vec<acn_core::terms::Condition>, b: Vec<acn_core::terms::Condition>| {
            for x in &a {
                match b.iter().find(|y| y.id == x.id) {
                    None => out.push(line(kind, &x.id, Some(show(x)), None)),
                    Some(y) if y != x => {
                        let mut after = show(y);
                        if y.class == x.class && y.description == x.description {
                            after.push_str(" (checks or judges changed)");
                        }
                        out.push(line(kind, &x.id, Some(show(x)), Some(after)))
                    }
                    _ => {}
                }
            }
            for y in b.iter().filter(|y| !a.iter().any(|x| x.id == y.id)) {
                out.push(line(kind, &y.id, None, Some(show(y))));
            }
        };
    items(
        DiffKind::Condition,
        old.conditions.clone(),
        new.conditions.clone(),
    );
    items(
        DiffKind::Disqualifier,
        old.disqualifiers.iter().map(|d| d.as_condition()).collect(),
        new.disqualifiers.iter().map(|d| d.as_condition()).collect(),
    );
    let mut field = |kind, what: &str, a: String, b: String| {
        if a != b {
            out.push(line(kind, what, Some(a), Some(b)));
        }
    };
    field(
        DiffKind::Field,
        "Deliverable",
        old.deliverable.description.clone(),
        new.deliverable.description.clone(),
    );
    field(
        DiffKind::Field,
        "Price",
        format!("{} sat", old.price.amount),
        format!("{} sat", new.price.amount),
    );
    field(
        DiffKind::Time,
        "Deadline",
        old.deadline.to_string(),
        new.deadline.to_string(),
    );
    field(
        DiffKind::Field,
        "Review window",
        format!("{} s", old.review_window),
        format!("{} s", new.review_window),
    );
    field(
        DiffKind::Field,
        "Revisions",
        old.rework_limit.to_string(),
        new.rework_limit.to_string(),
    );
    field(
        DiffKind::Field,
        "Settlement",
        format!("{:?}", old.settlement_trigger),
        format!("{:?}", new.settlement_trigger),
    );
    let keys = |t: &Terms| {
        t.verifiers
            .iter()
            .map(|v| {
                format!(
                    "{}… ({})",
                    v.pubkey.get(..8).unwrap_or_default(),
                    v.classes
                        .iter()
                        .map(|c| class_name(*c))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    field(DiffKind::Field, "Verifiers", keys(old), keys(new));
    let arb = |t: &Terms| {
        t.arbiter.as_deref().map_or("none".to_string(), |a| {
            format!("{}…", a.get(..8).unwrap_or_default())
        })
    };
    field(DiffKind::Field, "Arbiter", arb(old), arb(new));
    out
}

fn class_name(c: ConditionClass) -> &'static str {
    match c {
        ConditionClass::Mechanical => "mechanical",
        ConditionClass::Grounding => "grounding",
        ConditionClass::Judged => "judged",
        ConditionClass::Interpretive => "interpretive",
        ConditionClass::Human => "human",
        ConditionClass::Open => "open",
    }
}
