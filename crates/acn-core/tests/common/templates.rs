//! CR-003 Phase C conformance vectors: terms templates (spec §7 "Terms templates", V-28).

use std::collections::BTreeMap;

use acn_core::draft::Draft;
use acn_core::Event;
use serde_json::{json, Value};

use super::cos::cos_terms;
use super::*;

pub const AUTHOR: [u8; 32] = [8; 32];

pub fn template() -> Value {
    json!({
        "v": "acn-template/1", "name": "code-change/v1",
        "description": "A change to an existing code base, checked by the suite and a scope rule",
        "slots": {
            "checker": {"type": "pubkey", "guidance": "Who runs the checks"},
            "scope": {"type": "string", "guidance": "Paths the change may touch"}
        },
        "conditions": [{"id": "c1", "class": "mechanical", "description": "The existing test suite passes"}],
        "disqualifiers": [{"id": "d1", "class": "mechanical",
            "description": "Touches files outside {{scope}}", "runners": ["{{checker}}"]}]
    })
}

pub fn publish(w: &mut W, content: &Value) -> Event {
    w.add(Draft::template(content), &AUTHOR, 900)
}

pub fn slots() -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("checker".to_string(), json!(pk(&VER))),
        ("scope".to_string(), json!("src/relay/")),
    ])
}

/// Terms drafted from the template, carrying `template_ref`; `f` may tamper.
pub fn from_template(deadline: u64, tpl: &Event, f: impl FnOnce(&mut Value)) -> Value {
    cos_terms(deadline, |t| {
        t["conditions"][0]["description"] = json!("The existing test suite passes");
        t["disqualifiers"] = json!([{"id": "d1", "class": "mechanical",
            "description": "Touches files outside src/relay/", "runners": [pk(&VER)]}]);
        t["template_ref"] = json!({
            "event": tpl.id,
            "hash": acn_core::jcs::hash(&serde_json::from_str::<Value>(&tpl.content).unwrap()).unwrap(),
            "slots": slots()
        });
        f(t);
    })
}

/// REQUEST from the template, then PROMISE.
fn run(deadline: u64, publish_it: bool, f: impl FnOnce(&mut Value)) -> W {
    let mut w = W::default();
    let tpl_content = template();
    let tpl = Draft::template(&tpl_content).sign(&AUTHOR, 900).unwrap();
    if publish_it {
        w.push(tpl.clone());
    }
    let t = from_template(deadline, &tpl, f);
    let r = request(&mut w, "N", &t);
    w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    w
}

/// C-15: the reference re-instantiates exactly; counted.
pub fn c15() -> W {
    run(3301, true, |_| {})
}

/// C-16: the terms' disqualifier differs from what the slots produce (a weaker scope).
pub fn c16() -> W {
    run(3302, true, |t| {
        t["disqualifiers"][0]["description"] = json!("Touches files outside src/");
    })
}

/// C-16b: the slot values themselves are of the wrong type.
pub fn c16b() -> W {
    run(3303, true, |t| {
        t["template_ref"]["slots"]["checker"] = json!("someone")
    })
}

/// C-16c: `template_ref.hash` doesn't match the template's content.
pub fn c16c() -> W {
    run(3304, true, |t| {
        t["template_ref"]["hash"] = json!("00".repeat(32))
    })
}

/// C-16d: the template event isn't in the event set: not counted, nothing flagged.
pub fn c16d() -> W {
    run(3305, false, |_| {})
}

/// C-16e: a malformed template event (placeholder naming no slot).
pub fn c16e() -> W {
    let mut w = W::default();
    let mut bad = template();
    bad["conditions"][0]["description"] = json!("passes {{nothing}}");
    publish(&mut w, &bad);
    w
}
