//! What the parties should do about a commitment's payment, derived from the fold (pure).

use std::collections::BTreeMap;

use acn_core::kinds::{K_CLAIM, K_FUND, K_RELEASE, K_REQUEST, K_RULING, K_SATISFY};
use acn_core::terms::SettlementTrigger;
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
///
/// Checks the token's value and its complete spending policy against the §8.1 lock spec: only
/// `me` may redeem, with one signature, signing inputs only, and exactly one refund key (one
/// signature) opens after a locktime no earlier than the terms'. The refund key is compared with
/// `payment.refund_key` when the terms carry one; without it this function cannot tell a
/// substituted refund key from the requester's, so callers that know the requester should use
/// [`check_payment_from`].
pub fn check_payment(terms: &Terms, v: &Verified, me: &str) -> Result<(), String> {
    check(terms, v, me, None)
}

/// [`check_payment`], also requiring the refund key to be `requester`'s (x-only hex) when the
/// terms name no `payment.refund_key`.
pub fn check_payment_from(
    terms: &Terms,
    v: &Verified,
    me: &str,
    requester: &str,
) -> Result<(), String> {
    check(terms, v, me, Some(requester))
}

/// A key as x-only hex: Cashu writes compressed keys (`02`/`03` prefix), Nostr x-only ones.
fn xonly(k: &str) -> String {
    let k = k.to_ascii_lowercase();
    match k.len() {
        66 if k.starts_with("02") || k.starts_with("03") => k[2..].to_string(),
        _ => k,
    }
}

fn check(terms: &Terms, v: &Verified, me: &str, requester: Option<&str>) -> Result<(), String> {
    let Some(pay) = &terms.payment else {
        return Err("terms carry no payment".into());
    };
    check_lock(
        v,
        &LockSpec {
            me,
            hashlock: pay.hashlock.as_deref().unwrap_or_default(),
            amount: terms.price.amount,
            refund_to: pay.refund_key.as_deref().or(requester),
            min_locktime: pay.locktime,
            keysets: &pay.keysets,
        },
    )
}

/// What a received lock must be (Core §8.1): every proof redeemable by `me` alone with the
/// preimage of `hashlock`, refundable to `refund_to` alone after a locktime of at least
/// `min_locktime`, worth at least `amount`.
pub struct LockSpec<'a> {
    pub me: &'a str,
    pub hashlock: &'a str,
    pub amount: u64,
    /// The requester's key; `None` skips the refund-key comparison (not the shape checks).
    pub refund_to: Option<&'a str>,
    pub min_locktime: Option<u64>,
    /// Keyset ids the terms accept; empty accepts any keyset of the mint.
    pub keysets: &'a [String],
}

/// Check verified token facts against a [`LockSpec`]: the full spending policy, not just the
/// hashlock and first key. Used for request tokens (through the terms) and `FUND` tokens.
pub fn check_lock(v: &Verified, spec: &LockSpec<'_>) -> Result<(), String> {
    let me = spec.me;
    if !v.unspent {
        return Err("token already spent".into());
    }
    if v.amount < spec.amount {
        return Err(format!(
            "token is {} sat, terms say {}",
            v.amount, spec.amount
        ));
    }
    if spec.hashlock.is_empty() || v.hashlock != spec.hashlock {
        return Err("token hashlock differs from the terms".into());
    }
    if xonly(&v.receiver) != xonly(me) {
        return Err("token is not locked to me".into());
    }
    if v.receivers.len() != 1 || xonly(&v.receivers[0]) != xonly(me) {
        return Err(format!(
            "token lets {} key(s) redeem; only the performer may",
            v.receivers.len()
        ));
    }
    if v.n_sigs != 1 {
        return Err(format!(
            "token requires {} signature(s) to redeem; the lock spec needs exactly 1",
            v.n_sigs
        ));
    }
    if v.sig_flag != crate::SIG_INPUTS {
        return Err(format!(
            "token signature flag is {:?}, the lock spec uses {}",
            v.sig_flag,
            crate::SIG_INPUTS
        ));
    }
    if v.refund.len() != 1 || v.n_sigs_refund != 1 {
        return Err(format!(
            "token refund path has {} key(s) and needs {} signature(s); the lock spec has one \
             requester key",
            v.refund.len(),
            v.n_sigs_refund
        ));
    }
    if let Some(expected) = spec.refund_to {
        if xonly(&v.refund[0]) != xonly(expected) {
            return Err("token refunds to a key other than the requester's".into());
        }
    }
    if v.locktime == 0 {
        return Err("token has no locktime, so its refund path never opens".into());
    }
    if !spec.keysets.is_empty() {
        if v.keysets.is_empty() {
            return Err("the token's keysets aren't known, and the terms name keysets".into());
        }
        if let Some(k) = v.keysets.iter().find(|k| !spec.keysets.contains(k)) {
            return Err(format!("token uses keyset {k}, which the terms don't name"));
        }
    }
    if let Some(l) = spec.min_locktime {
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
    /// Contingent performer: the FUND token is locked to your hashlock; redeem it with your key
    /// `k`, then sign `CLAIM` with `k`.
    Claim { token: String },
    /// Contingent requester: funded and waiting. Once the performer redeems, the mint reveals
    /// `k` (NUT-07 witness), with or without a `CLAIM`. `token` is the `FUND` act's field, which
    /// is encrypted to the performer: the requester checks its own wallet's copy.
    LearnKey { token: String },
    /// Contingent requester: the performer's `CLAIM` revealed `k`; decrypt the deliverable.
    Key { key: String },
}

/// What `me` should do with the payment of `commitment` at `now`.
pub fn advise(events: &[Event], commitment: &str, me: &str, now: u64) -> Option<Advice> {
    advise_with(events, &BTreeMap::new(), commitment, me, now)
}

/// [`advise`] with terms documents referenced rather than inlined.
pub fn advise_with(
    events: &[Event],
    documents: &BTreeMap<String, Value>,
    commitment: &str,
    me: &str,
    now: u64,
) -> Option<Advice> {
    let out = fold(events, documents);
    let c = out.commitments.get(commitment)?;
    let st = c.state.as_ref()?;
    let terms = c.terms.as_ref()?;
    if terms.settlement_trigger == SettlementTrigger::Contingent {
        return advise_contingent(events, c, me, now);
    }
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

/// The token in the chain's `FUND`, and the key in its `CLAIM`, if any.
fn chain_field(
    events: &[Event],
    c: &acn_core::Commitment,
    kind: u16,
    field: &str,
) -> Option<String> {
    c.chain.iter().rev().find_map(|id| {
        let e = events.iter().find(|e| &e.id == id && e.kind == kind)?;
        let v: Value = serde_json::from_str(&e.content).ok()?;
        v.get(field)?.as_str().map(str::to_owned)
    })
}

fn advise_contingent(
    events: &[Event],
    c: &acn_core::Commitment,
    me: &str,
    now: u64,
) -> Option<Advice> {
    let st = c.state.as_ref()?;
    match st.phase {
        Phase::Funded => {
            let token = chain_field(events, c, K_FUND, "token")?;
            if me == c.performer {
                return Some(Advice::Claim { token });
            }
            if me == c.requester {
                let locktime = st.fund_locktime?;
                if now > locktime {
                    return Some(Advice::Refund {
                        token,
                        locktime,
                        ready: true,
                    });
                }
                return Some(Advice::LearnKey { token });
            }
            None
        }
        Phase::Settled if me == c.requester => {
            chain_field(events, c, K_CLAIM, "key").map(|key| Advice::Key { key })
        }
        _ => None,
    }
}

#[cfg(test)]
mod payment_tests {
    use super::*;
    use serde_json::json;

    const ME: &str = "aa00000000000000000000000000000000000000000000000000000000000001";
    const REQ: &str = "bb00000000000000000000000000000000000000000000000000000000000002";
    const OTHER: &str = "cc00000000000000000000000000000000000000000000000000000000000003";

    fn terms(refund_key: Option<&str>) -> Terms {
        serde_json::from_value(json!({
            "v": acn_core::terms::TERMS_VERSION, "deliverable": {"description": "x"},
            "conditions": [], "deadline": 5000, "review_window": 500, "rework_limit": 0,
            "price": {"amount": 10, "unit": "sat"},
            "payment": {"mint": "https://m", "hashlock": "ab".repeat(32), "locktime": 9000,
                        "refund_key": refund_key},
            "verifiers": [], "arbiter": null, "subcontracting": false, "disclosure": "private",
            "governance": null
        }))
        .unwrap()
    }

    fn good() -> Verified {
        Verified {
            amount: 10,
            hashlock: "ab".repeat(32),
            receiver: ME.into(),
            locktime: 9000,
            unspent: true,
            receivers: vec![ME.into()],
            n_sigs: 1,
            refund: vec![REQ.into()],
            n_sigs_refund: 1,
            sig_flag: crate::SIG_INPUTS.into(),
            keysets: vec![],
        }
    }

    fn rejects(v: Verified, why: &str) {
        let t = terms(None);
        let e = check_payment_from(&t, &v, ME, REQ).expect_err(why);
        assert!(!e.is_empty(), "{why}");
    }

    #[test]
    fn canonical_lock_passes() {
        let t = terms(None);
        check_payment(&t, &good(), ME).unwrap();
        check_payment_from(&t, &good(), ME, REQ).unwrap();
        // Compressed (Cashu) and x-only (Nostr) forms of a key are the same key.
        let mut v = good();
        v.refund = vec![format!("02{REQ}")];
        check_payment_from(&t, &v, ME, REQ).unwrap();
        check_payment_from(&terms(Some(&format!("03{REQ}"))), &good(), ME, REQ).unwrap();
    }

    #[test]
    fn keysets_named_by_the_terms_are_enforced() {
        let mut t = terms(None);
        t.payment.as_mut().unwrap().keysets = vec!["00aa".into()];
        // Unknown keysets with named ones: not established.
        assert!(check_payment_from(&t, &good(), ME, REQ).is_err());
        let mut v = good();
        v.keysets = vec!["00aa".into()];
        check_payment_from(&t, &v, ME, REQ).unwrap();
        v.keysets = vec!["00aa".into(), "00bb".into()];
        assert!(check_payment_from(&t, &v, ME, REQ)
            .unwrap_err()
            .contains("00bb"));
        // No keysets named: any keyset of the mint.
        check_payment_from(&terms(None), &v, ME, REQ).unwrap();
    }

    #[test]
    fn every_policy_deviation_is_rejected() {
        let mut v = good();
        v.unspent = false;
        rejects(v, "spent");
        let mut v = good();
        v.amount = 9;
        rejects(v, "short");
        let mut v = good();
        v.hashlock = "cd".repeat(32);
        rejects(v, "hashlock");
        let mut v = good();
        v.receiver = OTHER.into();
        v.receivers = vec![OTHER.into()];
        rejects(v, "receiver");
        let mut v = good();
        v.receivers.push(OTHER.into());
        rejects(v, "extra beneficiary");
        let mut v = good();
        v.receivers.clear();
        rejects(v, "no beneficiary list");
        let mut v = good();
        v.n_sigs = 2;
        rejects(v, "threshold");
        let mut v = good();
        v.sig_flag = "SIG_ALL".into();
        rejects(v, "SIG_ALL");
        let mut v = good();
        v.refund.clear();
        rejects(v, "no refund key");
        let mut v = good();
        v.refund.push(OTHER.into());
        rejects(v, "extra refund key");
        let mut v = good();
        v.n_sigs_refund = 2;
        rejects(v, "refund threshold");
        let mut v = good();
        v.refund = vec![OTHER.into()];
        rejects(v, "substituted refund key");
        let mut v = good();
        v.locktime = 8999;
        rejects(v, "early locktime");
        let mut v = good();
        v.locktime = 0;
        rejects(v, "no locktime");
    }

    #[test]
    fn refund_key_from_terms_or_requester() {
        let mut v = good();
        v.refund = vec![OTHER.into()];
        // Without the requester or a terms refund key, substitution is undetectable here.
        check_payment(&terms(None), &v, ME).unwrap();
        assert!(check_payment(&terms(Some(REQ)), &v, ME).is_err());
        assert!(check_payment_from(&terms(None), &v, ME, REQ).is_err());
        // The agreed terms win over the requester's identity key.
        check_payment_from(&terms(Some(OTHER)), &v, ME, REQ).unwrap();
    }

    #[test]
    fn legacy_facts_fail_closed() {
        // A `Verified` serialized before the policy fields existed establishes no policy.
        let mut j = serde_json::to_value(good()).unwrap();
        for f in ["receivers", "n_sigs", "refund", "n_sigs_refund", "sig_flag"] {
            j.as_object_mut().unwrap().remove(f);
        }
        let v: Verified = serde_json::from_value(j).unwrap();
        assert!(check_payment(&terms(None), &v, ME).is_err());
    }
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

#[cfg(test)]
mod contingent_tests {
    use super::*;
    use acn_core::draft::{public_key, Draft};
    use acn_core::kinds::*;
    use serde_json::json;

    /// CR-002 contingent: who is advised to do what as the commitment is funded and claimed.
    #[test]
    fn contingent_advice() {
        let (req, perf, ver) = ([1u8; 32], [2u8; 32], [4u8; 32]);
        let (rp, pp, vp) = (
            public_key(&req).unwrap(),
            public_key(&perf).unwrap(),
            public_key(&ver).unwrap(),
        );
        let k = [0x6bu8; 32];
        let key: String = k.iter().map(|b| format!("{b:02x}")).collect();
        let h = acn_core::jcs::sha256_hex(&k);
        let ct = "cc".repeat(32);
        let terms = json!({
            "v": acn_core::terms::TERMS_VERSION, "deliverable": {"description": "x"},
            "conditions": [{"id": "tests", "class": "mechanical"}], "deadline": 5000, "review_window": 500,
            "rework_limit": 0, "price": {"amount": 10, "unit": "sat"}, "payment": {"mint": "https://m"},
            "verifiers": [{"pubkey": vp, "classes": ["mechanical"]}], "arbiter": null,
            "subcontracting": false, "disclosure": "private", "governance": null,
            "settlement_trigger": "contingent", "settlement_verifier": vp, "fund_window": 300
        });
        let parties = [rp.as_str(), pp.as_str()];
        let r = Draft::request(&pp, "N", &terms, None)
            .unwrap()
            .sign(&req, 1000)
            .unwrap();
        let p = Draft::promise(
            &r.id,
            &r.id,
            &parties,
            &acn_core::jcs::hash(&terms).unwrap(),
            0.9,
            false,
        )
        .sign(&perf, 1100)
        .unwrap();
        let rep = Draft::act(K_REPORT, &r.id, &p.id, &parties)
            .content(json!({"ciphertext": ct, "hashlock": h}))
            .sign(&perf, 1200)
            .unwrap();
        let att = Draft::attest(K_ATTEST_MECH, &r.id, &rep.id, "tests", json!({"result": "pass", "ciphertext": ct, "hashlock": h, "plaintext": "dd".repeat(32)})).sign(&ver, 1250).unwrap();
        let fund = Draft::act(K_FUND, &r.id, &rep.id, &parties).content(json!({"hashlock": h, "amount": 10, "mint": "https://m", "locktime": 4000, "token": "ftok"})).sign(&req, 1300).unwrap();
        let mut events = vec![r.clone(), p, rep, att, fund.clone()];
        assert_eq!(
            advise(&events, &r.id, &pp, 1400),
            Some(Advice::Claim {
                token: "ftok".into()
            })
        );
        assert_eq!(
            advise(&events, &r.id, &rp, 1400),
            Some(Advice::LearnKey {
                token: "ftok".into()
            })
        );
        assert_eq!(
            advise(&events, &r.id, &rp, 4001),
            Some(Advice::Refund {
                token: "ftok".into(),
                locktime: 4000,
                ready: true
            }),
            "never claimed: refund after the funding lock"
        );
        let claim = Draft::act(K_CLAIM, &r.id, &fund.id, &parties)
            .content(json!({"key": key}))
            .sign(&perf, 1500)
            .unwrap();
        events.push(claim);
        assert_eq!(advise(&events, &r.id, &rp, 1600), Some(Advice::Key { key }));
        assert_eq!(advise(&events, &r.id, &pp, 1600), None);
    }
}
