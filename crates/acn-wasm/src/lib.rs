//! WASM entry point for presenter mode: the browser runs the *real* validator, projection and
//! scenario logic (no re-implementation in TypeScript).
//!
//! ABI (no wasm-bindgen needed):
//! - `acn_alloc(len) -> ptr`, `acn_free(ptr, len)`
//! - `acn_invoke(ptr, len) -> u64`: input is a UTF-8 JSON request `{"op": ..., ...}`; output is
//!   `(ptr << 32) | len` of a UTF-8 JSON response `{"ok": ...}` or `{"err": "..."}`, which the
//!   caller frees with `acn_free`.

use std::collections::BTreeMap;

use acn_core::draft::{instantiate, public_key, Draft, Template};
use acn_core::Event;
use acn_sim::{Cast, HintAction, PaymentCheck};
use acn_wallet::SimMint;
use serde::Deserialize;
use serde_json::{json, Value};

#[no_mangle]
pub extern "C" fn acn_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr`/`len` must come from `acn_alloc` or an `acn_invoke` result.
#[no_mangle]
pub unsafe extern "C" fn acn_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len));
}

/// # Safety
/// `ptr` must point to `len` initialized bytes allocated with `acn_alloc`.
#[no_mangle]
pub unsafe extern "C" fn acn_invoke(ptr: *const u8, len: usize) -> u64 {
    let input = std::slice::from_raw_parts(ptr, len);
    let out = match std::str::from_utf8(input)
        .map_err(|e| e.to_string())
        .and_then(invoke)
    {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "err": e }),
    };
    let mut bytes = out.to_string().into_bytes();
    bytes.shrink_to_fit();
    let (p, l) = (bytes.as_mut_ptr(), bytes.len());
    std::mem::forget(bytes);
    ((p as u64) << 32) | l as u64
}

fn arg<T: for<'de> Deserialize<'de>>(v: &Value, k: &str) -> Result<T, String> {
    serde_json::from_value(v.get(k).cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("`{k}`: {e}"))
}

fn secret(v: &Value, k: &str) -> Result<[u8; 32], String> {
    let s: String = arg(v, k)?;
    let b: Vec<u8> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2).unwrap_or("zz"), 16).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    b.try_into()
        .map_err(|_| "secret must be 32 bytes".to_string())
}

/// Dispatch one JSON request. Public for native tests.
pub fn invoke(req: &str) -> Result<Value, String> {
    let r: Value = serde_json::from_str(req).map_err(|e| e.to_string())?;
    let op = r["op"].as_str().ok_or("missing op")?;
    let ser = |v: &dyn erased::Ser| v.to_value();
    Ok(match op {
        "version" => json!(acn_core::kinds::VERSION),
        "pubkey" => json!(public_key(&secret(&r, "secret")?).map_err(|e| e.to_string())?),
        "sha256" => {
            let s = secret(&r, "bytes")?;
            json!(acn_core::jcs::sha256_hex(&s))
        }
        "sign" => {
            let d: Draft = arg(&r, "draft")?;
            let at: u64 = arg(&r, "created_at")?;
            let aux = secret(&r, "aux").unwrap_or([0u8; 32]);
            ser(&d
                .sign_with_aux(&secret(&r, "secret")?, at, &aux)
                .map_err(|e| e.to_string())?)
        }
        "fold" => {
            let events: Vec<Event> = arg(&r, "events")?;
            ser(&acn_core::fold(&events, &BTreeMap::new()))
        }
        "project" => {
            let events: Vec<Event> = arg(&r, "events")?;
            let me: String = arg(&r, "me")?;
            let now: u64 = arg(&r, "now")?;
            ser(&acn_view::project(&events, &me, now))
        }
        "scenarios" => ser(&acn_sim::SCENARIOS),
        "template" => {
            let cast: Cast = arg(&r, "cast")?;
            let id: String = arg(&r, "scenario")?;
            ser(&acn_sim::template(&id, &cast).ok_or("unknown scenario")?)
        }
        "request" => {
            let t: Template = arg(&r, "template")?;
            let tool: String = arg(&r, "tool")?;
            let now: u64 = arg(&r, "now")?;
            let performer: String = arg(&r, "performer")?;
            let hashlock: Option<String> = arg(&r, "hashlock")?;
            // Held-out instances: sha256 of the requester's seed (the host keeps the seed).
            let seed_hash: Option<String> = arg(&r, "requester_seed_hash").unwrap_or_default();
            let input = r.get("input").cloned().unwrap_or(json!({}));
            let terms = instantiate(
                &t,
                &tool,
                input,
                now,
                hashlock.as_deref(),
                seed_hash.as_deref(),
            )?;
            let mut d =
                Draft::request(&performer, &t.profile, &terms, None).map_err(|e| e.to_string())?;
            if let Some(escrow) = r.get("escrow").and_then(Value::as_str) {
                d = d.tag(vec!["escrow".into(), escrow.into()]);
            }
            if let (Some(seed), Some(to)) =
                (r.get("seed").and_then(Value::as_str), &t.requester_seed_to)
            {
                d = d.tag(vec!["seed".into(), seed.into(), to.clone()]);
            }
            ser(&d)
        }
        "react" => {
            let cast: Cast = arg(&r, "cast")?;
            let events: Vec<Event> = arg(&r, "events")?;
            let escrow: BTreeMap<String, String> = arg(&r, "escrow").unwrap_or_default();
            // Contingent performer keys `k` by commitment (the host generates them).
            let keys: BTreeMap<String, String> = arg(&r, "keys").unwrap_or_default();
            // Requester seeds the runner holds (held-out instances).
            let seeds: BTreeMap<String, String> = arg(&r, "seeds").unwrap_or_default();
            let payments: Option<BTreeMap<String, PaymentCheck>> =
                match r.get("payments").filter(|p| !p.is_null()) {
                    Some(_) => Some(arg(&r, "payments")?),
                    None => None,
                };
            ser(&acn_sim::react_with(
                &cast,
                &events,
                &escrow,
                payments.as_ref(),
                &keys,
                &seeds,
            ))
        }

        // --- Simulated mint (presenter mode): the state travels as JSON in `mint`. ---
        "mint_new" => ser(&SimMint::new()),
        "mint_issue" => {
            let mut m: SimMint = arg(&r, "mint")?;
            m.issue(
                &arg::<String>(&r, "owner")?,
                arg(&r, "amount")?,
                arg(&r, "now")?,
            )
            .map_err(|e| e.to_string())?;
            json!({ "mint": m })
        }
        "mint_lock" => {
            let mut m: SimMint = arg(&r, "mint")?;
            let commitment: Option<String> = arg(&r, "commitment").unwrap_or_default();
            let token = m
                .lock(
                    &arg::<String>(&r, "owner")?,
                    arg(&r, "amount")?,
                    &arg::<String>(&r, "hashlock")?,
                    &arg::<String>(&r, "receiver")?,
                    arg(&r, "locktime")?,
                    commitment.as_deref(),
                    arg(&r, "now")?,
                )
                .map_err(|e| e.to_string())?;
            json!({ "mint": m, "token": token })
        }
        "mint_tag" => {
            let mut m: SimMint = arg(&r, "mint")?;
            m.tag(
                &arg::<String>(&r, "token")?,
                &arg::<String>(&r, "commitment")?,
            );
            json!({ "mint": m })
        }
        "mint_redeem" => {
            let mut m: SimMint = arg(&r, "mint")?;
            let amount = m
                .redeem(
                    &arg::<String>(&r, "token")?,
                    &arg::<String>(&r, "by")?,
                    &arg::<String>(&r, "preimage")?,
                    arg(&r, "now")?,
                )
                .map_err(|e| e.to_string())?;
            json!({ "mint": m, "amount": amount })
        }
        "mint_refund" => {
            let mut m: SimMint = arg(&r, "mint")?;
            let amount = m
                .refund(
                    &arg::<String>(&r, "token")?,
                    &arg::<String>(&r, "by")?,
                    arg(&r, "now")?,
                )
                .map_err(|e| e.to_string())?;
            json!({ "mint": m, "amount": amount })
        }
        "mint_view" => {
            let m: SimMint = arg(&r, "mint")?;
            ser(&m.view(&arg::<String>(&r, "owner")?))
        }
        // The performer's check of a REQUEST's token against its terms (Core.md §8.2 step 3).
        "payment_check" => {
            let m: SimMint = arg(&r, "mint")?;
            let events: Vec<Event> = arg(&r, "events")?;
            let c: String = arg(&r, "commitment")?;
            let me: String = arg(&r, "me")?;
            let out = acn_core::fold(&events, &BTreeMap::new());
            let terms = out
                .commitments
                .get(&c)
                .and_then(|c| c.terms.clone())
                .ok_or("unknown commitment")?;
            let check = acn_wallet::settlement::request_token(&events, &c)
                .ok_or_else(|| "no token in the request".to_string())
                .and_then(|t| m.verify(&t).map_err(|e| e.to_string()))
                .and_then(|v| acn_wallet::settlement::check_payment(&terms, &v, &me));
            ser(&match check {
                Ok(()) => PaymentCheck::Ok,
                Err(reason) => PaymentCheck::Invalid { reason },
            })
        }
        "advise" => {
            let events: Vec<Event> = arg(&r, "events")?;
            ser(&acn_wallet::settlement::advise(
                &events,
                &arg::<String>(&r, "commitment")?,
                &arg::<String>(&r, "me")?,
                arg(&r, "now")?,
            ))
        }
        "hint" => {
            let events: Vec<Event> = arg(&r, "events")?;
            let c: String = arg(&r, "commitment")?;
            let now: u64 = arg(&r, "now")?;
            ser(&acn_sim::requester_hint(&events, &c, now))
        }
        "requester_draft" => {
            let events: Vec<Event> = arg(&r, "events")?;
            let c: String = arg(&r, "commitment")?;
            let action: HintAction = arg(&r, "action")?;
            let preimage: Option<String> = arg(&r, "preimage")?;
            let (d, nb) = acn_sim::requester_draft(&events, &c, &action, preimage.as_deref())
                .ok_or("act not possible now")?;
            json!({ "draft": d, "not_before": nb })
        }
        "bind" => {
            let agent: String = arg(&r, "agent")?;
            let expires: u64 = arg(&r, "expires")?;
            ser(&Draft::bind(&agent, expires, &[]))
        }
        "bind_ack" => {
            let binding: String = arg(&r, "binding")?;
            ser(&Draft::bind_ack(&binding))
        }
        other => return Err(format!("unknown op `{other}`")),
    })
}

mod erased {
    pub trait Ser {
        fn to_value(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Ser for T {
        fn to_value(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_through_json_abi() {
        let pk = invoke(r#"{"op":"pubkey","secret":"0101010101010101010101010101010101010101010101010101010101010101"}"#).unwrap();
        assert_eq!(pk.as_str().unwrap().len(), 64);
        let s = invoke(r#"{"op":"scenarios"}"#).unwrap();
        assert!(s.as_array().unwrap().len() >= 9);
        assert!(invoke(r#"{"op":"nope"}"#).is_err());
    }

    #[test]
    fn sim_mint_through_json_abi() {
        let m = invoke(r#"{"op":"mint_new"}"#).unwrap();
        let r = invoke(
            &json!({"op":"mint_issue","mint":m,"owner":"me","amount":100,"now":1}).to_string(),
        )
        .unwrap();
        let h = acn_core::jcs::sha256_hex(&[7u8; 32]);
        let r = invoke(&json!({"op":"mint_lock","mint":r["mint"],"owner":"me","amount":21,"hashlock":h,"receiver":"atlas","locktime":50,"now":2}).to_string()).unwrap();
        let tok = r["token"].as_str().unwrap().to_string();
        let bad = invoke(&json!({"op":"mint_redeem","mint":r["mint"],"token":tok,"by":"atlas","preimage":"00".repeat(32),"now":3}).to_string());
        assert!(bad.unwrap_err().contains("preimage"));
        let r = invoke(&json!({"op":"mint_redeem","mint":r["mint"],"token":tok,"by":"atlas","preimage":"07".repeat(32),"now":3}).to_string()).unwrap();
        let v = invoke(&json!({"op":"mint_view","mint":r["mint"],"owner":"atlas"}).to_string())
            .unwrap();
        assert_eq!(v["available"], 21);
    }
}
