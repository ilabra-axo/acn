//! Turn an MCP `CallToolResult` into an nmesh reply.

use nmesh_contracts::ContractRegistry;
use nmesh_core::EventId;
use nmesh_proto::StructuredMessage;
use rmcp::model::CallToolResult;
use serde_json::Value;

use crate::mcp::ToolBinding;

pub enum Reply {
    Structured(StructuredMessage),
    Text(String),
}

pub fn from_tool_result(
    registry: &ContractRegistry,
    binding: &ToolBinding,
    result: &CallToolResult,
    reply_to: EventId,
) -> Reply {
    let text = text_content(result);

    if result.is_error == Some(true) {
        return Reply::Text(format!("/{} error: {}", binding.command, text));
    }

    if let (Some(contract_id), Some(payload)) =
        (&binding.output_contract, &result.structured_content)
    {
        if let Some(contract) = registry.pinned_ref(contract_id) {
            match registry.validate(&contract, payload) {
                Ok(_) => {
                    // Many servers mirror structuredContent as JSON text; that makes a poor alt.
                    let mirrors_json =
                        serde_json::from_str::<Value>(&text).ok().as_ref() == Some(payload);
                    let alt = if text.is_empty() || mirrors_json {
                        alt_from_payload(payload)
                    } else {
                        text.clone()
                    };
                    return Reply::Structured(
                        StructuredMessage::new(contract, payload.clone(), truncate(&alt, 500))
                            .reply_to(reply_to),
                    );
                }
                Err(e) => tracing::warn!(tool = %binding.command, error = %e,
                    "structuredContent failed contract validation; falling back to text"),
            }
        }
    }

    if text.is_empty() {
        Reply::Text(
            result
                .structured_content
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "(no output)".into()),
        )
    } else {
        Reply::Text(text)
    }
}

fn text_content(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn alt_from_payload(v: &Value) -> String {
    ["title", "summary", "label", "name"]
        .iter()
        .find_map(|k| {
            v.get(*k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
        })
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars()
            .take(max - 1)
            .chain(std::iter::once('…'))
            .collect()
    }
}
