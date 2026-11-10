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
use std::time::Duration;

use acn_binding::Performer;
use acn_binding_nostr::{BlossomEvidence, MintWallet, SessionBinding};

use anyhow::Context;
use nmesh_contracts::ContractRegistry;
use nmesh_core::{
    Body, IdentityKind, Inbound, InboxOptions, PublicKey, Session, SessionConfig, ToBech32,
};
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
        let mut performer = Performer::new(
            sb.clone(),
            sb,
            Arc::new(shim::McpExecutor {
                hub: hub.clone(),
                registry: registry.clone(),
            }),
        );
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
            let ad = shim::service_advertisement(
                &bridge.hub,
                acn,
                &dh,
                &acn.mint_capabilities,
                bridge.performer.level(),
            );
            acn_binding_nostr::announce(&bridge.session, &bridge.config.agent.id, &ad).await?;
            match &acn.operator_key_file {
                Some(path) => {
                    let operator = nmesh_core::load_or_create_keys(path)?;
                    acn_binding_nostr::ensure_binding(
                        &bridge.session,
                        &operator,
                        acn.price_sat,
                        acn.binding_days,
                    )
                    .await?;
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
    pub async fn run(&self) -> anyhow::Result<()> {
        // Own messages are included so ACN state survives restarts (our earlier REPORTs are
        // replayed from relays); non-ACN own messages are ignored in `handle`.
        let mut inbox = self
            .session
            .inbox_with(InboxOptions {
                include_own: true,
                ..Default::default()
            })
            .await?;

        // Replay history before acting, so a REQUEST we already answered isn't run twice.
        let settle = Duration::from_millis(1500);
        while let Ok(Some(m)) = tokio::time::timeout(settle, inbox.recv()).await {
            if let Body::Act(e) = &m.body {
                self.performer.ingest((**e).clone());
            } else if m.sender != self.session.public_key() {
                self.handle(m).await;
            }
        }
        for c in self.performer.store.ids() {
            self.performer.drive(&c).await;
        }

        loop {
            tokio::select! {
                msg = inbox.recv() => match msg {
                    Some(m) => self.handle(m).await,
                    None => break,
                },
                _ = tokio::signal::ctrl_c() => break,
            }
        }
        self.session.shutdown().await;
        Ok(())
    }

    async fn handle(&self, msg: Inbound) {
        if msg.sender == self.session.public_key() {
            if let Body::Act(e) = &msg.body {
                self.performer.ingest((**e).clone());
            }
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
            Body::Act(e) if self.config.acn.enabled => {
                let p = self.performer.clone();
                let e = (**e).clone();
                tokio::spawn(async move {
                    p.on_event(e).await;
                });
                Ok(())
            }
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
