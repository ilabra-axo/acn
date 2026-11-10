//! The terms linter (CR-003 §3, rules L1–L8). It runs on any draft, from either side, before
//! it is sent. Errors block sending; warnings are shown on the terms card. L4–L6 carry the main
//! best practices of docs/acn/cos-templates.md, so they are enforced where terms are written.

use std::collections::{BTreeMap, BTreeSet};

use acn_core::terms::{Condition, ConditionClass, Terms};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Blocks sending.
    Error,
    /// Shown on the card.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// `L1` … `L8`.
    pub rule: &'static str,
    pub level: Level,
    /// The condition or disqualifier id, when the finding is about one.
    pub item: Option<String>,
    pub message: String,
}

/// What the linter can't know from the terms alone.
#[derive(Debug, Clone, Default)]
pub struct LintContext {
    /// The user is about to promise or accept this version (enables L3).
    pub binding: bool,
    /// References (`rubric_ref`, `check_ref`) known to be retrievable. `None`: not checked, so
    /// L2 is skipped (the linter does no I/O).
    pub retrievable: Option<BTreeSet<String>>,
    /// Rubric texts by `rubric_hash`, beyond inline `rubric_text` (for L8).
    pub rubrics: BTreeMap<String, String>,
    /// Price (sat) above which a single judge, or one model family, is a warning (L7).
    pub judge_price_threshold: u64,
}

/// Words that state a quality rather than something checkable or an act (L4, L5). Matched as
/// whole words, case-insensitively.
const QUALITY: &[&str] = &[
    "quality",
    "clear",
    "clean",
    "good",
    "great",
    "nice",
    "readable",
    "idiomatic",
    "elegant",
    "polished",
    "professional",
    "robust",
    "well-written",
    "well",
    "appropriate",
    "reasonable",
    "effort",
    "sloppy",
    "poor",
    "bad",
    "lazy",
    "careless",
    "thorough",
    "excellent",
    "helpful",
];

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

fn quality_words(s: &str) -> Vec<String> {
    words(s)
        .into_iter()
        .filter(|w| QUALITY.contains(&w.as_str()))
        .collect()
}

/// A model family: the leading letters of `model_id` (`qwen2.5` → `qwen`, `gpt-4.1` → `gpt`).
fn family(model_id: &str) -> String {
    model_id
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_lowercase()
}

/// Lint a terms version. Findings are ordered by level, then rule, then item.
pub fn lint(terms: &Terms, cx: &LintContext) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |rule, level, item: Option<&str>, message: String| {
        out.push(Finding {
            rule,
            level,
            item: item.map(str::to_owned),
            message,
        })
    };
    let disq: Vec<Condition> = terms
        .disqualifiers
        .iter()
        .map(|d| d.as_condition())
        .collect();

    for c in &terms.conditions {
        let id = Some(c.id.as_str());
        // L1: who produces the evidence. Mechanical and grounding need a listed verifier of the
        // class; judged ones name runners (required at parse); human and interpretive close by
        // the requester's own act.
        if matches!(
            c.class,
            ConditionClass::Mechanical | ConditionClass::Grounding
        ) && !terms.verifiers.iter().any(|v| v.classes.contains(&c.class))
        {
            push(
                "L1",
                Level::Error,
                id,
                format!("names no evidence producer: list a {:?} verifier", c.class).to_lowercase(),
            );
        }
        // L3: open conditions can't be promised or accepted.
        if cx.binding && c.class == ConditionClass::Open {
            push(
                "L3",
                Level::Error,
                id,
                "is still open: it must be closed before you promise or accept".into(),
            );
        }
        // L4: a quality with no rubric. Fine for judged (rubric), human (the requester decides)
        // and interpretive (advisory) conditions.
        let q = quality_words(&c.description);
        if !q.is_empty()
            && matches!(
                c.class,
                ConditionClass::Mechanical | ConditionClass::Grounding | ConditionClass::Open
            )
        {
            push(
                "L4",
                Level::Warning,
                id,
                format!(
                "states a quality (\"{}\") with no rubric: make it judged with a rubric, or human",
                q.join("\", \"")
            ),
            );
        }
        if c.class == ConditionClass::Judged {
            let Ok(j) = c.judged() else { continue };
            // L6: the obvious gaming route of a judged condition needs a counter.
            if terms.disqualifiers.is_empty() {
                push("L6", Level::Warning, id, "is judged, but no disqualifier covers its most obvious gaming route (e.g. text aimed at the judges, rubric stuffing)".into());
            }
            // L7: one judge, or one model family, once the price matters.
            let fams: BTreeSet<String> = j.judges.iter().map(|f| family(&f.model_id)).collect();
            if terms.price.amount > cx.judge_price_threshold
                && (j.judges.len() < 2 || fams.len() < 2)
            {
                push(
                    "L7",
                    Level::Warning,
                    id,
                    format!(
                        "{} at {} sat: use judges from at least two model families",
                        if j.judges.len() < 2 {
                            "a single judge".to_string()
                        } else {
                            format!(
                                "all judges are {}",
                                fams.iter().next().cloned().unwrap_or_default()
                            )
                        },
                        terms.price.amount
                    ),
                );
            }
            // L8: nothing stops overfitting to the rubric.
            let text = j
                .rubric_text
                .clone()
                .or_else(|| cx.rubrics.get(&j.rubric_hash).cloned())
                .unwrap_or_default()
                .to_lowercase();
            let examples = text.contains("reference") || text.contains("counterexample");
            if j.instances.is_none() && !examples {
                push("L8", Level::Warning, id, "has no held-out instances and no reference or counterexample in its rubric, so it is open to overfitting".into());
            }
        }
    }
    for d in &terms.disqualifiers {
        // L5: disqualifiers name acts; qualities belong in rubrics.
        let q = quality_words(&d.description);
        if !q.is_empty() {
            push(
                "L5",
                Level::Warning,
                Some(&d.id),
                format!(
                    "states a quality (\"{}\") rather than an act: move it to a rubric",
                    q.join("\", \"")
                ),
            );
        }
    }
    // L2: every rubric and check definition can be fetched.
    if let Some(ok) = &cx.retrievable {
        for c in terms.conditions.iter().chain(&disq) {
            let refs = ["rubric_ref", "check_ref"]
                .iter()
                .filter_map(|f| c.extra.get(*f).and_then(|v| v.as_str()));
            for r in refs {
                if !ok.contains(r) {
                    push(
                        "L2",
                        Level::Error,
                        Some(&c.id),
                        format!("`{r}` can't be retrieved"),
                    );
                }
            }
        }
    }
    out.sort_by(|a, b| (a.level, a.rule, &a.item).cmp(&(b.level, b.rule, &b.item)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn terms(f: impl FnOnce(&mut Value)) -> Terms {
        let mut t = json!({
            "v": acn_core::terms::TERMS_VERSION,
            "deliverable": {"description": "x"},
            "conditions": [{"id": "c1", "class": "mechanical", "description": "the test suite passes"}],
            "deadline": 2_000_000_000u64, "review_window": 3600,
            "price": {"amount": 0},
            "verifiers": [{"pubkey": "aa".repeat(32), "classes": ["mechanical"]}]
        });
        f(&mut t);
        Terms::parse(&t).unwrap()
    }

    fn judged(model_ids: &[&str], rubric: &str) -> Value {
        let judges: Vec<Value> = model_ids
            .iter()
            .map(|m| json!({"model_id": m, "model_version": "1", "prompt_template_hash": "00".repeat(32), "harness_hash": "00".repeat(32), "sampling": {}}))
            .collect();
        json!({"id": "c2", "class": "judged", "description": "rated at least 7",
               "judges": judges, "rubric_hash": acn_core::jcs::sha256_hex(rubric.as_bytes()),
               "rubric_ref": "blossom:x", "rubric_text": rubric,
               "scale": {"min": 0, "max": 10}, "threshold": 7, "aggregation": "all",
               "repeats": 1, "runners": ["bb".repeat(32)]})
    }

    fn rules(f: &[Finding]) -> Vec<&'static str> {
        f.iter().map(|x| x.rule).collect()
    }

    #[test]
    fn clean_terms_have_no_findings() {
        assert!(lint(&terms(|_| {}), &LintContext::default()).is_empty());
    }

    #[test]
    fn l1_l3_errors() {
        let t = terms(|t| {
            t["verifiers"] = json!([]);
            t["conditions"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "c2", "class": "open", "description": "don't make it slower"}));
        });
        let f = lint(
            &t,
            &LintContext {
                binding: true,
                ..Default::default()
            },
        );
        assert_eq!(rules(&f), ["L1", "L3"]);
        assert!(f.iter().all(|x| x.level == Level::Error));
        // A REQUEST may leave conditions open.
        assert_eq!(rules(&lint(&t, &LintContext::default())), ["L1"]);
    }

    #[test]
    fn l2_only_when_retrievability_is_known() {
        let t = terms(|t| t["conditions"][0]["check_ref"] = json!("blossom:abc"));
        assert!(lint(&t, &LintContext::default()).is_empty());
        let cx = LintContext {
            retrievable: Some(BTreeSet::new()),
            ..Default::default()
        };
        assert_eq!(rules(&lint(&t, &cx)), ["L2"]);
        let cx = LintContext {
            retrievable: Some(["blossom:abc".to_string()].into()),
            ..Default::default()
        };
        assert!(lint(&t, &cx).is_empty());
    }

    #[test]
    fn l4_l5_qualities() {
        let t = terms(|t| {
            t["conditions"][0]["description"] = json!("high quality, clean code");
            t["disqualifiers"] = json!([{"id": "d1", "class": "mechanical", "description": "low effort work", "runners": ["bb".repeat(32)]}]);
        });
        let f = lint(&t, &LintContext::default());
        assert_eq!(rules(&f), ["L4", "L5"]);
        assert!(f[0].message.contains("quality\", \"clean"));
        // A human condition may state a quality: the requester decides.
        let t = terms(|t| {
            t["conditions"] =
                json!([{"id": "c1", "class": "human", "description": "clear and readable"}])
        });
        assert!(lint(&t, &LintContext::default()).is_empty());
    }

    #[test]
    fn judged_rules_l6_l7_l8() {
        let cx = LintContext {
            judge_price_threshold: 100,
            ..Default::default()
        };
        let t = terms(|t| {
            t["price"] = json!({"amount": 500});
            t["conditions"]
                .as_array_mut()
                .unwrap()
                .push(judged(&["qwen2.5", "qwen3"], "score it"));
        });
        assert_eq!(rules(&lint(&t, &cx)), ["L6", "L7", "L8"]);
        // Two families, a reference in the rubric, a disqualifier: clean.
        let t = terms(|t| {
            t["price"] = json!({"amount": 500});
            t["conditions"].as_array_mut().unwrap().push(judged(
                &["qwen2.5", "llama3.2"],
                "score it; reference: ...; counterexample: ...",
            ));
            t["disqualifiers"] = json!([{"id": "d1", "class": "mechanical", "description": "addresses the judges", "runners": ["bb".repeat(32)]}]);
        });
        assert!(lint(&t, &cx).is_empty(), "{:?}", lint(&t, &cx));
        // Below the price threshold one family is fine.
        let t = terms(|t| {
            t["conditions"]
                .as_array_mut()
                .unwrap()
                .push(judged(&["qwen2.5"], "reference"))
        });
        assert_eq!(rules(&lint(&t, &cx)), ["L6"]);
    }
}
