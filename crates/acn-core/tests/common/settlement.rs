//! CR-002 Phase A conformance vectors: the verifier settlement trigger (spec §7, §8.2, §10).
//! Each vector is one commitment; the scenario gates run them under every arrival order.

use acn_core::draft::Draft;
use acn_core::kinds::*;
use acn_core::Event;
use serde_json::{json, Value};

use super::*;

/// Verifier-trigger terms: one mechanical condition, VER is both the listed verifier and the
/// settlement verifier holding the escrowed secret.
pub fn verifier_terms(deadline: u64, f: impl FnOnce(&mut Value)) -> Value {
    terms(|t| {
        t["deadline"] = json!(deadline);
        t["rework_limit"] = json!(1);
        t["conditions"] =
            json!([{"id": "tests", "class": "mechanical", "description": "tests pass"}]);
        t["verifiers"] = json!([{"pubkey": pk(&VER), "classes": ["mechanical"]}]);
        t["price"] = json!({"amount": 100, "unit": "sat"});
        t["payment"] =
            json!({"mint": "https://mint.test", "hashlock": hashlock(), "locktime": 999999});
        t["settlement_trigger"] = json!("verifier");
        t["settlement_verifier"] = json!(pk(&VER));
        f(t);
    })
}

pub struct V {
    pub w: W,
    pub r: Event,
    pub t: Value,
}

pub fn setup(deadline: u64, f: impl FnOnce(&mut Value)) -> V {
    let mut w = W::default();
    let t = verifier_terms(deadline, f);
    let r = request(&mut w, "N", &t);
    V { w, r, t }
}

impl V {
    pub fn ack(&mut self, by: &[u8; 32], at: u64) -> Event {
        let d = Draft::new(K_ESCROW_ACK)
            .tag(vec!["c".into(), self.r.id.clone()])
            .content(json!({"hashlock": hashlock()}));
        self.w.add(d, by, at)
    }
    pub fn promise(&mut self, at: u64) -> Event {
        let h = terms_hash(&self.t);
        let r = self.r.clone();
        self.w.add(d_promise(&r, &r, &h), &PERF, at)
    }
    pub fn report(&mut self, prev: &Event, at: u64) -> Event {
        let r = self.r.clone();
        self.w.add(d_report(&r, prev), &PERF, at)
    }
    pub fn attest(&mut self, rep: &Event, result: &str, at: u64) -> Event {
        let d = Draft::attest(
            K_ATTEST_MECH,
            &self.r.id,
            &rep.id,
            "tests",
            json!({"result": result, "method": "cargo test"}),
        );
        self.w.add(d, &VER, at)
    }
    pub fn release(&mut self, prev: &Event, preimage: &str, by: &[u8; 32], at: u64) -> Event {
        let [a, b] = parties();
        let d = Draft::act(K_RELEASE, &self.r.id, &prev.id, &[&a, &b])
            .content(json!({"preimage": preimage}));
        self.w.add(d, by, at)
    }
    /// ACK → PROMISE → REPORT; returns the REPORT.
    pub fn promised_report(&mut self) -> Event {
        self.ack(&VER, 1010);
        let p = self.promise(1100);
        self.report(&p, 1200)
    }
}

/// S-2: verifier trigger with a `human` condition: no attestation can pass it.
pub fn s2() -> W {
    setup(3002, |t| {
        t["conditions"] = json!([{"id": "taste", "class": "human"}]);
    })
    .w
}

/// S-4: PROMISE without an ESCROW_ACK; an ACK from a stranger doesn't count.
pub fn s4() -> W {
    let mut v = setup(3004, |_| {});
    v.ack(&STRANGER, 1010);
    v.promise(1100);
    v.w
}

/// S-5: happy path; the requester's later SATISFY is a recorded judgement.
pub fn s5() -> W {
    let mut v = setup(3005, |_| {});
    let rep = v.promised_report();
    v.attest(&rep, "pass", 1250);
    let rel = v.release(&rep, &preimage(), &VER, 1300);
    let r = v.r.clone();
    v.w.add(d_satisfy(&r, &rel, None), &REQ, 1400);
    v.w
}

/// S-6: RELEASE whose secret doesn't hash to H.
pub fn s6() -> W {
    let mut v = setup(3006, |_| {});
    let rep = v.promised_report();
    v.attest(&rep, "pass", 1250);
    v.release(&rep, &"00".repeat(32), &VER, 1300);
    v.w
}

/// S-7: RELEASE signed by a key other than the settlement verifier.
pub fn s7() -> W {
    let mut v = setup(3007, |_| {});
    let rep = v.promised_report();
    v.attest(&rep, "pass", 1250);
    v.release(&rep, &preimage(), &STRANGER, 1300);
    v.w
}

/// S-7b: RELEASE before every condition has passed.
pub fn s7b() -> W {
    let mut v = setup(3017, |_| {});
    let rep = v.promised_report();
    v.release(&rep, &preimage(), &VER, 1300);
    v.w
}

/// S-8: the verifier stays silent after the REPORT (lapse checked with `status_at`).
pub fn s8() -> W {
    let mut v = setup(3008, |_| {});
    v.promised_report();
    v.w
}

/// S-8b: the requester can't SATISFY or pre-empt the release with a REJECT while nothing
/// fails; after a failing attestation, REJECT sends it back for rework.
pub fn s8b() -> W {
    let mut v = setup(3018, |_| {});
    let rep = v.promised_report();
    let r = v.r.clone();
    v.w.add(d_satisfy(&r, &rep, Some(&preimage())), &REQ, 1210); // illegal: RELEASE settles
    v.w.add(d_reject(&r, &rep, "tests"), &REQ, 1220); // illegal: nothing fails yet
    v.w
}

pub fn s8c() -> W {
    let mut v = setup(3028, |_| {});
    let rep = v.promised_report();
    v.attest(&rep, "fail", 1250);
    let r = v.r.clone();
    v.w.add(d_reject(&r, &rep, "tests"), &REQ, 1300);
    v.w
}
