//! ACN demo scenarios.
//!
//! The simulator never decides commitment semantics: every reaction is computed from
//! `acn_core::fold` over the signed events seen so far, and every act it emits is an ordinary
//! signed ACN event that the requester's validator judges like any other (INV-12).
//!
//! - [`react`]: what the simulated actors do next (performer, verifier, judge, arbiter,
//!   gatekeeper, rogue), with a pacing delay and a narration line.
//! - [`requester_hint`]: what the requester (the human presenting) should do next, so a
//!   presenter can click it or let autopilot do it.
//!
//! Scenarios are selected by the requested tool name (terms `deliverable.tool`).

use std::collections::BTreeMap;

use acn_core::draft::{Draft, Template};
use acn_core::kinds::*;
use acn_core::{fold, status_at, Commitment, Event, Party, Phase, State};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Placeholder mint of presenter mode (an in-memory `acn_wallet::SimMint`). Live hosts put a real
/// Cashu mint URL in [`Cast::mint`].
pub const SIM_MINT: &str = "sim://mint";

fn default_mint() -> String {
    SIM_MINT.into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    /// "Atlas": an operator-bound performer agent.
    Performer,
    /// "Rogue": an unbound performer that equivocates.
    Rogue,
    /// "Verity": mechanical verifier (re-runs tests).
    Verifier,
    /// "Muse": LLM judge (interpretive opinions).
    Judge,
    /// "Solon": arbiter.
    Arbiter,
    /// "Gate": governance key that endorses the performer's acts.
    Gatekeeper,
}

impl Actor {
    pub const ALL: [Actor; 6] = [
        Actor::Performer,
        Actor::Rogue,
        Actor::Verifier,
        Actor::Judge,
        Actor::Arbiter,
        Actor::Gatekeeper,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Actor::Performer => "Atlas",
            Actor::Rogue => "Rogue",
            Actor::Verifier => "Verity",
            Actor::Judge => "Muse",
            Actor::Arbiter => "Solon",
            Actor::Gatekeeper => "Gate",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Actor::Performer => "Simulated performer agent (ACN demo). Bound to an operator key.",
            Actor::Rogue => {
                "Simulated misbehaving agent (ACN demo). Unbound, sends conflicting acts."
            }
            Actor::Verifier => "Simulated mechanical verifier: re-runs tests and attests results.",
            Actor::Judge => "Simulated LLM judge: gives opinions that never count as provenance.",
            Actor::Arbiter => "Simulated arbiter: rules on disputes, holds the escrowed preimage.",
            Actor::Gatekeeper => "Simulated governance gate: endorses the performer's reports.",
        }
    }
}

/// Public keys of the simulated cast (hex, x-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cast {
    pub performer: String,
    pub rogue: String,
    pub verifier: String,
    pub judge: String,
    pub arbiter: String,
    pub gatekeeper: String,
    /// Mint named in priced templates (`payment.mint`).
    #[serde(default = "default_mint")]
    pub mint: String,
}

/// The performer's verdict on a priced REQUEST's token (Core.md §8.2 step 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentCheck {
    /// Verified: DLEQ, unspent, matches the terms, locked to the performer.
    Ok,
    /// The performer must not work on it.
    Invalid { reason: String },
}

impl Cast {
    pub fn key(&self, a: Actor) -> &str {
        match a {
            Actor::Performer => &self.performer,
            Actor::Rogue => &self.rogue,
            Actor::Verifier => &self.verifier,
            Actor::Judge => &self.judge,
            Actor::Arbiter => &self.arbiter,
            Actor::Gatekeeper => &self.gatekeeper,
        }
    }
    pub fn actor_of(&self, pk: &str) -> Option<Actor> {
        Actor::ALL.into_iter().find(|a| self.key(*a) == pk)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioInfo {
    pub id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    /// Which agent offers it.
    pub actor: Actor,
    pub profile: &'static str,
    /// Spec features the scenario demonstrates.
    pub shows: &'static [&'static str],
}

pub const SCENARIOS: &[ScenarioInfo] = &[
    ScenarioInfo { id: "quick", title: "Quick tool call", actor: Actor::Performer, profile: "T0",
        summary: "A free transactional call: REQUEST → signed REPORT → you SATISFY.",
        shows: &["T0", "operator binding", "signed REPORT"] },
    ScenarioInfo { id: "paid", title: "Paid call (Cashu)", actor: Actor::Performer, profile: "T1",
        summary: "21 sat hash-locked to the performer, who verifies the token before working. Your SATISFY reveals the preimage: closing is paying.",
        shows: &["T1", "NUT-14 hashlock", "token verified", "INV-6 closure is settlement"] },
    ScenarioInfo { id: "negotiate", title: "Negotiation", actor: Actor::Performer, profile: "N",
        summary: "Atlas counters with a later deadline and a revision round; you accept, it promises, reports.",
        shows: &["COUNTER", "ACCEPT", "PROGRESS", "terms by hash"] },
    ScenarioInfo { id: "rework", title: "Rework loop", actor: Actor::Performer, profile: "N",
        summary: "The first report misses a section; you reject naming the condition; Atlas reworks; you accept.",
        shows: &["REJECT with gaps", "Reworking", "rework limit"] },
    ScenarioInfo { id: "verified", title: "Verified work", actor: Actor::Performer, profile: "N",
        summary: "Verity attests the tests pass; Muse gives opinions. Rejecting a verified condition is flagged.",
        shows: &["mechanical attestation", "INV-8 opinions don't count", "V-16 contradicted rejection"] },
    ScenarioInfo { id: "dispute", title: "Dispute & ruling", actor: Actor::Performer, profile: "N",
        summary: "50 sat escrowed with arbiter Solon. You reject; Atlas disputes; Solon rules and releases the preimage.",
        shows: &["DISPUTE", "RULING", "escrowed preimage (§8.4)"] },
    ScenarioInfo { id: "governed", title: "Governance gate", actor: Actor::Performer, profile: "N",
        summary: "Atlas's REPORT only takes effect once Gate endorses it.",
        shows: &["K_ENDORSE", "pending_endorsement", "V-14"] },
    ScenarioInfo { id: "silent", title: "Silent performer", actor: Actor::Performer, profile: "N",
        summary: "Atlas promises and goes quiet. Fast-forward the clock: the promise lapses (derived, never announced).",
        shows: &["status_at", "Lapsed", "WITHDRAW"] },
    ScenarioInfo { id: "rogue", title: "Rogue agent", actor: Actor::Rogue, profile: "N",
        summary: "An unbound agent sends PROMISE and DECLINE at once. The validator resolves it and flags both.",
        shows: &["unbound", "V-13 equivocation", "sibling resolution"] },
];

pub fn scenario(id: &str) -> Option<&'static ScenarioInfo> {
    SCENARIOS.iter().find(|s| s.id == id)
}

/// Terms template for a scenario, published in the agent's `K_SVC`.
pub fn template(id: &str, cast: &Cast) -> Option<Template> {
    let result =
        json!({"id": "result", "class": "human", "description": "The result is what I asked for"});
    let base = |conditions: Value| {
        json!({
            "v": acn_core::terms::TERMS_VERSION,
            "deliverable": {"description": scenario(id).map(|s| s.title).unwrap_or(id)},
            "conditions": conditions,
            "rework_limit": 0,
            "price": {"amount": 0, "unit": "sat"},
            "payment": null,
            "verifiers": [],
            "arbiter": null,
            "subcontracting": false,
            "disclosure": "private",
            "governance": null
        })
    };
    let (profile, deadline, review, terms, hashlock) = match id {
        "quick" => ("T0", 300, 3600, base(json!([result])), false),
        "paid" => {
            let mut t = base(json!([result]));
            t["price"] = json!({"amount": 21, "unit": "sat"});
            t["payment"] = json!({"mint": cast.mint, "keysets": []});
            ("T1", 300, 3600, t, true)
        }
        "negotiate" => ("N", 600, 3600, base(json!([result])), false),
        "rework" => {
            let mut t = base(
                json!([{"id": "sections", "class": "human", "description": "Covers every requested section"}]),
            );
            t["rework_limit"] = json!(1);
            ("N", 900, 3600, t, false)
        }
        "verified" => {
            let mut t = base(json!([
                {"id": "tests", "class": "mechanical", "description": "Unit tests pass"},
                {"id": "quality", "class": "interpretive", "description": "Readable, idiomatic code"}
            ]));
            t["rework_limit"] = json!(1);
            t["verifiers"] = json!([
                {"pubkey": cast.verifier, "classes": ["mechanical"]},
                {"pubkey": cast.judge, "classes": ["interpretive"]}
            ]);
            ("N", 900, 3600, t, false)
        }
        "dispute" => {
            let mut t =
                base(json!([{"id": "fit", "class": "human", "description": "Fits the brief"}]));
            t["price"] = json!({"amount": 50, "unit": "sat"});
            t["payment"] = json!({"mint": cast.mint, "keysets": []});
            t["arbiter"] = json!(cast.arbiter);
            ("N", 900, 3600, t, true)
        }
        "governed" => {
            let mut t = base(json!([result]));
            t["governance"] = json!({"keys": [cast.gatekeeper], "kinds": [K_REPORT], "window": 600, "party": "performer"});
            ("N", 900, 3600, t, false)
        }
        "silent" => ("N", 120, 3600, base(json!([result])), false),
        "rogue" => ("N", 600, 3600, base(json!([result])), false),
        _ => return None,
    };
    Some(Template {
        profile: profile.into(),
        deadline_secs: deadline,
        review_window_secs: review,
        terms,
        hashlock,
        locktime_margin_secs: 24 * 60 * 60,
    })
}

/// One simulated act to perform after `delay_ms`.
#[derive(Debug, Clone, Serialize)]
pub struct Move {
    pub actor: Actor,
    /// Stable key so hosts don't schedule the same move twice.
    pub key: String,
    pub delay_ms: u64,
    pub narration: String,
    pub draft: Draft,
    /// `created_at` must be ≥ this (V-15).
    pub not_before: u64,
}

struct Ctx<'a> {
    c: &'a Commitment,
    st: &'a State,
    members: Vec<&'a Event>,
    scenario: &'a str,
    parties: [&'a str; 2],
}

impl Ctx<'_> {
    fn exists(&self, kind: u16, by: &str, f: impl Fn(&Event) -> bool) -> bool {
        self.members
            .iter()
            .any(|e| e.kind == kind && e.pubkey == by && f(e))
    }
    fn prev_is(e: &Event, id: &str) -> bool {
        e.tags.iter().any(|t| {
            t.first().map(String::as_str) == Some("e")
                && t.get(1).map(String::as_str) == Some(id)
                && t.get(3).map(String::as_str) == Some("prev")
        })
    }
    fn on_report(e: &Event, report: &str, cond: &str) -> bool {
        let r = e.tags.iter().any(|t| {
            t.first().map(String::as_str) == Some("e")
                && t.get(1).map(String::as_str) == Some(report)
                && t.get(3).map(String::as_str) == Some("report")
        });
        let c = e.tags.iter().any(|t| {
            t.first().map(String::as_str) == Some("cond")
                && t.get(1).map(String::as_str) == Some(cond)
        });
        r && c
    }
    fn reports(&self) -> usize {
        self.c
            .chain_kinds
            .iter()
            .filter(|k| **k == K_REPORT)
            .count()
    }
    fn brief(&self) -> String {
        let input = self
            .c
            .terms
            .as_ref()
            .and_then(|t| t.deliverable.input.clone())
            .unwrap_or(Value::Null);
        input
            .as_object()
            .and_then(|o| o.values().find_map(|v| v.as_str().map(str::to_owned)))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "the brief".into())
    }
}

fn mv(actor: Actor, delay_ms: u64, narration: impl Into<String>, draft: Draft, ctx: &Ctx) -> Move {
    Move {
        actor,
        key: format!(
            "{actor:?}:{}:{}:{}",
            draft.kind,
            ctx.st.head,
            draft
                .tags
                .iter()
                .find(|t| t[0] == "cond")
                .map(|t| t[1].as_str())
                .unwrap_or("")
        ),
        delay_ms,
        narration: narration.into(),
        draft,
        not_before: ctx.st.head_at,
    }
}

/// What the simulated cast does next. `escrow` is the preimage the arbiter holds (if any).
/// Payments are not checked; see [`react_paid`].
pub fn react(cast: &Cast, events: &[Event], escrow: &BTreeMap<String, String>) -> Vec<Move> {
    react_inner(cast, events, escrow, None)
}

/// Like [`react`], but the performer only works on a priced commitment once its token is
/// verified (`payments[commitment] = Ok`), declines with `payment_invalid` if it is not, and
/// waits while the verdict is missing (the host is still checking with the mint).
pub fn react_paid(
    cast: &Cast,
    events: &[Event],
    escrow: &BTreeMap<String, String>,
    payments: &BTreeMap<String, PaymentCheck>,
) -> Vec<Move> {
    react_inner(cast, events, escrow, Some(payments))
}

fn react_inner(
    cast: &Cast,
    events: &[Event],
    escrow: &BTreeMap<String, String>,
    payments: Option<&BTreeMap<String, PaymentCheck>>,
) -> Vec<Move> {
    let out = fold(events, &BTreeMap::new());
    let mut moves = Vec::new();
    for c in out.commitments.values() {
        let (Some(st), Some(terms)) = (&c.state, &c.terms) else {
            continue;
        };
        let Some(scenario) = terms.deliverable.tool.as_deref() else {
            continue;
        };
        let Some(performer) = cast
            .actor_of(&c.performer)
            .filter(|a| matches!(a, Actor::Performer | Actor::Rogue))
        else {
            continue;
        };
        let members: Vec<&Event> = events
            .iter()
            .filter(|e| {
                e.id == c.id
                    || e.tags
                        .iter()
                        .any(|t| t[0] == "c" && t.get(1) == Some(&c.id))
            })
            .collect();
        let ctx = Ctx {
            c,
            st,
            members,
            scenario,
            parties: [c.requester.as_str(), c.performer.as_str()],
        };
        let priced = terms.payment.is_some() && terms.price.amount > 0;
        let check = match payments {
            Some(p) if priced => p.get(&c.id),
            _ => Some(&PaymentCheck::Ok),
        };
        if st.phase == Phase::Requested && performer == Actor::Performer {
            match check {
                None => continue,
                Some(PaymentCheck::Invalid { reason }) => {
                    if ctx.exists(K_DECLINE, &c.performer, |_| true) {
                        continue;
                    }
                    let d =
                        Draft::closing(K_DECLINE, &c.id, &st.head, &ctx.parties, "payment_invalid");
                    moves.push(mv(
                        performer,
                        1200,
                        format!("Atlas checks the token with the mint and declines: {reason}. It won't work unpaid."),
                        d,
                        &ctx,
                    ));
                    continue;
                }
                Some(PaymentCheck::Ok) => {}
            }
        }
        moves.extend(react_one(
            cast,
            performer,
            &ctx,
            escrow.get(&c.id),
            payments.is_some() && priced,
        ));
    }
    moves
}

fn react_one(
    cast: &Cast,
    performer: Actor,
    ctx: &Ctx,
    escrow: Option<&String>,
    token_verified: bool,
) -> Vec<Move> {
    let c = &ctx.c.id;
    let st = ctx.st;
    let head = st.head.as_str();
    let p = ctx.c.performer.as_str();
    let parties = &ctx.parties[..];
    let fresh = |kind: u16| !ctx.exists(kind, p, |e| Ctx::prev_is(e, head));
    let mut out = Vec::new();

    let report_draft = |version: usize| -> Draft {
        let brief = ctx.brief();
        let (title, body, note) = match (ctx.scenario, version) {
            ("rework", 1) => (
                "Landing page · draft 1",
                format!("Hero and features for {brief}. (Pricing section still to do.)"),
                "Draft 1 delivered",
            ),
            ("rework", _) => (
                "Landing page · draft 2",
                format!("Hero, features and pricing for {brief}."),
                "Revised: added the missing pricing section",
            ),
            ("verified", 1) => (
                "Parser module",
                format!("Implemented {brief}; 42 unit tests included."),
                "Code delivered with tests",
            ),
            ("verified", _) => (
                "Parser module · v2",
                format!("Implemented {brief}; tests re-run, edge cases documented."),
                "Revised after review",
            ),
            ("paid", _) => (
                "Paid result",
                format!("Here is the paid answer for {brief}."),
                "Result delivered; redeemable once you satisfy",
            ),
            ("dispute", _) => (
                "Logo concepts",
                format!("Three logo concepts for {brief}."),
                "Concepts delivered",
            ),
            _ => ("Done", format!("Completed: {brief}."), "Result delivered"),
        };
        let mut d = Draft::report(
            c,
            head,
            parties,
            Some(json!({"title": title, "body": body})),
            Some("nmesh.card/v1"),
            &[],
        );
        let mut v: Value = serde_json::from_str(&d.content).unwrap();
        v["note"] = json!(note);
        d.content = v.to_string();
        d
    };

    match (performer, st.phase) {
        // Rogue: conflicting acts against the same head (V-13); DECLINE wins as a closing act.
        (Actor::Rogue, Phase::Requested) if fresh(K_PROMISE) && fresh(K_DECLINE) => {
            out.push(mv(
                Actor::Rogue,
                900,
                "Rogue sends a PROMISE to one relay…",
                Draft::promise(c, head, parties, &st.terms, 0.99, false),
                ctx,
            ));
            out.push(mv(
                Actor::Rogue,
                950,
                "…and a conflicting DECLINE to another. Same signer, same prev.",
                Draft::closing(K_DECLINE, c, head, parties, "changed_mind"),
                ctx,
            ));
        }
        (Actor::Rogue, _) => {}

        // Transactional: REPORT straight from Requested (implicit promise).
        (_, Phase::Requested)
            if matches!(ctx.c.profile, acn_core::Profile::T0 | acn_core::Profile::T1)
                && fresh(K_REPORT) =>
        {
            out.push(mv(
                performer,
                1400,
                if token_verified {
                    "Atlas verified the locked token with the mint, runs the tool and signs a REPORT (an implicit promise in T1)."
                } else {
                    "Atlas runs the tool and signs a REPORT (an implicit promise in T0/T1)."
                },
                report_draft(1),
                ctx,
            ));
        }

        // Negotiation: counter first.
        (_, Phase::Requested) if ctx.scenario == "negotiate" => {
            if !ctx.exists(K_COUNTER, p, |_| true) {
                let mut t = serde_json::to_value(ctx.c.terms.as_ref().unwrap()).unwrap();
                t["deadline"] = json!(ctx.c.terms.as_ref().unwrap().deadline + 3600);
                t["rework_limit"] = json!(1);
                let d = Draft::counter(
                    c,
                    head,
                    parties,
                    &t,
                    "I can do it properly with one more hour and one revision round",
                )
                .unwrap();
                out.push(mv(
                    performer,
                    1300,
                    "Atlas counters: +1 h deadline and one revision round. New terms, new hash.",
                    d,
                    ctx,
                ));
            }
        }
        (_, Phase::Requested) if fresh(K_PROMISE) => {
            out.push(mv(
                performer,
                1000,
                if token_verified {
                    "Atlas verified the locked token with the mint (DLEQ + unspent), then promises against the terms hash."
                } else {
                    "Atlas promises against the terms hash (and forecasts its on-time odds)."
                },
                Draft::promise(c, head, parties, &st.terms, 0.85, token_verified),
                ctx,
            ));
        }

        (_, Phase::Promised) if ctx.scenario == "silent" => {}
        (_, Phase::Promised) => {
            let progressed = !ctx.c.progress.is_empty();
            if !progressed && !ctx.exists(K_PROGRESS, p, |_| true) {
                out.push(mv(
                    performer,
                    1100,
                    "Atlas pings PROGRESS: liveness only, it never moves the head.",
                    Draft::progress(c, head, parties, "About halfway there"),
                    ctx,
                ));
            } else if progressed && fresh(K_REPORT) {
                out.push(mv(
                    performer,
                    1500,
                    "Atlas signs a REPORT: an assertion of completion, not closure (INV-5).",
                    report_draft(1),
                    ctx,
                ));
            }
            // Governance: the REPORT is pending until Gate endorses it.
            if ctx.scenario == "governed" {
                for r in ctx
                    .members
                    .iter()
                    .filter(|e| e.kind == K_REPORT && e.pubkey == p && Ctx::prev_is(e, head))
                {
                    if !ctx.members.iter().any(|e| {
                        e.kind == K_ENDORSE
                            && e.tags
                                .iter()
                                .any(|t| t[0] == "e" && t.get(1) == Some(&r.id))
                    }) {
                        let d = Draft::endorse(
                            c,
                            &r.id,
                            &acn_core::jcs::sha256_hex(b"demo-policy-v1"),
                            true,
                        );
                        let mut m = mv(Actor::Gatekeeper, 2600, "Gate (a human sign-off) endorses the REPORT; only now does it take effect.", d, ctx);
                        m.key = format!("Gatekeeper:endorse:{}", r.id);
                        m.not_before = r.created_at;
                        out.push(m);
                    }
                }
            }
        }

        (_, Phase::Reworking) if fresh(K_REPORT) => {
            out.push(mv(
                performer,
                1600,
                "Atlas reworks and signs a new REPORT against the REJECT.",
                report_draft(ctx.reports() + 1),
                ctx,
            ));
        }

        (_, Phase::Reported) if ctx.scenario == "verified" => {
            let report = st.report.as_deref().unwrap_or_default();
            let attested = |by: &str, cond: &str, kind: u16| {
                ctx.exists(kind, by, |e| Ctx::on_report(e, report, cond))
            };
            let mut push = |actor: Actor,
                            kind: u16,
                            cond: &str,
                            delay: u64,
                            content: Value,
                            narration: &str| {
                let mut m = mv(
                    actor,
                    delay,
                    narration,
                    Draft::attest(kind, c, report, cond, content),
                    ctx,
                );
                m.key = format!("{actor:?}:{kind}:{report}:{cond}");
                out.push(m);
            };
            if !attested(&cast.verifier, "tests", K_ATTEST_MECH) {
                push(Actor::Verifier, K_ATTEST_MECH, "tests", 1300, json!({"result": "pass", "method": "cargo test (42 passed)", "env": "rust 1.93"}),
                    "Verity re-runs the tests: pass. A mechanical attestation from a listed verifier.");
            }
            if !attested(&cast.judge, "quality", K_INTERP_REPORT) {
                push(
                    Actor::Judge,
                    K_INTERP_REPORT,
                    "quality",
                    2000,
                    json!({"judge": "muse-llm", "verdict": "readable, idiomatic"}),
                    "Muse (an LLM judge) gives its opinion on quality: advisory only.",
                );
            }
            if ctx.reports() == 1 && !attested(&cast.judge, "tests", K_INTERP_REPORT) {
                push(Actor::Judge, K_INTERP_REPORT, "tests", 2600, json!({"judge": "muse-llm", "verdict": "tests look flaky"}),
                    "Muse also opines on the tests. Opinions can't touch a mechanical condition (INV-8).");
            }
        }

        // Dispute: after your REJECT, Atlas asks the arbiter.
        (_, Phase::Rejected) if ctx.scenario == "dispute" && !st.ruled && fresh(K_DISPUTE) => {
            out.push(mv(
                performer,
                1500,
                "Atlas disputes: it believes the report met the terms.",
                Draft::dispute(
                    c,
                    head,
                    parties,
                    "The report met the agreed terms; requesting arbitration",
                ),
                ctx,
            ));
        }
        (_, Phase::Disputed) if !ctx.exists(K_RULING, &cast.arbiter, |e| Ctx::prev_is(e, head)) => {
            let (for_performer, narration) = match escrow {
                Some(_) => (true, "Solon rules for the performer and releases the escrowed preimage: payment happens without you."),
                None => (false, "Solon has no escrowed preimage, so it can only rule for the requester."),
            };
            let d = Draft::ruling(c, head, parties, for_performer, escrow.map(String::as_str));
            out.push(mv(Actor::Arbiter, 2600, narration, d, ctx));
        }
        _ => {}
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HintAction {
    AcceptCounter,
    Satisfy,
    Reject { condition: String, note: String },
    Withdraw,
    Dispute { reason: String },
    AdvanceClock { secs: u64 },
    Wait,
    Done,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hint {
    pub commitment: String,
    pub action: HintAction,
    pub label: String,
    pub narration: String,
}

/// The requester's next scripted step for a commitment (for presenters and autopilot).
pub fn requester_hint(events: &[Event], commitment: &str, now: u64) -> Option<Hint> {
    let out = fold(events, &BTreeMap::new());
    let c = out.commitments.get(commitment)?;
    let (st, terms) = (c.state.as_ref()?, c.terms.as_ref()?);
    let scenario = terms.deliverable.tool.as_deref().unwrap_or_default();
    let status = status_at(c, now);
    let hint = |action: HintAction, label: &str, narration: &str| {
        Some(Hint {
            commitment: commitment.into(),
            action,
            label: label.into(),
            narration: narration.into(),
        })
    };
    let reports = c.chain_kinds.iter().filter(|k| **k == K_REPORT).count();

    if status.terminal {
        let narration = match st.phase {
            Phase::Satisfied if c.chain_kinds.last() == Some(&K_RULING) => "Closed by RULING: the arbiter released the preimage, so the performer is paid.",
            Phase::Satisfied if terms.hashlock().is_some() => "Satisfied: your SATISFY revealed the preimage. Closing and paying were one signed act.",
            Phase::Satisfied => "Satisfied: only the requester can close a commitment (INV-5).",
            Phase::Declined if scenario == "rogue" => "Both conflicting acts are kept as evidence; the closing DECLINE won and Rogue is flagged for equivocation.",
            Phase::Declined if terms.hashlock().is_some() => "Declined. Your locked token was never redeemable without s; reclaim it from your wallet after the locktime.",
            Phase::Rejected if terms.hashlock().is_some() => "Closed unsatisfied. s was never revealed, so the performer can't redeem; reclaim the token after the locktime.",
            Phase::Withdrawn => "Withdrawn. The lapse is on the record for scoring.",
            _ => "This commitment is closed. Every act and anomaly stays on the record.",
        };
        return hint(HintAction::Done, "Done", narration);
    }
    match st.phase {
        Phase::Countered if st.pending.as_ref().is_some_and(|p| p.by == c.performer) => hint(
            HintAction::AcceptCounter,
            "Accept counter-offer",
            "Accepting binds you both to the counter's terms hash; the counter becomes the promise.",
        ),
        Phase::Promised if status.lapsed == Some(Party::Performer) => hint(
            HintAction::Withdraw,
            "Withdraw",
            "The deadline passed with no REPORT: the commitment has lapsed. That's derived by everyone, never announced.",
        ),
        Phase::Promised if scenario == "silent" => hint(
            HintAction::AdvanceClock { secs: terms.deadline.saturating_sub(now) + 60 },
            "Fast-forward past the deadline",
            "Atlas promised and went quiet. Move the presentation clock past the deadline.",
        ),
        Phase::Reported => match scenario {
            "rework" if reports == 1 => hint(
                HintAction::Reject { condition: "sections".into(), note: "The pricing section is missing".into() },
                "Reject: missing section",
                "Reject, naming the failed condition. Below the rework limit, this sends it back to Atlas.",
            ),
            "verified" if reports == 1 => hint(
                HintAction::Reject { condition: "tests".into(), note: "I don't trust these tests".into() },
                "Reject the tests",
                "Reject the mechanical condition anyway: allowed (INV-5), but the validator flags it as contradicted by Verity's attestation.",
            ),
            "dispute" => hint(
                HintAction::Reject { condition: "fit".into(), note: "Not what we agreed".into() },
                "Reject the concepts",
                "Reject. With no rework allowed it is Rejected, but still disputable because the terms name an arbiter.",
            ),
            "paid" => hint(HintAction::Satisfy, "Satisfy & pay", "Satisfy: this reveals the preimage s, which lets Atlas redeem the 21 sat."),
            _ => hint(HintAction::Satisfy, "Accept result", "Accept the result: SATISFY closes the commitment."),
        },
        _ => hint(HintAction::Wait, "Waiting…", match st.phase {
            Phase::Requested => "Waiting for the performer to respond.",
            Phase::Countered => "Waiting for the other side to accept your counter.",
            Phase::Promised if scenario == "governed" => "Atlas has reported, but the REPORT is pending until Gate endorses it.",
            Phase::Promised | Phase::Reworking => "The performer is working.",
            Phase::Rejected => "Waiting to see whether Atlas disputes.",
            Phase::Disputed => "Waiting for the arbiter's ruling.",
            _ => "Waiting.",
        }),
    }
}

/// Requester drafts for a hint (shared by hosts so they sign the same thing).
pub fn requester_draft(
    events: &[Event],
    commitment: &str,
    action: &HintAction,
    preimage: Option<&str>,
) -> Option<(Draft, u64)> {
    let out = fold(events, &BTreeMap::new());
    let c = out.commitments.get(commitment)?;
    let st = c.state.as_ref()?;
    let parties = [c.requester.as_str(), c.performer.as_str()];
    let (cid, head) = (c.id.as_str(), st.head.as_str());
    let d = match action {
        HintAction::AcceptCounter => {
            Draft::accept(cid, head, &parties, &st.pending.as_ref()?.terms)
        }
        HintAction::Satisfy => Draft::satisfy(cid, head, &parties, preimage),
        HintAction::Reject { condition, note } => Draft::reject(
            cid,
            head,
            &parties,
            vec![(condition.as_str(), note.as_str())],
        ),
        HintAction::Withdraw => {
            Draft::closing(K_WITHDRAW, cid, head, &parties, "requester_withdrew")
        }
        HintAction::Dispute { reason } => Draft::dispute(cid, head, &parties, reason),
        _ => return None,
    };
    Some((d, st.head_at))
}
