//! Wallets for ACN settlement (docs/Core.md §8).
//!
//! - [`SimMint`]: a pure, serializable, Cashu-*shaped* mint for presenter mode and tests. It
//!   enforces the same rules as a NUT-14 HTLC lock: the receiver redeems with the preimage (also
//!   after the locktime), the refund key reclaims only after the locktime, and every lock can be
//!   spent once. Callers are trusted to say who they are; there are no blind signatures. The
//!   real path is `nmesh-wallet` (CDK) against `nmesh-mint`.
//! - [`WalletView`]: what UIs show for any wallet, simulated or real.
//! - [`settlement`]: what the parties should do with a payment, derived from the commitment fold.

pub mod settlement;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const SIM_MINT_URL: &str = "sim://mint";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockState {
    /// Unspent: redeemable by the receiver with the preimage; refundable after the locktime.
    Locked,
    Redeemed,
    Refunded,
}

/// One hash-locked payment (a NUT-14 HTLC token in the real system).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    pub id: String,
    pub amount: u64,
    /// H = sha256(s), hex.
    pub hashlock: String,
    /// Performer key (x-only hex): may redeem with the preimage.
    pub receiver: String,
    /// Requester key: may reclaim after `locktime`.
    pub refund: String,
    pub locktime: u64,
    pub commitment: Option<String>,
    pub state: LockState,
    pub created_at: u64,
    pub settled_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    TopUp,
    /// Funds moved from available into a lock I created.
    Lock,
    /// I redeemed a lock addressed to me.
    Redeem,
    /// I reclaimed my own expired lock.
    Refund,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub at: u64,
    pub kind: EntryKind,
    /// Signed change to the available balance.
    pub delta: i64,
    pub commitment: Option<String>,
    pub note: String,
}

/// What a wallet UI shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletView {
    pub owner: String,
    pub mint: String,
    pub simulated: bool,
    /// Spendable now.
    pub available: u64,
    /// In locks I created that are not settled yet.
    pub locked_out: u64,
    /// In unsettled locks addressed to me.
    pub incoming: u64,
    pub history: Vec<Entry>,
    pub locks: Vec<Lock>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum MintError {
    #[error("insufficient funds: have {have} sat, need {need} sat")]
    InsufficientFunds { have: u64, need: u64 },
    #[error("unknown token")]
    UnknownToken,
    #[error("token already spent")]
    AlreadySpent,
    #[error("preimage does not match the hashlock")]
    BadPreimage,
    #[error("not the receiver of this token")]
    NotReceiver,
    #[error("only the refund key can reclaim this token")]
    NotRefundKey,
    #[error("locktime not reached yet")]
    LocktimeNotReached,
    #[error("invalid amount")]
    InvalidAmount,
}

/// NUT-11 signature flag of a lock that signs inputs only (the only flag ACN locks use).
pub const SIG_INPUTS: &str = "SIG_INPUTS";

/// Facts a performer checks before promising (spec §8.2 step 3): the value of the token's
/// distinct proofs and its complete effective spending policy, which is identical on every proof.
///
/// The policy fields default to values [`settlement::check_payment`] rejects, so a producer that
/// does not establish them fails closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verified {
    pub amount: u64,
    pub hashlock: String,
    /// The key that may redeem with the preimage.
    pub receiver: String,
    pub locktime: u64,
    pub unspent: bool,
    /// Every key on the preimage path (NUT-11 `pubkeys`). A sound ACN lock lists `receiver` only.
    #[serde(default)]
    pub receivers: Vec<String>,
    /// Signatures required on the preimage path (`n_sigs`; an absent tag means 1).
    #[serde(default)]
    pub n_sigs: u64,
    /// Keys that may reclaim the token after `locktime` (`refund`).
    #[serde(default)]
    pub refund: Vec<String>,
    /// Signatures required on the refund path (`n_sigs_refund`; an absent tag means 1).
    #[serde(default)]
    pub n_sigs_refund: u64,
    /// NUT-11 `sigflag` ([`SIG_INPUTS`] when absent).
    #[serde(default)]
    pub sig_flag: String,
    /// The keyset ids of the token's proofs (Core §8.6: terms may name the keysets they accept).
    #[serde(default)]
    pub keysets: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimMint {
    balances: BTreeMap<String, u64>,
    locks: BTreeMap<String, Lock>,
    history: BTreeMap<String, Vec<Entry>>,
    seq: u64,
}

impl SimMint {
    pub fn new() -> Self {
        Self::default()
    }

    fn log(&mut self, owner: &str, e: Entry) {
        self.history.entry(owner.to_string()).or_default().push(e);
    }

    /// Test sats (the fake-Lightning top-up of the real mint).
    pub fn issue(&mut self, owner: &str, amount: u64, now: u64) -> Result<(), MintError> {
        if amount == 0 {
            return Err(MintError::InvalidAmount);
        }
        *self.balances.entry(owner.to_string()).or_default() += amount;
        self.log(
            owner,
            Entry {
                at: now,
                kind: EntryKind::TopUp,
                delta: amount as i64,
                commitment: None,
                note: "Top up (test sats)".into(),
            },
        );
        Ok(())
    }

    pub fn balance(&self, owner: &str) -> u64 {
        self.balances.get(owner).copied().unwrap_or(0)
    }

    /// Lock `amount` from `owner` to `receiver` under `hashlock`; refundable to `owner` after
    /// `locktime`. Returns the token (lock id).
    #[allow(clippy::too_many_arguments)]
    pub fn lock(
        &mut self,
        owner: &str,
        amount: u64,
        hashlock: &str,
        receiver: &str,
        locktime: u64,
        commitment: Option<&str>,
        now: u64,
    ) -> Result<String, MintError> {
        if amount == 0 {
            return Err(MintError::InvalidAmount);
        }
        let have = self.balance(owner);
        if have < amount {
            return Err(MintError::InsufficientFunds { have, need: amount });
        }
        self.balances.insert(owner.to_string(), have - amount);
        self.seq += 1;
        let id = format!("simtok{:08}", self.seq);
        self.locks.insert(
            id.clone(),
            Lock {
                id: id.clone(),
                amount,
                hashlock: hashlock.to_ascii_lowercase(),
                receiver: receiver.to_string(),
                refund: owner.to_string(),
                locktime,
                commitment: commitment.map(str::to_owned),
                state: LockState::Locked,
                created_at: now,
                settled_at: None,
            },
        );
        self.log(
            owner,
            Entry {
                at: now,
                kind: EntryKind::Lock,
                delta: -(amount as i64),
                commitment: commitment.map(str::to_owned),
                note: "Hash-locked to the performer".into(),
            },
        );
        Ok(id)
    }

    /// Attach the commitment id once the REQUEST is signed (the lock is made first).
    pub fn tag(&mut self, token: &str, commitment: &str) {
        if let Some(l) = self.locks.get_mut(token) {
            l.commitment = Some(commitment.to_string());
        }
        for list in self.history.values_mut() {
            for e in list
                .iter_mut()
                .filter(|e| e.kind == EntryKind::Lock && e.commitment.is_none())
            {
                e.commitment = Some(commitment.to_string());
            }
        }
    }

    /// The performer's check: DLEQ + NUT-07 in the real system.
    pub fn verify(&self, token: &str) -> Result<Verified, MintError> {
        let l = self.locks.get(token).ok_or(MintError::UnknownToken)?;
        Ok(Verified {
            amount: l.amount,
            hashlock: l.hashlock.clone(),
            receiver: l.receiver.clone(),
            locktime: l.locktime,
            unspent: l.state == LockState::Locked,
            // A sim lock has exactly the canonical policy: one receiver, the owner as refund key.
            receivers: vec![l.receiver.clone()],
            n_sigs: 1,
            refund: vec![l.refund.clone()],
            n_sigs_refund: 1,
            sig_flag: SIG_INPUTS.into(),
            keysets: vec![],
        })
    }

    pub fn lock_info(&self, token: &str) -> Option<&Lock> {
        self.locks.get(token)
    }

    /// Receiver path: preimage + receiver. Open even after the locktime (NUT-11).
    pub fn redeem(
        &mut self,
        token: &str,
        by: &str,
        preimage_hex: &str,
        now: u64,
    ) -> Result<u64, MintError> {
        let l = self
            .locks
            .get(token)
            .ok_or(MintError::UnknownToken)?
            .clone();
        if l.state != LockState::Locked {
            return Err(MintError::AlreadySpent);
        }
        if l.receiver != by {
            return Err(MintError::NotReceiver);
        }
        let ok = decode_hex(preimage_hex)
            .filter(|b| b.len() == 32)
            .is_some_and(|b| acn_core::jcs::sha256_hex(&b) == l.hashlock);
        if !ok {
            return Err(MintError::BadPreimage);
        }
        self.settle(
            &l,
            LockState::Redeemed,
            by,
            now,
            EntryKind::Redeem,
            "Redeemed with the revealed preimage",
        );
        Ok(l.amount)
    }

    /// Sender path: refund key after the locktime.
    pub fn refund(&mut self, token: &str, by: &str, now: u64) -> Result<u64, MintError> {
        let l = self
            .locks
            .get(token)
            .ok_or(MintError::UnknownToken)?
            .clone();
        if l.state != LockState::Locked {
            return Err(MintError::AlreadySpent);
        }
        if l.refund != by {
            return Err(MintError::NotRefundKey);
        }
        if now <= l.locktime {
            return Err(MintError::LocktimeNotReached);
        }
        self.settle(
            &l,
            LockState::Refunded,
            by,
            now,
            EntryKind::Refund,
            "Reclaimed after the locktime",
        );
        Ok(l.amount)
    }

    fn settle(
        &mut self,
        l: &Lock,
        state: LockState,
        to: &str,
        now: u64,
        kind: EntryKind,
        note: &str,
    ) {
        let rec = self.locks.get_mut(&l.id).expect("exists");
        rec.state = state;
        rec.settled_at = Some(now);
        *self.balances.entry(to.to_string()).or_default() += l.amount;
        self.log(
            to,
            Entry {
                at: now,
                kind,
                delta: l.amount as i64,
                commitment: l.commitment.clone(),
                note: note.into(),
            },
        );
    }

    pub fn view(&self, owner: &str) -> WalletView {
        let mine: Vec<Lock> = self
            .locks
            .values()
            .filter(|l| l.refund == owner || l.receiver == owner)
            .cloned()
            .collect();
        let open = |f: &dyn Fn(&Lock) -> bool| {
            mine.iter()
                .filter(|l| l.state == LockState::Locked && f(l))
                .map(|l| l.amount)
                .sum()
        };
        WalletView {
            owner: owner.to_string(),
            mint: SIM_MINT_URL.into(),
            simulated: true,
            available: self.balance(owner),
            locked_out: open(&|l| l.refund == owner),
            incoming: open(&|l| l.receiver == owner),
            history: self.history.get(owner).cloned().unwrap_or_default(),
            locks: mine,
        }
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &[u8; 32]) -> String {
        acn_core::jcs::sha256_hex(s)
    }

    #[test]
    fn htlc_rules() {
        let mut m = SimMint::new();
        let (alice, bob) = ("alice", "bob");
        let s = [7u8; 32];
        let pre: String = s.iter().map(|b| format!("{b:02x}")).collect();
        m.issue(alice, 100, 0).unwrap();
        assert_eq!(
            m.lock(alice, 500, &h(&s), bob, 50, None, 1),
            Err(MintError::InsufficientFunds {
                have: 100,
                need: 500
            })
        );
        let t = m.lock(alice, 21, &h(&s), bob, 50, Some("c1"), 1).unwrap();
        assert_eq!(m.view(alice).available, 79);
        assert_eq!(m.view(alice).locked_out, 21);
        assert_eq!(m.view(bob).incoming, 21);
        assert!(m.verify(&t).unwrap().unspent);
        let v = m.verify(&t).unwrap();
        assert_eq!(
            (v.receivers, v.refund),
            (vec![bob.into()], vec![alice.into()])
        );

        assert_eq!(m.redeem(&t, alice, &pre, 2), Err(MintError::NotReceiver));
        assert_eq!(
            m.redeem(&t, bob, &"00".repeat(32), 2),
            Err(MintError::BadPreimage)
        );
        assert_eq!(m.refund(&t, alice, 40), Err(MintError::LocktimeNotReached));
        assert_eq!(m.refund(&t, bob, 60), Err(MintError::NotRefundKey));
        assert_eq!(m.redeem(&t, bob, &pre, 3), Ok(21));
        assert_eq!(m.redeem(&t, bob, &pre, 4), Err(MintError::AlreadySpent));
        assert_eq!(m.refund(&t, alice, 60), Err(MintError::AlreadySpent));
        assert_eq!(m.view(bob).available, 21);
        assert_eq!(m.view(alice).locked_out, 0);

        // Receiver path still open after the locktime; refund only after it.
        let t2 = m.lock(alice, 5, &h(&s), bob, 50, None, 5).unwrap();
        assert_eq!(m.refund(&t2, alice, 51), Ok(5));
        assert_eq!(m.view(alice).available, 79);
        let kinds: Vec<_> = m.view(alice).history.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                EntryKind::TopUp,
                EntryKind::Lock,
                EntryKind::Lock,
                EntryKind::Refund
            ]
        );

        // Serializable (presenter mode keeps the mint as JSON).
        let back: SimMint = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}
