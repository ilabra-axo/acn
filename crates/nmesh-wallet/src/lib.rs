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
//! refund path (refund-key signature after the locktime) is done here with a manual swap.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acn_wallet::{Entry, EntryKind, Lock, LockState, Verified, WalletView};
use cdk::amount::{Amount, SplitTarget};
use cdk::nuts::nut00::KnownMethod;
use cdk::nuts::nut14::HTLCWitness;
use cdk::nuts::{
    Conditions, CurrencyUnit, KeySetInfo, PaymentMethod, PreMintSecrets, Proofs, PublicKey,
    SecretKey, SpendingConditions, State, SwapRequest, Token, Witness,
};
use cdk::wallet::{HttpClient, MintConnector, ReceiveOptions, SendOptions, Wallet};
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
}

pub struct CashuWallet {
    wallet: Wallet,
    url: String,
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

fn lock_id(token: &str) -> String {
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
        let w = Self {
            wallet,
            url: mint_url.trim_end_matches('/').to_string(),
            sk,
            me,
            index: Mutex::new(index),
            index_path,
        };
        w.save()?;
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
        let have = self.balance().await?;
        if have < amount {
            return Err(WalletError::InsufficientFunds { have, need: amount });
        }
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
        let token = self
            .wallet
            .prepare_send(
                Amount::from(amount),
                SendOptions {
                    conditions: Some(sc),
                    ..Default::default()
                },
            )
            .await
            .map_err(cdk_err)?
            .confirm(None)
            .await
            .map_err(cdk_err)?
            .to_string();
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

    /// The performer's check before promising (§8.2 step 3): same mint, valid DLEQ proofs
    /// (NUT-12), an HTLC lock (NUT-14) whose conditions are read from the proofs, unspent (NUT-07).
    pub async fn inspect(&self, token: &str) -> Result<Verified, WalletError> {
        let t = Token::from_str(token).map_err(cdk_err)?;
        let mint = t.mint_url().map_err(cdk_err)?.to_string();
        if mint.trim_end_matches('/') != self.url {
            return Err(WalletError::Invalid(format!(
                "token is from {mint}, this wallet uses {}",
                self.url
            )));
        }
        self.wallet.verify_token_dleq(&t).await.map_err(cdk_err)?;
        let proofs = self.proofs_of(&t).await?;
        let mut shape: Option<(String, String, u64)> = None;
        let mut amount = 0u64;
        for p in &proofs {
            let sc = SpendingConditions::try_from(&p.secret)
                .map_err(|_| WalletError::Invalid("token is not hash-locked".into()))?;
            let SpendingConditions::HTLCConditions { data, conditions } = sc else {
                return Err(WalletError::Invalid("token is P2PK, not an HTLC".into()));
            };
            let c = conditions.unwrap_or_default();
            let receiver = c
                .pubkeys
                .as_ref()
                .and_then(|v| v.first())
                .map(xonly)
                .ok_or_else(|| WalletError::Invalid("HTLC has no receiver key".into()))?;
            let this = (data.to_string(), receiver, c.locktime.unwrap_or(0));
            match &shape {
                None => shape = Some(this),
                Some(s) if *s == this => {}
                Some(_) => {
                    return Err(WalletError::Invalid(
                        "token proofs carry different locks".into(),
                    ))
                }
            }
            amount += u64::from(p.amount);
        }
        let (hashlock, receiver, locktime) =
            shape.ok_or_else(|| WalletError::Invalid("empty token".into()))?;
        let states = self
            .wallet
            .check_proofs_spent(proofs)
            .await
            .map_err(cdk_err)?;
        Ok(Verified {
            amount,
            hashlock,
            receiver,
            locktime,
            unspent: states.iter().all(|s| s.state == State::Unspent),
        })
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
    pub async fn refund(&self, token: &str) -> Result<u64, WalletError> {
        let t = Token::from_str(token).map_err(cdk_err)?;
        let mut proofs = self.proofs_of(&t).await?;
        let total: u64 = proofs.iter().map(|p| u64::from(p.amount)).sum();
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
        let pre = PreMintSecrets::random(
            active.id,
            Amount::from(total),
            &SplitTarget::default(),
            &fees,
        )
        .map_err(cdk_err)?;
        let client = HttpClient::new(self.url.parse().map_err(cdk_err)?, None);
        let resp = client
            .post_swap(SwapRequest::new(proofs, pre.blinded_messages()))
            .await
            .map_err(cdk_err)?;
        let fresh =
            cdk::dhke::construct_proofs(resp.signatures, pre.rs(), pre.secrets(), &active.keys)
                .map_err(cdk_err)?;
        let got: u64 = self
            .wallet
            .receive_proofs(fresh, ReceiveOptions::default(), None, None)
            .await
            .map_err(cdk_err)?
            .into();
        let id = lock_id(token);
        let commitment = {
            let mut ix = self.index.lock().unwrap();
            ix.locks.iter_mut().find(|l| l.id == id).and_then(|l| {
                l.state = LockState::Refunded;
                l.settled_at = Some(now());
                l.commitment.clone()
            })
        };
        self.log(
            EntryKind::Refund,
            got as i64,
            commitment.as_deref(),
            "Reclaimed after the locktime",
        );
        self.save()?;
        Ok(got)
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

fn sanitize(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
