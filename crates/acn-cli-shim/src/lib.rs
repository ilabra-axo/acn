//! `acn-cli-shim`: a second, independent reference binding (docs/harness-binding-API.md §3). It
//! turns any command-line agent harness into an L0 ACN performer: the commitment's input goes to
//! the command (JSON, or its `text` field, on stdin; `ACN_*` environment variables), stdout is
//! the result, files the command writes to `$ACN_OUTPUT_DIR` are artifacts, stderr is the log.
//!
//! It shares nothing with the MCP shim but the Binding API (`acn-binding`) and the reference
//! adapters (`acn-binding-nostr`): MCP is a convenience, not a requirement.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub use acn_binding::Disclosure;
use acn_binding::{Artifact, BoxFut, Executor, Job, Journal, Output, Performer, Quote, ServiceAd};
use acn_binding_nostr::{BlossomEvidence, MintWallet, SessionBinding};
use acn_core::{Profile, Terms};
use anyhow::Context;
use nmesh_core::{Body, IdentityKind, Session, SessionConfig, ToBech32};
use nmesh_proto::command::{ArgSpec, ArgType, SlashCommandDefinition};
use nmesh_proto::manifest::{AgentManifest, DataHandling, Hosting, ToolSummary};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

/// How the commitment's input reaches the command's stdin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stdin {
    /// The input object as JSON.
    #[default]
    Json,
    /// The input's `text` field, as is.
    Text,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub stdin: Stdin,
    /// Recorded in the derivation record's `tools[]` (e.g. the harness's version).
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub about: String,
    /// Who runs it (shown before first contact).
    #[serde(default = "default_operator")]
    pub operator: String,
    pub relays: Vec<String>,
    pub key_file: PathBuf,
    /// Operator key for the binding (Core §4). Omit to run unbound (flagged unaccountable, and
    /// no conformance level can be claimed).
    #[serde(default)]
    pub operator_key_file: Option<PathBuf>,
    /// Blossom servers: terms sent by reference are looked up here, and with
    /// `evidence = "public"` artifacts and derivation records are stored here.
    #[serde(default)]
    pub blossom: Vec<String>,
    /// Evidence disclosure: `"private"` (default) uploads nothing (inline results only, no
    /// conformance level claimed); `"public"` uploads stdout, output files, stderr and the
    /// derivation record to the Blossom servers, unencrypted, after the REPORT passes the gate.
    #[serde(default)]
    pub evidence: Disclosure,
    /// Serve profile N too (conformance level L1): counters that close open conditions,
    /// accepting the requester's counters, promises, rework, disputes of contradicted
    /// rejections (when the terms name an arbiter), held-out instances and disqualifiers.
    #[serde(default = "yes")]
    pub negotiate: bool,
    /// A governance key for the binding's gate (L2): it endorses (`K_ENDORSE`) the acts the
    /// gate policy allows, so the agent can take terms that require endorsed performer acts.
    /// Omit for no gate. Kept apart from the agent and operator keys.
    #[serde(default)]
    pub gate_key_file: Option<PathBuf>,
    /// What the gate endorses (`max_sat`, `tools`); a person's sign-off (`human_kinds`) needs a
    /// host with an approver, which the shims don't have, so those kinds are denied.
    #[serde(default)]
    pub gate_policy: acn_binding::GatePolicy,
    /// The durable execution journal (JSON). Default: next to `key_file`.
    #[serde(default)]
    pub journal: Option<PathBuf>,
    #[serde(default)]
    pub price_sat: u64,
    #[serde(default)]
    pub mint: Option<String>,
    #[serde(default = "default_capabilities")]
    pub mint_capabilities: Vec<String>,
    #[serde(default)]
    pub wallet_dir: Option<PathBuf>,
    #[serde(default = "default_deadline")]
    pub deadline_secs: u64,
    #[serde(default = "default_review")]
    pub review_window_secs: u64,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(rename = "tool")]
    pub tools: Vec<ToolConfig>,
}

fn default_operator() -> String {
    "self-hosted".into()
}
fn yes() -> bool {
    true
}

fn default_capabilities() -> Vec<String> {
    acn_binding_nostr::TEST_MINT_CAPABILITIES
        .iter()
        .map(|s| s.to_string())
        .collect()
}
fn default_deadline() -> u64 {
    600
}
fn default_review() -> u64 {
    3600
}
fn default_timeout() -> u64 {
    120
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let s =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(toml::from_str(&s)?)
    }
}

/// The Executor: one process per commitment.
pub struct CliExecutor {
    pub tools: Vec<ToolConfig>,
    pub timeout: Duration,
}

impl Executor for CliExecutor {
    fn quote(&self, terms: &Terms, _: Profile) -> Quote {
        match terms.deliverable.tool.as_deref() {
            Some(t) if self.tools.iter().any(|x| x.name == t) => Quote::Accept { p_on_time: 0.9 },
            other => Quote::Decline {
                reason: "unknown_tool".into(),
                note: format!("no tool {other:?}"),
            },
        }
    }

    fn run(&self, job: Job) -> BoxFut<'_, Result<Output, String>> {
        Box::pin(async move {
            let t = self
                .tools
                .iter()
                .find(|x| Some(&x.name) == job.tool.as_ref())
                .ok_or("unknown tool")?;
            let out_dir = std::env::temp_dir().join(format!(
                "acn-cli-shim-{}",
                &job.commitment[..16.min(job.commitment.len())]
            ));
            let _ = tokio::fs::remove_dir_all(&out_dir).await;
            tokio::fs::create_dir_all(&out_dir)
                .await
                .map_err(|e| e.to_string())?;
            let stdin = match t.stdin {
                Stdin::Json => job.input.to_string(),
                Stdin::Text => job.input["text"].as_str().unwrap_or_default().to_string(),
            };
            let mut child = tokio::process::Command::new(&t.command)
                .args(&t.args)
                .env("ACN_COMMITMENT", &job.commitment)
                .env("ACN_INPUT_JSON", job.input.to_string())
                .env("ACN_OUTPUT_DIR", &out_dir)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("starting `{}`: {e}", t.command))?;
            if let Some(mut s) = child.stdin.take() {
                s.write_all(stdin.as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
            }
            let out = tokio::time::timeout(self.timeout, child.wait_with_output())
                .await
                .map_err(|_| format!("`{}` timed out after {:?}", t.name, self.timeout))?
                .map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!(
                    "`{}` exited with {}: {}",
                    t.name,
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            let mut artifacts = vec![Artifact {
                role: "stdout".into(),
                media_type: "text/plain".into(),
                bytes: out.stdout.clone(),
            }];
            let mut entries = tokio::fs::read_dir(&out_dir)
                .await
                .map_err(|e| e.to_string())?;
            let mut files = Vec::new();
            while let Ok(Some(e)) = entries.next_entry().await {
                if e.file_type().await.map(|f| f.is_file()).unwrap_or(false) {
                    files.push(e.path());
                }
            }
            files.sort();
            for f in files {
                artifacts.push(Artifact {
                    role: f
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    media_type: "application/octet-stream".into(),
                    bytes: tokio::fs::read(&f).await.map_err(|e| e.to_string())?,
                });
            }
            let _ = tokio::fs::remove_dir_all(&out_dir).await;
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok(Output {
                result: Some(json!({"text": text})),
                contract: None,
                artifacts,
                tools: vec![
                    json!({"tool": t.name, "service": "cli", "command": t.command, "args": t.args, "version": t.version}),
                ],
                log: (!out.stderr.is_empty()).then_some(out.stderr),
            })
        })
    }
}

pub struct CliShim {
    session: Session,
    performer: Arc<Performer>,
    tools: usize,
}

impl CliShim {
    pub async fn start(cfg: Config) -> anyhow::Result<Self> {
        let keys = nmesh_core::load_or_create_keys(&cfg.key_file)?;
        let session = Session::connect(
            keys,
            SessionConfig {
                relays: cfg.relays.clone(),
                kind: IdentityKind::Agent,
                display_name: Some(cfg.name.clone()),
                about: Some(cfg.about.clone()),
                announce: true,
            },
        )
        .await?;
        let sb = Arc::new(SessionBinding(session.clone()));
        let journal = Journal::open(
            cfg.journal
                .clone()
                .unwrap_or_else(|| acn_binding_nostr::default_journal_path(&cfg.key_file)),
        )
        .map_err(anyhow::Error::msg)?;
        let mut performer = Performer::new(
            sb.clone(),
            sb,
            Arc::new(CliExecutor {
                tools: cfg.tools.clone(),
                timeout: Duration::from_secs(cfg.timeout_secs),
            }),
        )
        .with_min_price(cfg.price_sat)
        .with_negotiation(cfg.negotiate)
        .with_journal(journal)
        .with_disclosure(cfg.evidence);
        if !cfg.blossom.is_empty() {
            performer = performer.with_evidence(Arc::new(BlossomEvidence(
                nmesh_blossom::BlossomClient::new(cfg.blossom.clone(), session.keys().clone()),
            )));
        }
        if cfg.price_sat > 0 {
            let mint = cfg.mint.as_deref().context("price_sat > 0 needs `mint`")?;
            let wallet = nmesh_wallet::CashuWallet::open(
                mint,
                &session.keys().secret_key().secret_bytes(),
                cfg.wallet_dir.as_deref(),
            )
            .await?;
            performer = performer.with_wallet(Arc::new(MintWallet {
                wallet,
                capabilities: cfg.mint_capabilities.clone(),
            }));
        }
        if let Some(path) = &cfg.gate_key_file {
            let gk = nmesh_core::load_or_create_keys(path)?;
            performer = performer.with_gate(std::sync::Arc::new(acn_binding::PolicyGate::new(
                std::sync::Arc::new(acn_binding::KeySigner::new(gk.secret_key().secret_bytes())),
                cfg.gate_policy.clone(),
            )));
        }
        let level = cfg.operator_key_file.as_ref().and(performer.level());

        // What the desktop app needs: a manifest (consent) and one slash command per tool.
        let data_handling = DataHandling {
            sees_plaintext: true,
            retention: "none".into(),
            forwards_to: cfg
                .tools
                .iter()
                .map(|t| format!("cli:{}", t.command))
                .collect(),
        };
        let manifest = AgentManifest {
            id: cfg.id.clone(),
            name: cfg.name.clone(),
            description: cfg.about.clone(),
            hosting: Hosting::SelfHosted,
            operator: cfg.operator.clone(),
            data_handling: data_handling.clone(),
            tools: cfg
                .tools
                .iter()
                .map(|t| ToolSummary {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    output_contract: None,
                })
                .collect(),
        };
        session
            .publish_pinned_builder(manifest.to_builder()?)
            .await?;
        for t in &cfg.tools {
            let def = SlashCommandDefinition {
                id: format!("{}:{}", cfg.id, t.name),
                command: t.name.clone(),
                title: t.name.clone(),
                description: t.description.clone(),
                args: vec![ArgSpec {
                    name: "text".into(),
                    ty: ArgType::Text,
                    required: false,
                    label: "Task".into(),
                    choices: vec![],
                }],
                scopes: vec![format!("kind:{}", nmesh_proto::kinds::ACN_PRIVATE_ACT)],
            };
            session.publish_pinned_builder(def.to_builder()).await?;
        }
        let ad = ServiceAd {
            tools: cfg
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "terms_template": acn_binding::call_offer(&t.name, cfg.price_sat, cfg.mint.as_deref(), cfg.deadline_secs, cfg.review_window_secs),
                    })
                })
                .collect(),
            profiles: performer.profiles(),
            min_price_sat: cfg.price_sat,
            mints: cfg.mint.iter().map(|m| (m.clone(), cfg.mint_capabilities.clone())).collect(),
            blossom: cfg.blossom.clone(),
            binding_level: level,
            governance: performer.governance(),
            data_handling: serde_json::to_value(&data_handling)?,
            deadline_secs: cfg.deadline_secs,
            review_window_secs: cfg.review_window_secs,
        };
        acn_binding_nostr::announce(&session, &cfg.id, &ad).await?;
        if let Some(p) = &cfg.operator_key_file {
            let operator = nmesh_core::load_or_create_keys(p)?;
            let binding =
                acn_binding_nostr::ensure_binding(&session, &operator, cfg.price_sat, 30).await?;
            performer.add_context(binding);
        }
        Ok(Self {
            session,
            performer: Arc::new(performer),
            tools: cfg.tools.len(),
        })
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn npub(&self) -> String {
        self.session.public_key().to_bech32().unwrap_or_default()
    }

    pub fn tools(&self) -> usize {
        self.tools
    }

    /// Serve until the inbox closes. Chat messages get a short help text.
    pub async fn run(&self) -> anyhow::Result<()> {
        let s = self.session.clone();
        let help: Value = json!(format!(
            "I'm an ACN agent: request one of my {} tool(s) as a commitment (turn on \"Request commands as commitments\").",
            self.tools
        ));
        acn_binding_nostr::run(&self.session, self.performer.clone(), move |m| {
            if matches!(m.body, Body::Text(_)) {
                let s = s.clone();
                let text = help.as_str().unwrap_or_default().to_string();
                tokio::spawn(async move {
                    let _ = s.send_text(m.sender, &text).await;
                });
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses_and_evidence_defaults_private() {
        let c: Config = toml::from_str(include_str!("../shim.example.toml")).unwrap();
        assert_eq!(c.evidence, Disclosure::Public);
        let minimal = "id = \"x\"\nname = \"x\"\nrelays = []\nkey_file = \"k\"\n\
                       [[tool]]\nname = \"t\"\ncommand = \"true\"\n";
        let c: Config = toml::from_str(minimal).unwrap();
        assert_eq!(c.evidence, Disclosure::Private);
        assert_eq!(c.journal, None);
    }
}
