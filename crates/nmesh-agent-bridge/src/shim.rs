//! `acn-mcp-shim`: MCP tools as ACN commitments (docs/Core.md §6.3, §11), on the Binding API
//! (`acn-binding`, docs/harness-binding-API.md §3).
//!
//! The shim is the Executor: an MCP tool call per commitment. Signing and delivery are the
//! session's (`acn-binding-nostr`), evidence goes to Blossom (only with `evidence = "public"`),
//! payment to the CDK wallet, and every decision comes from `acn_core::fold` over the events seen
//! (INV-12). Effects are not repeated across restarts and relay replays because the performer's
//! execution journal records intent before execution (see `acn_binding::journal`).

use std::sync::Arc;

use acn_binding::{Artifact, BoxFut, Executor, Job, Level, Output, Quote, ServiceAd};
use acn_core::{Profile, Terms};
use nmesh_contracts::ContractRegistry;
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

use crate::config::AcnConfig;
use crate::mcp::McpHub;

/// An MCP tool call per commitment.
pub struct McpExecutor {
    pub hub: Arc<McpHub>,
    pub registry: Arc<ContractRegistry>,
}

impl Executor for McpExecutor {
    fn quote(&self, terms: &Terms, _: Profile) -> Quote {
        match terms
            .deliverable
            .tool
            .as_deref()
            .and_then(|t| self.hub.find(t))
        {
            Some(_) => Quote::Accept { p_on_time: 0.95 },
            None => Quote::Decline {
                reason: "unknown_tool".into(),
                note: format!("no tool {:?}", terms.deliverable.tool),
            },
        }
    }

    fn run(&self, job: Job) -> BoxFut<'_, Result<Output, String>> {
        Box::pin(async move {
            let b = job
                .tool
                .as_deref()
                .and_then(|t| self.hub.find(t))
                .ok_or("unknown tool")?;
            let args = job.input.as_object().cloned().unwrap_or_default();
            tracing::info!(commitment = %job.commitment, tool = %b.command, "acn: executing");
            let r = self.hub.call(b, args).await.map_err(|e| e.to_string())?;
            if r.is_error == Some(true) {
                return Err(text_of(&r));
            }
            let (payload, contract) =
                report_payload(&self.registry, b.output_contract.as_deref(), &r);
            Ok(Output {
                result: Some(payload),
                contract,
                // The whole tool result, as the server returned it: the artifact the inline
                // result is derived from. Uploaded only with `evidence = "public"`, and only
                // after the REPORT referencing it has passed the gate (acn-binding).
                artifacts: vec![Artifact {
                    role: "tool_result".into(),
                    media_type: "application/json".into(),
                    bytes: serde_json::to_vec(&r).unwrap_or_default(),
                }],
                tools: vec![json!({
                    "tool": b.tool.name,
                    "service": format!("mcp:{}", b.server),
                    "version": self.hub.server_version(&b.server),
                })],
                log: None,
            })
        })
    }
}

/// The conformance level the configuration may claim in `K_SVC`: L0 needs published evidence
/// (Blossom servers and `evidence = "public"`: the suite fetches the artifacts) and an operator
/// binding (discovery requires one in force). Hosts also AND this with
/// [`acn_binding::Performer::level`].
pub fn claimed_level(cfg: &AcnConfig) -> Option<Level> {
    (!cfg.blossom.is_empty()
        && cfg.evidence == acn_binding::Disclosure::Public
        && cfg.operator_key_file.is_some())
    .then_some(Level::L0)
}

/// The terms offer for one tool (`K_SVC` `terms_template`, Core §7).
pub fn offer(tool: &str, cfg: &AcnConfig) -> Value {
    acn_binding::call_offer(
        tool,
        cfg.price_sat,
        cfg.mint.as_deref(),
        cfg.deadline_secs,
        cfg.review_window_secs,
    )
}

/// `K_SVC` content (docs/Core.md §4).
pub fn service_advertisement(
    hub: &McpHub,
    cfg: &AcnConfig,
    data_handling: &Value,
    mint_capabilities: &[String],
    level: Option<Level>,
) -> ServiceAd {
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
                "terms_template": offer(&b.command, cfg),
            })
        })
        .collect();
    let paid = cfg.price_sat > 0 && cfg.mint.is_some();
    ServiceAd {
        tools,
        profiles: if paid {
            vec!["T0".into(), "T1".into()]
        } else {
            vec!["T0".into()]
        },
        min_price_sat: cfg.price_sat,
        mints: cfg
            .mint
            .iter()
            .map(|m| (m.clone(), mint_capabilities.to_vec()))
            .collect(),
        blossom: cfg.blossom.clone(),
        binding_level: level,
        data_handling: data_handling.clone(),
        deadline_secs: cfg.deadline_secs,
        review_window_secs: cfg.review_window_secs,
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

#[cfg(test)]
mod tests {
    use super::*;
    use acn_binding::Disclosure;

    #[test]
    fn l0_needs_public_evidence_and_an_operator_binding() {
        let full = AcnConfig {
            blossom: vec!["http://127.0.0.1:3400".into()],
            evidence: Disclosure::Public,
            operator_key_file: Some("operator.nsec".into()),
            ..Default::default()
        };
        assert_eq!(claimed_level(&full), Some(Level::L0));
        let private = AcnConfig {
            evidence: Disclosure::Private,
            ..full.clone()
        };
        assert_eq!(claimed_level(&private), None);
        let unbound = AcnConfig {
            operator_key_file: None,
            ..full.clone()
        };
        assert_eq!(claimed_level(&unbound), None);
        let no_store = AcnConfig {
            blossom: vec![],
            ..full
        };
        assert_eq!(claimed_level(&no_store), None);
    }
}
