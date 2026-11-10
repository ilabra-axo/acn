//! A Cashu wallet for ACN settlement (docs/Core.md §8), built on CDK.
//!
//! One [`CashuWallet`] per (identity, mint). Balances are real ecash held in a CDK wallet
//! database; on top of it a small index records the HTLC locks this identity created or received
//! and a history linked to commitments, so the UI can show *why* money moved.
//!
//! Keys: Cashu P2PK/HTLC keys are the identity's Nostr keys. A Nostr x-only key `X` is used as the
//! compressed key `02‖X`; NUT-11 signatures are BIP-340 over the x-only key, so the parity does
//! not matter.
//!
//! Refunds: CDK's `receive` always supplies the preimage path for HTLC proofs, so the NUT-14
//! refund path (refund-key signature after the locktime) is done here with a manual swap. Its
//! outputs are NUT-13 deterministic (counters reserved in the CDK database) and the swap intent is
//! written to the index before the swap is posted, so a swap whose response or process is lost is
//! completed on the next [`CashuWallet::open`] (or [`CashuWallet::recover_refunds`]) through the
//! mint's NUT-09 restore. CDK's own seed restore would also find these outputs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acn_wallet::{Entry, EntryKind, Lock, LockState, Verified, WalletView};
use cdk::amount::{Amount, SplitTarget};
use cdk::nuts::nut00::KnownMethod;
use cdk::nuts::nut10::Tag;
use cdk::nuts::nut14::HTLCWitness;
use cdk::nuts::{
    BlindSignature, Conditions, CurrencyUnit, Id, KeySetInfo, Kind, Nut10Secret, PaymentMethod,
    PreMintSecrets, Proofs, PublicKey, RestoreRequest, SecretKey, SigFlag, SpendingConditions,
    State, SwapRequest, Token, Witness,
};
use cdk::wallet::types::ProofInfo;
use cdk::wallet::{ReceiveOptions, SendOptions, Wallet};
use serde::{Deserialize, Serialize};

pub use acn_wallet;

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("cashu: {0}")]
    Cdk(String),
    #[error("{0}")]
    Invalid(String),
    #[error("insufficient funds: have {have} sat, need {need} sat")]
    InsufficientFunds { have: u64, need: u64 },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

fn cdk_err(e: impl std::fmt::Display) -> WalletError {
    WalletError::Cdk(e.to_string())
}

/// Persistent part of the wallet that CDK does not know about.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    /// NUT-13 seed of the CDK wallet (random per database, never derived from the Nostr key).
    seed: String,
    locks: Vec<Lock>,
    /// lock id -> serialized token
    tokens: BTreeMap<String, String>,
    history: Vec<Entry>,
    /// Refund swaps that may have reached the mint but whose outputs are not stored yet.
    #[serde(default)]
    pending_refunds: Vec<PendingRefund>,
}

/// A refund swap's intent, written before the swap is posted. The outputs are NUT-13
/// deterministic: `amount` split over the keyset's denominations, from `counter` on.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingRefund {
    lock_id: String,
    token: String,
    keyset: String,
    counter: u32,
    amount: u64,
    created_at: u64,
}

/// A refund ready to post: the persisted intent, its outputs and the signed swap.
struct PreparedRefund {
    pending: PendingRefund,
    outputs: PreMintSecrets,
    request: SwapRequest,
}

/// Where [`CashuWallet::refund_interrupted`] stops, simulating a crash (tests).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefundInterrupt {
    /// The intent is persisted; the swap is never posted.
    BeforeSubmit,
    /// The mint has executed the swap; its response is lost.
    AfterSubmit,
}

/// The effective NUT-14 spending policy of one proof, in the only shape ACN accepts (§8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
struct LockPolicy {
    hashlock: String,
    receiver: String,
    locktime: u64,
    refund: String,
}

pub struct CashuWallet {
    wallet: Wallet,
    url: String,
    seed: [u8; 64],
    sk: SecretKey,
    me: String,
    index: Mutex<Index>,
    index_path: Option<PathBuf>,
}

/// Compressed Cashu key for a Nostr x-only key (hex).
pub fn cashu_pubkey(xonly_hex: &str) -> Result<PublicKey, WalletError> {
    PublicKey::from_hex(format!("02{xonly_hex}"))
        .map_err(|e| WalletError::Invalid(format!("bad key {xonly_hex}: {e}")))
}

fn xonly(pk: &PublicKey) -> String {
    pk.x_only_public_key().to_string()
}

fn now() -> u64 {
    cdk::util::unix_time()
}

pub fn lock_id(token: &str) -> String {
    use sha2::Digest;
    let h = hex::encode(sha2::Sha256::digest(token.as_bytes()));
    format!("tok{}", &h[..16])
}

impl CashuWallet {
    /// Open (or create) the wallet of `secret` (the identity's 32-byte Nostr secret key) at
    /// `mint_url`. `dir = None` keeps everything in memory.
    pub async fn open(
        mint_url: &str,
        secret: &[u8; 32],
        dir: Option<&Path>,
    ) -> Result<Self, WalletError> {
        let sk = SecretKey::from_slice(secret).map_err(cdk_err)?;
        let me = xonly(&sk.public_key());
        let name = format!("{}-{}", &me[..16], sanitize(mint_url));
        let (index, index_path, db): (
            Index,
            _,
            Arc<dyn cdk::cdk_database::WalletDatabase<cdk::cdk_database::Error> + Send + Sync>,
        ) = match dir {
            Some(d) => {
                std::fs::create_dir_all(d)?;
                let ip = d.join(format!("{name}.json"));
                let index = std::fs::read(&ip)
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                let db = cdk_sqlite::WalletSqliteDatabase::new(d.join(format!("{name}.sqlite")))
                    .await
                    .map_err(cdk_err)?;
                (index, Some(ip), Arc::new(db))
            }
            None => (
                Index::default(),
                None,
                Arc::new(cdk_sqlite::wallet::memory::empty().await.map_err(cdk_err)?),
            ),
        };
        let mut index = index;
        if index.seed.is_empty() {
            let mut s = [0u8; 64];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut s);
            index.seed = hex::encode(s);
        }
        let seed: [u8; 64] = hex::decode(&index.seed)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| WalletError::Invalid("corrupt wallet seed".into()))?;
        let wallet = Wallet::new(mint_url, CurrencyUnit::Sat, db, seed, None).map_err(cdk_err)?;
        let pending = !index.pending_refunds.is_empty();
        let w = Self {
            wallet,
            url: mint_url.trim_end_matches('/').to_string(),
            seed,
            sk,
            me,
            index: Mutex::new(index),
            index_path,
        };
        w.save()?;
        if pending {
            // Best effort: an unreachable mint must not stop the wallet from opening; the
            // intent stays persisted and the next refund or open retries.
            if let Err(e) = w.recover_refunds().await {
                tracing::warn!(error = %e, "could not complete an interrupted refund yet");
            }
        }
        Ok(w)
    }

    pub fn mint_url(&self) -> &str {
        &self.url
    }

    /// This wallet's key (x-only hex) = the identity's Nostr key.
    pub fn owner(&self) -> &str {
        &self.me
    }

    fn save(&self) -> Result<(), WalletError> {
        if let Some(p) = &self.index_path {
            let b = serde_json::to_vec_pretty(&*self.index.lock().unwrap())
                .map_err(|e| WalletError::Invalid(e.to_string()))?;
            std::fs::write(p, b)?;
        }
        Ok(())
    }

    fn log(&self, kind: EntryKind, delta: i64, commitment: Option<&str>, note: &str) {
        self.index.lock().unwrap().history.push(Entry {
            at: now(),
            kind,
            delta,
            commitment: commitment.map(str::to_owned),
            note: note.into(),
        });
    }

    pub async fn balance(&self) -> Result<u64, WalletError> {
        Ok(self.wallet.total_balance().await.map_err(cdk_err)?.into())
    }

    /// Mint test sats: request a bolt11 quote and wait for it to be paid (the fake Lightning
    /// backend of `nmesh-mint` pays automatically; a real mint needs a real payment).
    pub async fn top_up(&self, amount: u64) -> Result<u64, WalletError> {
        if amount == 0 {
            return Err(WalletError::Invalid("amount must be > 0".into()));
        }
        let q = self
            .wallet
            .mint_quote(
                PaymentMethod::Known(KnownMethod::Bolt11),
                Some(Amount::from(amount)),
                None,
                None,
            )
            .await
            .map_err(cdk_err)?;
        self.wallet
            .wait_and_mint_quote(q, SplitTarget::default(), None, Duration::from_secs(30))
            .await
            .map_err(cdk_err)?;
        self.log(EntryKind::TopUp, amount as i64, None, "Top up (test sats)");
        self.save()?;
        self.balance().await
    }

    /// Hash-lock `amount` to `receiver` (x-only hex): redeemable with the preimage of `hashlock`
    /// and the receiver's signature; refundable to this wallet after `locktime` (NUT-14).
    pub async fn lock_htlc(
        &self,
        amount: u64,
        hashlock: &str,
        receiver: &str,
        locktime: u64,
        commitment: Option<&str>,
    ) -> Result<String, WalletError> {
        // The §8.1 lock spec, and exactly what `inspect` accepts: one receiver key, one
        // signature, SIG_INPUTS, this wallet's key as the single refund key.
        let cond = Conditions::new(
            Some(locktime),
            Some(vec![cashu_pubkey(receiver)?]),
            Some(vec![self.sk.public_key()]),
            None,
            None,
            None,
        )
        .map_err(cdk_err)?;
        let sc = SpendingConditions::new_htlc_hash(hashlock, Some(cond)).map_err(cdk_err)?;
        let token = self.send_with_conditions(amount, sc).await?;
        let id = lock_id(&token);
        {
            let mut ix = self.index.lock().unwrap();
            ix.locks.push(Lock {
                id: id.clone(),
                amount,
                hashlock: hashlock.to_ascii_lowercase(),
                receiver: receiver.to_string(),
                refund: self.me.clone(),
                locktime,
                commitment: commitment.map(str::to_owned),
                state: LockState::Locked,
                created_at: now(),
                settled_at: None,
            });
            ix.tokens.insert(id, token.clone());
        }
        self.log(
            EntryKind::Lock,
            -(amount as i64),
            commitment,
            "Hash-locked to the performer",
        );
        self.save()?;
        Ok(token)
    }

    /// A token of `amount` under arbitrary spending conditions, not recorded in the index. Only
    /// [`Self::lock_htlc`] builds ACN locks; this is exposed so tests can build hostile ones.
    #[doc(hidden)]
    pub async fn send_with_conditions(
        &self,
        amount: u64,
        conditions: SpendingConditions,
    ) -> Result<String, WalletError> {
        let have = self.balance().await?;
        if have < amount {
            return Err(WalletError::InsufficientFunds { have, need: amount });
        }
        Ok(self
            .wallet
            .prepare_send(
                Amount::from(amount),
                SendOptions {
                    conditions: Some(conditions),
                    ..Default::default()
                },
            )
            .await
            .map_err(cdk_err)?
            .confirm(None)
            .await
            .map_err(cdk_err)?
            .to_string())
    }

    /// Attach the commitment id once the REQUEST is signed (the lock is made first).
    pub fn tag(&self, token: &str, commitment: &str) -> Result<(), WalletError> {
        let id = lock_id(token);
        {
            let mut ix = self.index.lock().unwrap();
            for l in ix.locks.iter_mut().filter(|l| l.id == id) {
                l.commitment = Some(commitment.into());
            }
            if let Some(e) = ix
                .history
                .iter_mut()
                .rev()
                .find(|e| e.kind == EntryKind::Lock && e.commitment.is_none())
            {
                e.commitment = Some(commitment.into());
            }
        }
        self.save()
    }

    async fn keysets(&self) -> Result<Vec<KeySetInfo>, WalletError> {
        Ok(self
            .wallet
            .keysets(Default::default())
            .await
            .map_err(cdk_err)?
            .into_iter()
            .map(|k| KeySetInfo {
                id: k.id,
                unit: k.unit,
                active: k.active.unwrap_or(true),
                input_fee_ppk: k.input_fee_ppk,
                final_expiry: None,
            })
            .collect())
    }

    async fn proofs_of(&self, token: &Token) -> Result<Proofs, WalletError> {
        token.proofs(&self.keysets().await?).map_err(cdk_err)
    }

    /// The performer's check before promising (§8.2 step 3): same mint and unit, valid DLEQ
    /// proofs (NUT-12), distinct proofs, every proof under the same complete §8.1 HTLC policy
    /// (NUT-14), and the mint's state (NUT-07) for exactly those proofs.
    ///
    /// Locks outside the §8.1 shape (extra receiver keys, thresholds, `SIG_ALL`, no or several
    /// refund keys, unknown or repeated tags) are rejected here; the refund key itself is
    /// returned for [`acn_wallet::settlement::check_payment_from`] to compare with the requester.
    pub async fn inspect(&self, token: &str) -> Result<Verified, WalletError> {
        let t = Token::from_str(token).map_err(cdk_err)?;
        let mint = t.mint_url().map_err(cdk_err)?.to_string();
        if mint.trim_end_matches('/') != self.url {
            return Err(WalletError::Invalid(format!(
                "token is from {mint}, this wallet uses {}",
                self.url
            )));
        }
        if let Some(u) = t.unit().filter(|u| *u != CurrencyUnit::Sat) {
            return Err(WalletError::Invalid(format!("token is in {u}, not sat")));
        }
        self.wallet.verify_token_dleq(&t).await.map_err(cdk_err)?;
        let keysets = self.keysets().await?;
        let proofs = t.proofs(&keysets).map_err(cdk_err)?;
        if proofs.is_empty() {
            return Err(WalletError::Invalid("empty token".into()));
        }

        // Distinct inputs before any accounting: a repeated proof is one spendable input.
        let mut ys = HashSet::new();
        let mut secrets = HashSet::new();
        for p in &proofs {
            let y = p.y().map_err(cdk_err)?;
            if !ys.insert(y) || !secrets.insert(p.secret.to_string()) {
                return Err(WalletError::Invalid("token repeats a proof".into()));
            }
            if !keysets
                .iter()
                .any(|k| k.id == p.keyset_id && k.unit == CurrencyUnit::Sat)
            {
                return Err(WalletError::Invalid(format!(
                    "proof from unknown keyset {}",
                    p.keyset_id
                )));
            }
        }

        let mut policy: Option<LockPolicy> = None;
        let mut amount = 0u64;
        for p in &proofs {
            let this = lock_policy(&p.secret)?;
            match &policy {
                None => policy = Some(this),
                Some(s) if *s == this => {}
                Some(_) => {
                    return Err(WalletError::Invalid(
                        "token proofs carry different locks".into(),
                    ))
                }
            }
            amount = amount
                .checked_add(u64::from(p.amount))
                .ok_or_else(|| WalletError::Invalid("token amount overflows".into()))?;
        }
        let policy = policy.ok_or_else(|| WalletError::Invalid("empty token".into()))?;

        let states = self
            .wallet
            .check_proofs_spent(proofs)
            .await
            .map_err(cdk_err)?;
        if states.len() != ys.len() || !states.iter().all(|s| ys.remove(&s.y)) {
            return Err(WalletError::Invalid(
                "the mint's state response does not match the token's proofs".into(),
            ));
        }
        Ok(Verified {
            amount,
            hashlock: policy.hashlock,
            receiver: policy.receiver.clone(),
            locktime: policy.locktime,
            unspent: states.iter().all(|s| s.state == State::Unspent),
            receivers: vec![policy.receiver],
            n_sigs: 1,
            refund: vec![policy.refund],
            n_sigs_refund: 1,
            sig_flag: acn_wallet::SIG_INPUTS.into(),
            keysets: {
                let mut ids: Vec<String> = t
                    .proofs(&keysets)
                    .map_err(cdk_err)?
                    .iter()
                    .map(|p| p.keyset_id.to_string())
                    .collect();
                ids.sort();
                ids.dedup();
                ids
            },
        })
    }

    /// The mint's own capabilities (NUT-06 `/v1/info`), observed rather than declared: Core
    /// §8.6 asks payers to check a rail before relying on it. Returns the supported NUT numbers
    /// among 7, 10, 11, 12 and 14.
    pub async fn observed_nuts(&self) -> Result<Vec<u8>, WalletError> {
        let info = self
            .wallet
            .fetch_mint_info()
            .await
            .map_err(cdk_err)?
            .ok_or_else(|| WalletError::Invalid("the mint publishes no info (NUT-06)".into()))?;
        let n = &info.nuts;
        Ok([
            (7, n.nut07.supported),
            (10, n.nut10.supported),
            (11, n.nut11.supported),
            (12, n.nut12.supported),
            (14, n.nut14.supported),
        ]
        .into_iter()
        .filter_map(|(k, s)| s.then_some(k))
        .collect())
    }

    /// Receiver path: redeem an HTLC token addressed to me with the revealed preimage.
    pub async fn redeem(
        &self,
        token: &str,
        preimage: &str,
        commitment: Option<&str>,
    ) -> Result<u64, WalletError> {
        let got: u64 = self
            .wallet
            .receive(
                token,
                ReceiveOptions {
                    p2pk_signing_keys: vec![self.sk.clone()],
                    preimages: vec![preimage.to_string()],
                    ..Default::default()
                },
            )
            .await
            .map_err(cdk_err)?
            .into();
        self.mark(token, LockState::Redeemed);
        self.log(
            EntryKind::Redeem,
            got as i64,
            commitment,
            "Redeemed with the revealed preimage",
        );
        self.save()?;
        Ok(got)
    }

    /// Sender path: reclaim my own lock after its locktime (refund-key signature, NUT-14).
    /// Returns the amount received: the lock minus the mint's input fees.
    ///
    /// The swap intent is persisted before the swap is posted; if the response or the process
    /// is lost, [`Self::recover_refunds`] (also run by [`Self::open`]) completes it.
    pub async fn refund(&self, token: &str) -> Result<u64, WalletError> {
        self.recover_refunds().await?;
        let p = self.prepare_refund(token).await?;
        let sigs = self.submit_refund(&p).await?;
        self.finish_refund(&p.pending, &p.outputs, sigs).await
    }

    /// [`Self::refund`], stopped at `at` as if the process had died there (tests).
    #[doc(hidden)]
    pub async fn refund_interrupted(
        &self,
        token: &str,
        at: RefundInterrupt,
    ) -> Result<(), WalletError> {
        let p = self.prepare_refund(token).await?;
        if at == RefundInterrupt::AfterSubmit {
            self.submit_refund(&p).await?;
        }
        Ok(())
    }

    /// NUT-13 outputs of a refund intent (the same ones on every call).
    async fn refund_outputs(&self, p: &PendingRefund) -> Result<PreMintSecrets, WalletError> {
        let keyset = Id::from_str(&p.keyset).map_err(cdk_err)?;
        let fees = self
            .wallet
            .get_keyset_fees_and_amounts_by_id(keyset)
            .await
            .map_err(cdk_err)?;
        PreMintSecrets::from_seed(
            keyset,
            p.counter,
            &self.seed,
            Amount::from(p.amount),
            &SplitTarget::default(),
            &fees,
        )
        .map_err(cdk_err)
    }

    /// Sign the refund path, size the outputs net of input fees, reserve their NUT-13 counters
    /// and persist the intent. Nothing has reached the mint when this returns.
    async fn prepare_refund(&self, token: &str) -> Result<PreparedRefund, WalletError> {
        let t = Token::from_str(token).map_err(cdk_err)?;
        let mut proofs = self.proofs_of(&t).await?;
        let total = proofs
            .iter()
            .try_fold(0u64, |a, p| a.checked_add(u64::from(p.amount)))
            .ok_or_else(|| WalletError::Invalid("token amount overflows".into()))?;
        let fee: u64 = self
            .wallet
            .get_proofs_fee(&proofs)
            .await
            .map_err(cdk_err)?
            .total
            .into();
        let amount = total.checked_sub(fee).filter(|a| *a > 0).ok_or_else(|| {
            WalletError::Invalid(format!(
                "the {total} sat lock does not cover the mint's {fee} sat input fee"
            ))
        })?;
        for p in proofs.iter_mut() {
            let sig = self.sk.sign(&p.secret.to_bytes()).map_err(cdk_err)?;
            p.witness = Some(Witness::HTLCWitness(HTLCWitness {
                // Not the preimage: the mint takes the refund path once the locktime has passed.
                preimage: "00".repeat(32),
                signatures: Some(vec![sig.to_string()]),
            }));
        }
        let active = self.wallet.active_keyset().await.map_err(cdk_err)?;
        let fees = self
            .wallet
            .get_keyset_fees_and_amounts_by_id(active.id)
            .await
            .map_err(cdk_err)?;
        let n = Amount::from(amount)
            .split_targeted(&SplitTarget::default(), &fees)
            .map_err(cdk_err)?
            .len() as u32;
        let end = self
            .wallet
            .localstore
            .increment_keyset_counter(&active.id, n)
            .await
            .map_err(cdk_err)?;
        let pending = PendingRefund {
            lock_id: lock_id(token),
            token: token.to_string(),
            keyset: active.id.to_string(),
            counter: end - n,
            amount,
            created_at: now(),
        };
        let outputs = self.refund_outputs(&pending).await?;
        {
            let mut ix = self.index.lock().unwrap();
            ix.pending_refunds.retain(|p| p.lock_id != pending.lock_id);
            ix.pending_refunds.push(pending.clone());
        }
        // The intent must be durable before the mint can spend the inputs.
        self.save()?;
        let request = SwapRequest::new(proofs, outputs.blinded_messages());
        Ok(PreparedRefund {
            pending,
            outputs,
            request,
        })
    }

    async fn submit_refund(&self, p: &PreparedRefund) -> Result<Vec<BlindSignature>, WalletError> {
        match self
            .wallet
            .mint_connector()
            .post_swap(p.request.clone())
            .await
        {
            Ok(resp) => Ok(resp.signatures),
            Err(e) => {
                // Rejected, or an unknown outcome: reconcile with the mint now so a definite
                // rejection leaves no intent behind; an unknown one stays for the next attempt.
                if let Err(r) = self.recover_refunds().await {
                    tracing::warn!(error = %r, "refund outcome unknown; will retry recovery");
                }
                Err(cdk_err(e))
            }
        }
    }

    /// Store the refund's outputs (idempotent), then settle the lock and drop the intent.
    async fn finish_refund(
        &self,
        p: &PendingRefund,
        outputs: &PreMintSecrets,
        sigs: Vec<BlindSignature>,
    ) -> Result<u64, WalletError> {
        let keyset = Id::from_str(&p.keyset).map_err(cdk_err)?;
        let keys = self.wallet.keyset(keyset).await.map_err(cdk_err)?.keys;
        let fresh = cdk::dhke::construct_proofs(sigs, outputs.rs(), outputs.secrets(), &keys)
            .map_err(cdk_err)?;
        let mut ys = Vec::with_capacity(fresh.len());
        for f in &fresh {
            ys.push(f.y().map_err(cdk_err)?);
        }
        let known: HashSet<PublicKey> = self
            .wallet
            .localstore
            .get_proofs_by_ys(ys)
            .await
            .map_err(cdk_err)?
            .into_iter()
            .map(|i| i.y)
            .collect();
        let mut add = Vec::new();
        for f in fresh {
            if !known.contains(&f.y().map_err(cdk_err)?) {
                add.push(
                    ProofInfo::new(
                        f,
                        self.wallet.mint_url.clone(),
                        State::Unspent,
                        CurrencyUnit::Sat,
                    )
                    .map_err(cdk_err)?,
                );
            }
        }
        // The outputs come from this wallet's own secrets, so they are stored as they are.
        self.wallet
            .localstore
            .update_proofs(add, vec![])
            .await
            .map_err(cdk_err)?;
        let commitment = {
            let mut ix = self.index.lock().unwrap();
            ix.pending_refunds.retain(|q| q.lock_id != p.lock_id);
            ix.locks
                .iter_mut()
                .find(|l| l.id == p.lock_id)
                .and_then(|l| {
                    l.state = LockState::Refunded;
                    l.settled_at = Some(now());
                    l.commitment.clone()
                })
        };
        self.log(
            EntryKind::Refund,
            p.amount as i64,
            commitment.as_deref(),
            "Reclaimed after the locktime",
        );
        self.save()?;
        Ok(p.amount)
    }

    /// Complete refund swaps whose outcome was not recorded (lost response or crash): outputs
    /// the mint signed are restored (NUT-09) and stored; an intent whose inputs are still
    /// unspent never reached the mint and is dropped (the lock stays refundable). Returns the
    /// amount recovered.
    pub async fn recover_refunds(&self) -> Result<u64, WalletError> {
        let pending = self.index.lock().unwrap().pending_refunds.clone();
        let mut got = 0u64;
        for p in pending {
            got = got.saturating_add(self.recover_refund(&p).await?);
        }
        Ok(got)
    }

    async fn recover_refund(&self, p: &PendingRefund) -> Result<u64, WalletError> {
        let outputs = self.refund_outputs(p).await?;
        let resp = self
            .wallet
            .mint_connector()
            .post_restore(RestoreRequest {
                outputs: outputs.blinded_messages(),
            })
            .await
            .map_err(cdk_err)?;
        if !resp.signatures.is_empty() {
            // The swap's outputs are signed together, so all of them must come back.
            let mut by_b: HashMap<PublicKey, BlindSignature> = HashMap::new();
            if resp.outputs.len() == resp.signatures.len() {
                for (o, s) in resp.outputs.iter().zip(resp.signatures) {
                    by_b.insert(o.blinded_secret, s);
                }
            }
            let sigs: Option<Vec<BlindSignature>> = outputs
                .iter()
                .map(|m| by_b.remove(&m.blinded_message.blinded_secret))
                .collect();
            let sigs = sigs.filter(|_| by_b.is_empty()).ok_or_else(|| {
                WalletError::Cdk("the mint restored only part of a refund's outputs".into())
            })?;
            return self.finish_refund(p, &outputs, sigs).await;
        }
        // Nothing signed: the swap did not happen. Where did the inputs go?
        let t = Token::from_str(&p.token).map_err(cdk_err)?;
        let states = self
            .wallet
            .check_proofs_spent(self.proofs_of(&t).await?)
            .await
            .map_err(cdk_err)?;
        if states.iter().any(|s| s.state == State::Pending) {
            return Ok(0);
        }
        let spent = !states.is_empty() && states.iter().all(|s| s.state == State::Spent);
        {
            let mut ix = self.index.lock().unwrap();
            ix.pending_refunds.retain(|q| q.lock_id != p.lock_id);
            if spent {
                // Spent without our outputs: the receiver redeemed it.
                if let Some(l) = ix.locks.iter_mut().find(|l| l.id == p.lock_id) {
                    l.state = LockState::Redeemed;
                    l.settled_at = Some(now());
                }
            }
        }
        self.save()?;
        Ok(0)
    }

    /// Contingent settlement (CR-002): once the receiver has redeemed `token`, the mint's NUT-07
    /// state check returns each spent proof's witness, which carries the preimage. `None` while
    /// unspent, or if the mint returns no witness (no NUT-07). See
    /// docs/acn/spikes/cashu-preimage-visibility.md.
    pub async fn revealed_preimage(&self, token: &str) -> Result<Option<String>, WalletError> {
        let t = Token::from_str(token).map_err(cdk_err)?;
        let states = self
            .wallet
            .check_proofs_spent(self.proofs_of(&t).await?)
            .await
            .map_err(cdk_err)?;
        Ok(states.iter().find_map(|s| match (&s.state, &s.witness) {
            (State::Spent, Some(Witness::HTLCWitness(w))) => Some(w.preimage.clone()),
            _ => None,
        }))
    }

    /// Record an incoming lock (performer side) so the wallet can show it before redemption.
    pub fn note_incoming(&self, token: &str, v: &Verified, from: &str, commitment: &str) {
        let id = lock_id(token);
        {
            let mut ix = self.index.lock().unwrap();
            if ix.locks.iter().any(|l| l.id == id) {
                return;
            }
            ix.locks.push(Lock {
                id: id.clone(),
                amount: v.amount,
                hashlock: v.hashlock.clone(),
                receiver: self.me.clone(),
                refund: from.to_string(),
                locktime: v.locktime,
                commitment: Some(commitment.into()),
                state: LockState::Locked,
                created_at: now(),
                settled_at: None,
            });
            ix.tokens.insert(id, token.to_string());
        }
        let _ = self.save();
    }

    /// The token of lock `id` (e.g. a lock whose request was never published).
    pub fn token_of_lock(&self, id: &str) -> Option<String> {
        self.index.lock().unwrap().tokens.get(id).cloned()
    }

    /// The token I locked for `commitment` (requester side).
    pub fn token_for(&self, commitment: &str) -> Option<String> {
        let ix = self.index.lock().unwrap();
        let l = ix
            .locks
            .iter()
            .find(|l| l.commitment.as_deref() == Some(commitment))?;
        ix.tokens.get(&l.id).cloned()
    }

    /// Balance plus locks, with lock states refreshed from the mint (NUT-07): a lock of mine that
    /// is spent but not refunded by me was redeemed by the receiver, and vice versa.
    pub async fn view(&self) -> Result<WalletView, WalletError> {
        let pending: Vec<(String, String)> = {
            let ix = self.index.lock().unwrap();
            ix.locks
                .iter()
                .filter(|l| l.state == LockState::Locked)
                .filter_map(|l| ix.tokens.get(&l.id).map(|t| (l.id.clone(), t.clone())))
                .collect()
        };
        if !pending.is_empty() {
            let ks = self.keysets().await?;
            for (id, tok) in pending {
                let Ok(t) = Token::from_str(&tok) else {
                    continue;
                };
                let Ok(proofs) = t.proofs(&ks) else { continue };
                let Ok(states) = self.wallet.check_proofs_spent(proofs).await else {
                    continue;
                };
                if !states.is_empty() && states.iter().all(|s| s.state == State::Spent) {
                    let mut ix = self.index.lock().unwrap();
                    if let Some(l) = ix.locks.iter_mut().find(|l| l.id == id) {
                        // Spent by the other party: the receiver redeemed, or the sender refunded.
                        l.state = if l.refund == self.me {
                            LockState::Redeemed
                        } else {
                            LockState::Refunded
                        };
                        l.settled_at = Some(now());
                    }
                }
            }
            self.save()?;
        }

        let available = self.balance().await?;
        let ix = self.index.lock().unwrap();
        let open = |f: &dyn Fn(&Lock) -> bool| -> u64 {
            ix.locks
                .iter()
                .filter(|l| l.state == LockState::Locked && f(l))
                .map(|l| l.amount)
                .sum()
        };
        Ok(WalletView {
            owner: self.me.clone(),
            mint: self.url.clone(),
            simulated: false,
            available,
            locked_out: open(&|l| l.refund == self.me),
            incoming: open(&|l| l.receiver == self.me),
            history: ix.history.clone(),
            locks: ix.locks.clone(),
        })
    }

    /// Mark a lock settled locally (e.g. after I redeemed it), without waiting for `view`.
    pub fn mark(&self, token: &str, state: LockState) {
        let id = lock_id(token);
        {
            let mut ix = self.index.lock().unwrap();
            if let Some(l) = ix.locks.iter_mut().find(|l| l.id == id) {
                l.state = state;
                l.settled_at = Some(now());
            }
        }
        let _ = self.save();
    }
}

/// The complete effective spending policy of one proof, accepted only in the §8.1 shape: an
/// HTLC secret whose tags are exactly one `pubkeys` key (the receiver), a `locktime`, one
/// `refund` key, optionally `n_sigs`/`n_sigs_refund` of 1 and `sigflag` SIG_INPUTS, each at
/// most once. Anything else is rejected rather than reduced to its first key.
fn lock_policy(secret: &cdk::secret::Secret) -> Result<LockPolicy, WalletError> {
    let bad = |m: String| WalletError::Invalid(m);
    let n10 = Nut10Secret::try_from(secret)
        .map_err(|_| bad("token is not hash-locked (no NUT-10 secret)".into()))?;
    if n10.kind() != Kind::HTLC {
        return Err(bad("token is P2PK, not an HTLC".into()));
    }
    let SpendingConditions::HTLCConditions { data, .. } = SpendingConditions::try_from(n10.clone())
        .map_err(|e| bad(format!("malformed HTLC lock: {e}")))?
    else {
        return Err(bad("token is not an HTLC".into()));
    };
    let mut seen = HashSet::new();
    let (mut pubkeys, mut locktime, mut refund) = (None, None, None);
    for raw in n10.secret_data().tags().into_iter().flatten() {
        let tag =
            Tag::try_from(raw.clone()).map_err(|e| bad(format!("malformed lock tag: {e}")))?;
        if !seen.insert(tag.kind()) {
            return Err(bad(format!("lock repeats the {} tag", tag.kind())));
        }
        match tag {
            Tag::PubKeys(k) => pubkeys = Some(k),
            Tag::LockTime(t) => locktime = Some(t),
            Tag::Refund(k) => refund = Some(k),
            Tag::NSigs(1) | Tag::NSigsRefund(1) | Tag::SigFlag(SigFlag::SigInputs) => {}
            Tag::NSigs(n) => {
                return Err(bad(format!(
                    "lock requires {n} signatures to redeem; only single-key locks are supported"
                )))
            }
            Tag::NSigsRefund(n) => {
                return Err(bad(format!(
                    "lock requires {n} refund signatures; only a single refund key is supported"
                )))
            }
            Tag::SigFlag(f) => return Err(bad(format!("{f} locks are not supported"))),
            Tag::Custom(name, _) => return Err(bad(format!("unsupported lock tag {name:?}"))),
        }
    }
    let receiver = match pubkeys.as_deref() {
        Some([k]) => xonly(k),
        None | Some([]) => return Err(bad("HTLC has no receiver key".into())),
        Some(k) => {
            return Err(bad(format!(
                "HTLC lets {} keys redeem; only the performer may",
                k.len()
            )))
        }
    };
    let locktime =
        locktime.ok_or_else(|| bad("HTLC has no locktime, so it can never be refunded".into()))?;
    let refund = match refund.as_deref() {
        Some([k]) => xonly(k),
        None | Some([]) => return Err(bad("HTLC has no refund key".into())),
        Some(k) => {
            return Err(bad(format!(
                "HTLC has {} refund keys; only the requester may refund",
                k.len()
            )))
        }
    };
    Ok(LockPolicy {
        hashlock: data.to_string(),
        receiver,
        locktime,
        refund,
    })
}

fn sanitize(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdk::nuts::SecretData;

    fn key(b: u8) -> PublicKey {
        SecretKey::from_slice(&[b; 32]).unwrap().public_key()
    }

    fn secret(kind: Kind, tags: Vec<Vec<String>>) -> cdk::secret::Secret {
        let data = if kind == Kind::HTLC {
            "ab".repeat(32)
        } else {
            key(9).to_hex()
        };
        cdk::secret::Secret::try_from(Nut10Secret::new(kind, SecretData::new(data, Some(tags))))
            .unwrap()
    }

    fn t(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn canonical() -> Vec<Vec<String>> {
        vec![
            t(&["pubkeys", &key(2).to_hex()]),
            t(&["locktime", "1000"]),
            t(&["refund", &key(1).to_hex()]),
            t(&["sigflag", "SIG_INPUTS"]),
        ]
    }

    fn err(tags: Vec<Vec<String>>) -> String {
        lock_policy(&secret(Kind::HTLC, tags))
            .expect_err("must be rejected")
            .to_string()
    }

    #[test]
    fn canonical_policy() {
        let p = lock_policy(&secret(Kind::HTLC, canonical())).unwrap();
        assert_eq!(
            p,
            LockPolicy {
                hashlock: "ab".repeat(32),
                receiver: xonly(&key(2)),
                locktime: 1000,
                refund: xonly(&key(1)),
            }
        );
        // Explicit defaults are the same policy.
        let mut tags = canonical();
        tags.push(t(&["n_sigs", "1"]));
        tags.push(t(&["n_sigs_refund", "1"]));
        assert_eq!(lock_policy(&secret(Kind::HTLC, tags)).unwrap(), p);
    }

    #[test]
    fn non_canonical_policies_are_rejected() {
        let with = |extra: Vec<String>| {
            let mut tags = canonical();
            tags.push(extra);
            tags
        };
        let replace = |i: usize, tag: Vec<String>| {
            let mut tags = canonical();
            tags[i] = tag;
            tags
        };
        let without = |i: usize| {
            let mut tags = canonical();
            tags.remove(i);
            tags
        };
        let (k1, k2, k3) = (key(1).to_hex(), key(2).to_hex(), key(3).to_hex());
        assert!(err(replace(0, t(&["pubkeys", &k2, &k3]))).contains("2 keys redeem"));
        assert!(err(without(0)).contains("no receiver"));
        assert!(err(with(t(&["n_sigs", "2"]))).contains("2 signatures"));
        assert!(err(replace(3, t(&["sigflag", "SIG_ALL"]))).contains("SIG_ALL"));
        assert!(err(without(1)).contains("no locktime"));
        assert!(err(without(2)).contains("no refund key"));
        assert!(err(replace(2, t(&["refund", &k1, &k3]))).contains("2 refund keys"));
        let mut two_of_two = replace(2, t(&["refund", &k1, &k3]));
        two_of_two.push(t(&["n_sigs_refund", "2"]));
        assert!(err(two_of_two).contains("refund signatures"));
        assert!(err(with(t(&["n_sigs_refund", "2"]))).contains("malformed"));
        assert!(err(with(t(&["memo", "x"]))).contains("unsupported lock tag"));
        // CDK keeps the first of a repeated tag; another implementation might keep the last.
        assert!(err(with(t(&["pubkeys", &k3]))).contains("repeats"));
        assert!(err(with(t(&["refund", &k3]))).contains("repeats"));
        assert!(lock_policy(&secret(Kind::P2PK, canonical()))
            .unwrap_err()
            .to_string()
            .contains("P2PK"));
        assert!(lock_policy(&cdk::secret::Secret::generate())
            .unwrap_err()
            .to_string()
            .contains("not hash-locked"));
    }
}
