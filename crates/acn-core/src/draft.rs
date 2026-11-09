//! Building acts through the act schemas (spec §12: "sign acts only through the act schemas").
//!
//! A [`Draft`] is an unsigned event template. Sign it with any Nostr signer (rust-nostr
//! `EventBuilder::new(kind, content).tags(tags)`, NIP-46, NIP-07...), or with [`Draft::sign`].

use k256::schnorr::SigningKey;
use serde_json::{json, Value};

use crate::event::{compute_id, Event};
use crate::jcs;
use crate::kinds::*;
use crate::terms::Terms;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Draft {
    pub kind: u16,
    pub tags: Vec<Vec<String>>,
    pub content: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    #[error("invalid secret key")]
    BadKey,
    #[error("signing failed")]
    Sign,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn t(name: &str, v: impl Into<String>) -> Vec<String> {
    vec![name.to_string(), v.into()]
}

impl Draft {
    pub fn new(kind: u16) -> Self {
        Self {
            kind,
            tags: vec![t("v", VERSION)],
            content: "{}".into(),
        }
    }

    pub fn tag(mut self, tag: Vec<String>) -> Self {
        self.tags.push(tag);
        self
    }

    pub fn content(mut self, v: Value) -> Self {
        self.content = v.to_string();
        self
    }

    /// A terms template (`K_TERMS_TEMPLATE`, CR-003 §2.5). Publish it signed; its event id and
    /// content hash are what `template_ref` names.
    pub fn template(content: &Value) -> Self {
        let name = content["name"].as_str().unwrap_or_default().to_string();
        Self::new(K_TERMS_TEMPLATE)
            .tag(vec!["name".into(), name])
            .content(content.clone())
    }

    /// Set one field of the JSON content (e.g. `performer_seed_hash` on a binding act).
    pub fn field(mut self, key: &str, v: Value) -> Self {
        let mut c: Value = serde_json::from_str(&self.content).unwrap_or_else(|_| json!({}));
        if let Some(o) = c.as_object_mut() {
            o.insert(key.into(), v);
        }
        self.content = c.to_string();
        self
    }

    /// Commitment id, `prev` and counterparties: the envelope of every non-root act.
    pub fn act(kind: u16, commitment: &str, prev: &str, parties: &[&str]) -> Self {
        let mut d = Self::new(kind).tag(t("c", commitment)).tag(vec![
            "e".into(),
            prev.into(),
            String::new(),
            "prev".into(),
        ]);
        for p in parties {
            d = d.tag(t("p", *p));
        }
        d
    }

    pub fn terms_hash(self, hash: &str) -> Self {
        self.tag(t("terms", hash))
    }

    /// Sign with a raw 32-byte secret key. `aux` is BIP-340 auxiliary randomness; pass fresh
    /// random bytes in production (all-zero is acceptable for tests).
    pub fn sign_with_aux(
        &self,
        secret: &[u8; 32],
        created_at: u64,
        aux: &[u8; 32],
    ) -> Result<Event, DraftError> {
        let sk = SigningKey::from_bytes(secret).map_err(|_| DraftError::BadKey)?;
        let pubkey = hex::encode(sk.verifying_key().to_bytes());
        let id = compute_id(&pubkey, created_at, self.kind, &self.tags, &self.content);
        let msg = hex::decode(&id).expect("hex");
        let sig = sk.sign_raw(&msg, aux).map_err(|_| DraftError::Sign)?;
        Ok(Event {
            id,
            pubkey,
            created_at,
            kind: self.kind,
            tags: self.tags.clone(),
            content: self.content.clone(),
            sig: hex::encode(sig.to_bytes()),
        })
    }

    pub fn sign(&self, secret: &[u8; 32], created_at: u64) -> Result<Event, DraftError> {
        self.sign_with_aux(secret, created_at, &[0u8; 32])
    }

    // --- act constructors (spec §6.2) ---

    /// Root REQUEST. Terms are inlined (≤ 16 KiB) and bound by their JCS hash.
    pub fn request(
        performer: &str,
        profile: &str,
        terms: &Value,
        token: Option<&str>,
    ) -> Result<Self, DraftError> {
        let hash = jcs::hash(terms)?;
        let mut content = json!({ "terms": terms });
        if let Some(tok) = token {
            content["token"] = json!(tok);
        }
        Ok(Self::new(K_REQUEST)
            .tag(t("p", performer))
            .tag(t("profile", profile))
            .terms_hash(&hash)
            .content(content))
    }

    pub fn counter(
        c: &str,
        prev: &str,
        parties: &[&str],
        terms: &Value,
        reason: &str,
    ) -> Result<Self, DraftError> {
        let hash = jcs::hash(terms)?;
        Ok(Self::act(K_COUNTER, c, prev, parties)
            .terms_hash(&hash)
            .content(json!({ "terms": terms, "reason": reason })))
    }

    pub fn accept(c: &str, prev: &str, parties: &[&str], terms_hash: &str) -> Self {
        Self::act(K_ACCEPT, c, prev, parties).terms_hash(terms_hash)
    }

    pub fn promise(
        c: &str,
        prev: &str,
        parties: &[&str],
        terms_hash: &str,
        p_on_time: f64,
        token_verified: bool,
    ) -> Self {
        Self::act(K_PROMISE, c, prev, parties)
            .terms_hash(terms_hash)
            .content(json!({ "p_on_time": p_on_time, "token_verified": token_verified }))
    }

    pub fn closing(kind: u16, c: &str, prev: &str, parties: &[&str], reason: &str) -> Self {
        Self::act(kind, c, prev, parties).content(json!({ "reason": reason }))
    }

    pub fn progress(c: &str, prev: &str, parties: &[&str], note: &str) -> Self {
        Self::act(K_PROGRESS, c, prev, parties).content(json!({ "note": note }))
    }

    /// REPORT with an optional small inline result (e.g. MCP structuredContent) and artifacts.
    pub fn report(
        c: &str,
        prev: &str,
        parties: &[&str],
        result: Option<Value>,
        contract: Option<&str>,
        artifacts: &[&str],
    ) -> Self {
        let mut content = json!({});
        if let Some(r) = result {
            content["result"] = r;
        }
        if let Some(k) = contract {
            content["contract"] = json!(k);
        }
        let mut d = Self::act(K_REPORT, c, prev, parties).content(content);
        for a in artifacts {
            d = d.tag(t("x", *a));
        }
        d
    }

    pub fn satisfy(c: &str, prev: &str, parties: &[&str], preimage: Option<&str>) -> Self {
        let content = match preimage {
            Some(s) => json!({ "preimage": s }),
            None => json!({}),
        };
        Self::act(K_SATISFY, c, prev, parties).content(content)
    }

    pub fn reject(c: &str, prev: &str, parties: &[&str], gaps: Vec<(&str, &str)>) -> Self {
        let gaps: Vec<Value> = gaps
            .into_iter()
            .map(|(cond, note)| json!({ "condition": cond, "evidence": [], "note": note }))
            .collect();
        Self::act(K_REJECT, c, prev, parties).content(json!({ "gaps": gaps }))
    }

    pub fn dispute(c: &str, prev: &str, parties: &[&str], reason: &str) -> Self {
        Self::act(K_DISPUTE, c, prev, parties).content(json!({ "reason": reason }))
    }

    pub fn ruling(
        c: &str,
        prev: &str,
        parties: &[&str],
        for_performer: bool,
        preimage: Option<&str>,
    ) -> Self {
        let mut content =
            json!({ "decision": if for_performer { "performer" } else { "requester" } });
        if let Some(s) = preimage {
            content["preimage"] = json!(s);
        }
        Self::act(K_RULING, c, prev, parties).content(content)
    }

    /// Attestation / interpretive report on a REPORT for one condition (annotating).
    pub fn attest(kind: u16, c: &str, report: &str, condition: &str, content: Value) -> Self {
        Self::new(kind)
            .tag(t("c", c))
            .tag(vec![
                "e".into(),
                report.into(),
                String::new(),
                "report".into(),
            ])
            .tag(t("cond", condition))
            .content(content)
    }

    pub fn endorse(c: &str, act: &str, policy: &str, human: bool) -> Self {
        Self::new(K_ENDORSE)
            .tag(t("c", c))
            .tag(vec![
                "e".into(),
                act.into(),
                String::new(),
                "endorses".into(),
            ])
            .tag(t("policy", policy))
            .tag(t("endorser", if human { "human" } else { "automated" }))
    }

    pub fn bind(agent: &str, expires: u64, scopes: &[(&str, &str)]) -> Self {
        let mut d = Self::new(K_BIND)
            .tag(t("p", agent))
            .tag(t("expires", expires.to_string()));
        for (k, v) in scopes {
            d = d.tag(vec!["scope".into(), (*k).into(), (*v).into()]);
        }
        d
    }

    pub fn bind_ack(binding: &str) -> Self {
        Self::new(K_BIND_ACK).tag(t("e", binding))
    }

    pub fn bind_revoke(binding: &str) -> Self {
        Self::new(K_BIND_REVOKE).tag(t("e", binding))
    }
}

/// Condition id used by [`tool_call_terms`]: the requester's own judgement of the result.
pub const RESULT_CONDITION: &str = "result";

/// Minimal valid terms for a transactional tool call (spec §6.3/§6.4), with a single `human`
/// condition so a `REJECT` can name what was wrong.
pub fn tool_call_terms(
    tool: &str,
    input: Value,
    deadline: u64,
    review_window: u64,
    price_sat: u64,
) -> Value {
    serde_json::to_value(Terms {
        v: crate::terms::TERMS_VERSION.into(),
        deliverable: crate::terms::Deliverable {
            description: format!("Call `{tool}`"),
            tool: Some(tool.into()),
            input: Some(input),
            ..Default::default()
        },
        conditions: vec![crate::terms::Condition {
            id: RESULT_CONDITION.into(),
            class: crate::terms::ConditionClass::Human,
            description: "The result is what I asked for".into(),
            extra: Default::default(),
        }],
        deadline,
        review_window,
        rework_limit: 0,
        price: crate::terms::Price {
            amount: price_sat,
            unit: "sat".into(),
        },
        payment: None,
        verifiers: vec![],
        arbiter: None,
        provenance: None,
        subcontracting: false,
        disclosure: Default::default(),
        governance: None,
        settlement_trigger: Default::default(),
        settlement_verifier: None,
        fund_window: None,
        disqualifiers: vec![],
        template_ref: None,
    })
    .expect("terms serialize")
}

/// A terms template as published in `K_SVC` (spec §7 "Templates"). Times are relative and are
/// fixed when a requester instantiates it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Template {
    /// `T0`, `T1` or `N`.
    pub profile: String,
    pub deadline_secs: u64,
    pub review_window_secs: u64,
    /// Partial terms; `deadline`, `review_window`, `deliverable.tool/input` and the payment
    /// hashlock/locktime are filled in by [`instantiate`].
    pub terms: Value,
    /// The requester must supply a hashlock `H = sha256(s)` (conditional settlement).
    #[serde(default)]
    pub hashlock: bool,
    /// Extra seconds after `deadline + review_window` before the refund path opens (§8.2).
    #[serde(default = "default_margin")]
    pub locktime_margin_secs: u64,
    /// Held-out instances (CR-003 §2.3): the requester generates a seed, commits its hash in the
    /// terms, and gives the seed privately (NIP-44, a `seed` tag) to this runner key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester_seed_to: Option<String>,
}

fn default_margin() -> u64 {
    24 * 60 * 60
}

/// Fill a template into concrete terms for one request. `requester_seed_hash` fills every
/// `instances.requester_seed_hash` (required when the template has `requester_seed_to`).
pub fn instantiate(
    t: &Template,
    tool: &str,
    input: Value,
    now: u64,
    hashlock: Option<&str>,
    requester_seed_hash: Option<&str>,
) -> Result<Value, String> {
    let mut terms = t.terms.clone();
    let obj = terms
        .as_object_mut()
        .ok_or("template terms must be an object")?;
    obj.entry("v").or_insert(json!(crate::terms::TERMS_VERSION));
    let deadline = now + t.deadline_secs;
    obj.insert("deadline".into(), json!(deadline));
    obj.insert("review_window".into(), json!(t.review_window_secs));
    let deliverable = obj.entry("deliverable").or_insert(json!({}));
    deliverable["tool"] = json!(tool);
    deliverable["input"] = input;
    if deliverable.get("description").is_none() {
        deliverable["description"] = json!(format!("Call `{tool}`"));
    }
    if t.hashlock {
        let h = hashlock.ok_or("this template requires a hashlock")?;
        let payment = obj
            .get_mut("payment")
            .filter(|p| p.is_object())
            .ok_or("hashlock template needs `payment`")?;
        payment["hashlock"] = json!(h);
        payment["locktime"] = json!(deadline + t.review_window_secs + t.locktime_margin_secs);
    }
    if t.requester_seed_to.is_some() {
        let h = requester_seed_hash.ok_or("this template requires a requester seed")?;
        for list in ["conditions", "disqualifiers"] {
            for c in obj
                .get_mut(list)
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                if let Some(i) = c.get_mut("instances").and_then(Value::as_object_mut) {
                    i.insert("requester_seed_hash".into(), json!(h));
                }
            }
        }
    }
    Terms::parse(&terms)?;
    Ok(terms)
}

/// x-only public key (hex) for a raw secret key.
pub fn public_key(secret: &[u8; 32]) -> Result<String, DraftError> {
    let sk = SigningKey::from_bytes(secret).map_err(|_| DraftError::BadKey)?;
    Ok(hex::encode(sk.verifying_key().to_bytes()))
}
