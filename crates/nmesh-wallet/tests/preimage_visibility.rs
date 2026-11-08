//! CR-002 Phase B spike: after the receiver redeems a hash-locked (NUT-14) token, can the payer
//! learn the preimage from the mint's NUT-07 state check, without the receiver publishing it?
//! Report: docs/acn/spikes/cashu-preimage-visibility.md.
//!
//! Runs against an in-process CDK mint by default. Set `NMESH_SPIKE_MINT=<url>` to run it
//! against another mint (e.g. Nutshell); `NMESH_SPIKE_OUT=<dir>` saves the checkstate request.

use std::str::FromStr;
use std::sync::Arc;

use cdk::nuts::{CurrencyUnit, KeySetInfo, Token, Witness};
use cdk::wallet::Wallet;
use nmesh_wallet::CashuWallet;
use sha2::{Digest, Sha256};

#[tokio::test(flavor = "multi_thread")]
async fn payer_learns_the_preimage_from_checkstate() -> anyhow::Result<()> {
    let (url, local) = match std::env::var("NMESH_SPIKE_MINT") {
        Ok(u) => (u, None),
        Err(_) => {
            let m = nmesh_mint::start(nmesh_mint::MintConfig {
                listen: "127.0.0.1:0".parse()?,
                url: None,
                db: None,
                mnemonic: None,
            })
            .await?;
            (m.url.clone(), Some(m))
        }
    };
    // Fresh keys per run, so an external mint's state from earlier runs doesn't interfere.
    let mut seed = [0u8; 32];
    rand_bytes(&mut seed);
    let mut payer_key = seed;
    payer_key[0] = 1;
    let mut perf_key = seed;
    perf_key[0] = 2;
    let payer = CashuWallet::open(&url, &payer_key, None).await?;
    let perf = CashuWallet::open(&url, &perf_key, None).await?;
    payer.top_up(64).await?;

    let k: [u8; 32] = seed;
    let h = hex::encode(Sha256::digest(k));
    let tok = payer
        .lock_htlc(21, &h, perf.owner(), cdk::util::unix_time() + 3600, None)
        .await?;

    // Before redemption: unspent, no witness.
    let before = states(&url, &tok).await?;
    assert!(before.iter().all(|s| s.witness.is_none()));

    perf.redeem(&tok, &hex::encode(k), None).await?;

    // The payer, using only its own token (it knows every proof's Y), asks the mint.
    let after = states(&url, &tok).await?;
    let mut revealed = 0;
    for s in &after {
        println!("Y={} state={:?} witness={:?}", s.y, s.state, s.witness);
        if let Some(Witness::HTLCWitness(w)) = &s.witness {
            assert_eq!(
                w.preimage,
                hex::encode(k),
                "the witness carries the preimage"
            );
            revealed += 1;
        }
    }
    assert_eq!(
        revealed,
        after.len(),
        "every spent proof reveals the preimage"
    );

    if let Ok(dir) = std::env::var("NMESH_SPIKE_OUT") {
        let ys: Vec<String> = after.iter().map(|s| s.y.to_hex()).collect();
        std::fs::write(
            format!("{dir}/checkstate-request.json"),
            serde_json::json!({ "Ys": ys }).to_string(),
        )?;
        std::fs::write(format!("{dir}/preimage.txt"), hex::encode(k))?;
    }
    if let Some(m) = local {
        m.stop();
    }
    Ok(())
}

async fn states(url: &str, tok: &str) -> anyhow::Result<Vec<cdk::nuts::ProofState>> {
    let w = Wallet::new(
        url,
        CurrencyUnit::Sat,
        Arc::new(cdk_sqlite::wallet::memory::empty().await?),
        [7; 64],
        None,
    )?;
    let ks: Vec<KeySetInfo> = w
        .keysets(Default::default())
        .await?
        .into_iter()
        .map(|k| KeySetInfo {
            id: k.id,
            unit: k.unit,
            active: k.active.unwrap_or(true),
            input_fee_ppk: k.input_fee_ppk,
            final_expiry: None,
        })
        .collect();
    Ok(w.check_proofs_spent(Token::from_str(tok)?.proofs(&ks)?)
        .await?)
}

fn rand_bytes(out: &mut [u8; 32]) {
    use sha2::Digest;
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    out.copy_from_slice(&Sha256::digest(format!("{t}-{}", std::process::id())));
}
