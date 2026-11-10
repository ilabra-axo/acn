//! Real ecash against an in-process `nmesh-mint` (CDK + fake Lightning): the NUT-14 rules ACN
//! settlement relies on (docs/Core.md §8).

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use acn_wallet::LockState;
use cdk::nuts::{Conditions, CurrencyUnit, SigFlag, SpendingConditions, Token, TokenV4};
use nmesh_wallet::{cashu_pubkey, CashuWallet, RefundInterrupt};
use sha2::{Digest, Sha256};

fn key(b: u8) -> [u8; 32] {
    [b; 32]
}

async fn test_mint() -> anyhow::Result<nmesh_mint::RunningMint> {
    nmesh_mint::start(nmesh_mint::MintConfig {
        listen: "127.0.0.1:0".parse()?,
        url: None,
        db: None,
        mnemonic: None,
    })
    .await
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("nmesh-wallet-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn v4(token: &str) -> TokenV4 {
    match Token::from_str(token).unwrap() {
        Token::TokenV4(t) => t,
        Token::TokenV3(_) => panic!("CDK sends v4 tokens"),
    }
}

fn terms(mint: &str, h: &str, price: u64, locktime: u64) -> anyhow::Result<acn_core::Terms> {
    Ok(serde_json::from_value(serde_json::json!({
        "v": acn_core::terms::TERMS_VERSION,
        "deliverable": {"description": "x"}, "conditions": [], "deadline": locktime,
        "review_window": 60, "rework_limit": 0, "price": {"amount": price, "unit": "sat"},
        "payment": {"mint": mint, "hashlock": h, "locktime": locktime},
        "verifiers": [], "arbiter": null, "subcontracting": false, "disclosure": "private",
        "governance": null
    }))?)
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
    acn_wallet::settlement::check_payment_from(&terms, &ver, perf.owner(), req.owner())
        .map_err(anyhow::Error::msg)?;
    assert_eq!(ver.refund, vec![req.owner().to_string()]);
    assert!(acn_wallet::settlement::check_payment(&terms, &ver, other.owner()).is_err());
    perf.note_incoming(&tok, &ver, req.owner(), "c1");

    // Wrong preimage, wrong key, early refund: all rejected by the mint.
    assert!(perf.redeem(&tok, &"00".repeat(32), None).await.is_err());
    assert!(other.redeem(&tok, &pre, None).await.is_err());
    assert!(req.refund(&tok).await.is_err());

    // Redeem; a second redeem is a double spend.
    assert_eq!(
        req.revealed_preimage(&tok).await?,
        None,
        "unspent: nothing revealed yet"
    );
    assert_eq!(perf.redeem(&tok, &pre, Some("c1")).await?, 21);
    // Contingent settlement relies on this: the payer learns the preimage from the mint.
    assert_eq!(
        req.revealed_preimage(&tok).await?.as_deref(),
        Some(pre.as_str())
    );
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

/// A06/A07: inspection establishes distinct proofs and the complete §8.1 policy of every proof.
#[tokio::test(flavor = "multi_thread")]
async fn inspect_rejects_hostile_tokens() -> anyhow::Result<()> {
    let mint = test_mint().await?;
    let req = CashuWallet::open(&mint.url, &key(1), None).await?;
    let perf = CashuWallet::open(&mint.url, &key(2), None).await?;
    let other = CashuWallet::open(&mint.url, &key(3), None).await?;
    req.top_up(200).await?;
    let h = hex::encode(Sha256::digest([7u8; 32]));
    let h2 = hex::encode(Sha256::digest([8u8; 32]));
    let lt = cdk::util::unix_time() + 3600;
    let t = terms(&mint.url, &h, 7, lt)?;
    let (p, o, r) = (
        cashu_pubkey(perf.owner())?,
        cashu_pubkey(other.owner())?,
        cashu_pubkey(req.owner())?,
    );
    let err = |r: Result<acn_wallet::Verified, nmesh_wallet::WalletError>| match r {
        Ok(v) => panic!("accepted: {v:?}"),
        Err(e) => e.to_string(),
    };

    // 7 sat = 3 proofs (4 + 2 + 1).
    let tok = req.lock_htlc(7, &h, perf.owner(), lt, None).await?;
    let v = perf.inspect(&tok).await?;
    assert_eq!(v.amount, 7);
    acn_wallet::settlement::check_payment_from(&t, &v, perf.owner(), req.owner())
        .map_err(anyhow::Error::msg)?;

    // A repeated proof, in the same keyset group or in another, is not extra value.
    let mut dup = v4(&tok);
    let first = dup.token[0].proofs[0].clone();
    dup.token[0].proofs.push(first.clone());
    assert!(err(perf.inspect(&dup.to_string()).await).contains("repeats a proof"));
    let mut dup = v4(&tok);
    let mut group = dup.token[0].clone();
    group.proofs = vec![first];
    dup.token.push(group);
    assert!(err(perf.inspect(&dup.to_string()).await).contains("repeats a proof"));

    // Proofs of two different locks in one token.
    let tok2 = req.lock_htlc(3, &h2, perf.owner(), lt, None).await?;
    let mut mixed = v4(&tok);
    mixed.token.extend(v4(&tok2).token);
    assert!(err(perf.inspect(&mixed.to_string()).await).contains("different locks"));

    let htlc = |pubkeys: Vec<_>, refund: Vec<_>, n_sigs, sig_flag| {
        let c = Conditions::new(
            Some(lt),
            Some(pubkeys),
            Some(refund),
            n_sigs,
            sig_flag,
            None,
        )
        .unwrap();
        SpendingConditions::new_htlc_hash(&h, Some(c)).unwrap()
    };

    // An extra beneficiary could redeem instead of the performer.
    let extra = req
        .send_with_conditions(7, htlc(vec![p, o], vec![r], None, None))
        .await?;
    assert!(err(perf.inspect(&extra).await).contains("2 keys redeem"));

    // A 2-of-2 threshold: the performer cannot redeem alone.
    let multi = req
        .send_with_conditions(7, htlc(vec![p, o], vec![r], Some(2), None))
        .await?;
    assert!(err(perf.inspect(&multi).await).contains("requires 2 signatures"));

    // SIG_ALL changes what the redeeming signature must cover.
    let all = req
        .send_with_conditions(7, htlc(vec![p], vec![r], None, Some(SigFlag::SigAll)))
        .await?;
    assert!(err(perf.inspect(&all).await).contains("SIG_ALL"));

    // Several refund keys: someone else may refund too.
    let refunds = req
        .send_with_conditions(7, htlc(vec![p], vec![r, o], None, None))
        .await?;
    assert!(err(perf.inspect(&refunds).await).contains("2 refund keys"));

    // A substituted refund key is well-formed; only a check against the requester catches it.
    let swapped = req
        .send_with_conditions(7, htlc(vec![p], vec![o], None, None))
        .await?;
    let v = perf.inspect(&swapped).await?;
    assert_eq!(v.refund, vec![other.owner().to_string()]);
    let e = acn_wallet::settlement::check_payment_from(&t, &v, perf.owner(), req.owner())
        .expect_err("substituted refund key");
    assert!(e.contains("refunds to a key other"), "{e}");

    mint.stop();
    Ok(())
}

/// A09: a refund swap whose response (or process) is lost is completed on reopen, and an
/// intent that never reached the mint leaves the lock refundable.
#[tokio::test(flavor = "multi_thread")]
async fn refund_recovers_after_interruption() -> anyhow::Result<()> {
    let mint = test_mint().await?;
    let dir = temp_dir("refund-recovery");
    let perf = CashuWallet::open(&mint.url, &key(2), None).await?;
    let req = CashuWallet::open(&mint.url, &key(1), Some(&dir)).await?;
    req.top_up(50).await?;
    let h = hex::encode(Sha256::digest([7u8; 32]));
    let lt = cdk::util::unix_time() + 2;
    let lost = req.lock_htlc(5, &h, perf.owner(), lt, Some("c1")).await?;
    let unsent = req.lock_htlc(7, &h, perf.owner(), lt, Some("c2")).await?;
    assert_eq!(req.balance().await?, 38);
    tokio::time::sleep(Duration::from_secs(4)).await;

    req.refund_interrupted(&unsent, RefundInterrupt::BeforeSubmit)
        .await?;
    req.refund_interrupted(&lost, RefundInterrupt::AfterSubmit)
        .await?;
    // The mint spent the lock, but the wallet never stored the outputs.
    assert!(!perf.inspect(&lost).await?.unspent);
    assert!(perf.inspect(&unsent).await?.unspent);
    assert_eq!(req.balance().await?, 38);
    drop(req);

    let req = CashuWallet::open(&mint.url, &key(1), Some(&dir)).await?;
    let v = req.view().await?;
    assert_eq!(v.available, 43, "the lost swap's outputs were restored");
    let state = |c: &str| {
        v.locks
            .iter()
            .find(|l| l.commitment.as_deref() == Some(c))
            .map(|l| l.state)
    };
    assert_eq!(state("c1"), Some(LockState::Refunded));
    assert_eq!(state("c2"), Some(LockState::Locked), "never submitted");
    assert!(v
        .history
        .iter()
        .any(|e| e.kind == acn_wallet::EntryKind::Refund && e.commitment.as_deref() == Some("c1")));

    // The unsubmitted intent was dropped; the lock refunds normally, and the restored proofs
    // are spendable.
    assert_eq!(req.refund(&unsent).await?, 7);
    assert_eq!(req.balance().await?, 50);
    assert_eq!(req.recover_refunds().await?, 0, "nothing left to recover");
    let spend = req
        .lock_htlc(50, &h, perf.owner(), cdk::util::unix_time() + 60, None)
        .await?;
    assert_eq!(perf.inspect(&spend).await?.amount, 50);

    drop(req);
    let _ = std::fs::remove_dir_all(&dir);
    mint.stop();
    Ok(())
}

/// A CDK mint like `nmesh-mint`, but charging `input_fee_ppk` on every input.
async fn fee_mint(input_fee_ppk: u64) -> anyhow::Result<(String, tokio::task::JoinHandle<()>)> {
    use cdk::mint::{MintBuilder, MintMeltLimits};
    use cdk::nuts::nut00::KnownMethod;
    use cdk::nuts::PaymentMethod;
    use cdk::types::{FeeReserve, QuoteTTL};

    let db = Arc::new(cdk_sqlite::mint::memory::empty().await?);
    let mut builder = MintBuilder::new(db.clone());
    let backend = cdk_fake_wallet::FakeWallet::new(
        FeeReserve {
            min_fee_reserve: 1.into(),
            percent_fee_reserve: 1.0,
        },
        HashMap::default(),
        HashSet::default(),
        1,
        CurrencyUnit::Sat,
    );
    builder
        .add_payment_processor(
            CurrencyUnit::Sat,
            PaymentMethod::Known(KnownMethod::Bolt11),
            MintMeltLimits::new(1, 1_000_000),
            Arc::new(backend),
        )
        .await?;
    builder.set_unit_fee(&CurrencyUnit::Sat, input_fee_ppk)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let builder = builder.with_urls(vec![url.clone()]);
    let seed = bip39::Mnemonic::generate(12)?.to_seed_normalized("");
    let mint = Arc::new(builder.build_with_seed(db, &seed).await?);
    mint.set_quote_ttl(QuoteTTL::new(10_000, 10_000)).await?;
    mint.start().await?;
    let router = cdk_axum::create_mint_router(mint, vec!["bolt11".to_string()]).await?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok((url, task))
}

/// A18: refund outputs are the inputs minus the mint's input fees.
#[tokio::test(flavor = "multi_thread")]
async fn refund_pays_input_fees() -> anyhow::Result<()> {
    let (url, task) = fee_mint(100).await?;
    let req = CashuWallet::open(&url, &key(1), None).await?;
    let perf = CashuWallet::open(&url, &key(2), None).await?;
    req.top_up(100).await?;
    let h = hex::encode(Sha256::digest([7u8; 32]));
    let tok = req
        .lock_htlc(21, &h, perf.owner(), cdk::util::unix_time() + 2, None)
        .await?;
    let inputs: u64 = v4(&tok).token.iter().map(|g| g.proofs.len() as u64).sum();
    let fee = (inputs * 100).div_ceil(1000);
    let before = req.balance().await?;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(req.refund(&tok).await?, 21 - fee);
    assert_eq!(req.balance().await?, before + 21 - fee);
    task.abort();
    Ok(())
}
