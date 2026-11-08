//! Minimal MCP server over stdio used to demo / test the agent bridge.
//!
//! Tools:
//! - `echo <text>`           → plain text
//! - `card <title> [body]`   → structured `nmesh.card/v1` (via outputSchema + x-nmesh-contract),
//!   with a button that triggers `/echo`
//! - `form`                  → structured `nmesh.form/v1` whose submission calls `/card`
//! - `status`                → structured `nmesh.table/v1`

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Map, Value};

#[derive(Clone)]
struct Echo;

fn obj(v: Value) -> Arc<Map<String, Value>> {
    match v {
        Value::Object(m) => Arc::new(m),
        _ => unreachable!(),
    }
}

fn tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "echo",
            "Echo the given text back",
            obj(json!({
                "type": "object",
                "properties": {"text": {"type": "string", "description": "Text to echo"}},
                "required": ["text"]
            })),
        ),
        Tool::new(
            "card",
            "Return a card with the given title and body",
            obj(json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "body": {"type": "string"}
                },
                "required": ["title"]
            })),
        )
        .with_raw_output_schema(contract_schema("nmesh.card/v1")),
        Tool::new(
            "form",
            "Ask for a card's title and body using a form",
            obj(json!({"type": "object", "properties": {}})),
        )
        .with_raw_output_schema(contract_schema("nmesh.form/v1")),
        Tool::new(
            "status",
            "Show a status table",
            obj(json!({"type": "object", "properties": {}})),
        )
        .with_raw_output_schema(contract_schema("nmesh.table/v1")),
    ]
}

/// An outputSchema that just points at an nmesh contract; the bridge validates against it.
fn contract_schema(id: &str) -> Arc<Map<String, Value>> {
    obj(json!({"type": "object", "x-nmesh-contract": id}))
}

impl ServerHandler for Echo {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: tools(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = request.arguments.unwrap_or_default();
        let get = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let result = match request.name.as_ref() {
            "echo" => CallToolResult::success(vec![ContentBlock::text(get("text"))]),
            "card" => {
                let mut card = json!({
                    "title": get("title"),
                    "actions": [{"id": "/echo You pressed the button", "label": "Press me", "style": "primary"}]
                });
                let body = get("body");
                if !body.is_empty() {
                    card["body"] = Value::String(body);
                }
                CallToolResult::structured(card)
            }
            "form" => CallToolResult::structured(json!({
                "title": "Make a card",
                "instructions": "I'll turn this into a card.",
                "fields": [
                    {"name": "title", "label": "Title", "type": "text", "required": true},
                    {"name": "body", "label": "Body", "type": "multiline"}
                ],
                "submit_label": "Create card",
                "submit_action": "/card"
            })),
            "status" => CallToolResult::structured(json!({
                "title": "Services",
                "columns": [
                    {"key": "name", "label": "Service"},
                    {"key": "state", "label": "State"},
                    {"key": "latency", "label": "p50 ms", "type": "number"}
                ],
                "rows": [
                    {"name": "relay", "state": "healthy", "latency": 12},
                    {"name": "bridge", "state": "healthy", "latency": 4},
                    {"name": "blossom", "state": "degraded", "latency": 230}
                ]
            })),
            other => {
                return Err(ErrorData::invalid_params(
                    format!("unknown tool {other}"),
                    None,
                ));
            }
        };
        Ok(result.into())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = Echo.serve(rmcp::transport::stdio()).await?;
    server.waiting().await?;
    Ok(())
}
