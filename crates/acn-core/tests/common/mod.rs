#![allow(dead_code)]

pub mod judged;
pub mod settlement;

use std::collections::BTreeMap;

use acn_core::draft::{public_key, Draft};
use acn_core::jcs;
use acn_core::Event;
use serde_json::{json, Value};

pub const REQ: [u8; 32] = [1; 32];
pub const PERF: [u8; 32] = [2; 32];
pub const ARB: [u8; 32] = [3; 32];
pub const VER: [u8; 32] = [4; 32];
pub const OP: [u8; 32] = [5; 32];
pub const GOV: [u8; 32] = [6; 32];
pub const STRANGER: [u8; 32] = [7; 32];

/// Payment secret `s` and its hashlock `H`.
pub const SECRET: [u8; 32] = [9; 32];

pub fn pk(k: &[u8; 32]) -> String {
    public_key(k).unwrap()
}

pub fn preimage() -> String {
    hex::encode(SECRET)
}

pub fn hashlock() -> String {
    jcs::sha256_hex(&SECRET)
}

pub type Docs = BTreeMap<String, Value>;

/// A scenario under construction.
#[derive(Default, Clone)]
pub struct W {
    pub events: Vec<Event>,
    pub docs: Docs,
}

impl W {
    pub fn add(&mut self, d: Draft, who: &[u8; 32], at: u64) -> Event {
        let e = d.sign(who, at).expect("sign");
        self.events.push(e.clone());
        e
    }

    pub fn push(&mut self, e: Event) -> Event {
        self.events.push(e.clone());
        e
    }

    pub fn fold(&self) -> acn_core::Output {
        acn_core::fold(&self.events, &self.docs)
    }
}

pub fn parties() -> [String; 2] {
    [pk(&REQ), pk(&PERF)]
}

/// Base terms; `f` customises them.
pub fn terms(f: impl FnOnce(&mut Value)) -> Value {
    let mut t = json!({
        "v": "acn-terms/0.4",
        "deliverable": { "description": "do the thing" },
        "conditions": [],
        "deadline": 2000,
        "review_window": 500,
        "rework_limit": 0,
        "price": { "amount": 0, "unit": "sat" },
        "payment": null,
        "verifiers": [],
        "arbiter": null,
        "subcontracting": false,
        "disclosure": "public",
        "governance": null
    });
    f(&mut t);
    t
}

pub fn terms_hash(t: &Value) -> String {
    jcs::hash(t).unwrap()
}

/// Root REQUEST signed by the requester at t=1000.
pub fn request(w: &mut W, profile: &str, t: &Value) -> Event {
    w.add(
        Draft::request(&pk(&PERF), profile, t, None).unwrap(),
        &REQ,
        1000,
    )
}

pub fn act(kind: u16, c: &Event, prev: &Event) -> Draft {
    let [a, b] = parties();
    Draft::act(kind, &c.id, &prev.id, &[&a, &b])
}

pub fn p2() -> [String; 2] {
    parties()
}

pub fn d_report(c: &Event, prev: &Event) -> Draft {
    let [a, b] = parties();
    Draft::report(
        &c.id,
        &prev.id,
        &[&a, &b],
        Some(json!({"ok": true})),
        None,
        &[],
    )
}

pub fn d_satisfy(c: &Event, prev: &Event, preimage: Option<&str>) -> Draft {
    let [a, b] = parties();
    Draft::satisfy(&c.id, &prev.id, &[&a, &b], preimage)
}

pub fn d_promise(c: &Event, prev: &Event, terms_hash: &str) -> Draft {
    let [a, b] = parties();
    Draft::promise(&c.id, &prev.id, &[&a, &b], terms_hash, 0.9, true)
}

pub fn d_closing(kind: u16, c: &Event, prev: &Event) -> Draft {
    let [a, b] = parties();
    Draft::closing(kind, &c.id, &prev.id, &[&a, &b], "because")
}

pub fn d_reject(c: &Event, prev: &Event, cond: &str) -> Draft {
    let [a, b] = parties();
    Draft::reject(&c.id, &prev.id, &[&a, &b], vec![(cond, "not good")])
}

/// Every arrival order for small sets, a deterministic sample for larger ones.
pub fn orders(n: usize) -> Vec<Vec<usize>> {
    let base: Vec<usize> = (0..n).collect();
    if n <= 6 {
        let mut out = Vec::new();
        permute(&mut base.clone(), 0, &mut out);
        return out;
    }
    let mut out = vec![base.clone(), base.iter().rev().copied().collect()];
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    for _ in 0..300 {
        let mut v = base.clone();
        for i in (1..n).rev() {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            v.swap(i, (seed >> 33) as usize % (i + 1));
        }
        out.push(v);
    }
    out
}

fn permute(v: &mut Vec<usize>, k: usize, out: &mut Vec<Vec<usize>>) {
    if k == v.len() {
        out.push(v.clone());
        return;
    }
    for i in k..v.len() {
        v.swap(k, i);
        permute(v, k + 1, out);
        v.swap(k, i);
    }
}
