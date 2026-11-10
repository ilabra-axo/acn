//! The published example templates (`docs/acn/templates/`, CR-003 §4): each parses as an
//! `acn-template/1`, its rubrics are the files beside it, its judges are the fingerprints this
//! harness produces for `judges.example.toml`, and it instantiates into terms that parse, close
//! and re-instantiate exactly (V-28). Lives here rather than in `acn-core` so the pinned judge
//! fingerprints can't drift from the harness.

use std::collections::BTreeMap;

use acn_core::draft::Draft;
use acn_core::template::{check, TemplateRef, TemplateStatus, TermsTemplate};
use acn_core::terms::Terms;
use serde_json::{json, Value};

const CODE_CHANGE: &str = include_str!("../../../docs/acn/templates/code-change.v1.json");
const RESEARCH_BRIEF: &str = include_str!("../../../docs/acn/templates/research-brief.v1.json");

const RUBRICS: &[&str] = &[
    include_str!("../../../docs/acn/templates/rubrics/code-change-quality.v1.md"),
    include_str!("../../../docs/acn/templates/rubrics/code-change-hardcoding.v1.md"),
    include_str!("../../../docs/acn/templates/rubrics/research-brief-quality.v1.md"),
    include_str!("../../../docs/acn/templates/rubrics/research-brief-citations.v1.md"),
    include_str!("../../../docs/acn/templates/rubrics/rubric-stuffing.v1.md"),
];

fn hex(b: u8) -> String {
    format!("{b:02x}").repeat(32)
}

fn checker() -> String {
    acn_core::draft::public_key(&[0x31; 32]).unwrap()
}

fn panel_fingerprints() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("judges.example.toml");
    let panel = acn_judge::Panel::load(&path).expect("judges.example.toml loads");
    serde_json::to_value(panel.fingerprints()).unwrap()
}

/// Every judged condition and disqualifier: rubric shipped beside the template, judges pinned
/// to this harness and panel.
fn check_judged(name: &str, content: &Value) {
    let fps = panel_fingerprints();
    let rubrics: BTreeMap<String, &str> = RUBRICS
        .iter()
        .map(|r| (acn_core::jcs::sha256_hex(r.as_bytes()), *r))
        .collect();
    let items = content["conditions"]
        .as_array()
        .unwrap()
        .iter()
        .chain(content["disqualifiers"].as_array().unwrap());
    let mut judged = 0;
    for item in items.filter(|i| i["class"] == "judged") {
        judged += 1;
        let id = item["id"].as_str().unwrap();
        let hash = item["rubric_hash"].as_str().unwrap();
        assert!(
            rubrics.contains_key(hash),
            "{name} {id}: rubric_hash names no file in docs/acn/templates/rubrics/"
        );
        assert_eq!(
            item["rubric_ref"],
            json!(format!("blossom:{hash}")),
            "{name} {id}: rubric_ref"
        );
        assert_eq!(
            item["judges"], fps,
            "{name} {id}: judges must be the fingerprints of judges.example.toml under this harness; update the template to:\n{}",
            serde_json::to_string(&fps).unwrap()
        );
    }
    assert!(judged > 0, "{name}: expected judged items");
}

/// Instantiate, wrap in complete terms with a `template_ref`, and check them as a validator
/// would.
fn instantiate(content: &Value, slots: BTreeMap<String, Value>, rework_limit: u32) -> Terms {
    let tpl = TermsTemplate::parse(content).expect("template parses");
    let (conditions, disqualifiers) = tpl.instantiate(&slots).expect("template instantiates");
    // Every item has a note, and the notes fill (a drafter shows them beside the diff).
    let notes = tpl.notes_for(&slots).expect("notes fill");
    for id in conditions.iter().map(|c| &c.id).chain(disqualifiers.iter().map(|d| &d.id)) {
        assert!(notes.get(id).is_some_and(|n| !n.contains("{{")), "note for {id}");
    }
    let event = Draft::template(content).sign(&[0x41; 32], 1_000).unwrap();
    let terms = json!({
        "v": acn_core::terms::TERMS_VERSION,
        "deliverable": {"description": "example"},
        "conditions": conditions,
        "disqualifiers": disqualifiers,
        "deadline": 2_000_000_000u64,
        "review_window": 3_600,
        "rework_limit": rework_limit,
        "price": {"amount": 20_000},
        "verifiers": [{"pubkey": checker(), "classes": ["mechanical"]}],
        "template_ref": {"event": event.id, "hash": acn_core::jcs::hash(content).unwrap(), "slots": slots}
    });
    let t = Terms::check(&terms).unwrap_or_else(|(code, e)| panic!("terms: {code:?}: {e}"));
    assert_eq!(
        t.unclosed(),
        None,
        "every condition and disqualifier is closed"
    );
    let r: &TemplateRef = t.template_ref.as_ref().unwrap();
    assert_eq!(check(&t, r, Some(content)), TemplateStatus::Counted);

    // Published as a K_TERMS_TEMPLATE, it is a valid act (no anomaly).
    let out = acn_core::fold(&[event], &BTreeMap::new());
    assert!(
        out.anomalies.is_empty(),
        "template event: {:?}",
        out.anomalies
    );
    t
}

#[test]
fn code_change_v1() {
    let content: Value = serde_json::from_str(CODE_CHANGE).unwrap();
    check_judged("code-change/v1", &content);
    let slots = BTreeMap::from([
        ("checker".to_string(), json!(checker())),
        ("scope_paths".to_string(), json!("src/relay/")),
        ("suite_check".to_string(), json!(hex(0xa1))),
        ("coverage_check".to_string(), json!(hex(0xa2))),
        ("pool_root".to_string(), json!(hex(0xa3))),
        ("requester_seed_hash".to_string(), json!(hex(0xa4))),
    ]);
    let t = instantiate(&content, slots.clone(), 0);
    assert!(t.has_instances());
    assert_eq!(
        t.disqualifier("d3").unwrap().description,
        "Touches files outside src/relay/ without flagging it in REPORT"
    );
    assert_eq!(
        t.condition("c2").unwrap().extra["instances"]["sample_size"],
        json!(5)
    );

    // Held-out instances: the terms must not allow rework (D-39).
    let tpl = TermsTemplate::parse(&content).unwrap();
    let (c, d) = tpl.instantiate(&slots).unwrap();
    let mut bad = serde_json::to_value(&t).unwrap();
    bad["conditions"] = json!(c);
    bad["disqualifiers"] = json!(d);
    bad["rework_limit"] = json!(1);
    assert!(Terms::check(&bad).is_err());
}

#[test]
fn research_brief_v1() {
    let content: Value = serde_json::from_str(RESEARCH_BRIEF).unwrap();
    check_judged("research-brief/v1", &content);
    let slots = BTreeMap::from([
        ("checker".to_string(), json!(checker())),
        ("structure_check".to_string(), json!(hex(0xb1))),
        ("citations_check".to_string(), json!(hex(0xb2))),
        ("similarity_check".to_string(), json!(hex(0xb3))),
    ]);
    let t = instantiate(&content, slots, 1);
    assert_eq!(
        t.condition("c1").unwrap().description,
        "800-1200 words; sections Summary, Obligations, Timeline, Sources present"
    );
    assert_eq!(
        t.condition("c3").unwrap().description,
        "Accurate, clear for a non-specialist reader, covers obligations and timeline"
    );
}
