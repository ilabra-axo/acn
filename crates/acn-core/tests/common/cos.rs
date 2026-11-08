//! CR-003 conformance vectors: conditions-of-satisfaction authorship (spec §7, §10, INV-14).
//! Each vector is one commitment; the scenario gates run them under every arrival order.

use acn_core::draft::Draft;
use acn_core::kinds::*;
use acn_core::Event;
use serde_json::{json, Value};

use super::judged::{judged_terms, setup};
use super::*;

/// `d1`: a mechanical disqualifier VER may attest.
pub fn d1() -> Value {
    json!({"id": "d1", "class": "mechanical", "description": "modifies or deletes existing tests",
           "window": "until_satisfy", "runners": [pk(&VER)]})
}

/// REQUEST (0.5 terms with `c1` and `d1`, one rework) → PROMISE → REPORT → `c1` passes.
pub struct Disq {
    pub w: W,
    pub r: Event,
    pub rep: Event,
}

pub fn disq(deadline: u64, f: impl FnOnce(&mut Value)) -> Disq {
    let mut w = W::default();
    let t = cos_terms(deadline, |t| {
        t["rework_limit"] = json!(1);
        t["disqualifiers"] = json!([d1()]);
        f(t);
    });
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    let rep = w.add(d_report(&r, &p), &PERF, 1200);
    let mut d = Disq { w, r, rep };
    d.attest("c1", "pass", &VER, 1250);
    d
}

impl Disq {
    pub fn attest(&mut self, id: &str, result: &str, by: &[u8; 32], at: u64) -> Event {
        let d = Draft::attest(
            K_ATTEST_MECH,
            &self.r.id,
            &self.rep.id,
            id,
            json!({"result": result, "method": "ci"}),
        );
        self.w.add(d, by, at)
    }
    pub fn reject(&mut self, gap: &str, at: u64) -> Event {
        let (r, rep) = (self.r.clone(), self.rep.clone());
        self.w.add(d_reject(&r, &rep, gap), &REQ, at)
    }
}

/// C-6: every condition passes, `d1` is attested as hit before SATISFY; the requester rejects
/// naming it. The commitment fails outright (no rework, although one is allowed).
pub fn c6() -> W {
    let mut d = disq(3106, |_| {});
    d.attest("d1", "hit", &VER, 1260);
    d.reject("d1", 1300);
    d.w
}

/// C-6b: verifier trigger: conditions pass but `d1` is hit; the RELEASE is refused, and the
/// requester may REJECT although no condition failed.
pub fn c6b() -> W {
    let mut w = W::default();
    let t = cos_terms(3107, |t| {
        t["disqualifiers"] = json!([d1()]);
        t["price"] = json!({"amount": 100, "unit": "sat"});
        t["payment"] =
            json!({"mint": "https://mint.test", "hashlock": hashlock(), "locktime": 999999});
        t["settlement_trigger"] = json!("verifier");
        t["settlement_verifier"] = json!(pk(&VER));
    });
    let r = request(&mut w, "N", &t);
    let ack = Draft::new(K_ESCROW_ACK)
        .tag(vec!["c".into(), r.id.clone()])
        .content(json!({"hashlock": hashlock()}));
    w.add(ack, &VER, 1050);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    let rep = w.add(d_report(&r, &p), &PERF, 1200);
    let mut d = Disq { w, r, rep };
    d.attest("c1", "pass", &VER, 1250);
    d.attest("d1", "hit", &VER, 1260);
    let [a, b] = parties();
    let rel = Draft::act(K_RELEASE, &d.r.id, &d.rep.id, &[&a, &b])
        .content(json!({"preimage": preimage()}));
    d.w.add(rel, &VER, 1270);
    d.reject("d1", 1300);
    d.w
}

/// C-6c: a hit attested after SATISFY is outside the `until_satisfy` window.
pub fn c6c() -> W {
    let mut d = disq(3108, |_| {});
    let (r, rep) = (d.r.clone(), d.rep.clone());
    d.w.add(d_satisfy(&r, &rep, None), &REQ, 1300);
    d.attest("d1", "hit", &VER, 1400);
    d.w
}

/// C-7: the hit is signed by a key the disqualifier doesn't name; the requester stays silent.
pub fn c7() -> W {
    let mut d = disq(3109, |_| {});
    d.attest("d1", "hit", &STRANGER, 1260);
    d.w
}

/// C-7b: as C-7, but the requester rejects naming `d1` anyway (pretextual disqualification).
pub fn c7b() -> W {
    let mut d = disq(3110, |_| {});
    d.attest("d1", "hit", &STRANGER, 1260);
    d.reject("d1", 1300);
    d.w
}

/// C-8 (resolved): an `until_audit_end` disqualifier needs a holdback, which acn-terms doesn't
/// have; the terms don't parse.
pub fn c8() -> W {
    let mut w = W::default();
    let t = cos_terms(3111, |t| {
        let mut d = d1();
        d["window"] = json!("until_audit_end");
        t["disqualifiers"] = json!([d]);
    });
    request(&mut w, "N", &t);
    w
}

/// `c2`: an open condition (the requester's intent, not yet drafted).
pub fn open_c2() -> Value {
    json!({"id": "c2", "class": "open", "description": "no performance regression",
           "suggested_class": "mechanical"})
}

/// v1 of the C-5 demo: `c1` closed, `c2` open.
pub fn draft_v1(deadline: u64) -> Value {
    cos_terms(deadline, |t| {
        t["conditions"].as_array_mut().unwrap().push(open_c2());
    })
}

/// C-3: PROMISE against a version with an open condition.
pub fn c3() -> W {
    let mut w = W::default();
    let t = draft_v1(3113);
    let r = request(&mut w, "N", &t);
    w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    w
}

/// C-4: PROMISE against a mechanical condition with no evidence producer (no listed verifier).
pub fn c4() -> W {
    let mut w = W::default();
    let t = cos_terms(3114, |t| t["verifiers"] = json!([]));
    let r = request(&mut w, "N", &t);
    w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    w
}

/// C-5: the performer's COUNTER closes the open condition and adds a disqualifier; the
/// requester accepts, and the commitment binds to the performer-drafted version.
pub fn c5() -> W {
    let mut w = W::default();
    let v1 = draft_v1(3115);
    let r = request(&mut w, "N", &v1);
    let v2 = cos_terms(3115, |t| {
        t["conditions"].as_array_mut().unwrap().push(json!({
            "id": "c2", "class": "mechanical",
            "description": "p95 latency within 5% of the baseline benchmark",
            "check_hash": "cd".repeat(32), "check_ref": "https://blossom.example/bench"
        }));
        t["disqualifiers"] = json!([d1()]);
    });
    let [a, b] = parties();
    let ctr = w.add(
        Draft::counter(
            &r.id,
            &r.id,
            &[&a, &b],
            &v2,
            "closed c2 as a benchmark check; added d1",
        )
        .unwrap(),
        &PERF,
        1100,
    );
    w.add(
        Draft::accept(&r.id, &ctr.id, &[&a, &b], &terms_hash(&v2)),
        &REQ,
        1150,
    );
    w
}

/// C-5b: the performer can't ACCEPT a counter that still leaves a condition open.
pub fn c5b() -> W {
    let mut w = W::default();
    let v1 = cos_terms(3116, |_| {});
    let r = request(&mut w, "N", &v1);
    let v2 = draft_v1(3116);
    let [a, b] = parties();
    let ctr = w.add(
        Draft::counter(&r.id, &r.id, &[&a, &b], &v2, "also no regression").unwrap(),
        &REQ,
        1100,
    );
    w.add(
        Draft::accept(&r.id, &ctr.id, &[&a, &b], &terms_hash(&v2)),
        &PERF,
        1150,
    );
    w
}

/// C-5c: a T1 REPORT is an implicit promise, so it needs closed terms too.
pub fn c5c() -> W {
    let mut w = W::default();
    let t = draft_v1(3117);
    let r = request(&mut w, "T1", &t);
    w.add(d_report(&r, &r), &PERF, 1100);
    w
}

/// C-9: older terms use none of the new fields: a mechanical condition with no listed verifier
/// can still be promised, exactly as before CR-003.
pub fn c9() -> W {
    let mut w = W::default();
    let t = terms(|t| {
        t["deadline"] = json!(3118);
        t["conditions"] = json!([{"id": "c1", "class": "mechanical", "description": "tests"}]);
    });
    let r = request(&mut w, "N", &t);
    w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    w
}

/// A judged disqualifier: the judges score whether the act happened; at or above θ is a hit.
pub fn c_judged_hit() -> W {
    let mut dq = judged_terms(|_| {})["conditions"][1].clone();
    dq["id"] = json!("d2");
    dq["description"] = json!("repeats rubric terms without substance");
    dq["window"] = json!("until_satisfy");
    dq["runners"] = json!([pk(&VER)]);
    let mut d = disq(3112, |t| t["disqualifiers"] = json!([d1(), dq]));
    let mut a = super::judged::attestation(super::judged::PASSING, false);
    a["result"] = json!("hit");
    let (r, rep) = (d.r.clone(), d.rep.clone());
    d.w.add(
        Draft::attest(K_ATTEST_JUDGED, &r.id, &rep.id, "d2", a),
        &VER,
        1260,
    );
    d.w
}

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
