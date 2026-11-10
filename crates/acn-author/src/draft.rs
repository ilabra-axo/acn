//! The drafters (CR-003 §3). Deterministic reference implementations: they match templates and
//! requirements by shared words, never invent a key (keys come from the caller's configuration
//! or the terms themselves), and leave what they can't close `open`. A harness can put a model
//! behind the same signatures; the validator judges the output either way.

use std::collections::BTreeMap;

use acn_core::template::{TemplateRef, TermsTemplate};
use acn_core::terms::{Condition, ConditionClass, Disqualifier, Terms, Verifier};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::lint::{lint_value, Finding, LintContext};

/// A template the drafter may use: its content and, if published, its event id (then the
/// drafted terms carry `template_ref`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateCandidate {
    #[serde(default)]
    pub event: Option<String>,
    pub content: Value,
}

/// What the requester wants: a goal and its requirements in plain words, one per entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Intent {
    pub goal: String,
    #[serde(default)]
    pub requirements: Vec<String>,
    #[serde(default)]
    pub price_sat: u64,
    pub deadline: u64,
    pub review_window: u64,
}

/// The requester's configuration: slot values it will put in a template (keys among them), and
/// the verifiers it uses. The drafter never invents a key.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequesterConfig {
    #[serde(default)]
    pub slots: BTreeMap<String, Value>,
    #[serde(default)]
    pub verifiers: Vec<Verifier>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestDraft {
    pub terms: Value,
    /// The template used, by name.
    pub template: Option<String>,
    /// One line per decision (which template, what was left open and why).
    pub notes: Vec<String>,
    /// The linter on the draft (a REQUEST may be open, so no L3).
    pub findings: Vec<Finding>,
}

/// What the performer can be held to: its templates, slot values it fills, and verifiers it
/// proposes adding (e.g. its public CI). Keys a template needs and the caller doesn't give are
/// taken from the request's own verifiers, never invented.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub templates: Vec<TemplateCandidate>,
    #[serde(default)]
    pub slots: BTreeMap<String, Value>,
    #[serde(default)]
    pub verifiers: Vec<Verifier>,
}

/// One change the counter makes, explained in a line (shown beside the diff).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub item: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CounterDraft {
    pub terms: Value,
    pub template: Option<String>,
    pub changes: Vec<Change>,
    /// The changes as the `COUNTER`'s `reason`.
    pub reason: String,
    /// The linter on the counter, as the requester will accept it (L3 on).
    pub findings: Vec<Finding>,
}

const STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "than", "then", "make", "makes",
    "don't", "dont", "not", "any", "all", "its", "it's", "are", "was", "were", "will", "should",
    "must", "have", "has", "been", "being", "does", "doesn't", "what", "when", "which", "while",
    "your", "their", "there", "these", "those", "only", "also", "more", "less", "over", "under",
];

/// Words that name the same checkable concept (so "don't make it slower" meets "p95 latency").
const CONCEPTS: &[(&str, &[&str])] = &[
    (
        "perf",
        &[
            "slow",
            "slower",
            "fast",
            "faster",
            "speed",
            "latency",
            "performance",
            "benchmark",
            "p95",
            "p99",
            "throughput",
        ],
    ),
    ("test", &["test", "tests", "suite", "passes", "pass"]),
    (
        "cite",
        &[
            "cite",
            "cited",
            "citation",
            "citations",
            "source",
            "sources",
        ],
    ),
    (
        "size",
        &["words", "word", "length", "long", "short", "pages"],
    ),
];

/// Content words (≥ 4 letters or a concept word, stemmed crudely by trimming a plural `s`),
/// with concept synonyms folded together.
fn content_words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty() && !STOP.contains(w))
        .filter_map(|w| {
            if let Some((c, _)) = CONCEPTS.iter().find(|(_, ws)| ws.contains(&w)) {
                return Some(format!("#{c}"));
            }
            (w.len() >= 4).then(|| w.strip_suffix('s').unwrap_or(w).to_string())
        })
        .collect()
}

fn overlap(a: &str, b: &str) -> usize {
    let b = content_words(b);
    let mut seen: Vec<String> = Vec::new();
    for w in content_words(a) {
        if b.contains(&w) && !seen.contains(&w) {
            seen.push(w);
        }
    }
    seen.len()
}

/// A requirement's likely class, for `suggested_class` on an open condition.
fn suggest(req: &str) -> ConditionClass {
    const MECH: &[&str] = &[
        "slow",
        "slower",
        "fast",
        "faster",
        "latency",
        "performance",
        "benchmark",
        "test",
        "tests",
        "pass",
        "passes",
        "size",
        "words",
        "word",
        "count",
        "seconds",
        "memory",
        "coverage",
        "compile",
        "compiles",
        "build",
        "builds",
        "lint",
        "format",
        "schema",
        "valid",
    ];
    const GROUND: &[&str] = &[
        "cite",
        "cited",
        "citation",
        "citations",
        "source",
        "sources",
        "data",
        "fact",
        "facts",
    ];
    const OPINION: &[&str] = &[
        "clear",
        "readable",
        "accurate",
        "quality",
        "good",
        "well",
        "concise",
        "useful",
        "idiomatic",
    ];
    let w: Vec<String> = req
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_owned)
        .collect();
    let has = |set: &[&str]| w.iter().any(|x| set.contains(&x.as_str()));
    if has(MECH) {
        ConditionClass::Mechanical
    } else if has(GROUND) {
        ConditionClass::Grounding
    } else if has(OPINION) {
        ConditionClass::Judged
    } else {
        ConditionClass::Human
    }
}

fn class_name(c: ConditionClass) -> String {
    serde_json::to_value(c)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn template_text(t: &TermsTemplate) -> String {
    let items: Vec<String> = t
        .conditions
        .iter()
        .chain(&t.disqualifiers)
        .filter_map(|i| i["description"].as_str().map(str::to_owned))
        .collect();
    format!(
        "{} {} {}",
        t.name.replace(['-', '/'], " "),
        t.description,
        items.join(" ")
    )
}

fn has_instances(conditions: &[Condition], disqualifiers: &[Disqualifier]) -> bool {
    conditions
        .iter()
        .cloned()
        .chain(disqualifiers.iter().map(Disqualifier::as_condition))
        .any(|c| acn_core::terms::instances_of(&c).is_some())
}

fn next_id(prefix: &str, taken: &[String]) -> String {
    (1..)
        .map(|n| format!("{prefix}{n}"))
        .find(|id| !taken.contains(id))
        .unwrap_or_default()
}

fn reference(c: &TemplateCandidate, slots: &BTreeMap<String, Value>) -> Option<TemplateRef> {
    Some(TemplateRef {
        event: c.event.clone()?,
        hash: acn_core::jcs::hash(&c.content).ok()?,
        slots: slots.clone(),
    })
}

/// The requester drafter: a template where one fits (its slots filled from the configuration),
/// and every requirement it doesn't cover as an `open` condition with a `suggested_class`.
pub fn draft_request(
    intent: &Intent,
    templates: &[TemplateCandidate],
    cfg: &RequesterConfig,
) -> RequestDraft {
    let mut notes = Vec::new();
    let want = format!("{} {}", intent.goal, intent.requirements.join(" "));
    // Best fillable template by shared words (at least two).
    let best = templates
        .iter()
        .filter_map(|c| {
            let t = TermsTemplate::parse(&c.content).ok()?;
            let slots: BTreeMap<String, Value> = cfg
                .slots
                .iter()
                .filter(|(k, _)| t.slots.contains_key(*k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let filled = t.instantiate(&slots).ok()?;
            let score = overlap(&want, &template_text(&t));
            (score >= 2).then_some((score, c, t, slots, filled))
        })
        .max_by_key(|(score, ..)| *score);

    let (mut conditions, disqualifiers, mut template_ref, name) = match best {
        Some((_, c, t, slots, (conds, disq))) => {
            notes.push(format!(
                "Matched template {}; slots filled from your configuration.",
                t.name
            ));
            (conds, disq, reference(c, &slots), Some(t.name))
        }
        None => {
            if !templates.is_empty() {
                notes.push(
                    "No template fits (or none can be filled from your configuration).".into(),
                );
            }
            (vec![], vec![], None, None)
        }
    };
    let mut opened = false;
    for req in &intent.requirements {
        let covered = conditions
            .iter()
            .map(|c| (c.id.clone(), c.description.clone()))
            .chain(
                disqualifiers
                    .iter()
                    .map(|d| (d.id.clone(), d.description.clone())),
            )
            .find(|(_, d)| overlap(req, d) >= 2);
        if let Some((id, _)) = covered {
            notes.push(format!("\"{req}\": covered by {id}."));
            continue;
        }
        let taken: Vec<String> = conditions
            .iter()
            .map(|c| c.id.clone())
            .chain(disqualifiers.iter().map(|d| d.id.clone()))
            .collect();
        let id = next_id("c", &taken);
        let class = suggest(req);
        conditions.push(Condition {
            id: id.clone(),
            class: ConditionClass::Open,
            description: req.clone(),
            extra: json!({"suggested_class": class_name(class)})
                .as_object()
                .cloned()
                .unwrap_or_default(),
        });
        notes.push(format!("{id} left open (suggested: {}): the performer's counter closes it, and nobody can promise until it does.", class_name(class)));
        opened = true;
    }
    if opened && template_ref.take().is_some() {
        notes.push("The template reference is left out: the open conditions change the template's conditions, so it wouldn't re-instantiate (V-28).".into());
    }
    if conditions.is_empty() {
        conditions.push(Condition {
            id: "c1".into(),
            class: ConditionClass::Human,
            description: format!("The result does what I asked: {}", intent.goal),
            extra: Default::default(),
        });
        notes
            .push("c1: no checkable requirement, so you judge the result yourself (human).".into());
    }
    let instances = has_instances(&conditions, &disqualifiers);
    let mut terms = json!({
        "v": acn_core::terms::TERMS_VERSION,
        "deliverable": {"description": intent.goal},
        "conditions": conditions,
        "deadline": intent.deadline,
        "review_window": intent.review_window,
        "rework_limit": if instances { 0 } else { 1 },
        "price": {"amount": intent.price_sat, "unit": "sat"},
        "verifiers": cfg.verifiers,
    });
    if !disqualifiers.is_empty() {
        terms["disqualifiers"] = json!(disqualifiers);
    }
    if let Some(r) = template_ref {
        terms["template_ref"] = json!(r);
    }
    let findings = lint_value(&terms, &LintContext::default());
    RequestDraft {
        terms,
        template: name,
        notes,
        findings,
    }
}

/// How one template would close a request's open conditions.
struct Fit<'a> {
    candidate: &'a TemplateCandidate,
    template: TermsTemplate,
    slots: BTreeMap<String, Value>,
    conditions: Vec<Condition>,
    disqualifiers: Vec<Disqualifier>,
    /// open id → template condition index
    closes: BTreeMap<String, usize>,
    /// every requester condition is either equal to a template one or closed by one
    exact: bool,
}

fn fit<'a>(terms: &Terms, c: &'a TemplateCandidate, caps: &Capabilities) -> Option<Fit<'a>> {
    let t = TermsTemplate::parse(&c.content).ok()?;
    let mut slots: BTreeMap<String, Value> = caps
        .slots
        .iter()
        .filter(|(k, _)| t.slots.contains_key(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // Keys the template needs: the request's own mechanical verifier, never an invented one.
    let named = terms
        .verifiers
        .iter()
        .chain(&caps.verifiers)
        .find(|v| v.classes.contains(&ConditionClass::Mechanical))
        .map(|v| v.pubkey.clone());
    for (k, s) in &t.slots {
        if !slots.contains_key(k)
            && s.default.is_none()
            && s.ty == acn_core::template::SlotType::Pubkey
        {
            slots.insert(k.clone(), json!(named.clone()?));
        }
    }
    let (conditions, disqualifiers) = t.instantiate(&slots).ok()?;
    let mut closes = BTreeMap::new();
    let mut exact = true;
    for rc in &terms.conditions {
        if rc.class == ConditionClass::Open {
            let suggested = rc
                .extra
                .get("suggested_class")
                .and_then(|v| serde_json::from_value::<ConditionClass>(v.clone()).ok());
            // The closer shares the most concepts with the open condition (at least one), has
            // the suggested class if there is one, and wins ties by having the same id.
            let i = conditions
                .iter()
                .enumerate()
                .filter(|(_, x)| suggested.is_none_or(|s| s == x.class))
                .map(|(i, x)| (overlap(&rc.description, &x.description), x.id == rc.id, i))
                .filter(|(n, _, _)| *n >= 1)
                .max_by_key(|(n, same, _)| (*n, *same))
                .map(|(_, _, i)| i)?;
            closes.insert(rc.id.clone(), i);
        } else if !conditions.contains(rc) {
            exact = false;
        }
    }
    // Every template condition must be one of the request's (equal or closing an open one).
    let used = conditions
        .iter()
        .enumerate()
        .all(|(i, x)| closes.values().any(|j| *j == i) || terms.conditions.contains(x));
    Some(Fit {
        candidate: c,
        template: t,
        slots,
        conditions,
        disqualifiers,
        closes,
        exact: exact && used,
    })
}

/// The performer drafter: close every open condition from one of the performer's templates,
/// add the template's disqualifiers, and explain each change in a line. `None` when there is
/// nothing to close or no template closes it all (then the performer declines or writes its
/// own counter).
pub fn draft_counter(terms: &Terms, caps: &Capabilities) -> Option<CounterDraft> {
    let open: Vec<&Condition> = terms
        .conditions
        .iter()
        .filter(|c| c.class == ConditionClass::Open)
        .collect();
    if open.is_empty() {
        return None;
    }
    let f = caps
        .templates
        .iter()
        .filter_map(|c| fit(terms, c, caps))
        .max_by_key(|f| (f.exact, f.closes.len()))?;
    let notes = f.template.notes_for(&f.slots).unwrap_or_default();
    let line = |id: &str, fallback: String| notes.get(id).cloned().unwrap_or(fallback);
    let mut changes = Vec::new();

    let (conditions, template_ref) = if f.exact {
        // The template's conditions as they are: the reference re-instantiates (V-28).
        (f.conditions.clone(), reference(f.candidate, &f.slots))
    } else {
        // Keep the request's conditions; replace each open one by its closer, under the
        // request's id so the diff lines up. No reference: this isn't the template.
        let cs = terms
            .conditions
            .iter()
            .map(|rc| match f.closes.get(&rc.id) {
                Some(i) => Condition {
                    id: rc.id.clone(),
                    ..f.conditions[*i].clone()
                },
                None => rc.clone(),
            })
            .collect();
        (cs, None)
    };
    for o in &open {
        let i = f.closes[&o.id];
        let closer = &f.conditions[i];
        changes.push(Change {
            item: o.id.clone(),
            line: line(
                &closer.id,
                format!(
                    "closed as {}: {}",
                    class_name(closer.class),
                    closer.description
                ),
            ),
        });
    }
    let mut disqualifiers = terms.disqualifiers.clone();
    for d in &f.disqualifiers {
        if disqualifiers.iter().any(|x| x == d) {
            continue;
        }
        let mut d = d.clone();
        let taken: Vec<String> = conditions
            .iter()
            .map(|c| c.id.clone())
            .chain(disqualifiers.iter().map(|x| x.id.clone()))
            .collect();
        let original = d.id.clone();
        if taken.contains(&d.id) {
            d.id = next_id("d", &taken);
        }
        changes.push(Change {
            item: d.id.clone(),
            line: line(
                &original,
                format!("added: {} (a hit fails the commitment)", d.description),
            ),
        });
        disqualifiers.push(d);
    }
    let template_ref = template_ref.filter(|_| disqualifiers == f.disqualifiers);

    let mut t = serde_json::to_value(terms).ok()?;
    t["conditions"] = json!(conditions);
    if disqualifiers.is_empty() {
        t.as_object_mut()?.remove("disqualifiers");
    } else {
        t["disqualifiers"] = json!(disqualifiers);
    }
    match template_ref {
        Some(r) => t["template_ref"] = json!(r),
        None => {
            t.as_object_mut()?.remove("template_ref");
        }
    }
    let mut verifiers = terms.verifiers.clone();
    for v in &caps.verifiers {
        if !verifiers.contains(v) {
            changes.push(Change {
                item: "verifiers".into(),
                line: format!(
                    "added {} as a verifier for {}",
                    &v.pubkey[..8.min(v.pubkey.len())],
                    v.classes
                        .iter()
                        .map(|c| class_name(*c))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
            verifiers.push(v.clone());
        }
    }
    t["verifiers"] = json!(verifiers);
    if has_instances(&conditions, &disqualifiers) {
        t["rework_limit"] = json!(0);
    }
    let reason = changes
        .iter()
        .map(|c| format!("{}: {}", c.item, c.line))
        .collect::<Vec<_>>()
        .join("; ");
    let findings = lint_value(
        &t,
        &LintContext {
            binding: true,
            ..Default::default()
        },
    );
    Some(CounterDraft {
        terms: t,
        template: Some(f.template.name.clone()),
        changes,
        reason,
        findings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> String {
        format!("{b:02x}").repeat(32)
    }

    fn retry_template() -> Value {
        json!({
            "v": "acn-template/1", "name": "retry-change/v1",
            "description": "A retry or backoff change to a network client",
            "slots": {"checker": {"type": "pubkey"}, "max_regression": {"type": "number", "default": 5}},
            "conditions": [
                {"id": "c1", "class": "mechanical", "description": "The existing test suite passes"},
                {"id": "c2", "class": "mechanical", "description": "p95 latency within {{max_regression}}% of the baseline (benchmark suite)"}
            ],
            "disqualifiers": [
                {"id": "d1", "class": "mechanical", "description": "Disables or skips the benchmark", "runners": ["{{checker}}"]}
            ],
            "notes": {"c2": "closed as a benchmark check (p95 within {{max_regression}}%)", "d1": "so the check can't be dodged"}
        })
    }

    fn request(conditions: Value) -> Terms {
        Terms::parse(&json!({
            "v": acn_core::terms::TERMS_VERSION,
            "deliverable": {"description": "Retry with backoff in the relay client"},
            "conditions": conditions,
            "deadline": 2_000_000_000u64, "review_window": 3600, "rework_limit": 1,
            "price": {"amount": 0},
            "verifiers": [{"pubkey": key(0xaa), "classes": ["mechanical"]}]
        }))
        .unwrap()
    }

    #[test]
    fn counter_closes_from_a_template_and_references_it() {
        let t = request(json!([
            {"id": "c1", "class": "mechanical", "description": "The existing test suite passes"},
            {"id": "c2", "class": "open", "description": "Don't make it slower", "suggested_class": "mechanical"}
        ]));
        let caps = Capabilities {
            templates: vec![TemplateCandidate {
                event: Some(key(0x11)),
                content: retry_template(),
            }],
            ..Default::default()
        };
        let d = draft_counter(&t, &caps).unwrap();
        assert_eq!(
            d.reason,
            "c2: closed as a benchmark check (p95 within 5%); d1: so the check can't be dodged"
        );
        assert!(d.findings.is_empty(), "{:?}", d.findings);
        let terms = Terms::parse(&d.terms).unwrap();
        assert_eq!(terms.unclosed(), None);
        // The checker is the request's own verifier, and the reference re-instantiates.
        assert_eq!(terms.disqualifiers[0].runners(), vec![key(0xaa)]);
        let r = terms.template_ref.as_ref().unwrap();
        assert_eq!(
            acn_core::template::check(&terms, r, Some(&retry_template())),
            acn_core::template::TemplateStatus::Counted
        );
    }

    #[test]
    fn counter_without_an_exact_fit_keeps_the_request_and_drops_the_reference() {
        // The requester's c1 differs from the template's, so this isn't the template.
        let t = request(json!([
            {"id": "c1", "class": "mechanical", "description": "cargo test passes on Linux"},
            {"id": "c9", "class": "open", "description": "no slower: p95 latency against the baseline", "suggested_class": "mechanical"}
        ]));
        let caps = Capabilities {
            templates: vec![TemplateCandidate {
                event: Some(key(0x11)),
                content: retry_template(),
            }],
            ..Default::default()
        };
        let d = draft_counter(&t, &caps).unwrap();
        let terms = Terms::parse(&d.terms).unwrap();
        assert!(terms.template_ref.is_none());
        assert_eq!(
            terms.conditions[0].description,
            "cargo test passes on Linux"
        );
        assert_eq!(terms.conditions[1].id, "c9");
        assert!(terms.conditions[1].description.starts_with("p95 latency"));
        assert_eq!(terms.unclosed(), None);
    }

    #[test]
    fn counter_declines_what_it_cannot_close_and_never_invents_keys() {
        let open =
            json!([{"id": "c1", "class": "open", "description": "Write a sonnet about relays"}]);
        let caps = Capabilities {
            templates: vec![TemplateCandidate {
                event: None,
                content: retry_template(),
            }],
            ..Default::default()
        };
        assert!(draft_counter(&request(open), &caps).is_none());
        // No verifier anywhere: the checker slot can't be filled, so no draft.
        let mut t = request(
            json!([{"id": "c2", "class": "open", "description": "Don't make it slower: p95 latency", "suggested_class": "mechanical"}]),
        );
        t.verifiers.clear();
        assert!(draft_counter(&t, &caps).is_none());
    }

    #[test]
    fn request_matches_a_template_and_leaves_the_rest_open() {
        let intent = Intent {
            goal: "Retry with backoff in the relay client".into(),
            requirements: vec![
                "The existing test suite passes".into(),
                "Explain the change in the README".into(),
            ],
            price_sat: 100,
            deadline: 2_000_000_000,
            review_window: 3600,
        };
        let cfg = RequesterConfig {
            slots: BTreeMap::from([("checker".into(), json!(key(0xaa)))]),
            verifiers: vec![Verifier {
                pubkey: key(0xaa),
                classes: vec![ConditionClass::Mechanical],
            }],
        };
        let d = draft_request(
            &intent,
            &[TemplateCandidate {
                event: Some(key(0x11)),
                content: retry_template(),
            }],
            &cfg,
        );
        assert_eq!(d.template.as_deref(), Some("retry-change/v1"));
        let t = Terms::parse(&d.terms).unwrap();
        let open: Vec<&Condition> = t
            .conditions
            .iter()
            .filter(|c| c.class == ConditionClass::Open)
            .collect();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].description, "Explain the change in the README");
        assert_eq!(open[0].extra["suggested_class"], json!("human"));
        assert!(
            t.template_ref.is_none(),
            "open conditions change the template's conditions"
        );
        assert!(
            d.notes.iter().any(|n| n.contains("covered by c1")),
            "{:?}",
            d.notes
        );
    }

    #[test]
    fn request_without_templates() {
        let intent = Intent {
            goal: "A logo".into(),
            requirements: vec!["Loads in under 2 seconds".into()],
            deadline: 2_000_000_000,
            review_window: 3600,
            ..Default::default()
        };
        let d = draft_request(&intent, &[], &RequesterConfig::default());
        let t = Terms::parse(&d.terms).unwrap();
        assert_eq!(t.conditions[0].class, ConditionClass::Open);
        assert_eq!(
            t.conditions[0].extra["suggested_class"],
            json!("mechanical")
        );
        let d = draft_request(
            &Intent {
                requirements: vec![],
                ..intent
            },
            &[],
            &RequesterConfig::default(),
        );
        assert_eq!(
            Terms::parse(&d.terms).unwrap().conditions[0].class,
            ConditionClass::Human
        );
    }
}
