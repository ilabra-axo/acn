//! Live mode end to end: desktop app layer ↔ mock relay ↔ simulated cast, with the presenter's
//! "Next" (run_hint) driving the requester. Every scenario must reach its intended outcome, and
//! priced scenarios move real ecash (test sats) at an in-process CDK mint.

use std::time::Duration;

use acn_core::Phase;
use acn_sim::{Actor, HintAction, SCENARIOS};
use nmesh_acn_sim::{start, Config, MintSetup};
use nmesh_app::{App, AppConfig};
use nostr_relay_builder::prelude::{LocalRelay, RateLimit, RelayBuilder};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("nmesh-sim-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test]
async fn every_scenario_plays_live() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    // Sixteen scenarios in well under a minute: lift the default 60 notes/minute per connection.
    let relay = LocalRelay::new(RelayBuilder::default().rate_limit(RateLimit {
        max_reqs: 500,
        notes_per_minute: 6_000,
    }));
    relay.run().await?;
    let url = relay.url().await.to_string();
    let dir = temp_dir("live");
    let cast = start(Config {
        relays: vec![url.clone()],
        data: dir.join("cast"),
        pace: 0.05,
        mint: MintSetup::Host {
            listen: "127.0.0.1:0".parse()?,
            db: None,
        },
        judges: None,
    })
    .await?;
    let mut cfg = AppConfig::new(vec![url], dir.join("app"));
    cfg.mint = Some(cast.mint_url().to_string());
    let (app, _events) = App::start(cfg).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let w = app.top_up(300).await?;
    assert_eq!((w.available, w.simulated), (300, false));

    // The "🎬 ACN demo" button: discovery by K_SVC, Atlas first.
    let found = app.discover_demo().await?;
    let found: Vec<_> = found.iter().map(|c| c.pubkey.clone()).collect();
    assert_eq!(
        found,
        [
            cast.public_key(Actor::Performer).to_hex(),
            cast.public_key(Actor::Rogue).to_hex()
        ]
    );

    for agent in [Actor::Performer, Actor::Rogue] {
        let pk = cast.public_key(agent).to_hex();
        let contact = app.add_contact(&pk).await?;
        let acn = contact.acn.clone().expect("cast offers ACN");
        assert!(acn.simulated);
        assert_eq!(
            acn.binding.bound,
            agent == Actor::Performer,
            "{agent:?}: {}",
            acn.binding.detail
        );
        app.accept_agent(&pk).await?;
    }

    // `ONLY=judged,contingent cargo test ...` plays a subset (the final balance checks then fail).
    let only = std::env::var("ONLY").ok();
    for s in SCENARIOS {
        if only
            .as_deref()
            .is_some_and(|o| !o.split(",").any(|x| x == s.id))
        {
            continue;
        }
        let pk = cast.public_key(s.actor).to_hex();
        let c = app.request(&pk, &format!("/{} the demo", s.id)).await?;
        let id = c.id.clone();
        let mut done = false;
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let hint = app.hint(&pk, &id).await?.expect("hint");
            match hint.action {
                HintAction::Done => {
                    done = true;
                    break;
                }
                HintAction::Wait => {}
                _ => {
                    app.run_hint(&pk, &id).await?;
                }
            }
        }
        let view = app
            .commitments(&pk)
            .await?
            .into_iter()
            .find(|v| v.id == id)
            .unwrap();
        assert!(
            done,
            "{}: stuck in {:?}; timeline: {:#?}",
            s.id,
            view.phase,
            view.timeline
                .iter()
                .map(|e| (e.act, e.status, &e.summary))
                .collect::<Vec<_>>()
        );
        let expected = match s.id {
            "silent" => Phase::Withdrawn,
            "rogue" => Phase::Declined,
            "judged" | "contingent" => Phase::Settled,
            "injected" | "disqualified" => Phase::Rejected,
            _ => Phase::Satisfied,
        };
        assert_eq!(view.phase, Some(expected), "{}", s.id);
        if let Some(st) = &view.settlement {
            assert!(!st.simulated, "real test-mint ecash");
            assert_eq!(st.mint, cast.mint_url());
        }
        // Money: the requester's lock is redeemed by Atlas once the preimage is public.
        let price = match s.id {
            "paid" => 21,
            "dispute" => 50,
            "judged" | "injected" => 60,
            "judged-auto" => 40,
            "contingent" => 30,
            _ => 0,
        };
        if s.id == "injected" {
            // Nothing released: the token stays locked until its locktime (then the requester refunds).
            let mine = app.wallet().await?.expect("wallet");
            let lock = mine
                .locks
                .iter()
                .find(|l| l.commitment.as_deref() == Some(id.as_str()))
                .expect("lock linked to the commitment");
            assert_eq!(
                (lock.amount, lock.state),
                (60, acn_wallet::LockState::Locked)
            );
        } else if price > 0 {
            let mut atlas = 0;
            for _ in 0..100 {
                atlas = cast.performer_wallet().await?.available;
                if atlas >= price {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let mut mine = app.wallet().await?.expect("wallet");
            for _ in 0..50 {
                let redeemed = mine.locks.iter().any(|l| {
                    l.commitment.as_deref() == Some(id.as_str())
                        && l.state == acn_wallet::LockState::Redeemed
                });
                if redeemed {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                mine = app.wallet().await?.expect("wallet");
            }
            let lock = mine
                .locks
                .iter()
                .find(|l| l.commitment.as_deref() == Some(id.as_str()))
                .expect("lock linked to the commitment");
            assert_eq!(lock.amount, price);
            assert_eq!(lock.state, acn_wallet::LockState::Redeemed, "{}", s.id);
            assert!(atlas >= price, "{}: Atlas has {atlas}", s.id);
        }
        let codes: Vec<_> = view.anomalies.iter().map(|a| a.code).collect();
        match s.id {
            "verified" => assert!(
                codes.contains(&"contradicted_rejection")
                    && codes.contains(&"wrong_evidence_class"),
                "{codes:?}"
            ),
            "rogue" => assert!(
                codes.contains(&"equivocation") && codes.contains(&"unbound"),
                "{codes:?}"
            ),
            "dispute" => assert_eq!(view.chain.last().map(|e| e.act), Some("RULING")),
            "injected" => assert!(codes.contains(&"injection_detected"), "{codes:?}"),
            "disqualified" => assert!(codes.contains(&"disqualified"), "{codes:?}"),
            // The app fetched Atlas's public template, so the counter's reference counts (V-28).
            "drafted" => {
                let tv = view
                    .terms_card
                    .as_ref()
                    .and_then(|c| c.template.as_ref())
                    .expect("template");
                assert_eq!(
                    tv.status,
                    acn_core::template::TemplateStatus::Counted,
                    "{tv:?}"
                );
                assert_eq!(tv.name.as_deref(), Some("retry-change/v1"));
            }
            // CR-003 §2.3: the judges' attestation on c2 revealed the review cases both seeds
            // select, and the app (which kept its seed) reproduces exactly that sample, as a
            // requester-side runner would.
            "held-out" => {
                let line = view
                    .timeline
                    .iter()
                    .find(|e| e.act == "ATTEST_JUDGED")
                    .map(|e| e.summary.clone())
                    .expect("a judged attestation on c2");
                let pool: Vec<serde_json::Value> = acn_sim::REVIEW_POOL
                    .iter()
                    .map(|s| serde_json::json!(s))
                    .collect();
                let held = app.held_out(&pk, &id, pool.clone()).await?;
                let (mut picked, _) = acn_core::instances::reveal(
                    &pool,
                    &held.requester_seed,
                    &held.performer_seed,
                    &id,
                    acn_sim::REVIEW_SAMPLE,
                )
                .expect("reveal");
                picked.sort_unstable();
                let shown: Vec<String> = picked.iter().map(|i| i.to_string()).collect();
                assert!(
                    line.contains(&format!("held-out cases {}", shown.join(", "))) || {
                        // The attestation lists them in rank order; compare as sets.
                        let got: std::collections::BTreeSet<u32> = line
                            .split("held-out cases ")
                            .nth(1)
                            .unwrap_or_default()
                            .split(" with")
                            .next()
                            .unwrap_or_default()
                            .split(", ")
                            .filter_map(|x| x.parse().ok())
                            .collect();
                        got == picked.iter().copied().collect()
                    },
                    "{line} vs {picked:?}"
                );
            }
            "contingent" => {
                // Sealed until CLAIM revealed k; now the card shows the decrypted result.
                let r = view.report.as_ref().expect("report");
                assert_eq!(r.payload["title"], "Parser module", "{:?}", r.payload);
            }
            _ => {}
        }
        // Third-party acts (verifier, judge, arbiter, gate) are grouped into the performer's thread.
        if s.id == "verified" {
            assert!(view.timeline.iter().any(|e| e.act == "ATTEST_MECH"));
        }
    }

    // Board: every scenario closed, across both agents; closed ones are paged.
    let b = app.board(3).await;
    assert_eq!(b.closed_total, SCENARIOS.len());
    assert_eq!(b.items.len(), 3);
    assert!(b.items.iter().all(|c| c.lane == acn_view::Lane::Closed));
    assert!(b
        .items
        .windows(2)
        .all(|w| w[0].updated_at >= w[1].updated_at));
    assert_eq!(app.board(100).await.items.len(), SCENARIOS.len());

    // 300 - 21 (paid) - 50 (dispute, ruled for Atlas) - 60 (judged) - 40 (judged-auto)
    // - 30 (contingent); 60 (injected) stays locked until its locktime.
    let mine = app.wallet().await?.unwrap();
    assert_eq!((mine.available, mine.locked_out), (39, 60));
    assert_eq!(cast.performer_wallet().await?.available, 201);
    // Not enough left for another dispute (50 sat): refused before anything is signed.
    let pk = cast.public_key(Actor::Performer).to_hex();
    let err = app.request(&pk, "/dispute again").await.unwrap_err();
    assert!(err.to_string().contains("insufficient funds"), "{err}");

    // Reset demo: old commitments disappear, also after a restart that refetches history.
    assert!(!app.commitments(&pk).await?.is_empty());
    app.reset_demo().await;
    assert!(app.commitments(&pk).await?.is_empty());
    assert_eq!(app.wallet().await?.unwrap().available, 39, "money is kept");
    app.shutdown().await;
    let mut cfg = AppConfig::new(vec![relay.url().await.to_string()], dir.join("app"));
    cfg.mint = Some(cast.mint_url().to_string());
    let (app, _events) = App::start(cfg).await?;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        app.commitments(&pk).await?.is_empty(),
        "history refetch respects the reset"
    );
    let c = app.request(&pk, "/quick after reset").await?;
    assert_eq!(app.commitments(&pk).await?.len(), 1);
    for _ in 0..100 {
        if app.commitments(&pk).await?[0].phase == Some(Phase::Reported) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(app.commitments(&pk).await?[0].id, c.id);
    assert_eq!(app.commitments(&pk).await?[0].phase, Some(Phase::Reported));
    let b = app.board(10).await;
    assert_eq!((b.items.len(), b.closed_total), (1, 0));
    assert_eq!(b.items[0].lane, acn_view::Lane::Review);
    assert!(b.items[0].needs_me);

    app.shutdown().await;
    cast.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// The judged scenarios with real judges (an `acn-judge` panel) through relay, app and mint.
/// Needs the models the panel names, so it is opt-in:
/// `NMESH_JUDGES=crates/acn-judge/judges.example.toml cargo test -p nmesh-acn-sim --test live -- --ignored`
#[tokio::test]
#[ignore = "needs real judge endpoints (NMESH_JUDGES)"]
async fn judged_scenarios_with_real_judges() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let path = std::env::var("NMESH_JUDGES").expect("NMESH_JUDGES=<judges.toml>");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    let judges = acn_judge::Panel::load(&path)?;
    let relay = LocalRelay::new(RelayBuilder::default());
    relay.run().await?;
    let url = relay.url().await.to_string();
    let dir = temp_dir("judges");
    let cast = start(Config {
        relays: vec![url.clone()],
        data: dir.join("cast"),
        pace: 0.05,
        mint: MintSetup::Host {
            listen: "127.0.0.1:0".parse()?,
            db: None,
        },
        judges: Some(judges),
    })
    .await?;
    let mut cfg = AppConfig::new(vec![url], dir.join("app"));
    cfg.mint = Some(cast.mint_url().to_string());
    let (app, _events) = App::start(cfg).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    app.top_up(200).await?;
    let pk = cast.public_key(Actor::Performer).to_hex();
    app.add_contact(&pk).await?;
    app.accept_agent(&pk).await?;

    for id in ["judged", "injected", "held-out"] {
        let c = app.request(&pk, &format!("/{id} the ACN protocol")).await?;
        let mut done = false;
        // Real judges on a CPU take minutes.
        for _ in 0..1200 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match app.hint(&pk, &c.id).await?.expect("hint").action {
                HintAction::Done => {
                    done = true;
                    break;
                }
                HintAction::Wait => {}
                _ => {
                    app.run_hint(&pk, &c.id).await?;
                }
            }
        }
        let view = app
            .commitments(&pk)
            .await?
            .into_iter()
            .find(|v| v.id == c.id)
            .unwrap();
        let summaries: Vec<_> = view
            .timeline
            .iter()
            .map(|e| (e.act, e.status, &e.summary))
            .collect();
        assert!(done, "{id}: stuck in {:?}: {summaries:#?}", view.phase);
        let judged = view
            .timeline
            .iter()
            .find(|e| e.act == "ATTEST_JUDGED")
            .expect("a judged attestation");
        let codes: Vec<_> = view.anomalies.iter().map(|a| a.code).collect();
        assert!(
            !codes.iter().any(|c| matches!(
                *c,
                "judge_mismatch" | "rubric_mismatch" | "malformed" | "unlisted_verifier"
            )),
            "{id}: the real attestation must count: {codes:?}"
        );
        println!("{id}: {:?} · {}", view.phase, judged.summary);
        if id == "injected" {
            assert!(codes.contains(&"injection_detected"), "{codes:?}");
            assert_eq!(view.phase, Some(Phase::Rejected));
        } else if id == "held-out" {
            // The judges saw the sampled review cases; the attestation reveals them and counts.
            assert!(
                judged.summary.contains("held-out cases"),
                "{}",
                judged.summary
            );
            assert!(
                matches!(view.phase, Some(Phase::Satisfied) | Some(Phase::Rejected)),
                "{:?}",
                view.phase
            );
        } else {
            // Real opinions: either released (pass) or rejected as below threshold.
            assert!(
                matches!(view.phase, Some(Phase::Settled) | Some(Phase::Rejected)),
                "{:?}",
                view.phase
            );
        }
    }
    app.shutdown().await;
    cast.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
