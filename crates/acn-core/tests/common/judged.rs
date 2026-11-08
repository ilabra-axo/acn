//! CR-001 conformance vectors for the `judged` condition class (spec §7, §9, V-11, V-17).
//! Each vector is one commitment; the scenario gates run them under every arrival order.

use acn_core::draft::Draft;
use acn_core::kinds::*;
use acn_core::Event;
use serde_json::{json, Value};

use super::*;

pub const RUBRIC: &str = "Score 0-10: accuracy, coverage of the source, clarity. 500 words max.";

pub fn rubric_hash() -> String {
    acn_core::jcs::sha256_hex(RUBRIC.as_bytes())
}

pub fn judge(model: &str, version: &str) -> Value {
    json!({
        "model_id": model,
        "model_version": version,
        "prompt_template_hash": "11".repeat(32),
        "harness_hash": "22".repeat(32),
        "sampling": {"temperature": 0, "max_tokens": 512, "seed": 7}
    })
}

/// Terms of the CR-001 demo: `c1` mechanical, `c2` judged (two judges from different families,
/// commit-reveal rubric, 0-10, θ = 7, all, 3 repeats, runner = the requester).
pub fn judged_terms(f: impl FnOnce(&mut Value)) -> Value {
    terms(|t| {
        t["deliverable"] = json!({"description": "500-word technical summary"});
        t["conditions"] = json!([
            {"id": "c1", "class": "mechanical", "description": "plain text, at most 500 words"},
            {"id": "c2", "class": "judged", "description": "two judges rate it at least 7/10",
             "judges": [judge("claude", "2026-09-01"), judge("gpt", "2026-08-15")],
             "rubric_hash": rubric_hash(), "rubric_disclosure": "commit_reveal",
             "scale": {"min": 0, "max": 10}, "threshold": 7, "aggregation": "all",
             "repeats": 3, "runners": [pk(&REQ)],
             "audit": null, "holdback": null}
        ]);
        t["verifiers"] = json!([{"pubkey": pk(&VER), "classes": ["mechanical"]}]);
        f(t);
    })
}

fn median(runs: &[f64]) -> f64 {
    let mut v = runs.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// A well-formed `K_ATTEST_JUDGED` with consistent medians, aggregate (`all`) and result.
pub fn attestation(scores: &[(&str, &str, &[f64])], injection: bool) -> Value {
    let judges: Vec<Value> = scores
        .iter()
        .map(|(m, v, runs)| {
            let mut j = judge(m, v);
            j["runs"] = json!(runs);
            j["median"] = json!(median(runs));
            j["rationale_hash"] = json!("33".repeat(32));
            j
        })
        .collect();
    let agg = scores
        .iter()
        .map(|(_, _, r)| median(r))
        .fold(f64::INFINITY, f64::min);
    let pass = agg >= 7.0 && !injection;
    let mut c = json!({
        "judges": judges,
        "aggregate_score": agg,
        "result": if pass { "pass" } else { "fail" },
        "rubric_text_hash": rubric_hash(),
        "rubric_ref": "https://blossom.example/".to_string() + &rubric_hash(),
        "rubric_text": RUBRIC,
        "injection_detected": injection
    });
    if injection {
        c["injection_span_hash"] = json!("44".repeat(32));
    }
    c
}

pub const PASSING: &[(&str, &str, &[f64])] = &[
    ("claude", "2026-09-01", &[8.0, 7.0, 8.0]),
    ("gpt", "2026-08-15", &[7.0, 9.0, 8.0]),
];

/// REQUEST → PROMISE → REPORT, with a distinct deadline so vectors don't share a commitment.
pub struct Setup {
    pub w: W,
    pub r: Event,
    pub rep: Event,
}

pub fn setup(deadline: u64, f: impl FnOnce(&mut Value)) -> Setup {
    let mut w = W::default();
    let t = judged_terms(|t| {
        t["deadline"] = json!(deadline);
        f(t);
    });
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    let rep = w.add(d_report(&r, &p), &PERF, 1200);
    Setup { w, r, rep }
}

impl Setup {
    pub fn judged(&mut self, content: Value, by: &[u8; 32], at: u64) -> Event {
        self.w.add(
            Draft::attest(K_ATTEST_JUDGED, &self.r.id, &self.rep.id, "c2", content),
            by,
            at,
        )
    }
}

/// J-1: both judges pass, runner is the requester; the requester's binding auto-SATISFYs.
pub fn j1() -> W {
    let mut s = setup(2101, |_| {});
    s.w.add(
        Draft::attest(
            K_ATTEST_MECH,
            &s.r.id,
            &s.rep.id,
            "c1",
            json!({"result": "pass", "method": "wc -w"}),
        ),
        &VER,
        1250,
    );
    s.judged(attestation(PASSING, false), &REQ, 1260);
    let rep = s.rep.clone();
    s.w.add(d_satisfy(&s.r, &rep, None), &REQ, 1270);
    s.w
}

/// J-2: an interpretive report offered for the judged condition.
pub fn j2() -> W {
    let mut s = setup(2102, |_| {});
    let (r, rep) = (s.r.clone(), s.rep.clone());
    s.w.add(
        Draft::attest(
            K_INTERP_REPORT,
            &r.id,
            &rep.id,
            "c2",
            json!({"judge": "llm", "verdict": "9/10"}),
        ),
        &REQ,
        1260,
    );
    s.w
}

/// J-3: the performer signs the judged attestation.
pub fn j3() -> W {
    let mut s = setup(2103, |_| {});
    s.judged(attestation(PASSING, false), &PERF, 1260);
    s.w
}

/// J-3b: terms that list the performer as a runner are flagged on the REQUEST.
pub fn j3b() -> W {
    setup(2113, |t| {
        t["conditions"][1]["runners"] = json!([pk(&REQ), pk(&PERF)])
    })
    .w
}

/// J-4: one judge's model_version differs from the terms.
pub fn j4() -> W {
    let mut s = setup(2104, |_| {});
    s.judged(
        attestation(
            &[PASSING[0], ("gpt", "2026-10-01", &[7.0, 9.0, 8.0])],
            false,
        ),
        &REQ,
        1260,
    );
    s.w
}

/// J-5: commit-reveal; the revealed rubric hash differs.
pub fn j5() -> W {
    let mut s = setup(2105, |_| {});
    let mut a = attestation(PASSING, false);
    a["rubric_text_hash"] = json!("55".repeat(32));
    a["rubric_text"] = json!("a different, easier rubric");
    s.judged(a, &REQ, 1260);
    s.w
}

/// J-6: injection detected.
pub fn j6() -> W {
    let mut s = setup(2106, |_| {});
    s.judged(attestation(PASSING, true), &REQ, 1260);
    s.w
}

/// J-7: aggregation `all`, one judge below θ.
pub fn j7() -> W {
    let mut s = setup(2107, |_| {});
    s.judged(
        attestation(
            &[PASSING[0], ("gpt", "2026-08-15", &[6.0, 6.0, 8.0])],
            false,
        ),
        &REQ,
        1260,
    );
    s.w
}

/// J-8: repeats 3, the attestation carries 2 runs per judge.
pub fn j8() -> W {
    let mut s = setup(2108, |_| {});
    s.judged(
        attestation(
            &[
                ("claude", "2026-09-01", &[8.0, 8.0]),
                ("gpt", "2026-08-15", &[8.0, 9.0]),
            ],
            false,
        ),
        &REQ,
        1260,
    );
    s.w
}

/// J-11: one mechanical and one judged condition (no evidence needed).
pub fn j11() -> W {
    setup(2111, |_| {}).w
}

/// Extra: declared aggregate/result disagree with the recomputation; an unlisted runner.
pub fn j_declared_and_unlisted() -> W {
    let mut s = setup(2112, |_| {});
    let mut a = attestation(
        &[PASSING[0], ("gpt", "2026-08-15", &[6.0, 6.0, 8.0])],
        false,
    );
    a["result"] = json!("pass");
    s.judged(a, &REQ, 1260);
    s.judged(attestation(PASSING, false), &STRANGER, 1261);
    s.w
}
