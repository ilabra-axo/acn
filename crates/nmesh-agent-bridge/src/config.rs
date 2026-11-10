use std::collections::BTreeMap;
use std::path::PathBuf;

pub use acn_binding::Disclosure;
use nmesh_proto::manifest::Hosting;
use serde::Deserialize;

/// Agent bridge configuration (TOML). See `crates/nmesh-agent-bridge/agent.example.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub agent: AgentSection,
    pub relays: Vec<String>,
    #[serde(default, rename = "mcp")]
    pub servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub acn: AcnConfig,
}

/// ACN commitments (docs/Core.md). The bridge acts as `acn-mcp-shim`: each MCP tool can be
/// requested as a T0 commitment and is answered with a signed REPORT.
#[derive(Debug, Clone, Deserialize)]
pub struct AcnConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Price per call (sat). Above 0, calls are T1 with a hash-locked Cashu token at `mint`.
    #[serde(default)]
    pub price_sat: u64,
    /// The Cashu mint the agent is paid at (needed when `price_sat` > 0).
    #[serde(default)]
    pub mint: Option<String>,
    /// The mint's rail capabilities, as advertised in `K_SVC` (Core §8.6).
    #[serde(default = "default_capabilities")]
    pub mint_capabilities: Vec<String>,
    /// Where the agent's wallet keeps its state. Omit to keep it in memory.
    #[serde(default)]
    pub wallet_dir: Option<PathBuf>,
    /// Blossom servers: where terms sent by reference are looked up, and (with
    /// `evidence = "public"`) where artifacts and derivation records go (Core §9).
    #[serde(default)]
    pub blossom: Vec<String>,
    /// Evidence disclosure: `"private"` (default) uploads nothing, the REPORT carries the inline
    /// result only and no conformance level is claimed; `"public"` uploads the whole tool result,
    /// the derivation record and logs to the Blossom servers, unencrypted, after the REPORT has
    /// passed the gate.
    #[serde(default)]
    pub evidence: Disclosure,
    /// The durable execution journal (JSON). Default: next to `agent.key_file`
    /// (`<key>.acn-journal.json`).
    #[serde(default)]
    pub journal: Option<PathBuf>,
    /// Suggested deadline for requests, seconds from now.
    #[serde(default = "default_deadline")]
    pub deadline_secs: u64,
    #[serde(default = "default_review")]
    pub review_window_secs: u64,
    /// Operator key used to issue a binding for this agent key (self-hosted convenience).
    /// Omit to run unbound: clients will flag the agent as unaccountable.
    #[serde(default)]
    pub operator_key_file: Option<PathBuf>,
    #[serde(default = "default_binding_days")]
    pub binding_days: u64,
}

impl Default for AcnConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            price_sat: 0,
            mint: None,
            mint_capabilities: default_capabilities(),
            wallet_dir: None,
            blossom: vec![],
            evidence: Disclosure::default(),
            journal: None,
            deadline_secs: default_deadline(),
            review_window_secs: default_review(),
            operator_key_file: None,
            binding_days: default_binding_days(),
        }
    }
}

fn default_capabilities() -> Vec<String> {
    [
        "hashlock",
        "timelock_refund",
        "p2pk",
        "reveals_preimage_to_payer",
    ]
    .map(String::from)
    .to_vec()
}

fn yes() -> bool {
    true
}
fn default_deadline() -> u64 {
    600
}
fn default_review() -> u64 {
    24 * 60 * 60
}
fn default_binding_days() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentSection {
    /// Stable id (manifest `d` tag).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub about: String,
    /// Path to the agent's own nsec. Created if missing. Never reuse a human's key.
    pub key_file: PathBuf,
    pub hosting: Hosting,
    /// Who operates this bridge (npub, NIP-05, org name). Shown to users before inviting.
    pub operator: String,
    #[serde(default = "default_retention")]
    pub retention: String,
    /// Extra recipients of plaintext beyond the MCP servers (e.g. an LLM provider).
    #[serde(default)]
    pub forwards_to: Vec<String>,
    /// If set, only these pubkeys (npub/hex) may invoke the agent. Empty = anyone.
    #[serde(default)]
    pub allow: Vec<String>,
}

fn default_retention() -> String {
    "none".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    /// Short name; namespaces auto-generated contract ids: `mcp.<name>.<tool>/v1`.
    pub name: String,
    #[serde(flatten)]
    pub transport: McpTransport,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
    },
}

impl AgentConfig {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&s)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_config() {
        let s = include_str!("../agent.example.toml");
        let c: AgentConfig = toml::from_str(s).unwrap();
        assert_eq!(c.agent.hosting, Hosting::SelfHosted);
        assert_eq!(c.servers.len(), 2);
        assert!(matches!(c.servers[0].transport, McpTransport::Stdio { .. }));
        assert!(matches!(c.servers[1].transport, McpTransport::Http { .. }));
        assert_eq!(c.acn.evidence, Disclosure::Public);
    }

    #[test]
    fn evidence_is_private_by_default() {
        let c: AcnConfig = toml::from_str("price_sat = 0").unwrap();
        assert_eq!(c.evidence, Disclosure::Private);
        assert_eq!(AcnConfig::default().evidence, Disclosure::Private);
    }
}
