//! CR-003 Phase B conformance vectors: held-out instances and the two-party seed (spec §7
//! "Instances", V-27). Each vector is one commitment; the gates run every arrival order.

use acn_core::draft::Draft;
use acn_core::instances as ins;
use acn_core::kinds::*;
use acn_core::Event;
use serde_json::{json, Value};

use super::cos::cos_terms;
use super::*;

pub const POOL: u32 = 20;
pub const SAMPLE: u32 = 5;
pub const RS: [u8; 32] = [0x51; 32];
pub const PS: [u8; 32] = [0x52; 32];

pub fn pool() -> Vec<Value> {
    (0..POOL)
        .map(|i| json!(format!("integration case {i}")))
        .collect()
}

fn bytes() -> Vec<Vec<u8>> {
    pool().iter().map(ins::item_bytes).collect()
}

pub fn seed_hash(s: &[u8; 32]) -> String {
    acn_core::jcs::sha256_hex(s)
}

pub fn instances() -> Value {
    json!({
        "pool_description": "Integration tests exercising the changed behaviour",
        "pool_rule": "Written by the requester before REQUEST; not in the repo",
        "pool_root": hex::encode(ins::root(&bytes())),
        "pool_size": POOL, "sample_size": SAMPLE,
        "requester_seed_hash": seed_hash(&RS)
    })
}

/// `c1` mechanical with held-out instances, checked by VER.
pub fn held_out_terms(deadline: u64, f: impl FnOnce(&mut Value)) -> Value {
    cos_terms(deadline, |t| {
        t["conditions"][0]["instances"] = instances();
        f(t);
    })
}

/// The attestation content for `c1`: the requester seed and the derived sample, with proofs.
pub fn attest_content(commitment: &str, rs: &[u8; 32], indices: Option<Vec<u32>>) -> Value {
    let cid = ins::from_hex(commitment).unwrap();
    let idx = indices.unwrap_or_else(|| ins::sample(rs, &PS, &cid, POOL, SAMPLE));
    let b = bytes();
    let items: Vec<Value> = idx
        .iter()
        .map(|&i| {
            let proof: Vec<String> = ins::proof(&b, i as usize).iter().map(hex::encode).collect();
            json!({"index": i, "item": pool()[i as usize], "proof": proof})
        })
        .collect();
    json!({"result": "pass", "method": "held-out suite", "requester_seed": hex::encode(rs), "instances": items})
}

pub struct H {
    pub w: W,
    pub r: Event,
    pub rep: Event,
}

/// REQUEST → PROMISE (committing PS) → REPORT (revealing `reveal`, or nothing).
pub fn setup(deadline: u64, reveal: Option<[u8; 32]>) -> H {
    let mut w = W::default();
    let t = held_out_terms(deadline, |_| {});
    let r = request(&mut w, "N", &t);
    let p = w.add(
        d_promise(&r, &r, &terms_hash(&t)).field("performer_seed_hash", json!(seed_hash(&PS))),
        &PERF,
        1100,
    );
    let mut d = d_report(&r, &p);
    if let Some(s) = reveal {
        d = d.field("performer_seed", json!(hex::encode(s)));
    }
    let rep = w.add(d, &PERF, 1200);
    H { w, r, rep }
}

impl H {
    pub fn attest(&mut self, content: Value, at: u64) -> Event {
        let d = Draft::attest(K_ATTEST_MECH, &self.r.id, &self.rep.id, "c1", content);
        self.w.add(d, &VER, at)
    }
}

/// C-10: valid seeds, derivation and proofs; the runner's attestation counts; satisfied.
pub fn c10() -> W {
    let mut h = setup(3201, Some(PS));
    let c = attest_content(&h.r.id, &RS, None);
    h.attest(c, 1250);
    let (r, rep) = (h.r.clone(), h.rep.clone());
    h.w.add(d_satisfy(&r, &rep, None), &REQ, 1300);
    h.w
}

/// C-10b: the performer's COUNTER carries the instance terms and commits its seed; the
/// requester accepts; the REPORT's seed is checked against the counter's commitment.
pub fn c10b() -> W {
    let mut w = W::default();
    let v1 = cos_terms(3202, |_| {});
    let r = request(&mut w, "N", &v1);
    let v2 = held_out_terms(3202, |_| {});
    let [a, b] = parties();
    let ctr = w.add(
        Draft::counter(&r.id, &r.id, &[&a, &b], &v2, "graded on held-out cases")
            .unwrap()
            .field("performer_seed_hash", json!(seed_hash(&PS))),
        &PERF,
        1100,
    );
    let acc = w.add(
        Draft::accept(&r.id, &ctr.id, &[&a, &b], &terms_hash(&v2)),
        &REQ,
        1150,
    );
    let rep = w.add(
        d_report(&r, &acc).field("performer_seed", json!(hex::encode(PS))),
        &PERF,
        1200,
    );
    let mut h = H { w, r, rep };
    let c = attest_content(&h.r.id, &RS, None);
    h.attest(c, 1250);
    h.w
}

/// C-10c: a PROMISE against instance terms without `performer_seed_hash`.
pub fn c10c() -> W {
    let mut w = W::default();
    let t = held_out_terms(3203, |_| {});
    let r = request(&mut w, "N", &t);
    w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    w
}

/// C-11: the revealed requester seed doesn't match its hash.
pub fn c11() -> W {
    let mut h = setup(3204, Some(PS));
    let c = attest_content(&h.r.id, &[0x99; 32], None);
    h.attest(c, 1250);
    h.w
}

/// C-12: the REPORT omits the performer seed.
pub fn c12() -> W {
    setup(3205, None).w
}

/// C-12b: the REPORT reveals a seed other than the committed one.
pub fn c12b() -> W {
    setup(3206, Some([0x77; 32])).w
}

/// C-13: the attested indices don't follow the derivation (the runner picked its own).
pub fn c13() -> W {
    let mut h = setup(3207, Some(PS));
    let c = attest_content(&h.r.id, &RS, Some(vec![0, 1, 2, 3, 4]));
    h.attest(c, 1250);
    h.w
}

/// C-14: a sampled item doesn't prove against `pool_root` (the item was swapped).
pub fn c14() -> W {
    let mut h = setup(3208, Some(PS));
    let mut c = attest_content(&h.r.id, &RS, None);
    c["instances"][2]["item"] = json!("an easier case");
    h.attest(c, 1250);
    h.w
}
