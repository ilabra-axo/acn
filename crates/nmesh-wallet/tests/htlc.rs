//! Real ecash against an in-process `nmesh-mint` (CDK + fake Lightning): the NUT-14 rules ACN
//! settlement relies on (docs/Core.md §8).

use std::time::Duration;

use acn_wallet::LockState;
use nmesh_wallet::CashuWallet;
use sha2::{Digest, Sha256};

fn key(b: u8) -> [u8; 32] {
    [b; 32]
}

#[tokio::test(flavor = "multi_thread")]
async fn htlc_lock_redeem_refund() -> anyhow::Result<()> {
    let mint = nmesh_mint::start(nmesh_mint::MintConfig {
        listen: "127.0.0.1:0".parse()?,
        url: None,
        db: None,
        mnemonic: None,
    })
    .await?;
    let dir = std::env::temp_dir().join(format!("nmesh-wallet-test-{}", std::process::id()));
    let req = CashuWallet::open(&mint.url, &key(1), Some(&dir)).await?;
    let perf = CashuWallet::open(&mint.url, &key(2), None).await?;
    let other = CashuWallet::open(&mint.url, &key(3), None).await?;

    assert_eq!(req.top_up(100).await?, 100);

    let s = [7u8; 32];
    let pre = hex::encode(s);
    let h = hex::encode(Sha256::digest(s));
    let now = cdk::util::unix_time();

    // Lock 21 sat to the performer.
    let tok = req
        .lock_htlc(21, &h, perf.owner(), now + 3600, None)
        .await?;
    req.tag(&tok, "c1")?;
    let v = req.view().await?;
    assert_eq!((v.available, v.locked_out), (79, 21));
    assert_eq!(v.locks[0].commitment.as_deref(), Some("c1"));

    // The performer verifies: DLEQ, HTLC shape, unspent; and checks it against the terms.
    let ver = perf.inspect(&tok).await?;
    assert_eq!(ver.amount, 21);
    assert_eq!(ver.hashlock, h);
    assert_eq!(ver.receiver, perf.owner());
    assert!(ver.unspent);
    let terms: acn_core::Terms = serde_json::from_value(serde_json::json!({
        "v": acn_core::terms::TERMS_VERSION,
        "deliverable": {"description": "x"}, "conditions": [], "deadline": now + 60,
        "review_window": 60, "rework_limit": 0, "price": {"amount": 21, "unit": "sat"},
        "payment": {"mint": mint.url, "hashlock": h, "locktime": now + 3600},
        "verifiers": [], "arbiter": null, "subcontracting": false, "disclosure": "private",
        "governance": null
    }))?;
    acn_wallet::settlement::check_payment(&terms, &ver, perf.owner())
        .map_err(anyhow::Error::msg)?;
    assert!(acn_wallet::settlement::check_payment(&terms, &ver, other.owner()).is_err());
    perf.note_incoming(&tok, &ver, req.owner(), "c1");

    // Wrong preimage, wrong key, early refund: all rejected by the mint.
    assert!(perf.redeem(&tok, &"00".repeat(32), None).await.is_err());
    assert!(other.redeem(&tok, &pre, None).await.is_err());
    assert!(req.refund(&tok).await.is_err());

    // Redeem; a second redeem is a double spend.
    assert_eq!(perf.redeem(&tok, &pre, Some("c1")).await?, 21);
    assert!(perf.redeem(&tok, &pre, None).await.is_err());
    assert!(!perf.inspect(&tok).await?.unspent);
    let pv = perf.view().await?;
    assert_eq!((pv.available, pv.incoming), (21, 0));
    assert_eq!(pv.locks[0].state, LockState::Redeemed);

    // The requester's view learns from the mint (NUT-07) that the performer redeemed.
    let v = req.view().await?;
    assert_eq!((v.available, v.locked_out), (79, 0));
    assert_eq!(v.locks[0].state, LockState::Redeemed);

    // Refund path: lock with a short locktime, reclaim after it.
    let tok2 = req
        .lock_htlc(5, &h, perf.owner(), cdk::util::unix_time() + 2, Some("c2"))
        .await?;
    assert_eq!(req.token_for("c2").as_deref(), Some(tok2.as_str()));
    assert!(req.refund(&tok2).await.is_err());
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(req.refund(&tok2).await?, 5);
    assert!(
        perf.redeem(&tok2, &pre, None).await.is_err(),
        "already refunded"
    );
    let v = req.view().await?;
    assert_eq!(v.available, 79);
    assert_eq!(v.locks[1].state, LockState::Refunded);

    // Insufficient funds is reported, not attempted.
    assert!(matches!(
        req.lock_htlc(1000, &h, perf.owner(), now + 3600, None)
            .await,
        Err(nmesh_wallet::WalletError::InsufficientFunds { .. })
    ));

    // The index survives a reopen (history linked to commitments).
    drop(req);
    let again = CashuWallet::open(&mint.url, &key(1), Some(&dir)).await?;
    let v = again.view().await?;
    assert_eq!(v.available, 79);
    assert!(v
        .history
        .iter()
        .any(|e| e.commitment.as_deref() == Some("c1")));
    let _ = std::fs::remove_dir_all(&dir);

    mint.stop();
    Ok(())
}
