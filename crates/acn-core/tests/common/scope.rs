//! Binding scope against negotiated terms (V-8, audit A05) and settlement-time evidence cutoffs
//! for `RELEASE` (V-20, V-25, audit A15). Each vector is one commitment; the scenario gates run
//! them under every arrival order.

use acn_core::draft::Draft;
use acn_core::kinds::*;
use acn_core::Event;
use serde_json::{json, Value};

use super::cos::{cos_terms, d1, Disq};
use super::settlement::setup;
use super::*;

pub const MINT: &str = "https://mint.test";
pub const OTHER_MINT: &str = "https://other.test";

/// Terms at `amount` sat on `mint`.
pub fn priced(amount: u64, mint: &str) -> Value {
    terms(|t| {
        t["price"] = json!({"amount": amount, "unit": "sat"});
        t["payment"] = json!({ "mint": mint });
    })
}

/// An acknowledged performer binding (operator OP) with `scopes`, signed at `at`.
pub fn bind(w: &mut W, expires: u64, scopes: &[(&str, &str)], at: u64) -> Event {
    let b = w.add(Draft::bind(&pk(&PERF), expires, scopes), &OP, at);
    w.add(Draft::bind_ack(&b.id), &PERF, at + 1);
    b
}

fn counter(w: &mut W, r: &Event, prev: &Event, t: &Value, by: &[u8; 32], at: u64) -> Event {
    let [a, b] = parties();
    w.add(
        Draft::counter(&r.id, &prev.id, &[&a, &b], t, "new price").unwrap(),
        by,
        at,
    )
}

fn accept(w: &mut W, r: &Event, ctr: &Event, t: &Value, by: &[u8; 32], at: u64) -> Event {
    let [a, b] = parties();
    w.add(
        Draft::accept(&r.id, &ctr.id, &[&a, &b], &terms_hash(t)),
        by,
        at,
    )
}

/// e: BIND (performer capped at 500 sat), ACK, REQUEST at `request_price`.
fn capped(w: &mut W, request_price: u64) -> Event {
    bind(w, 5000, &[("max_sat", "500")], 900);
    request(w, "N", &priced(request_price, MINT))
}

/// SC-1: e: BIND, ACK, REQUEST(100), COUNTER(req, 1000), ACCEPT(perf). The ACCEPT commits the
/// performer to 1000 sat, above its `max_sat`: `out_of_scope`, the counter stays pending.
pub fn sc1() -> W {
    let mut w = W::default();
    let r = capped(&mut w, 100);
    let t2 = priced(1000, MINT);
    let ctr = counter(&mut w, &r, &r, &t2, &REQ, 1100);
    accept(&mut w, &r, &ctr, &t2, &PERF, 1200);
    w
}

/// SC-2: e: BIND, ACK, REQUEST(100), COUNTER(perf, 1000), ACCEPT(req). The performer's own
/// counter offers 1000 sat: `out_of_scope`; the ACCEPT under it is off the effective chain.
pub fn sc2() -> W {
    let mut w = W::default();
    let r = capped(&mut w, 100);
    let t2 = priced(1000, MINT);
    let ctr = counter(&mut w, &r, &r, &t2, &PERF, 1100);
    accept(&mut w, &r, &ctr, &t2, &REQ, 1200);
    w
}

/// SC-3: e: BIND A (2000, expires 1250), ACK A, BIND B (500), ACK B, REQUEST(100),
/// COUNTER(req, 1000), ACCEPT(perf, under A), REPORT(perf, after A expired). The REPORT performs
/// the accepted 1000-sat terms, which only the expired binding covered: `out_of_scope`.
pub fn sc3() -> W {
    let mut w = W::default();
    bind(&mut w, 1250, &[("max_sat", "2000")], 900);
    bind(&mut w, 5000, &[("max_sat", "500")], 910);
    let r = request(&mut w, "N", &priced(100, MINT));
    let t2 = priced(1000, MINT);
    let ctr = counter(&mut w, &r, &r, &t2, &REQ, 1100);
    let acc = accept(&mut w, &r, &ctr, &t2, &PERF, 1200);
    w.add(d_report(&r, &acc), &PERF, 1300);
    w
}

/// SC-4: e: BIND (mint scope), ACK, REQUEST(at the scoped mint), COUNTER(req, another mint),
/// ACCEPT(perf). The accepted terms pay at a mint outside the binding: `out_of_scope`.
pub fn sc4() -> W {
    let mut w = W::default();
    bind(&mut w, 5000, &[("max_sat", "500"), ("mint", MINT)], 900);
    let r = request(&mut w, "N", &priced(100, MINT));
    let t2 = priced(100, OTHER_MINT);
    let ctr = counter(&mut w, &r, &r, &t2, &REQ, 1100);
    accept(&mut w, &r, &ctr, &t2, &PERF, 1200);
    w
}

/// SC-5: e: BIND, ACK, REQUEST(1000, above the cap), COUNTER(perf, 400), ACCEPT(req),
/// REPORT(perf). The performer only ever commits to 400 sat: in scope throughout.
pub fn sc5() -> W {
    let mut w = W::default();
    let r = capped(&mut w, 1000);
    let t2 = priced(400, MINT);
    let ctr = counter(&mut w, &r, &r, &t2, &PERF, 1100);
    let acc = accept(&mut w, &r, &ctr, &t2, &REQ, 1200);
    w.add(d_report(&r, &acc), &PERF, 1300);
    w
}

/// SC-6: e: BIND, ACK, REQUEST(1000, above the cap), COUNTER(req, 400), ACCEPT(perf),
/// REPORT(perf). The performer's ACCEPT commits to the countered 400 sat: in scope.
pub fn sc6() -> W {
    let mut w = W::default();
    let r = capped(&mut w, 1000);
    let t2 = priced(400, MINT);
    let ctr = counter(&mut w, &r, &r, &t2, &REQ, 1100);
    let acc = accept(&mut w, &r, &ctr, &t2, &PERF, 1200);
    w.add(d_report(&r, &acc), &PERF, 1300);
    w
}

/// SC-7: e: BIND, ACK, REQUEST(1000, above the cap), DECLINE(perf). Refusing commits to no
/// value: applied, Declined.
pub fn sc7() -> W {
    let mut w = W::default();
    let r = capped(&mut w, 1000);
    w.add(d_closing(K_DECLINE, &r, &r), &PERF, 1100);
    w
}

/// SC-7b: e: BIND A (2000, expires 1250), ACK A, BIND B (500), ACK B, REQUEST(1000),
/// PROMISE(under A), REVOKE(after A expired). Only B is in force and the terms exceed it, but a
/// REVOKE commits to no value: applied, Revoked.
pub fn sc7b() -> W {
    let mut w = W::default();
    bind(&mut w, 1250, &[("max_sat", "2000")], 900);
    bind(&mut w, 5000, &[("max_sat", "500")], 910);
    let t = priced(1000, MINT);
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    w.add(d_closing(K_REVOKE, &r, &p), &PERF, 1300);
    w
}

/// S-5b: e: REQUEST, ACK, PROMISE, REPORT, ATTEST pass, RELEASE, ATTEST fail (after RELEASE).
/// The RELEASE is decided on the evidence up to it: Settled.
pub fn s5b() -> W {
    let mut v = setup(3025, |_| {});
    let rep = v.promised_report();
    v.attest(&rep, "pass", 1250);
    v.release(&rep, &preimage(), &VER, 1300);
    v.attest(&rep, "fail", 1400);
    v.w
}

/// S-7c: e: REQUEST, ACK, PROMISE, REPORT, RELEASE, ATTEST pass (after RELEASE). Evidence
/// signed after the RELEASE can't validate it: `bad_release`.
pub fn s7c() -> W {
    let mut v = setup(3027, |_| {});
    let rep = v.promised_report();
    v.release(&rep, &preimage(), &VER, 1300);
    v.attest(&rep, "pass", 1310);
    v.w
}

/// C-6g: e: REQUEST, ESCROW_ACK, PROMISE, REPORT, c1 pass, RELEASE, d1 hit (after RELEASE).
/// The hit comes after the act that settles the REPORT: `late`, not counted; still Settled.
pub fn c6g() -> W {
    let mut w = W::default();
    let t = cos_terms(3113, |t| {
        t["disqualifiers"] = json!([d1()]);
        t["price"] = json!({"amount": 100, "unit": "sat"});
        t["payment"] = json!({"mint": MINT, "hashlock": hashlock(), "locktime": 999999});
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
    let [a, b] = parties();
    let rel = Draft::act(K_RELEASE, &d.r.id, &d.rep.id, &[&a, &b])
        .content(json!({"preimage": preimage()}));
    d.w.add(rel, &VER, 1270);
    d.attest("d1", "hit", &VER, 1300);
    d.w
}
