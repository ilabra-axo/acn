//! What the parties should do about a commitment's payment, derived from the fold (pure).

use std::collections::BTreeMap;

use acn_core::kinds::{K_RELEASE, K_REQUEST, K_RULING, K_SATISFY};
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
    /// Performer: the preimage is public in a SATISFY, a performer-side RULING or a verifier
    /// RELEASE; redeem now.
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

    let paid = matches!(st.phase, Phase::Satisfied | Phase::Settled);
    if me == c.performer && paid {
        // The settling act: the last SATISFY / RULING / RELEASE on the chain (a judgement
        // SATISFY after a RELEASE carries no preimage, so look for the one that does).
        let e = c.chain.iter().rev().find_map(|id| {
            events.iter().find(|e| {
                &e.id == id
                    && matches!(e.kind, K_SATISFY | K_RULING | K_RELEASE)
                    && e.content.contains("preimage")
            })
        })?;
        let v: Value = serde_json::from_str(&e.content).ok()?;
        let preimage = v.get("preimage")?.as_str()?.to_string();
        return Some(Advice::Redeem { token, preimage });
    }
    if me == c.requester && status.terminal && !paid {
        let locktime = terms.payment.as_ref()?.locktime?;
        return Some(Advice::Refund {
            token,
            locktime,
            ready: now > locktime,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use acn_core::draft::{public_key, Draft};
    use acn_core::kinds::*;
    use serde_json::json;

    /// CR-002 verifier trigger: after RELEASE the performer is advised to redeem with `s`.
    #[test]
    fn performer_redeems_after_release() {
        let (req, perf, ver) = ([1u8; 32], [2u8; 32], [4u8; 32]);
        let (rp, pp, vp) = (
            public_key(&req).unwrap(),
            public_key(&perf).unwrap(),
            public_key(&ver).unwrap(),
        );
        let s = [9u8; 32];
        let pre: String = s.iter().map(|b| format!("{b:02x}")).collect();
        let h = acn_core::jcs::sha256_hex(&s);
        let terms = json!({
            "v": acn_core::terms::TERMS_VERSION, "deliverable": {"description": "x"},
            "conditions": [{"id": "tests", "class": "mechanical"}], "deadline": 5000, "review_window": 500,
            "rework_limit": 0, "price": {"amount": 10, "unit": "sat"},
            "payment": {"mint": "https://m", "hashlock": h, "locktime": 99999},
            "verifiers": [{"pubkey": vp, "classes": ["mechanical"]}], "arbiter": null,
            "subcontracting": false, "disclosure": "private", "governance": null,
            "settlement_trigger": "verifier", "settlement_verifier": vp
        });
        let parties = [rp.as_str(), pp.as_str()];
        let r = Draft::request(&pp, "N", &terms, Some("tok"))
            .unwrap()
            .sign(&req, 1000)
            .unwrap();
        let ack = Draft::new(K_ESCROW_ACK)
            .tag(vec!["c".into(), r.id.clone()])
            .content(json!({"hashlock": h}))
            .sign(&ver, 1001)
            .unwrap();
        let p = Draft::promise(
            &r.id,
            &r.id,
            &parties,
            &acn_core::jcs::hash(&terms).unwrap(),
            0.9,
            true,
        )
        .sign(&perf, 1100)
        .unwrap();
        let rep = Draft::report(&r.id, &p.id, &parties, None, None, &[])
            .sign(&perf, 1200)
            .unwrap();
        let att = Draft::attest(
            K_ATTEST_MECH,
            &r.id,
            &rep.id,
            "tests",
            json!({"result": "pass"}),
        )
        .sign(&ver, 1250)
        .unwrap();
        let rel = Draft::act(K_RELEASE, &r.id, &rep.id, &parties)
            .content(json!({"preimage": pre}))
            .sign(&ver, 1300)
            .unwrap();
        let events = vec![r.clone(), ack, p, rep, att, rel];
        assert_eq!(
            advise(&events, &r.id, &pp, 2000),
            Some(Advice::Redeem {
                token: "tok".into(),
                preimage: pre
            })
        );
        assert_eq!(
            advise(&events, &r.id, &rp, 2000),
            None,
            "nothing to refund once settled"
        );
    }
}
