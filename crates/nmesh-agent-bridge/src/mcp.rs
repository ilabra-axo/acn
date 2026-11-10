//! MCP client side: connect to configured servers, list tools, call them.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context;
use nmesh_contracts::{schema_hash, ContractRegistry};
use nmesh_proto::command::{ArgSpec, ArgType};
use nmesh_proto::manifest::{ContractDefinition, ToolSummary};
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Map, Value};

use crate::config::{McpServerConfig, McpTransport};

type McpClient = RunningService<RoleClient, ()>;

/// One MCP tool exposed as a slash command.
#[derive(Clone)]
pub struct ToolBinding {
    pub server: String,
    pub tool: Tool,
    /// Slash command name: the tool name, prefixed with `<server>.` on collisions.
    pub command: String,
    pub args: Vec<ArgSpec>,
    /// Contract for `structuredContent`, if the tool declares an `outputSchema`.
    pub output_contract: Option<String>,
}

impl ToolBinding {
    pub fn summary(&self) -> ToolSummary {
        ToolSummary {
            name: self.command.clone(),
            description: self
                .tool
                .description
                .as_deref()
                .unwrap_or_default()
                .to_string(),
            output_contract: self.output_contract.clone(),
        }
    }
}

pub struct McpHub {
    clients: BTreeMap<String, Arc<McpClient>>,
    pub bindings: Vec<ToolBinding>,
}

impl McpHub {
    /// Connect to every configured server. Servers that fail to start are logged and skipped
    /// so one broken server doesn't take the whole agent down.
    pub async fn connect(servers: &[McpServerConfig], registry: &mut ContractRegistry) -> Self {
        let mut clients = BTreeMap::new();
        let mut tools: Vec<(String, Tool)> = Vec::new();

        for s in servers {
            match connect_one(s).await {
                Ok(client) => match client.list_all_tools().await {
                    Ok(list) => {
                        tracing::info!(server = %s.name, tools = list.len(), "connected to MCP server");
                        tools.extend(list.into_iter().map(|t| (s.name.clone(), t)));
                        clients.insert(s.name.clone(), Arc::new(client));
                    }
                    Err(e) => tracing::error!(server = %s.name, error = %e, "tools/list failed"),
                },
                Err(e) => {
                    tracing::error!(server = %s.name, error = %e, "failed to connect MCP server")
                }
            }
        }

        let mut name_count: BTreeMap<String, usize> = BTreeMap::new();
        for (_, t) in &tools {
            *name_count.entry(t.name.to_string()).or_default() += 1;
        }

        let bindings = tools
            .into_iter()
            .map(|(server, tool)| {
                let command = if name_count[tool.name.as_ref()] > 1 {
                    format!("{server}.{}", tool.name)
                } else {
                    tool.name.to_string()
                };
                let output_contract = tool
                    .output_schema
                    .as_ref()
                    .and_then(|schema| register_output_contract(registry, &server, &tool, schema));
                ToolBinding {
                    args: args_from_input_schema(&tool.input_schema),
                    server,
                    tool,
                    command,
                    output_contract,
                }
            })
            .collect();

        Self { clients, bindings }
    }

    /// `name/version` of a connected server, as it introduced itself (for derivation records).
    pub fn server_version(&self, server: &str) -> Option<String> {
        let peer = self.clients.get(server)?.peer_info()?;
        let info = peer.server_info.as_ref()?;
        Some(format!("{}/{}", info.name, info.version))
    }

    pub fn find(&self, command: &str) -> Option<&ToolBinding> {
        self.bindings.iter().find(|b| b.command == command)
    }

    pub async fn call(
        &self,
        binding: &ToolBinding,
        arguments: Map<String, Value>,
    ) -> anyhow::Result<CallToolResult> {
        let client = self
            .clients
            .get(&binding.server)
            .context("MCP server not connected")?;
        let params =
            CallToolRequestParams::new(binding.tool.name.clone()).with_arguments(arguments);
        Ok(client.call_tool(params).await?)
    }
}

async fn connect_one(s: &McpServerConfig) -> anyhow::Result<McpClient> {
    match &s.transport {
        McpTransport::Stdio { command, args, env } => {
            let cmd = tokio::process::Command::new(command).configure(|c| {
                c.args(args).envs(env);
            });
            let transport =
                TokioChildProcess::new(cmd).with_context(|| format!("spawning `{command}`"))?;
            Ok(().serve(transport).await?)
        }
        McpTransport::Http { url } => {
            let transport = StreamableHttpClientTransport::from_uri(url.as_str());
            Ok(().serve(transport).await?)
        }
    }
}

/// Derive a contract from a tool's `outputSchema`.
///
/// If the schema declares `"x-nmesh-contract": "<id>"` and that id is already known, the tool
/// is mapped onto it (so tools can emit built-in contracts like `nmesh.card/v1` and get rich
/// rendering for free). Otherwise a contract `mcp.<server>.<tool>/v1` is registered from the
/// schema, which the generic renderer can display.
fn register_output_contract(
    registry: &mut ContractRegistry,
    server: &str,
    tool: &Tool,
    schema: &Map<String, Value>,
) -> Option<String> {
    if let Some(id) = schema.get("x-nmesh-contract").and_then(Value::as_str) {
        if registry.get(id).is_some() {
            return Some(id.to_string());
        }
        tracing::warn!(tool = %tool.name, contract = id, "unknown x-nmesh-contract; deriving one");
    }
    let schema = Value::Object(schema.clone());
    let id = format!("mcp.{server}.{}/v1", tool.name);
    let def = ContractDefinition {
        id: id.clone(),
        title: tool.title.clone().unwrap_or_else(|| tool.name.to_string()),
        description: tool.description.as_deref().unwrap_or_default().to_string(),
        schema: schema.clone(),
        capabilities: vec![],
    };
    match registry.insert(def) {
        Ok(_) => {
            tracing::debug!(%id, hash = %schema_hash(&schema), "derived contract from outputSchema");
            Some(id)
        }
        Err(e) => {
            tracing::warn!(tool = %tool.name, error = %e, "outputSchema is not a valid JSON Schema");
            None
        }
    }
}

/// Map a JSON Schema `inputSchema` to NIP-CD positional args.
///
/// Required properties first, in schema order; then at most one optional property. If the last
/// argument is a string it becomes a `text` arg (consumes the rest of the line). Tools that
/// don't fit this shape can always be called with a JSON object: `/tool {"a": 1}`.
fn args_from_input_schema(schema: &Map<String, Value>) -> Vec<ArgSpec> {
    let props = match schema.get("properties").and_then(Value::as_object) {
        Some(p) => p,
        None => return vec![],
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let spec = |name: &str, required: bool| {
        let p = &props[name];
        let (ty, choices) = match (p.get("type").and_then(Value::as_str), p.get("enum")) {
            (_, Some(Value::Array(e))) => (
                ArgType::Enum,
                e.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            ),
            (Some("number" | "integer"), _) => (ArgType::Number, vec![]),
            (Some("boolean"), _) => (ArgType::Bool, vec![]),
            _ => (ArgType::Word, vec![]),
        };
        ArgSpec {
            name: name.to_string(),
            ty,
            required,
            label: p
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or(name)
                .to_string(),
            choices,
        }
    };

    let mut out: Vec<ArgSpec> = required
        .iter()
        .filter(|n| props.contains_key(**n))
        .map(|n| spec(n, true))
        .collect();
    if let Some(opt) = props.keys().find(|k| !required.contains(&k.as_str())) {
        out.push(spec(opt, false));
    }
    if let Some(last) = out.last_mut() {
        let is_string = props[&last.name].get("type").and_then(Value::as_str) == Some("string");
        if last.ty == ArgType::Word && is_string {
            last.ty = ArgType::Text;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn input_schema_to_args() {
        let schema = json!({
            "type": "object",
            "properties": {
                "city": {"type": "string"},
                "days": {"type": "integer"},
                "units": {"type": "string", "enum": ["c", "f"]}
            },
            "required": ["days", "city"]
        });
        let args = args_from_input_schema(schema.as_object().unwrap());
        let names: Vec<_> = args
            .iter()
            .map(|a| (a.name.as_str(), a.ty, a.required))
            .collect();
        assert_eq!(
            names,
            vec![
                ("days", ArgType::Number, true),
                ("city", ArgType::Word, true),
                ("units", ArgType::Enum, false),
            ]
        );
    }

    #[test]
    fn trailing_string_becomes_text() {
        let schema = json!({
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"]
        });
        let args = args_from_input_schema(schema.as_object().unwrap());
        assert_eq!(args[0].ty, ArgType::Text);
    }
}
