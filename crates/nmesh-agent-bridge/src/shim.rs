//! `acn-mcp-shim`: MCP tools as ACN T0 commitments (docs/Core.md §6.3, §11).
//!
//! The shim owns no commitment semantics: every decision is taken from `acn_core::fold` over the
//! events it has seen (INV-12), which also makes it idempotent across restarts and relay replays.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use acn_core::draft::Draft;
use acn_core::kinds::*;
use acn_core::{fold, Phase, Profile};
use nmesh_contracts::ContractRegistry;
use nmesh_core::{PublicKey, Session, Timestamp};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

use crate::config::AcnConfig;
use crate::mcp::McpHub;

#[derive(Default)]
pub struct Store {
    /// commitment id -> events.
    by_commitment: Mutex<HashMap<String, BTreeMap<String, acn_core::Event>>>,
}

impl Store {
    /// Add an event; returns its commitment id.
    pub fn add(&self, e: acn_core::Event) -> String {
        let c = if e.kind == K_REQUEST {
            e.id.clone()
        } else {
            e.tags
                .iter()
                .find(|t| t.first().map(String::as_str) == Some("c"))
                .and_then(|t| t.get(1).cloned())
                .unwrap_or_default()
        };
        self.by_commitment
            .lock()
            .unwrap()
            .entry(c.clone())
            .or_default()
            .insert(e.id.clone(), e);
        c
    }

    pub fn events(&self, c: &str) -> Vec<acn_core::Event> {
        self.by_commitment
            .lock()
            .unwrap()
            .get(c)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn ids(&self) -> Vec<String> {
        self.by_commitment.lock().unwrap().keys().cloned().collect()
    }
}

/// `K_SVC` content (docs/Core.md §4).
pub fn service_advertisement(hub: &McpHub, cfg: &AcnConfig, data_handling: &Value) -> Value {
    let tools: Vec<Value> = hub
        .bindings
        .iter()
        .map(|b| {
            let input = Value::Object((*b.tool.input_schema).clone());
            let output = b
                .tool
                .output_schema
                .as_ref()
                .map(|s| Value::Object((**s).clone()));
            json!({
                "name": b.command,
                "description": b.tool.description.as_deref().unwrap_or_default(),
                "input_schema_hash": acn_core::jcs::hash(&input).ok(),
                "output_schema_hash": output.as_ref().and_then(|o| acn_core::jcs::hash(o).ok()),
                "contract": b.output_contract,
            })
        })
        .collect();
    json!({
        "v": acn_core::kinds::VERSION,
        "tools": tools,
        "profiles": ["T0"],
        "price": { "min": cfg.price_sat, "unit": "sat" },
        "mints": [],
        "transport": "nostr",
        "terms_defaults": { "deadline_secs": cfg.deadline_secs, "review_window_secs": cfg.review_window_secs },
        "silence_threshold_sat": 0,
        "data_handling": data_handling,
    })
}

pub async fn publish_service(session: &Session, id: &str, svc: &Value) -> anyhow::Result<()> {
    let d = Draft::new(K_SVC)
        .tag(vec!["d".into(), id.into()])
        .tag(vec!["alt".into(), "ACN service advertisement".into()])
        .content(svc.clone());
    let ev = session.sign_act(&d, None)?;
    session.publish_act(&ev).await?;
    Ok(())
}

/// Issue (if needed) an operator binding for the agent key and acknowledge it (docs/Core.md §4).
pub async fn ensure_binding(session: &Session, cfg: &AcnConfig) -> anyhow::Result<()> {
    let Some(path) = &cfg.operator_key_file else {
        tracing::warn!("acn: no operator_key_file; agent will be flagged unaccountable");
        return Ok(());
    };
    let operator = nmesh_core::load_or_create_keys(path)?;
    let me = session.public_key();
    let (_, existing) = session.acn_public(me, Duration::from_secs(3)).await?;
    let now = Timestamp::now().as_secs();
    let status =
        acn_core::binding_status(&existing, &me.to_hex(), K_REPORT, cfg.price_sat, None, now);
    if matches!(status, acn_core::BindingStatus::Bound { .. }) {
        tracing::info!("acn: operator binding already in place");
        return Ok(());
    }
    let performer_kinds = [
        K_COUNTER, K_ACCEPT, K_PROMISE, K_DECLINE, K_REVOKE, K_PROGRESS, K_REPORT, K_DISPUTE,
    ]
    .map(|k| k.to_string())
    .join(",");
    let max = cfg.price_sat.to_string();
    let bind = Draft::bind(
        &me.to_hex(),
        now + cfg.binding_days * 24 * 60 * 60,
        &[
            ("kinds", performer_kinds.as_str()),
            ("max_sat", max.as_str()),
        ],
    )
    .sign_with_aux(&operator.secret_key().secret_bytes(), now, &rand32())?;
    session.publish_act(&bind).await?;
    let ack = session.sign_act(&Draft::bind_ack(&bind.id), Some(now))?;
    session.publish_act(&ack).await?;
    tracing::info!(operator = %operator.public_key().to_hex(), "acn: published operator binding");
    Ok(())
}

fn rand32() -> [u8; 32] {
    // BIP-340 aux randomness; uniqueness matters more than quality here.
    let keys = nmesh_core::Keys::generate();
    keys.secret_key().secret_bytes()
}

pub struct Shim<'a> {
    pub session: &'a Session,
    pub hub: &'a McpHub,
    pub registry: &'a ContractRegistry,
    pub store: &'a Store,
}

impl Shim<'_> {
    /// Re-evaluate commitment `c` and perform the performer's move, if any.
    pub async fn drive(&self, c: &str) -> anyhow::Result<()> {
        let me = self.session.public_key().to_hex();
        let events = self.store.events(c);
        let out = fold(&events, &BTreeMap::new());
        let Some(cm) = out.commitments.get(c) else {
            return Ok(());
        };
        let (Some(st), Some(terms)) = (&cm.state, &cm.terms) else {
            return Ok(());
        };
        if cm.performer != me || st.phase != Phase::Requested {
            if cm.performer == me && st.phase == Phase::Satisfied {
                tracing::info!(commitment = %c, "acn: satisfied");
            }
            return Ok(());
        }
        let requester = PublicKey::parse(&cm.requester)?;
        let parties = [cm.requester.as_str(), me.as_str()];
        let decline = |reason: &str, note: String| {
            Draft::closing(K_DECLINE, c, &st.head, &parties, reason)
                .content(json!({ "reason": reason, "note": note }))
        };

        let now = Timestamp::now().as_secs();
        // Performer-side settlement policy (Binding API policy filters): this binding only
        // settles through the requester.
        let draft = if !terms.settlement_trigger.is_requester() {
            decline(
                "trigger_unsupported",
                "this agent accepts the requester settlement trigger only".into(),
            )
        } else if cm.profile != Profile::T0 {
            decline("profile_unsupported", "this agent offers T0 only".into())
        } else if terms.price.amount > 0 {
            decline(
                "payment_unsupported",
                "paid calls need Cashu settlement (not yet supported)".into(),
            )
        } else if now > terms.deadline {
            decline("expired", "request deadline already passed".into())
        } else if let Some(binding) = terms
            .deliverable
            .tool
            .as_deref()
            .and_then(|t| self.hub.find(t))
        {
            let input = terms.deliverable.input.clone().unwrap_or_else(|| json!({}));
            let args = input.as_object().cloned().unwrap_or_default();
            tracing::info!(commitment = %c, tool = %binding.command, "acn: executing REQUEST");
            match self.hub.call(binding, args).await {
                Ok(result) if result.is_error != Some(true) => {
                    let (payload, contract) =
                        report_payload(self.registry, binding.output_contract.as_deref(), &result);
                    Draft::report(
                        c,
                        &st.head,
                        &parties,
                        Some(payload),
                        contract.as_deref(),
                        &[],
                    )
                }
                Ok(result) => decline("tool_error", text_of(&result)),
                Err(e) => decline("tool_error", e.to_string()),
            }
        } else {
            decline(
                "unknown_tool",
                format!("no tool {:?}", terms.deliverable.tool),
            )
        };

        let act = self.session.sign_act(&draft, Some(st.head_at))?;
        self.store.add(act.clone());
        self.session.send_act(requester, &act).await?;
        tracing::info!(commitment = %c, act = acn_core::kinds::name(act.kind), "acn: sent");
        Ok(())
    }
}

/// REPORT content: structuredContent bound to its contract when it validates, else text.
fn report_payload(
    registry: &ContractRegistry,
    contract: Option<&str>,
    r: &CallToolResult,
) -> (Value, Option<String>) {
    if let (Some(id), Some(sc)) = (contract, &r.structured_content) {
        if let Some(cref) = registry.pinned_ref(id) {
            if registry.validate(&cref, sc).is_ok() {
                return (sc.clone(), Some(id.to_string()));
            }
        }
    }
    match &r.structured_content {
        Some(sc) => (sc.clone(), None),
        None => (json!({ "text": text_of(r) }), None),
    }
}

fn text_of(r: &CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}
