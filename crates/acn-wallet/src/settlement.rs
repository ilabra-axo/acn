//! What the parties should do about a commitment's payment, derived from the fold (pure).

use std::collections::BTreeMap;

use acn_core::kinds::{K_REQUEST, K_RULING, K_SATISFY};
use acn_core::{fold, Event, Phase, Terms};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Verified;

/// The payment carried by a REQUEST (`content.token`): a sim token id, or a NIP-44 ciphertext of
/// a Cashu token addressed to the performer (live mode).
pub fn request_token(events: &[Event], commitment: &str) -> Option<String> {
    let req = events
        .iter()
        .find(|e| e.id == commitment && e.kind == K_REQUEST)?;
    let v: Value = serde_json::from_str(&req.content).ok()?;
    v.get("token").and_then(Value::as_str).map(str::to_owned)
}

/// Spec §8.2 step 3: a performer MUST NOT rely on an unverified token.
pub fn check_payment(terms: &Terms, v: &Verified, me: &str) -> Result<(), String> {
    let Some(pay) = &terms.payment else {
        return Err("terms carry no payment".into());
    };
    if !v.unspent {
        return Err("token already spent".into());
    }
    if v.amount < terms.price.amount {
        return Err(format!(
            "token is {} sat, terms say {}",
            v.amount, terms.price.amount
        ));
    }
    if Some(v.hashlock.as_str()) != pay.hashlock.as_deref() {
        return Err("token hashlock differs from the terms".into());
    }
    if v.receiver != me {
        return Err("token is not locked to me".into());
    }
    if let Some(l) = pay.locktime {
        if v.locktime < l {
            return Err("token locktime is earlier than the terms".into());
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Advice {
    /// Performer: the preimage is public in a SATISFY or performer-side RULING; redeem now.
    Redeem { token: String, preimage: String },
    /// Requester: closed without satisfaction; reclaim once `locktime` has passed.
    Refund {
        token: String,
        locktime: u64,
        ready: bool,
    },
}

/// What `me` should do with the payment of `commitment` at `now`.
pub fn advise(events: &[Event], commitment: &str, me: &str, now: u64) -> Option<Advice> {
    let out = fold(events, &BTreeMap::new());
    let c = out.commitments.get(commitment)?;
    let st = c.state.as_ref()?;
    let terms = c.terms.as_ref()?;
    let token = request_token(events, commitment)?;
    terms.hashlock()?;
    let status = acn_core::status_at(c, now);

    if me == c.performer && st.phase == Phase::Satisfied {
        let closing = c.chain.last()?;
        let e = events
            .iter()
            .find(|e| &e.id == closing && matches!(e.kind, K_SATISFY | K_RULING))?;
        let v: Value = serde_json::from_str(&e.content).ok()?;
        let preimage = v.get("preimage")?.as_str()?.to_string();
        return Some(Advice::Redeem { token, preimage });
    }
    if me == c.requester && status.terminal && st.phase != Phase::Satisfied {
        let locktime = terms.payment.as_ref()?.locktime?;
        return Some(Advice::Refund {
            token,
            locktime,
            ready: now > locktime,
        });
    }
    None
}
