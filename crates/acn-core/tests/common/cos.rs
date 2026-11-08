//! CR-003 conformance vectors: conditions-of-satisfaction authorship (spec §7, §10, INV-14).
//! Each vector is one commitment; the scenario gates run them under every arrival order.

use serde_json::{json, Value};

use super::judged::{judged_terms, setup};
use super::*;

/// Base `acn-terms/0.5` terms with one mechanical condition checked by VER.
pub fn cos_terms(deadline: u64, f: impl FnOnce(&mut Value)) -> Value {
    terms(|t| {
        t["v"] = json!("acn-terms/0.5");
        t["deadline"] = json!(deadline);
        t["conditions"] =
            json!([{"id": "c1", "class": "mechanical", "description": "the test suite passes"}]);
        t["verifiers"] = json!([{"pubkey": pk(&VER), "classes": ["mechanical"]}]);
        f(t);
    })
}

/// C-1: the terms declare `commit_reveal`.
pub fn c1() -> W {
    setup(3101, |t| {
        t["conditions"][1]["rubric_disclosure"] = json!("commit_reveal");
    })
    .w
}

/// C-2: a judged condition without `rubric_ref`.
pub fn c2() -> W {
    setup(3102, |t| {
        t["conditions"][1]
            .as_object_mut()
            .unwrap()
            .remove("rubric_ref");
    })
    .w
}

/// C-2b: a mechanical check named by `check_hash` with no retrievable `check_ref`.
pub fn c2b() -> W {
    let mut w = W::default();
    let t = cos_terms(3103, |t| {
        t["conditions"][0]["check_hash"] = json!("ab".repeat(32));
    });
    request(&mut w, "N", &t);
    w
}

/// Inline `rubric_text` that is not the committed rubric (malformed, not hidden).
pub fn c2c() -> W {
    let mut w = W::default();
    let t = judged_terms(|t| {
        t["deadline"] = json!(3104);
        t["conditions"][1]["rubric_text"] = json!("an easier rubric");
    });
    request(&mut w, "N", &t);
    w
}
