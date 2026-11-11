//! Bridge MCP servers onto Nostr.
//!
//! An agent is a **peer**, not a plugin: it has its own keypair, publishes a kind 0 profile with
//! `"bot": true`, an nmesh agent manifest (who runs it, what it does with your data) and one
//! NIP-CD slash command definition per MCP tool. Humans talk to it over NIP-17 DMs:
//!
//! ```text
//! human ── /tool args ──▶ agent (DM, gift-wrapped)
//!                          ├─ bind args per NIP-CD (or JSON object)
//!                          ├─ tools/call on the MCP server
//!                          └─ reply: nmesh structured message (if structuredContent validates
//!                                    against the tool's contract) else plain text
//! ```
//!
//! With `[acn]` enabled the bridge is also **`acn-mcp-shim`** (docs/Core.md §11): a performer on
//! the Binding API (`acn-binding`). It publishes a `K_SVC` service advertisement, an operator
//! binding and its Blossom server list, answers T0 and (with a mint) T1 `REQUEST`s for its tools
//! with signed `REPORT`s carrying artifacts and a derivation record, and redeems paid calls
//! (see [`shim`]).
//!
//! Interop note: ContextVM (MCP-over-Nostr transport) is complementary. A future
//! `transport = "contextvm"` lets the bridge reach MCP servers that are themselves on Nostr.

pub mod config;
pub mod mcp;
mod reply;
pub mod shim;

use std::collections::HashSet;
use std::sync::Arc;

use acn_binding::{Journal, Performer};
use acn_binding_nostr::{BlossomEvidence, MintWallet, RunOptions, SessionBinding};

use anyhow::Context;
use nmesh_contracts::ContractRegistry;
use nmesh_core::{Body, IdentityKind, Inbound, PublicKey, Session, SessionConfig, ToBech32};
use nmesh_proto::command::{bind_args, parse_invocation, SlashCommandDefinition};
use nmesh_proto::manifest::{AgentManifest, DataHandling};

use crate::config::AgentConfig;
use crate::mcp::McpHub;

enum Args<'a> {
    Text(&'a str),
    Named(serde_json::Map<String, serde_json::Value>),
}

pub struct Bridge {
    config: AgentConfig,
    session: Session,
    hub: Arc<McpHub>,
    registry: Arc<ContractRegistry>,
    allow: Option<HashSet<PublicKey>>,
    performer: Arc<Performer>,
}

impl Bridge {
    pub async fn start(config: AgentConfig) -> anyhow::Result<Self> {
        let keys = nmesh_core::load_or_create_keys(&config.agent.key_file)
            .with_context(|| format!("loading agent key {}", config.agent.key_file.display()))?;

        let mut registry = ContractRegistry::with_builtins();
        let hub = McpHub::connect(&config.servers, &mut registry).await;

        let session = Session::connect(
            keys,
            SessionConfig {
                relays: config.relays.clone(),
                kind: IdentityKind::Agent,
                display_name: Some(config.agent.name.clone()),
                about: Some(config.agent.about.clone()),
                announce: true,
            },
        )
        .await?;

        let allow = if config.agent.allow.is_empty() {
            None
        } else {
            Some(
                config
                    .agent
                    .allow
                    .iter()
                    .map(|s| PublicKey::parse(s))
                    .collect::<Result<_, _>>()
                    .context("invalid pubkey in agent.allow")?,
            )
        };

        let hub = Arc::new(hub);
        let registry = Arc::new(registry);
        let acn = &config.acn;
        let sb = Arc::new(SessionBinding(session.clone()));
        let journal_path = acn
            .journal
            .clone()
            .unwrap_or_else(|| acn_binding_nostr::default_journal_path(&config.agent.key_file));
        let journal = Journal::open(journal_path).map_err(anyhow::Error::msg)?;
        // The same admission for history and live acts: ACN on, and the requester allowed.
        let admit_allow = allow.clone();
        let enabled = acn.enabled;
        let mut performer = Performer::new(
            sb.clone(),
            sb,
            Arc::new(shim::McpExecutor {
                hub: hub.clone(),
                registry: registry.clone(),
            }),
        )
        .with_journal(journal)
        .with_negotiation(acn.negotiate)
        .with_disclosure(acn.evidence)
        .with_admission(Arc::new(move |requester: &str| {
            enabled
                && admit_allow.as_ref().is_none_or(|a: &HashSet<PublicKey>| {
                    PublicKey::parse(requester).is_ok_and(|pk| a.contains(&pk))
                })
        }));
        if !acn.blossom.is_empty() {
            performer = performer.with_evidence(Arc::new(BlossomEvidence(
                nmesh_blossom::BlossomClient::new(acn.blossom.clone(), session.keys().clone()),
            )));
        }
        if acn.price_sat > 0 {
            let mint = acn
                .mint
                .as_deref()
                .context("acn.price_sat > 0 needs acn.mint, the mint the agent is paid at")?;
            let wallet = nmesh_wallet::CashuWallet::open(
                mint,
                &session.keys().secret_key().secret_bytes(),
                acn.wallet_dir.as_deref(),
            )
            .await?;
            performer = performer.with_wallet(Arc::new(MintWallet {
                wallet,
                capabilities: acn.mint_capabilities.clone(),
            }));
        }
        let performer = performer.with_min_price(acn.price_sat);
        let bridge = Self {
            config,
            session,
            hub,
            registry,
            allow,
            performer: Arc::new(performer),
        };
        bridge.publish_capabilities().await?;
        if bridge.config.acn.enabled {
            let acn = &bridge.config.acn;
            let dh = serde_json::to_value(&bridge.manifest().data_handling)?;
            let level = shim::claimed_level(acn).and(bridge.performer.level());
            let ad =
                shim::service_advertisement(&bridge.hub, acn, &dh, &acn.mint_capabilities, level);
            acn_binding_nostr::announce(&bridge.session, &bridge.config.agent.id, &ad).await?;
            match &acn.operator_key_file {
                Some(path) => {
                    let operator = nmesh_core::load_or_create_keys(path)?;
                    let binding = acn_binding_nostr::ensure_binding(
                        &bridge.session,
                        &operator,
                        acn.price_sat,
                        acn.binding_days,
                    )
                    .await?;
                    bridge.performer.add_context(binding);
                }
                None => {
                    tracing::warn!("acn: no operator_key_file; agent will be flagged unaccountable")
                }
            }
        }
        Ok(bridge)
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn manifest(&self) -> AgentManifest {
        let a = &self.config.agent;
        let mut forwards_to: Vec<String> = self
            .config
            .servers
            .iter()
            .map(|s| format!("mcp:{}", s.name))
            .collect();
        forwards_to.extend(a.forwards_to.iter().cloned());
        AgentManifest {
            id: a.id.clone(),
            name: a.name.clone(),
            description: a.about.clone(),
            hosting: a.hosting,
            operator: a.operator.clone(),
            data_handling: DataHandling {
                sees_plaintext: true,
                retention: a.retention.clone(),
                forwards_to,
            },
            tools: self.hub.bindings.iter().map(|b| b.summary()).collect(),
        }
    }

    async fn publish_capabilities(&self) -> anyhow::Result<()> {
        // Pinned: republished whenever a relay reconnects (an in-memory relay forgets them).
        self.session
            .publish_pinned_builder(self.manifest().to_builder()?)
            .await?;

        for b in &self.hub.bindings {
            let def = SlashCommandDefinition {
                id: format!("{}:{}", self.config.agent.id, b.command),
                command: b.command.clone(),
                title: b.tool.title.clone().unwrap_or_else(|| b.command.clone()),
                description: b
                    .tool
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .to_string(),
                args: b.args.clone(),
                scopes: vec![format!(
                    "kind:{}",
                    nmesh_proto::kinds::PRIVATE_DIRECT_MESSAGE
                )],
            };
            self.session
                .publish_pinned_builder(def.to_builder())
                .await?;
        }
        tracing::info!(
            npub = %self.session.public_key().to_bech32().unwrap_or_default(),
            tools = self.hub.bindings.len(),
            "agent online"
        );
        Ok(())
    }

    /// Serve requests until the inbox closes or ctrl-c.
    ///
    /// ACN acts go through [`acn_binding_nostr::run_with`]: history is replayed into the
    /// performer and every admitted commitment driven, with the journal preventing repeated
    /// work. Plain messages (slash commands, actions) reach [`Self::handle`] only if they are
    /// not history and haven't been handled before: a restart never re-runs a tool.
    pub async fn run(&self) -> anyhow::Result<()> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let opts = RunOptions {
            allow: self.allow.clone(),
            acn: self.config.acn.enabled,
            ..Default::default()
        };
        let serve =
            acn_binding_nostr::run_with(&self.session, self.performer.clone(), opts, move |m| {
                let _ = tx.send(m);
            });
        tokio::pin!(serve);
        loop {
            tokio::select! {
                r = &mut serve => {
                    if let Err(e) = r {
                        tracing::error!(error = %e, "inbox failed");
                    }
                    break;
                }
                Some(m) = rx.recv() => self.handle(m).await,
                _ = tokio::signal::ctrl_c() => break,
            }
        }
        self.session.shutdown().await;
        Ok(())
    }

    /// A plain (non-ACN) message from an allowed sender.
    async fn handle(&self, msg: Inbound) {
        if msg.sender == self.session.public_key() {
            return;
        }
        if let Some(allow) = &self.allow {
            if !allow.contains(&msg.sender) {
                tracing::debug!(sender = %msg.sender, "ignoring message from non-allowed sender");
                return;
            }
        }
        let result = match &msg.body {
            Body::Text(t) => self.handle_text(&msg, t).await,
            Body::Action(a) => self.handle_action(&msg, a).await,
            _ => Ok(()),
        };
        if let Err(e) = result {
            tracing::error!(error = %e, "failed to handle message");
        }
    }

    async fn handle_text(&self, msg: &Inbound, text: &str) -> anyhow::Result<()> {
        let me = self.session.public_key();
        let Some(inv) = parse_invocation(text).filter(|i| i.is_for(&me)) else {
            if text.trim() == "help" || text.trim() == "/help" {
                return self.send_help(msg.sender).await;
            }
            return Ok(());
        };
        if inv.command == "help" {
            return self.send_help(msg.sender).await;
        }
        self.invoke(msg, &inv.command, Args::Text(&inv.rest)).await
    }

    /// Actions (button presses, form submits) whose `action_id` is a slash command are routed
    /// to that tool. Form `values` become named arguments; otherwise the text after the command
    /// is bound positionally. Other actions are acknowledged.
    async fn handle_action(
        &self,
        msg: &Inbound,
        action: &nmesh_proto::Action,
    ) -> anyhow::Result<()> {
        let action_id = action.payload.get("action_id").and_then(|v| v.as_str());
        let values = action.payload.get("values").and_then(|v| v.as_object());
        let me = self.session.public_key();
        match action_id
            .and_then(parse_invocation)
            .filter(|i| i.is_for(&me))
        {
            Some(inv) => {
                let args = match values {
                    Some(v) => Args::Named(v.clone()),
                    None => Args::Text(&inv.rest),
                };
                self.invoke(msg, &inv.command, args).await
            }
            None => {
                self.session
                    .send_text(msg.sender, &format!("Got it: {}", action.alt))
                    .await?;
                Ok(())
            }
        }
    }

    async fn invoke(&self, msg: &Inbound, command: &str, args: Args<'_>) -> anyhow::Result<()> {
        let Some(binding) = self.hub.find(command) else {
            self.session
                .send_text(
                    msg.sender,
                    &format!("Unknown command /{command}. Try /help."),
                )
                .await?;
            return Ok(());
        };
        let acn = &self.config.acn;
        if acn.enabled && acn.price_sat > 0 {
            // The tool is priced in the ACN offer: the direct path would run it for free.
            self.session
                .send_text(
                    msg.sender,
                    &format!(
                        "/{} costs {} sat: request it as an ACN commitment (turn on \"Request commands as commitments\").",
                        binding.command, acn.price_sat
                    ),
                )
                .await?;
            return Ok(());
        }
        let args = match args {
            Args::Named(m) => m,
            Args::Text(rest) => match bind_args(&binding.args, rest) {
                Ok(a) => a,
                Err(e) => {
                    self.session
                        .send_text(msg.sender, &format!("/{}: {e}", binding.command))
                        .await?;
                    return Ok(());
                }
            },
        };

        tracing::info!(sender = %msg.sender, command = %binding.command, "calling tool");
        match self.hub.call(binding, args).await {
            Ok(result) => {
                let out = reply::from_tool_result(&self.registry, binding, &result, msg.id);
                match out {
                    reply::Reply::Structured(s) => {
                        self.session.send_structured(msg.sender, &s).await?;
                    }
                    reply::Reply::Text(t) => {
                        self.session.send_text(msg.sender, &t).await?;
                    }
                }
            }
            Err(e) => {
                self.session
                    .send_text(msg.sender, &format!("/{} failed: {e}", binding.command))
                    .await?;
            }
        }
        Ok(())
    }

    async fn send_help(&self, to: PublicKey) -> anyhow::Result<()> {
        let mut lines = vec![format!(
            "{} — operated by {} ({:?}). I can read what you send me.",
            self.config.agent.name, self.config.agent.operator, self.config.agent.hosting
        )];
        for b in &self.hub.bindings {
            let args: Vec<String> = b
                .args
                .iter()
                .map(|a| {
                    if a.required {
                        format!("<{}>", a.name)
                    } else {
                        format!("[{}]", a.name)
                    }
                })
                .collect();
            lines.push(format!(
                "/{} {} — {}",
                b.command,
                args.join(" "),
                b.tool.description.as_deref().unwrap_or_default()
            ));
        }
        self.session.send_text(to, &lines.join("\n")).await?;
        Ok(())
    }
}
