//! Conformance scenarios (spec §15 P0 gates).
//!
//! - one scenario per act and per anomaly code (`every_anomaly_code_is_exercised`);
//! - each scenario's output is identical for every arrival order (`fold_is_order_independent`).

mod common;

use std::collections::BTreeSet;

use acn_core::draft::Draft;
use acn_core::kinds::*;
use acn_core::{status_at, Code, ConditionStatus, Party, Phase, Profile};
use common::*;
use serde_json::json;

type Scenario = (&'static str, fn() -> W);

fn phase(w: &W, c: &acn_core::Event) -> Phase {
    w.fold().commitments[&c.id]
        .state
        .as_ref()
        .expect("state")
        .phase
}

// ---------- scenario builders ----------

fn t0_happy() -> W {
    let mut w = W::default();
    let r = request(&mut w, "T0", &terms(|_| {}));
    let rep = w.add(d_report(&r, &r), &PERF, 1100);
    w.add(d_satisfy(&r, &rep, None), &REQ, 1200);
    w
}

fn t1_preimage() -> W {
    let mut w = W::default();
    let t = terms(
        |t| t["payment"] = json!({"mint": "https://mint.test", "keysets": ["00ab"], "hashlock": hashlock(), "locktime": 999999}),
    );
    let r = request(&mut w, "T1", &t);
    let rep = w.add(d_report(&r, &r), &PERF, 1100);
    w.add(d_satisfy(&r, &rep, Some(&"00".repeat(32))), &REQ, 1200); // bad_preimage
    w.add(d_satisfy(&r, &rep, None), &REQ, 1201); // malformed: preimage required
    w.add(d_satisfy(&r, &rep, Some(&preimage())), &REQ, 1202);
    w
}

fn n_full_rework() -> W {
    let mut w = W::default();
    let t = terms(|t| {
        t["rework_limit"] = json!(1);
        t["conditions"] = json!([{"id": "c1", "class": "human"}]);
    });
    let h = terms_hash(&t);
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &h), &PERF, 1100);
    w.add(
        {
            let [a, b] = parties();
            Draft::progress(&r.id, &p.id, &[&a, &b], "halfway")
        },
        &PERF,
        1150,
    );
    let rep1 = w.add(d_report(&r, &p), &PERF, 1200);
    let rej = w.add(d_reject(&r, &rep1, "c1"), &REQ, 1300);
    let rep2 = w.add(d_report(&r, &rej), &PERF, 1400);
    w.add(d_satisfy(&r, &rep2, None), &REQ, 1500);
    w
}

fn n_counter_accept() -> W {
    let mut w = W::default();
    let t = terms(|_| {});
    let r = request(&mut w, "N", &t);
    let t2 = terms(|t| t["deadline"] = json!(3000));
    let [a, b] = parties();
    let ctr = w.add(
        Draft::counter(&r.id, &r.id, &[&a, &b], &t2, "need more time").unwrap(),
        &PERF,
        1100,
    );
    // The counterer can't accept its own counter.
    w.add(
        Draft::accept(&r.id, &ctr.id, &[&a, &b], &terms_hash(&t2)),
        &PERF,
        1150,
    );
    // Wrong terms.
    w.add(
        Draft::accept(&r.id, &ctr.id, &[&a, &b], &terms_hash(&t)),
        &REQ,
        1160,
    );
    w.add(
        Draft::accept(&r.id, &ctr.id, &[&a, &b], &terms_hash(&t2)),
        &REQ,
        1200,
    );
    w
}

fn counter_while_promised() -> W {
    let mut w = W::default();
    let t = terms(|_| {});
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    let t2 = terms(|t| t["price"] = json!({"amount": 5, "unit": "sat"}));
    let [a, b] = parties();
    let ctr = w.add(
        Draft::counter(&r.id, &p.id, &[&a, &b], &t2, "cheaper?").unwrap(),
        &REQ,
        1150,
    );
    w.add(d_report(&r, &ctr), &PERF, 1200);
    w
}

fn rework_limit_and_no_arbiter() -> W {
    let mut w = W::default();
    let t = terms(|t| t["conditions"] = json!([{"id": "c1", "class": "human"}]));
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    let rep = w.add(d_report(&r, &p), &PERF, 1200);
    let rej = w.add(d_reject(&r, &rep, "c1"), &REQ, 1300);
    let [a, b] = parties();
    w.add(
        Draft::dispute(&r.id, &rej.id, &[&a, &b], "unfair"),
        &PERF,
        1350,
    ); // no arbiter
    w
}

fn dispute_ruling() -> W {
    let mut w = W::default();
    let t = terms(|t| {
        t["arbiter"] = json!(pk(&ARB));
        t["conditions"] = json!([{"id": "c1", "class": "human"}]);
        t["payment"] = json!({"mint": "https://mint.test", "hashlock": hashlock()});
    });
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    let rep = w.add(d_report(&r, &p), &PERF, 1200);
    let rej = w.add(d_reject(&r, &rep, "c1"), &REQ, 1300);
    let [a, b] = parties();
    let dis = w.add(
        Draft::dispute(&r.id, &rej.id, &[&a, &b], "unfair"),
        &PERF,
        1350,
    );
    w.add(
        Draft::ruling(&r.id, &dis.id, &[&a, &b], true, Some(&preimage())),
        &STRANGER,
        1400,
    );
    w.add(
        Draft::ruling(&r.id, &dis.id, &[&a, &b], true, Some(&preimage())),
        &ARB,
        1401,
    );
    w
}

fn dispute_window_elapsed() -> W {
    let mut w = W::default();
    let t = terms(|t| {
        t["arbiter"] = json!(pk(&ARB));
        t["conditions"] = json!([{"id": "c1", "class": "human"}]);
    });
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    let rep = w.add(d_report(&r, &p), &PERF, 1200);
    let rej = w.add(d_reject(&r, &rep, "c1"), &REQ, 1300);
    let [a, b] = parties();
    w.add(
        Draft::dispute(&r.id, &rej.id, &[&a, &b], "too late"),
        &PERF,
        1300 + 501,
    );
    w
}

fn terminals() -> W {
    let mut w = W::default();
    let t = terms(|_| {});
    let r1 = request(&mut w, "N", &t);
    w.add(d_closing(K_DECLINE, &r1, &r1), &PERF, 1100);
    let t2 = terms(|t| t["deadline"] = json!(2001));
    let r2 = request(&mut w, "N", &t2);
    w.add(d_closing(K_WITHDRAW, &r2, &r2), &REQ, 1100);
    let t3 = terms(|t| t["deadline"] = json!(2002));
    let r3 = request(&mut w, "N", &t3);
    let p3 = w.add(d_promise(&r3, &r3, &terms_hash(&t3)), &PERF, 1100);
    w.add(d_closing(K_REVOKE, &r3, &p3), &PERF, 1200);
    w
}

fn siblings() -> W {
    let mut w = W::default();
    let t = terms(|_| {});
    let r1 = request(&mut w, "N", &t);
    // Closing beats term-preserving.
    w.add(d_promise(&r1, &r1, &terms_hash(&t)), &PERF, 1100);
    w.add(d_closing(K_WITHDRAW, &r1, &r1), &REQ, 1100);
    // Term-preserving beats term-changing.
    let t2 = terms(|t| t["deadline"] = json!(2500));
    let r2 = request(&mut w, "N", &t2);
    let promise = w.add(d_promise(&r2, &r2, &terms_hash(&t2)), &PERF, 1100);
    let [a, b] = parties();
    w.add(
        Draft::counter(
            &r2.id,
            &r2.id,
            &[&a, &b],
            &terms(|t| t["deadline"] = json!(2600)),
            "change",
        )
        .unwrap(),
        &REQ,
        1100,
    );
    // An act built on the losing counter is off-chain.
    let _ = promise;
    w
}

fn equivocation() -> W {
    let mut w = W::default();
    let t = terms(|_| {});
    let r = request(&mut w, "N", &t);
    w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    w.add(d_closing(K_DECLINE, &r, &r), &PERF, 1100);
    w
}

fn off_chain() -> W {
    let mut w = W::default();
    let t = terms(|_| {});
    let r = request(&mut w, "N", &t);
    let [a, b] = parties();
    let loser = w.add(
        Draft::counter(
            &r.id,
            &r.id,
            &[&a, &b],
            &terms(|t| t["deadline"] = json!(9000)),
            "x",
        )
        .unwrap(),
        &REQ,
        1100,
    );
    w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    // Builds on the stale counter → not on the effective chain.
    w.add(
        Draft::accept(
            &r.id,
            &loser.id,
            &[&a, &b],
            &terms_hash(&terms(|t| t["deadline"] = json!(9000))),
        ),
        &PERF,
        1200,
    );
    w
}

fn time_regress() -> W {
    let mut w = W::default();
    let r = request(&mut w, "T0", &terms(|_| {}));
    w.add(d_report(&r, &r), &PERF, 900);
    w
}

fn illegal_and_unauthorized() -> W {
    let mut w = W::default();
    let r = request(&mut w, "T0", &terms(|_| {}));
    w.add(d_satisfy(&r, &r, None), &REQ, 1100); // illegal from Requested
    w.add(d_report(&r, &r), &REQ, 1100); // unauthorized: requester can't report
    w.add(d_promise(&r, &r, &r.id), &PERF, 1100); // illegal: PROMISE in T0 (terms tag is a 64-hex id)
    w
}

fn envelope_errors() -> W {
    let mut w = W::default();
    let r = request(&mut w, "T0", &terms(|_| {}));
    // bad_sig: tampered content.
    let mut forged = d_report(&r, &r).sign(&PERF, 1100).unwrap();
    forged.content = r#"{"result":"tampered"}"#.into();
    w.push(forged);
    // bad_version.
    let mut d = d_report(&r, &r);
    d.tags[0] = vec!["v".into(), "acn/9.9".into()];
    w.add(d, &PERF, 1101);
    // unknown_kind inside the block (7719 is unallocated).
    w.add(Draft::act(7719, &r.id, &r.id, &[]), &PERF, 1102);
    // malformed: REJECT without gaps.
    w.add(
        act(K_REJECT, &r, &r).content(json!({"gaps": []})),
        &REQ,
        1103,
    );
    // Unrelated kinds are ignored entirely.
    w.add(Draft::new(1).content(json!({})), &STRANGER, 1104);
    w
}

fn linkage() -> W {
    let mut w = W::default();
    let a = request(&mut w, "T0", &terms(|_| {}));
    let b = request(&mut w, "T0", &terms(|t| t["deadline"] = json!(2100)));
    // foreign_prev: in commitment A, prev from commitment B.
    w.add(d_report(&a, &b), &PERF, 1100);
    // pending_ref: unknown commitment root, and unknown prev.
    let ghost = Draft::report(&"ab".repeat(32), &"cd".repeat(32), &[], None, None, &[]);
    w.add(ghost, &PERF, 1100);
    let mut unknown_prev = W::default();
    let tmp = request(
        &mut unknown_prev,
        "T0",
        &terms(|t| t["deadline"] = json!(2200)),
    );
    w.add(d_report(&a, &tmp), &PERF, 1101);
    w
}

fn pending_terms() -> W {
    let mut w = W::default();
    let t = terms(|t| t["deadline"] = json!(4242));
    // REQUEST that references terms by hash only (stored off-relay).
    let d = Draft::new(K_REQUEST)
        .tag(vec!["p".into(), pk(&PERF)])
        .tag(vec!["profile".into(), "T0".into()])
        .terms_hash(&terms_hash(&t))
        .content(json!({}));
    let r = w.add(d, &REQ, 1000);
    w.add(d_report(&r, &r), &PERF, 1100);
    w
}

fn terms_mismatch_inline() -> W {
    let mut w = W::default();
    let t = terms(|_| {});
    let mut d = Draft::request(&pk(&PERF), "T0", &t, None).unwrap();
    d.content = json!({"terms": terms(|t| t["deadline"] = json!(1))}).to_string();
    let r = w.add(d, &REQ, 1000);
    w.add(d_report(&r, &r), &PERF, 1100);
    w
}

fn bindings() -> W {
    let mut w = W::default();
    let perf = pk(&PERF);
    // Bound commitment.
    let bind = w.add(
        Draft::bind(&perf, 5000, &[("kinds", "7703,7708"), ("max_sat", "10")]),
        &OP,
        900,
    );
    w.add(Draft::bind_ack(&bind.id), &PERF, 901);
    let t = terms(|_| {});
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    w.add(d_report(&r, &p), &PERF, 1200);
    // Out of scope: price above max_sat.
    let t2 = terms(|t| t["price"] = json!({"amount": 50, "unit": "sat"}));
    let r2 = request(&mut w, "N", &t2);
    w.add(d_promise(&r2, &r2, &terms_hash(&t2)), &PERF, 1100);
    // Out of scope: kind not granted (DECLINE = 7704).
    let t3 = terms(|t| t["deadline"] = json!(2003));
    let r3 = request(&mut w, "N", &t3);
    w.add(d_closing(K_DECLINE, &r3, &r3), &PERF, 1100);
    w
}

fn revoked_binding() -> W {
    let mut w = W::default();
    let perf = pk(&PERF);
    let bind = w.add(Draft::bind(&perf, 5000, &[]), &OP, 900);
    w.add(Draft::bind_ack(&bind.id), &PERF, 901);
    w.add(Draft::bind_revoke(&bind.id), &OP, 1150);
    let t = terms(|_| {});
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100); // before revocation: ok
    w.add(d_report(&r, &p), &PERF, 1200); // after: revoked
    w
}

fn evidence() -> W {
    let mut w = W::default();
    let t = terms(|t| {
        t["conditions"] = json!([
            {"id": "tests", "class": "mechanical"},
            {"id": "quality", "class": "interpretive"},
            {"id": "ok", "class": "human"}
        ]);
        t["verifiers"] = json!([{"pubkey": pk(&VER), "classes": ["mechanical", "interpretive"]}]);
    });
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    let rep = w.add(d_report(&r, &p), &PERF, 1200);
    w.add(
        Draft::attest(
            K_ATTEST_MECH,
            &r.id,
            &rep.id,
            "tests",
            json!({"result": "pass", "method": "cargo test"}),
        ),
        &VER,
        1250,
    );
    // Interpretive report can't count toward a mechanical condition (INV-8).
    w.add(
        Draft::attest(
            K_INTERP_REPORT,
            &r.id,
            &rep.id,
            "tests",
            json!({"verdict": "fail", "judge": "llm"}),
        ),
        &VER,
        1251,
    );
    w.add(
        Draft::attest(
            K_INTERP_REPORT,
            &r.id,
            &rep.id,
            "quality",
            json!({"verdict": "good", "judge": "llm"}),
        ),
        &VER,
        1252,
    );
    // Unlisted verifier.
    w.add(
        Draft::attest(
            K_ATTEST_MECH,
            &r.id,
            &rep.id,
            "tests",
            json!({"result": "fail"}),
        ),
        &STRANGER,
        1253,
    );
    // Rejection contradicted by the passing mechanical attestation.
    w.add(d_reject(&r, &rep, "tests"), &REQ, 1300);
    w
}

fn governance() -> W {
    let mut w = W::default();
    let gov = json!({"keys": [pk(&GOV)], "kinds": [K_REPORT], "window": 100});
    let t1 = terms(|t| t["governance"] = gov.clone());
    // Endorsed REPORT applies.
    let r1 = request(&mut w, "N", &t1);
    let p1 = w.add(d_promise(&r1, &r1, &terms_hash(&t1)), &PERF, 1100);
    let rep1 = w.add(d_report(&r1, &p1), &PERF, 1200);
    w.add(
        Draft::endorse(&r1.id, &rep1.id, &"aa".repeat(32), true),
        &GOV,
        1250,
    );
    // Unendorsed REPORT stays pending.
    let t2 = terms(|t| {
        t["governance"] = gov.clone();
        t["deadline"] = json!(2005);
    });
    let r2 = request(&mut w, "N", &t2);
    let p2 = w.add(d_promise(&r2, &r2, &terms_hash(&t2)), &PERF, 1100);
    w.add(d_report(&r2, &p2), &PERF, 1200);
    // Window observed elapsed: a sibling REVOKE after the window wins, REPORT is unendorsed.
    let t3 = terms(|t| {
        t["governance"] = gov.clone();
        t["deadline"] = json!(2006);
    });
    let r3 = request(&mut w, "N", &t3);
    let p3 = w.add(d_promise(&r3, &r3, &terms_hash(&t3)), &PERF, 1100);
    w.add(d_report(&r3, &p3), &PERF, 1200);
    w.add(d_closing(K_REVOKE, &r3, &p3), &PERF, 1400);
    w
}

fn late() -> W {
    let mut w = W::default();
    let t = terms(|_| {}); // deadline 2000, review 500
    let r = request(&mut w, "T0", &t);
    let rep = w.add(d_report(&r, &r), &PERF, 2100);
    w.add(d_satisfy(&r, &rep, None), &REQ, 2700);
    w
}

const ALL: &[Scenario] = &[
    ("t0_happy", t0_happy),
    ("t1_preimage", t1_preimage),
    ("n_full_rework", n_full_rework),
    ("n_counter_accept", n_counter_accept),
    ("counter_while_promised", counter_while_promised),
    ("rework_limit_and_no_arbiter", rework_limit_and_no_arbiter),
    ("dispute_ruling", dispute_ruling),
    ("dispute_window_elapsed", dispute_window_elapsed),
    ("terminals", terminals),
    ("siblings", siblings),
    ("equivocation", equivocation),
    ("off_chain", off_chain),
    ("time_regress", time_regress),
    ("illegal_and_unauthorized", illegal_and_unauthorized),
    ("envelope_errors", envelope_errors),
    ("linkage", linkage),
    ("pending_terms", pending_terms),
    ("terms_mismatch_inline", terms_mismatch_inline),
    ("bindings", bindings),
    ("revoked_binding", revoked_binding),
    ("evidence", evidence),
    ("governance", governance),
    ("late", late),
    // CR-001 conformance vectors (judged condition class).
    ("J-1", common::judged::j1),
    ("J-2", common::judged::j2),
    ("J-3", common::judged::j3),
    ("J-3b", common::judged::j3b),
    ("J-4", common::judged::j4),
    ("J-5", common::judged::j5),
    ("J-6", common::judged::j6),
    ("J-7", common::judged::j7),
    ("J-8", common::judged::j8),
    ("J-11", common::judged::j11),
    ("J-12", common::judged::j_declared_and_unlisted),
    ("J-13", common::judged::j13),
    ("J-14", common::judged::j14),
    ("J-15", common::judged::j15),
    ("J-16", common::judged::j16),
    ("J-17", common::judged::j17),
    ("J-18", common::judged::j18),
    // CR-002 Phase A: verifier settlement trigger.
    ("S-2", common::settlement::s2),
    ("S-4", common::settlement::s4),
    ("S-5", common::settlement::s5),
    ("S-6", common::settlement::s6),
    ("S-7", common::settlement::s7),
    ("S-7b", common::settlement::s7b),
    ("S-8", common::settlement::s8),
    ("S-8b", common::settlement::s8b),
    ("S-8c", common::settlement::s8c),
    // CR-002 Phase C: contingent settlement.
    ("S-9", common::settlement::s9),
    ("S-11", common::settlement::s11),
    ("S-12", common::settlement::s12),
    ("S-13", common::settlement::s13),
    ("S-14", common::settlement::s14),
    ("S-15", common::settlement::s15),
    ("S-16", common::settlement::s16),
    ("S-17", common::settlement::s17),
    ("S-18", common::settlement::s18),
    // CR-003 Phase A: conditions-of-satisfaction authorship.
    ("C-1", common::cos::c1),
    ("C-2", common::cos::c2),
    ("C-2b", common::cos::c2b),
    ("C-2c", common::cos::c2c),
];

// ---------- CR-003 vectors (spec §10 conformance table) ----------

fn check_cos(name: &str, w: &W, out: &acn_core::Output) {
    let e = &w.events;
    let req = &e[0];
    let ctx = format!("{name}: {:?}", out.anomalies);
    let has = |i: usize, code: Code| out.has(&e[i].id, code);
    match name {
        // Hidden criteria: the terms version is rejected, so no commitment forms.
        "C-1" | "C-2" | "C-2b" => {
            assert!(has(0, Code::HiddenCriteria), "{ctx}");
            assert!(!out.commitments.contains_key(&req.id), "{ctx}");
        }
        "C-2c" => {
            assert!(has(0, Code::Malformed), "{ctx}");
            assert!(!has(0, Code::HiddenCriteria), "{ctx}");
            assert!(!out.commitments.contains_key(&req.id), "{ctx}");
        }
        other => panic!("no expectation for {other}"),
    }
}

const COS: &[Scenario] = &[
    ("C-1", common::cos::c1),
    ("C-2", common::cos::c2),
    ("C-2b", common::cos::c2b),
    ("C-2c", common::cos::c2c),
];

/// CR-003 §5.2: every Phase A vector, under every arrival-order permutation.
#[test]
fn cr003_cos_vectors_under_every_arrival_order() {
    for (name, build) in COS {
        let w = build();
        for order in orders(w.events.len()) {
            let shuffled: Vec<_> = order.iter().map(|&i| w.events[i].clone()).collect();
            check_cos(name, &w, &acn_core::fold(&shuffled, &w.docs));
        }
    }
}

/// C-9 (part): terms before 0.5 keep their meaning; only `commit_reveal` stops parsing.
#[test]
fn cr003_older_terms_keep_their_meaning() {
    let old = |f: fn(&mut serde_json::Value)| {
        common::judged::judged_terms(|t| {
            t["v"] = json!("acn-terms/0.4");
            let c = t["conditions"][1].as_object_mut().unwrap();
            c.remove("rubric_ref");
            c.insert("rubric_disclosure".into(), json!("public"));
            f(t);
        })
    };
    assert!(acn_core::Terms::parse(&old(|_| {})).is_ok());
    let reveal = acn_core::Terms::check(&old(|t| {
        t["conditions"][1]["rubric_disclosure"] = json!("commit_reveal")
    }));
    assert_eq!(reveal.unwrap_err().0, Code::HiddenCriteria);
    // The 0.5 fields don't exist before 0.5.
    assert!(
        acn_core::Terms::parse(&old(|t| t["conditions"][1]["rubric_ref"] = json!("x"))).is_err()
    );
    // In 0.5, rubric_disclosure is gone (rubrics are public).
    let v05 =
        common::judged::judged_terms(|t| t["conditions"][1]["rubric_disclosure"] = json!("public"));
    assert_eq!(acn_core::Terms::check(&v05).unwrap_err().0, Code::Malformed);
}

// ---------- CR-001 vectors (spec §10 conformance table) ----------

/// Expected outcome of one CR-001 vector, checked against a fold of `w`'s events in any order.
fn check_judged(name: &str, w: &W, out: &acn_core::Output) {
    use acn_core::Assurance;
    let e = &w.events;
    let (req, rep) = (&e[0], &e[2]);
    if name == "J-5" {
        // CR-003 §2.1: commit_reveal is a hidden criterion; the REQUEST never makes a commitment.
        assert!(
            out.has(&req.id, Code::HiddenCriteria),
            "{name}: {:?}",
            out.anomalies
        );
        assert!(!out.commitments.contains_key(&req.id));
        return;
    }
    if name == "J-14" {
        // Unparseable terms: the REQUEST is retained but never becomes a commitment.
        assert!(
            out.has(&req.id, Code::Malformed),
            "{name}: {:?}",
            out.anomalies
        );
        assert!(!out.commitments.contains_key(&req.id));
        return;
    }
    let c = &out.commitments[&req.id];
    let c2 = c.conditions.get("c2");
    let has = |i: usize, code: Code| out.has(&e[i].id, code);
    let codes = |i: usize| {
        out.anomalies_for(&e[i].id)
            .map(|a| a.code)
            .collect::<Vec<_>>()
    };
    let ctx = format!("{name}: {:?}", out.anomalies);

    assert_eq!(c.assurance, Some(Assurance::Opinion), "{ctx}");
    match name {
        "J-1" => {
            assert_eq!(c2, Some(&ConditionStatus::Pass), "{ctx}");
            assert_eq!(
                c.conditions.get("c1"),
                Some(&ConditionStatus::Pass),
                "{ctx}"
            );
            assert_eq!(c.state.as_ref().unwrap().phase, Phase::Satisfied, "{ctx}");
            assert!(
                out.anomalies.iter().all(|a| a.code == Code::Unbound),
                "{ctx}"
            );
        }
        "J-2" => {
            assert!(has(3, Code::WrongEvidenceClass), "{ctx}");
            assert_eq!(c2, Some(&ConditionStatus::Unverified), "{ctx}");
        }
        "J-3" => {
            assert_eq!(e[3].pubkey, pk(&PERF));
            assert_eq!(codes(3), [Code::SelfJudged], "{ctx}");
            assert_eq!(c2, Some(&ConditionStatus::Unverified), "{ctx}");
        }
        "J-3b" => assert!(has(0, Code::SelfJudged), "{ctx}"),
        "J-4" | "J-8" => {
            assert_eq!(codes(3), [Code::JudgeMismatch], "{ctx}");
            assert_eq!(c2, Some(&ConditionStatus::Unverified), "{ctx}");
        }
        "J-18" => {
            assert_eq!(codes(3), [Code::RubricMismatch], "{ctx}");
            assert_eq!(c2, Some(&ConditionStatus::Unverified), "{ctx}");
        }
        "J-6" => {
            assert_eq!(c2, Some(&ConditionStatus::Fail), "{ctx}");
            assert!(out.has(&rep.id, Code::InjectionDetected), "{ctx}");
            assert_eq!(rep.pubkey, pk(&PERF), "recorded on the performer's act");
        }
        "J-7" => {
            assert_eq!(c2, Some(&ConditionStatus::Fail), "{ctx}");
            assert_eq!(codes(3), [Code::BelowThreshold], "{ctx}");
        }
        "J-13" => {
            assert_eq!(c2, Some(&ConditionStatus::Pass), "{ctx}");
            assert_eq!(
                c.conditions.get("c3"),
                Some(&ConditionStatus::Fail),
                "{ctx}"
            );
        }
        "J-11" => {
            let t = c.terms.as_ref().unwrap();
            let classes: Vec<_> = t.conditions.iter().map(|c| c.class.assurance()).collect();
            assert_eq!(classes, [Assurance::Check, Assurance::Opinion]);
            assert!(
                out.anomalies.iter().all(|a| a.code == Code::Unbound),
                "{ctx}"
            );
        }
        // e: REQUEST, PROMISE, REPORT, ATTEST_JUDGED, REJECT
        "J-15" | "J-16" => {
            assert_eq!(c2, Some(&ConditionStatus::Pass), "{ctx}");
            assert_eq!(codes(4), [Code::OverruledJudgement], "{ctx}");
            assert_eq!(c.state.as_ref().unwrap().phase, Phase::Rejected, "{ctx}");
        }
        "J-17" => {
            assert_eq!(c2, Some(&ConditionStatus::Fail), "{ctx}");
            assert!(codes(4).is_empty(), "{ctx}");
            assert_eq!(c.state.as_ref().unwrap().phase, Phase::Rejected, "{ctx}");
        }
        "J-12" => {
            assert!(has(3, Code::Malformed), "{ctx}");
            assert!(has(4, Code::UnlistedVerifier), "{ctx}");
            assert_eq!(c2, Some(&ConditionStatus::Unverified), "{ctx}");
        }
        other => panic!("no expectation for {other}"),
    }
}

const JUDGED: &[Scenario] = &[
    ("J-1", common::judged::j1),
    ("J-2", common::judged::j2),
    ("J-3", common::judged::j3),
    ("J-3b", common::judged::j3b),
    ("J-4", common::judged::j4),
    ("J-5", common::judged::j5),
    ("J-6", common::judged::j6),
    ("J-7", common::judged::j7),
    ("J-8", common::judged::j8),
    ("J-11", common::judged::j11),
    ("J-12", common::judged::j_declared_and_unlisted),
    ("J-13", common::judged::j13),
    ("J-14", common::judged::j14),
    ("J-15", common::judged::j15),
    ("J-16", common::judged::j16),
    ("J-17", common::judged::j17),
    ("J-18", common::judged::j18),
];

/// CR-001 §3.2: every vector's expected outcome, under every arrival-order permutation.
#[test]
fn cr001_judged_vectors_under_every_arrival_order() {
    for (name, build) in JUDGED {
        let w = build();
        let orders = orders(w.events.len());
        assert!(orders.len() > 1 || w.events.len() == 1);
        for order in orders {
            let shuffled: Vec<_> = order.iter().map(|&i| w.events[i].clone()).collect();
            check_judged(name, &w, &acn_core::fold(&shuffled, &w.docs));
        }
    }
}

/// `judged` needs acn-terms/0.3; 0.2 documents without it still parse.
#[test]
fn cr001_judged_requires_terms_0_3() {
    let judged = common::judged::judged_terms(|t| t["v"] = json!("acn-terms/0.2"));
    assert!(acn_core::Terms::parse(&judged).is_err());
    assert!(acn_core::Terms::parse(&terms(|t| t["v"] = json!("acn-terms/0.2"))).is_ok());
    assert!(acn_core::Terms::parse(&terms(|t| t["v"] = json!("acn-terms/9.9"))).is_err());
}

/// The validator rejects a judged condition satisfied by an interpretive report, by a
/// performer-run judge, or by a mismatched judge fingerprint (CR-001 "done when").
#[test]
fn cr001_judged_condition_cannot_be_satisfied_by_the_wrong_evidence() {
    for build in [common::judged::j2, common::judged::j3, common::judged::j4] {
        let w = build();
        let out = w.fold();
        let c = &out.commitments[&w.events[0].id];
        assert_ne!(c.conditions.get("c2"), Some(&ConditionStatus::Pass));
    }
}

// ---------- CR-002 Phase A vectors (spec §10 conformance table) ----------

fn check_settlement(name: &str, w: &W, out: &acn_core::Output) {
    let e = &w.events;
    let req = &e[0];
    let ctx = format!("{name}: {:?}", out.anomalies);
    let has = |i: usize, code: Code| out.has(&e[i].id, code);
    if name == "S-2" {
        assert!(out.has(&req.id, Code::InvalidSettlementCombo), "{ctx}");
        assert!(!out.commitments.contains_key(&req.id), "{ctx}");
        return;
    }
    let c = &out.commitments[&req.id];
    let st = c.state.as_ref().unwrap();
    match name {
        // e: REQUEST, ACK(stranger), PROMISE
        "S-4" => {
            assert!(has(1, Code::Unauthorized), "{ctx}");
            assert!(has(2, Code::MissingEscrowAck), "{ctx}");
            assert_eq!(st.phase, Phase::Requested, "{ctx}");
        }
        // e: REQUEST, ACK, PROMISE, REPORT, ATTEST, RELEASE, SATISFY
        "S-5" => {
            assert_eq!(st.phase, Phase::Settled, "{ctx}");
            assert!(st.judged, "SATISFY recorded as a judgement");
            assert_eq!(c.chain_kinds.last(), Some(&K_SATISFY), "{ctx}");
            assert!(status_at(c, 99_999).terminal);
            assert!(
                out.anomalies.iter().all(|a| a.code == Code::Unbound),
                "{ctx}"
            );
        }
        "S-6" | "S-7" | "S-7b" => {
            assert!(
                has(5 - usize::from(name == "S-7b"), Code::BadRelease),
                "{ctx}"
            );
            assert_eq!(st.phase, Phase::Reported, "{ctx}");
        }
        "S-8" => {
            assert_eq!(st.phase, Phase::Reported);
            let before = status_at(c, 1300);
            assert_eq!(before.awaiting, Some(Party::Verifier));
            assert_eq!(before.lapsed, None);
            let after = status_at(c, 1200 + 500 + 1);
            assert_eq!(after.lapsed, Some(Party::Verifier), "verifier fault");
        }
        // e: REQUEST, ACK, PROMISE, REPORT, SATISFY(illegal), REJECT(illegal)
        "S-8b" => {
            assert!(has(4, Code::IllegalTransition), "{ctx}");
            assert!(has(5, Code::IllegalTransition), "{ctx}");
            assert_eq!(st.phase, Phase::Reported, "{ctx}");
        }
        // e: REQUEST, ACK, PROMISE, REPORT, ATTEST(fail), REJECT
        "S-8c" => {
            assert_eq!(st.phase, Phase::Reworking, "{ctx}");
            assert_eq!(status_at(c, 1300).awaiting, Some(Party::Performer));
        }
        other => panic!("no expectation for {other}"),
    }
}

const SETTLEMENT: &[Scenario] = &[
    ("S-2", common::settlement::s2),
    ("S-4", common::settlement::s4),
    ("S-5", common::settlement::s5),
    ("S-6", common::settlement::s6),
    ("S-7", common::settlement::s7),
    ("S-7b", common::settlement::s7b),
    ("S-8", common::settlement::s8),
    ("S-8b", common::settlement::s8b),
    ("S-8c", common::settlement::s8c),
];

/// CR-002 §4.3 Phase A: every vector's expected outcome, under every arrival order.
#[test]
fn cr002_verifier_trigger_vectors_under_every_arrival_order() {
    for (name, build) in SETTLEMENT {
        let w = build();
        for order in orders(w.events.len()) {
            let shuffled: Vec<_> = order.iter().map(|&i| w.events[i].clone()).collect();
            check_settlement(name, &w, &acn_core::fold(&shuffled, &w.docs));
        }
    }
}

// e (contingent): REQUEST, PROMISE, REPORT(offer), then per vector.
fn check_contingent(name: &str, w: &W, out: &acn_core::Output) {
    let e = &w.events;
    let req = &e[0];
    let ctx = format!("{name}: {:?}", out.anomalies);
    let has = |i: usize, code: Code| out.has(&e[i].id, code);
    if name == "S-9" {
        assert!(out.has(&req.id, Code::ContingentNondeterministic), "{ctx}");
        assert!(!out.commitments.contains_key(&req.id), "{ctx}");
        return;
    }
    let c = &out.commitments[&req.id];
    let st = c.state.as_ref().unwrap();
    let tests = c.conditions.get("tests");
    match name {
        // + ATTEST(pass), FUND, CLAIM
        "S-11" => {
            assert_eq!(st.phase, Phase::Settled, "{ctx}");
            assert_eq!(
                c.chain_kinds,
                [K_REQUEST, K_PROMISE, K_REPORT, K_FUND, K_CLAIM],
                "{ctx}"
            );
            assert!(
                out.anomalies.iter().all(|a| a.code == Code::Unbound),
                "{ctx}"
            );
            assert!(status_at(c, 99_999).terminal);
        }
        "S-12" => {
            assert!(has(3, Code::AttestationMismatch), "{ctx}");
            assert_eq!(tests, Some(&ConditionStatus::Unverified), "not counted");
            assert_eq!(st.phase, Phase::Offered);
        }
        // + ATTEST, FUND, CLAIM(bad)
        "S-13" => {
            assert!(has(5, Code::BadKey), "{ctx}");
            assert_eq!(st.phase, Phase::Funded, "{ctx}");
        }
        "S-14" => {
            assert_eq!(st.phase, Phase::Offered);
            assert_eq!(c.passed_at, Some(1250));
            let before = status_at(c, 1500);
            assert_eq!(
                (before.awaiting, before.lapsed),
                (Some(Party::Requester), None)
            );
            let after = status_at(c, 1250 + 300 + 1);
            assert_eq!(after.lapsed, Some(Party::Requester), "requester fault");
        }
        // + FUND (no attestation)
        "S-15" => {
            assert!(has(3, Code::PrematureFund), "{ctx}");
            assert_eq!(st.phase, Phase::Funded, "valid, flagged");
        }
        // + ATTEST, FUND(wrong H)
        "S-16" => {
            assert!(has(4, Code::FundMismatch), "{ctx}");
            assert_eq!(st.phase, Phase::Offered, "{ctx}");
        }
        "S-17" => {
            assert_eq!(st.phase, Phase::Funded);
            assert_eq!(status_at(c, 4000).awaiting, Some(Party::Performer));
            assert_eq!(
                status_at(c, 5001).lapsed,
                Some(Party::Performer),
                "performer fault"
            );
        }
        // + ATTEST, SATISFY(illegal), REJECT(illegal)
        "S-18" => {
            assert!(
                has(4, Code::IllegalTransition) && has(5, Code::IllegalTransition),
                "{ctx}"
            );
            assert_eq!(st.phase, Phase::Offered);
        }
        other => panic!("no expectation for {other}"),
    }
}

const CONTINGENT: &[Scenario] = &[
    ("S-9", common::settlement::s9),
    ("S-11", common::settlement::s11),
    ("S-12", common::settlement::s12),
    ("S-13", common::settlement::s13),
    ("S-14", common::settlement::s14),
    ("S-15", common::settlement::s15),
    ("S-16", common::settlement::s16),
    ("S-17", common::settlement::s17),
    ("S-18", common::settlement::s18),
];

/// CR-002 §4.3 Phase C: every contingent vector's expected outcome, under every arrival order.
#[test]
fn cr002_contingent_vectors_under_every_arrival_order() {
    for (name, build) in CONTINGENT {
        let w = build();
        for order in orders(w.events.len()) {
            let shuffled: Vec<_> = order.iter().map(|&i| w.events[i].clone()).collect();
            check_contingent(name, &w, &acn_core::fold(&shuffled, &w.docs));
        }
    }
}

/// The other invalid combinations, and the version gate for the new fields.
#[test]
fn cr002_settlement_combinations() {
    use common::settlement::verifier_terms;
    let parse = |t: serde_json::Value| acn_core::Terms::parse(&t).unwrap();
    assert!(parse(verifier_terms(1, |_| {})).settlement_combo().is_ok());
    assert!(parse(common::settlement::contingent_terms(1, |_| {}))
        .settlement_combo()
        .is_ok());
    for (why, t) in [
        (
            "no verifier",
            verifier_terms(1, |t| t["settlement_verifier"] = json!(null)),
        ),
        (
            "no hashlock",
            verifier_terms(1, |t| t["payment"]["hashlock"] = json!(null)),
        ),
        (
            "arbiter",
            verifier_terms(1, |t| t["arbiter"] = json!(pk(&ARB))),
        ),
        (
            "interpretive",
            verifier_terms(1, |t| t["conditions"][0]["class"] = json!("interpretive")),
        ),
        (
            "no conditions",
            verifier_terms(1, |t| t["conditions"] = json!([])),
        ),
        (
            "contingent with a hashlock in the terms",
            verifier_terms(1, |t| t["settlement_trigger"] = json!("contingent")),
        ),
        (
            "contingent without fund_window",
            common::settlement::contingent_terms(1, |t| t["fund_window"] = json!(null)),
        ),
        (
            "contingent with an unlisted verifier",
            common::settlement::contingent_terms(1, |t| t["verifiers"] = json!([])),
        ),
        (
            "requester + verifier key",
            verifier_terms(1, |t| t["settlement_trigger"] = json!("requester")),
        ),
    ] {
        assert!(parse(t).settlement_combo().is_err(), "{why}");
    }
    // New fields need acn-terms/0.4; plain 0.3 terms still parse and default to requester.
    assert!(
        acn_core::Terms::parse(&verifier_terms(1, |t| t["v"] = json!("acn-terms/0.3"))).is_err()
    );
    let old = parse(terms(|t| t["v"] = json!("acn-terms/0.3")));
    assert_eq!(
        old.settlement_trigger,
        acn_core::terms::SettlementTrigger::Requester
    );
    // Defaults don't change the terms hash (S-1): the new fields aren't serialized.
    let v = serde_json::to_value(&old).unwrap();
    assert!(v.get("settlement_trigger").is_none() && v.get("settlement_verifier").is_none());
}

// ---------- gates ----------

#[test]
fn fold_is_order_independent() {
    for (name, build) in ALL {
        let w = build();
        let base = w.fold();
        for order in orders(w.events.len()) {
            let shuffled: Vec<_> = order.iter().map(|&i| w.events[i].clone()).collect();
            let out = acn_core::fold(&shuffled, &w.docs);
            assert_eq!(
                out, base,
                "scenario `{name}` differs for arrival order {order:?}"
            );
        }
        // Duplicated deliveries (multi-relay) change nothing either.
        let mut doubled = w.events.clone();
        doubled.extend(w.events.iter().cloned());
        assert_eq!(
            acn_core::fold(&doubled, &w.docs),
            base,
            "scenario `{name}` with duplicates"
        );
    }
}

#[test]
fn every_anomaly_code_is_exercised() {
    let mut seen = BTreeSet::new();
    for (_, build) in ALL {
        seen.extend(build().fold().anomalies.iter().map(|a| a.code));
    }
    let missing: Vec<_> = Code::ALL.iter().filter(|c| !seen.contains(c)).collect();
    assert!(missing.is_empty(), "codes without a scenario: {missing:?}");
}

#[test]
fn every_act_kind_is_exercised() {
    let mut seen = BTreeSet::new();
    for (_, build) in ALL {
        let out = build().fold();
        for c in out.commitments.values() {
            seen.extend(c.chain_kinds.iter().copied());
            if !c.progress.is_empty() {
                seen.insert(K_PROGRESS);
            }
        }
    }
    let missing: Vec<_> = ACTS
        .iter()
        .filter(|k| !seen.contains(k))
        .map(|k| name(*k))
        .collect();
    assert!(
        missing.is_empty(),
        "acts never applied in any scenario: {missing:?}"
    );
}

// ---------- expectations per scenario ----------

#[test]
fn t0_and_t1() {
    let w = t0_happy();
    let out = w.fold();
    let c = &out.commitments[&w.events[0].id];
    assert_eq!(c.profile, Profile::T0);
    assert_eq!(phase(&w, &w.events[0]), Phase::Satisfied);
    assert!(out.has(&w.events[1].id, Code::Unbound));
    assert_eq!(c.report.as_ref().unwrap()["result"], json!({"ok": true}));

    let w = t1_preimage();
    let out = w.fold();
    assert!(out.has(&w.events[2].id, Code::BadPreimage));
    assert!(out.has(&w.events[3].id, Code::Malformed));
    assert_eq!(phase(&w, &w.events[0]), Phase::Satisfied);
}

#[test]
fn negotiated_paths() {
    let w = n_full_rework();
    let c = &w.fold().commitments[&w.events[0].id];
    let st = c.state.as_ref().unwrap();
    assert_eq!(st.phase, Phase::Satisfied);
    assert_eq!(st.rework_count, 1);
    assert_eq!(c.progress.len(), 1);

    let w = n_counter_accept();
    let out = w.fold();
    assert!(out.has(&w.events[2].id, Code::Unauthorized));
    assert!(out.has(&w.events[3].id, Code::TermsMismatch));
    let st = out.commitments[&w.events[0].id].state.clone().unwrap();
    assert_eq!(st.phase, Phase::Promised);
    assert_eq!(
        st.terms,
        terms_hash(&terms(|t| t["deadline"] = json!(3000)))
    );

    let w = counter_while_promised();
    assert_eq!(phase(&w, &w.events[0]), Phase::Reported);

    let w = rework_limit_and_no_arbiter();
    let out = w.fold();
    assert_eq!(phase(&w, &w.events[0]), Phase::Rejected);
    assert!(out.has(&w.events[4].id, Code::IllegalTransition));
}

#[test]
fn disputes() {
    let w = dispute_ruling();
    let out = w.fold();
    assert!(out.has(&w.events[5].id, Code::Unauthorized));
    assert_eq!(phase(&w, &w.events[0]), Phase::Satisfied);
    assert!(
        out.commitments[&w.events[0].id]
            .state
            .as_ref()
            .unwrap()
            .ruled
    );

    let w = dispute_window_elapsed();
    assert!(w.fold().has(&w.events[4].id, Code::IllegalTransition));
}

#[test]
fn terminal_acts() {
    let w = terminals();
    let out = w.fold();
    let phases: BTreeSet<_> = out
        .commitments
        .values()
        .map(|c| c.state.as_ref().unwrap().phase as u8)
        .collect();
    assert_eq!(phases.len(), 3);
    for c in out.commitments.values() {
        assert!(status_at(c, 1500).terminal);
    }
}

#[test]
fn sibling_resolution() {
    let w = siblings();
    let out = w.fold();
    // r1: WITHDRAW (closing) beats PROMISE.
    assert_eq!(phase(&w, &w.events[0]), Phase::Withdrawn);
    assert!(out.has(&w.events[1].id, Code::Stale));
    // r2: PROMISE (term-preserving) beats COUNTER (term-changing).
    assert_eq!(phase(&w, &w.events[3]), Phase::Promised);
    assert!(out.has(&w.events[5].id, Code::Stale));

    let w = equivocation();
    let out = w.fold();
    assert_eq!(phase(&w, &w.events[0]), Phase::Declined);
    assert!(out.has(&w.events[1].id, Code::Equivocation));
    assert!(out.has(&w.events[2].id, Code::Equivocation));

    let w = off_chain();
    let out = w.fold();
    assert!(out.has(&w.events[1].id, Code::Stale));
    assert!(out.has(&w.events[3].id, Code::Stale));
}

#[test]
fn envelope_and_linkage() {
    let w = envelope_errors();
    let out = w.fold();
    assert!(out.has(&w.events[1].id, Code::BadSig));
    assert!(out.has(&w.events[2].id, Code::BadVersion));
    assert!(out.has(&w.events[3].id, Code::UnknownKind));
    assert!(out.has(&w.events[4].id, Code::Malformed));
    assert!(out.anomalies_for(&w.events[5].id).next().is_none());

    let w = linkage();
    let out = w.fold();
    assert!(out.has(&w.events[2].id, Code::ForeignPrev));
    assert!(out.has(&w.events[3].id, Code::PendingRef));
    assert!(out.has(&w.events[4].id, Code::PendingRef));

    let w = time_regress();
    assert!(w.fold().has(&w.events[1].id, Code::TimeRegress));

    let w = illegal_and_unauthorized();
    let out = w.fold();
    assert!(out.has(&w.events[1].id, Code::IllegalTransition));
    assert!(out.has(&w.events[2].id, Code::Unauthorized));
    assert!(out.has(&w.events[3].id, Code::IllegalTransition));
}

#[test]
fn terms_availability() {
    let mut w = pending_terms();
    let out = w.fold();
    assert!(out.commitments[&w.events[0].id].state.is_none());
    assert!(out.has(&w.events[1].id, Code::PendingTerms));
    // Supplying the document resolves it; a document under the wrong key is ignored.
    let t = terms(|t| t["deadline"] = json!(4242));
    w.docs.insert("ff".repeat(32), t.clone());
    assert!(w.fold().commitments[&w.events[0].id].state.is_none());
    w.docs.insert(terms_hash(&t), t);
    assert_eq!(phase(&w, &w.events[0]), Phase::Reported);

    let w = terms_mismatch_inline();
    let out = w.fold();
    assert!(out.has(&w.events[0].id, Code::TermsMismatch));
    assert!(out.commitments.is_empty());
}

#[test]
fn binding_scope_and_revocation() {
    let w = bindings();
    let out = w.fold();
    let bound = &out.commitments[&w.events[2].id];
    assert!(matches!(
        bound.performer_binding,
        acn_core::BindingStatus::Bound { .. }
    ));
    assert!(!out.has(&w.events[3].id, Code::Unbound));
    assert!(out.has(&w.events[6].id, Code::OutOfScope)); // price
    assert!(out.has(&w.events[8].id, Code::OutOfScope)); // kind

    let w = revoked_binding();
    let out = w.fold();
    assert!(!out
        .anomalies_for(&w.events[4].id)
        .any(|a| a.code == Code::Revoked));
    assert!(out.has(&w.events[5].id, Code::Revoked));
}

#[test]
fn evidence_classes() {
    let w = evidence();
    let out = w.fold();
    let c = &out.commitments[&w.events[0].id];
    // REJECT moved it on; conditions are evaluated against the current report (still rep).
    assert_eq!(c.state.as_ref().unwrap().phase, Phase::Rejected);
    assert_eq!(c.conditions["tests"], ConditionStatus::Pass);
    assert_eq!(
        c.conditions["quality"],
        ConditionStatus::Advisory {
            verdicts: vec!["good".into()]
        }
    );
    assert_eq!(c.conditions["ok"], ConditionStatus::Human);
    assert!(out.has(&w.events[4].id, Code::WrongEvidenceClass));
    assert!(out.has(&w.events[6].id, Code::UnlistedVerifier));
    assert!(out.has(&w.events[7].id, Code::ContradictedRejection));
}

#[test]
fn governance_endorsements() {
    let w = governance();
    let out = w.fold();
    assert_eq!(phase(&w, &w.events[0]), Phase::Reported);
    assert_eq!(phase(&w, &w.events[4]), Phase::Promised);
    assert!(out.has(&w.events[6].id, Code::PendingEndorsement));
    assert_eq!(phase(&w, &w.events[7]), Phase::Revoked);
    assert!(out.has(&w.events[9].id, Code::Unendorsed));
}

#[test]
fn time_status() {
    let w = late();
    let out = w.fold();
    assert!(out.has(&w.events[1].id, Code::Late));
    assert!(out.has(&w.events[2].id, Code::Late));

    // Lapse is derived, never announced.
    let mut w = W::default();
    let t = terms(|_| {});
    let r = request(&mut w, "N", &t);
    let p = w.add(d_promise(&r, &r, &terms_hash(&t)), &PERF, 1100);
    let c = w.fold().commitments[&r.id].clone();
    assert_eq!(status_at(&c, 1500).awaiting, Some(Party::Performer));
    assert_eq!(status_at(&c, 2001).lapsed, Some(Party::Performer));
    w.add(d_report(&r, &p), &PERF, 1200);
    let c = w.fold().commitments[&r.id].clone();
    assert_eq!(status_at(&c, 1600).awaiting, Some(Party::Requester));
    assert_eq!(status_at(&c, 1701).lapsed, Some(Party::Requester));
}
