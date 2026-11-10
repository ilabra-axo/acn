//! Every scenario plays to its intended end with the requester on autopilot — the same loop the
//! live simulator and presenter mode run, minus relays and timers.

use std::collections::BTreeMap;

use acn_core::draft::{instantiate, public_key, Draft};
use acn_core::kinds::*;
use acn_core::{Event, Phase};
use acn_sim::*;
use serde_json::json;

struct World {
    actor_keys: BTreeMap<Actor, [u8; 32]>,
    me: [u8; 32],
    cast: Cast,
    events: Vec<Event>,
    clock: u64,
    escrow: BTreeMap<String, String>,
    secret: [u8; 32],
    /// The performer's per-commitment secret (the host's job in live and presenter mode).
    keys: BTreeMap<String, String>,
    /// Requester seeds the runner received (held-out instances).
    seeds: BTreeMap<String, String>,
}

impl World {
    fn new() -> Self {
        let keys: BTreeMap<Actor, [u8; 32]> = Actor::ALL
            .iter()
            .enumerate()
            .map(|(i, a)| (*a, [i as u8 + 10; 32]))
            .collect();
        let pk = |a: Actor| public_key(&keys[&a]).unwrap();
        // The cast's public templates (CR-003 §2.5), as every host publishes them.
        let signed: Vec<(String, Event)> = templates()
            .into_iter()
            .map(|(name, a, d)| (name, d.sign(&keys[&a], 999_000).unwrap()))
            .collect();
        let events: Vec<Event> = signed.iter().map(|(_, e)| e.clone()).collect();
        let cast = Cast {
            performer: pk(Actor::Performer),
            rogue: pk(Actor::Rogue),
            verifier: pk(Actor::Verifier),
            judge: pk(Actor::Judge),
            arbiter: pk(Actor::Arbiter),
            gatekeeper: pk(Actor::Gatekeeper),
            mint: SIM_MINT.into(),
            judge_panel: None,
            templates: signed
                .iter()
                .map(|(n, e)| (n.clone(), e.id.clone()))
                .collect(),
        };
        Self {
            actor_keys: keys,
            me: [1; 32],
            cast,
            events,
            clock: 1_000_000,
            escrow: BTreeMap::new(),
            secret: [9; 32],
            keys: BTreeMap::new(),
            seeds: BTreeMap::new(),
        }
    }

    fn sign(&mut self, d: &Draft, key: [u8; 32], not_before: u64) -> Event {
        self.clock = self.clock.max(not_before) + 1;
        let e = d.sign(&key, self.clock).unwrap();
        self.events.push(e.clone());
        e
    }

    fn request(&mut self, id: &str) -> String {
        let info = scenario(id).unwrap();
        let t = template(id, &self.cast).unwrap();
        let h = acn_core::jcs::sha256_hex(&self.secret);
        let seed = [0x5e; 32];
        let seed_hash = acn_core::jcs::sha256_hex(&seed);
        let terms = instantiate(
            &t,
            id,
            json!({"brief": "the demo"}),
            self.clock,
            t.hashlock.then_some(h.as_str()),
            Some(&seed_hash),
        )
        .unwrap();
        let d = Draft::request(self.cast.key(info.actor), &t.profile, &terms, None).unwrap();
        let r = self.sign(&d, self.me, 0);
        // The secret's holder: the arbiter, or the settlement verifier (verifier trigger).
        let holder = t.terms["arbiter"].is_string()
            || (t.terms["settlement_trigger"] == "verifier"
                && t.terms["settlement_verifier"].is_string());
        if holder && t.hashlock {
            self.escrow.insert(r.id.clone(), hex(&self.secret));
        }
        // The performer host keeps one secret per commitment (`k` when contingent).
        self.keys.insert(r.id.clone(), hex(&[0x6b; 32]));
        // The requester gives its seed to the runner (a NIP-44 `seed` tag in the live host).
        if t.requester_seed_to.is_some() {
            self.seeds.insert(r.id.clone(), hex(&seed));
        }
        r.id
    }

    /// Run actors and requester autopilot until nothing happens.
    fn play(&mut self, c: &str) -> Vec<String> {
        let mut narration = vec![];
        for _ in 0..40 {
            let moves = react_with(
                &self.cast,
                &self.events,
                &self.escrow,
                None,
                &self.keys,
                &self.seeds,
            );
            if !moves.is_empty() {
                for m in moves {
                    narration.push(m.narration.clone());
                    if let Some(e) = &m.escrow {
                        // The holder receives the escrowed key (NIP-44 in the live host).
                        assert_eq!(e.to, Actor::Verifier);
                        self.escrow.insert(c.to_string(), e.secret.clone());
                    }
                    let key = self.actor_keys[&m.actor];
                    self.sign(&m.draft, key, m.not_before);
                }
                continue;
            }
            let hint = requester_hint(&self.events, c, self.clock).unwrap();
            match &hint.action {
                HintAction::Done => break,
                HintAction::Wait => panic!("stuck waiting: {}", hint.narration),
                HintAction::AdvanceClock { secs } => self.clock += secs,
                action => {
                    let pre = hex(&self.secret);
                    let (d, nb) = requester_draft(&self.events, c, action, Some(&pre)).unwrap();
                    self.sign(&d, self.me, nb);
                }
            }
            narration.push(hint.narration);
        }
        narration
    }

    fn view(&self, c: &str) -> acn_view::Projection {
        let me = public_key(&self.me).unwrap();
        acn_view::project(&self.events, &me, self.clock)
            .into_iter()
            .find(|p| p.id == c)
            .unwrap()
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn run(id: &str) -> acn_view::Projection {
    let mut w = World::new();
    let c = w.request(id);
    let story = w.play(&c);
    assert!(!story.is_empty(), "{id}: nothing happened");
    w.view(&c)
}

fn codes(p: &acn_view::Projection) -> Vec<&'static str> {
    p.anomalies.iter().map(|a| a.code).collect()
}

#[test]
fn every_scenario_has_a_template_and_reaches_an_end() {
    for s in SCENARIOS {
        let p = run(s.id);
        assert!(p.terminal, "{}: ended in {:?}", s.id, p.phase);
    }
}

#[test]
fn scenario_outcomes() {
    let acts = |p: &acn_view::Projection| p.chain.iter().map(|e| e.act).collect::<Vec<_>>();

    let p = run("quick");
    assert_eq!(acts(&p), ["REQUEST", "REPORT", "SATISFY"]);

    let p = run("paid");
    assert_eq!(p.phase, Some(Phase::Satisfied));
    let s = p.settlement.unwrap();
    assert!(s.simulated && s.amount_sat == 21 && s.status == acn_view::SettlementStatus::Paid);

    let p = run("negotiate");
    assert_eq!(
        acts(&p),
        ["REQUEST", "COUNTER", "ACCEPT", "REPORT", "SATISFY"]
    );
    assert!(p
        .timeline
        .iter()
        .any(|e| e.act == "PROGRESS" && e.status == acn_view::EntryStatus::Annotation));

    let p = run("rework");
    assert_eq!(p.phase, Some(Phase::Satisfied));
    assert_eq!(p.rework_count, 1);
    assert_eq!(p.report.unwrap().version, 2);

    let p = run("verified");
    assert!(codes(&p).contains(&"contradicted_rejection"));
    assert!(codes(&p).contains(&"wrong_evidence_class"));
    assert_eq!(p.phase, Some(Phase::Satisfied));

    let p = run("dispute");
    assert_eq!(acts(&p).last(), Some(&"RULING"));
    assert_eq!(
        p.settlement.unwrap().status,
        acn_view::SettlementStatus::Paid
    );

    let p = run("governed");
    assert_eq!(p.phase, Some(Phase::Satisfied));
    assert!(p.timeline.iter().any(|e| e.act == "ENDORSE"));

    let p = run("silent");
    assert_eq!(p.phase, Some(Phase::Withdrawn));

    // CR-003: Atlas's counter closes the open condition; the version in force is Atlas's.
    let p = run("drafted");
    assert_eq!(p.phase, Some(Phase::Satisfied), "{:?}", acts(&p));
    assert_eq!(acts(&p)[..3], ["REQUEST", "COUNTER", "ACCEPT"]);
    assert!(p
        .disqualifiers
        .iter()
        .all(|d| d.status == Some(acn_core::DisqualifierStatus::Clear)));
    let card = p.terms_card.as_ref().expect("terms card");
    assert_eq!(
        (card.version, card.drafted_by_role),
        (2, acn_view::Role::Performer)
    );
    assert_eq!(p.versions.len(), 2);
    // CR-003 Phase C: Atlas's counter is an instantiation of its published template.
    let tv = card.template.as_ref().expect("drafted from a template");
    assert_eq!(tv.status, acn_core::template::TemplateStatus::Counted);
    assert_eq!(tv.name.as_deref(), Some("retry-change/v1"));
    assert_eq!((tv.record.commitments, tv.record.paid), (1, 1));
    let p = run("disqualified");
    assert_eq!(p.phase, Some(Phase::Rejected), "{:?}", acts(&p));
    assert!(codes(&p).contains(&"disqualified"));
    assert_eq!(p.rework_count, 0, "breach: no rework");
    let r = &p
        .terms_card
        .as_ref()
        .unwrap()
        .template
        .as_ref()
        .unwrap()
        .record;
    assert_eq!((r.commitments, r.rejected, r.disqualified), (1, 1, 1));

    // CR-003 Phase B: the runner attests the sample both seeds select; it counts.
    let p = run("held-out");
    assert_eq!(
        p.phase,
        Some(Phase::Satisfied),
        "{:?} {:?}",
        acts(&p),
        codes(&p)
    );
    assert!(codes(&p).iter().all(|c| *c == "unbound"), "{:?}", codes(&p));
    let card = p.terms_card.as_ref().unwrap();
    let i = card.done_means[0]
        .instances
        .as_ref()
        .expect("pool shown on the card");
    assert_eq!((i.pool_size, i.sample_size), (20, 5));
    // A judged condition with its own held-out pool: the judges' attestation reveals 3 review
    // cases, drawn by the same seeds, each proved against that pool's root (V-27 counts it).
    let r = card.done_means[1]
        .instances
        .as_ref()
        .expect("review pool shown on the card");
    assert_eq!((r.pool_size, r.sample_size), (8, 3));
    let c2 = p.conditions.iter().find(|c| c.id == "c2").unwrap();
    assert_eq!(
        c2.status,
        Some(acn_core::ConditionStatus::Pass),
        "{:?}",
        codes(&p)
    );

    let p = run("rogue");
    assert_eq!(p.phase, Some(Phase::Declined));
    assert!(codes(&p).contains(&"equivocation"));
    assert!(
        codes(&p).contains(&"unbound")
            || !matches!(p.binding, acn_core::BindingStatus::Bound { .. })
    );

    // CR-001 MVP: the verifier runs both judges and releases.
    let p = run("judged");
    assert_eq!(p.phase, Some(Phase::Settled), "{:?}", acts(&p));
    let tl = p.timeline.iter().map(|e| e.act).collect::<Vec<_>>();
    assert!(
        tl.contains(&"ATTEST_JUDGED") && tl.contains(&"RELEASE"),
        "{tl:?} / {:?}",
        acts(&p)
    );

    let p = run("injected");
    assert_eq!(p.phase, Some(Phase::Rejected), "{:?}", acts(&p));
    assert!(codes(&p).contains(&"injection_detected"), "{:?}", codes(&p));
    assert!(!acts(&p).contains(&"RELEASE"));

    let p = run("judged-auto");
    assert_eq!(p.phase, Some(Phase::Satisfied), "{:?}", acts(&p));
    // Spec §12: what payment depends on, per trigger.
    use acn_view::PaymentDependency as D;
    assert_eq!(p.payment_depends_on, Some(D::Requester { arbiter: None }));
    assert!(matches!(
        run("judged").payment_depends_on,
        Some(D::Verifier { .. })
    ));
    assert!(matches!(
        run("contingent").payment_depends_on,
        Some(D::Contingent { .. })
    ));
    assert!(matches!(
        run("dispute").payment_depends_on,
        Some(D::Requester { arbiter: Some(_) })
    ));
    assert_eq!(run("quick").payment_depends_on, None);

    // CR-002 contingent: sealed until CLAIM reveals k.
    let p = run("contingent");
    assert_eq!(p.phase, Some(Phase::Settled), "{:?}", acts(&p));
    assert!(acts(&p).contains(&"FUND") && acts(&p).contains(&"CLAIM"));
    assert_eq!(p.report.unwrap().payload["title"], "Parser module");
}

/// Lanes and whose-move as a scenario plays (deduplicated consecutive states).
fn lanes(id: &str) -> Vec<(acn_view::Lane, Option<acn_view::Role>)> {
    let mut w = World::new();
    let c = w.request(id);
    let mut seen = vec![];
    let mut note = |w: &World| {
        let p = w.view(&c);
        let s = (p.lane, p.next);
        if seen.last() != Some(&s) {
            seen.push(s);
        }
    };
    note(&w);
    for _ in 0..40 {
        let moves = react_with(&w.cast, &w.events, &w.escrow, None, &w.keys, &w.seeds);
        if !moves.is_empty() {
            for m in moves {
                if let Some(e) = &m.escrow {
                    w.escrow.insert(c.clone(), e.secret.clone());
                }
                let key = w.actor_keys[&m.actor];
                w.sign(&m.draft, key, m.not_before);
                note(&w);
            }
            continue;
        }
        let hint = requester_hint(&w.events, &c, w.clock).unwrap();
        match &hint.action {
            HintAction::Done => break,
            HintAction::Wait => panic!("stuck"),
            HintAction::AdvanceClock { secs } => w.clock += secs,
            action => {
                let pre = hex(&w.secret);
                let (d, nb) = requester_draft(&w.events, &c, action, Some(&pre)).unwrap();
                let me = w.me;
                w.sign(&d, me, nb);
            }
        }
        note(&w);
    }
    seen
}

#[test]
fn board_lanes_follow_the_workflow() {
    use acn_view::{Lane::*, Role::*};
    assert_eq!(
        lanes("negotiate"),
        [
            (Requested, Some(Performer)),
            (Negotiating, Some(Requester)),
            (InProgress, Some(Performer)),
            (Review, Some(Requester)),
            (Closed, None)
        ]
    );
    assert_eq!(
        lanes("dispute"),
        [
            (Requested, Some(Performer)),
            (InProgress, Some(Performer)),
            (Review, Some(Requester)),
            (Dispute, Some(Performer)),
            (Dispute, Some(Arbiter)),
            (Closed, None)
        ]
    );
    let gov = lanes("governed");
    assert!(gov.contains(&(InProgress, Some(Governance))), "{gov:?}");
    assert_eq!(
        lanes("quick"),
        [
            (Requested, Some(Performer)),
            (Review, Some(Requester)),
            (Closed, None)
        ]
    );
    // Overdue work stays in progress (lapse is derived, not a state).
    let silent = lanes("silent");
    assert_eq!(silent.last(), Some(&(Closed, None)));

    for id in ["judged", "injected", "judged-auto", "contingent"] {
        let l = lanes(id);
        assert_eq!(l.last(), Some(&(Closed, None)), "{id}: {l:?}");
    }

    // needs_me is relative to the viewer's role.
    let mut w = World::new();
    let c = w.request("quick");
    let me = public_key(&w.me).unwrap();
    let p = acn_view::project(&w.events, &me, w.clock)
        .into_iter()
        .find(|p| p.id == c)
        .unwrap();
    assert!(!p.needs_me && p.my_role == Requester);
    let atlas = w.cast.performer.clone();
    let p = acn_view::project(&w.events, &atlas, w.clock)
        .into_iter()
        .find(|p| p.id == c)
        .unwrap();
    assert!(p.needs_me && p.my_role == Performer);
}

#[test]
fn performer_waits_for_and_obeys_the_payment_check() {
    let mut w = World::new();
    let c = w.request("paid");
    let none = BTreeMap::new();
    assert!(
        react_paid(&w.cast, &w.events, &w.escrow, &none).is_empty(),
        "waits for a verdict"
    );

    let bad = BTreeMap::from([(
        c.clone(),
        PaymentCheck::Invalid {
            reason: "token already spent".into(),
        },
    )]);
    let m = react_paid(&w.cast, &w.events, &w.escrow, &bad);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].draft.kind, K_DECLINE);
    assert!(m[0].draft.content.contains("payment_invalid"));

    let ok = BTreeMap::from([(c.clone(), PaymentCheck::Ok)]);
    let m = react_paid(&w.cast, &w.events, &w.escrow, &ok);
    assert_eq!(m[0].draft.kind, K_REPORT);

    // Free scenarios never wait.
    let mut w = World::new();
    w.request("quick");
    assert!(!react_paid(&w.cast, &w.events, &w.escrow, &none).is_empty());
}

#[test]
fn reactions_are_idempotent() {
    let mut w = World::new();
    w.request("verified");
    let a = react(&w.cast, &w.events, &w.escrow);
    let b = react(&w.cast, &w.events, &w.escrow);
    let keys = |m: &[Move]| m.iter().map(|m| m.key.clone()).collect::<Vec<_>>();
    assert_eq!(keys(&a), keys(&b));
    // Once performed, the same move is not proposed again.
    for m in a {
        let k = w.actor_keys[&m.actor];
        w.sign(&m.draft, k, m.not_before);
    }
    let after = react(&w.cast, &w.events, &w.escrow);
    assert!(after.iter().all(|m| m.draft.kind != K_PROMISE));
}
