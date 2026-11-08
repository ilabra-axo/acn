//! Live mode end to end: desktop app layer ↔ mock relay ↔ simulated cast, with the presenter's
//! "Next" (run_hint) driving the requester. Every scenario must reach its intended outcome, and
//! priced scenarios move real ecash (test sats) at an in-process CDK mint.

use std::time::Duration;

use acn_core::Phase;
use acn_sim::{Actor, HintAction, SCENARIOS};
use nmesh_acn_sim::{start, Config, MintSetup};
use nmesh_app::{App, AppConfig};
use nostr_relay_builder::MockRelay;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("nmesh-sim-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test]
async fn every_scenario_plays_live() -> anyhow::Result<()> {
    let relay = MockRelay::run().await?;
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
    })
    .await?;
    let mut cfg = AppConfig::new(vec![url], dir.join("app"));
    cfg.mint = Some(cast.mint_url().to_string());
    let (app, _events) = App::start(cfg).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let w = app.top_up(100).await?;
    assert_eq!((w.available, w.simulated), (100, false));

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

    for s in SCENARIOS {
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
            _ => 0,
        };
        if price > 0 {
            let mut atlas = 0;
            for _ in 0..100 {
                atlas = cast.performer_wallet().await?.available;
                if atlas >= price {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let mine = app.wallet().await?.expect("wallet");
            let lock = mine
                .locks
                .iter()
                .find(|l| l.commitment.as_deref() == Some(id.as_str()))
                .expect("lock linked to the commitment");
            assert_eq!(lock.amount, price);
            assert_eq!(lock.state, acn_wallet::LockState::Redeemed, "{}", s.id);
            assert_eq!(mine.locked_out, 0);
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

    // 100 - 21 (paid) - 50 (dispute, ruled for Atlas).
    assert_eq!(app.wallet().await?.unwrap().available, 29);
    assert_eq!(cast.performer_wallet().await?.available, 71);
    // Not enough left for another dispute (50 sat): refused before anything is signed.
    let pk = cast.public_key(Actor::Performer).to_hex();
    let err = app.request(&pk, "/dispute again").await.unwrap_err();
    assert!(err.to_string().contains("insufficient funds"), "{err}");

    // Reset demo: old commitments disappear, also after a restart that refetches history.
    assert!(!app.commitments(&pk).await?.is_empty());
    app.reset_demo().await;
    assert!(app.commitments(&pk).await?.is_empty());
    assert_eq!(app.wallet().await?.unwrap().available, 29, "money is kept");
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
