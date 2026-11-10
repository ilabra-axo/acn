//! Looking up other participants: profile, agent manifest, slash commands.

use std::time::Duration;

use nmesh_proto::command::SlashCommandDefinition;
use nmesh_proto::kinds;
use nmesh_proto::manifest::AgentManifest;
use nostr_sdk::prelude::*;

use crate::{CoreError, Session};

const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// What we know about another participant.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    /// `"bot": true` in kind 0 (NIP-24).
    pub is_bot: bool,
    /// Present if the peer published an nmesh agent manifest.
    pub agent: Option<AgentManifest>,
    pub commands: Vec<SlashCommandDefinition>,
    /// ACN service advertisement (`K_SVC` content), if the peer offers ACN commitments.
    pub acn_svc: Option<serde_json::Value>,
    /// Operator binding status of this key (ACN V-8), evaluated now.
    pub acn_binding: acn_core::BindingStatus,
    /// The binding events themselves, so commitment folds can evaluate V-8 per act.
    pub acn_binding_events: Vec<acn_core::Event>,
    /// The relays returned a profile (kind 0). Without one, an empty result says nothing about
    /// whether the key is an agent: it may simply not have published, or not to our relays.
    pub found_profile: bool,
}

impl Default for PeerInfo {
    fn default() -> Self {
        Self {
            name: None,
            about: None,
            picture: None,
            is_bot: false,
            agent: None,
            commands: vec![],
            acn_svc: None,
            acn_binding: acn_core::BindingStatus::Unbound,
            acn_binding_events: vec![],
            found_profile: false,
        }
    }
}

impl PeerInfo {
    pub fn is_agent(&self) -> bool {
        self.is_bot || self.agent.is_some() || self.acn_svc.is_some()
    }
}

impl Session {
    /// Fetch profile, agent manifest and slash commands for `pk` from our relays.
    pub async fn peer_info(&self, pk: PublicKey) -> Result<PeerInfo, CoreError> {
        let filter = Filter::new().author(pk).kinds([
            Kind::Metadata,
            kinds::kind(kinds::AGENT_MANIFEST),
            kinds::kind(kinds::SLASH_COMMAND_DEFINITION),
        ]);
        let events = self.client().fetch_events(filter, FETCH_TIMEOUT).await?;

        // Keep only the newest event per (kind, d-tag): relays may return superseded versions.
        let mut newest: std::collections::HashMap<(u16, String), &Event> = Default::default();
        for ev in events.iter() {
            let key = (
                ev.kind.as_u16(),
                ev.tags.identifier().unwrap_or_default().to_string(),
            );
            let slot = newest.entry(key).or_insert(ev);
            if ev.created_at > slot.created_at {
                *slot = ev;
            }
        }

        let mut info = PeerInfo::default();
        let mut latest_md: Option<&Event> = None;
        for ev in newest.into_values() {
            match ev.kind.as_u16() {
                0 => latest_md = Some(ev),
                kinds::AGENT_MANIFEST => {
                    if let Ok(m) = AgentManifest::from_parts(ev.kind, &ev.tags, &ev.content) {
                        info.agent = Some(m);
                    }
                }
                kinds::SLASH_COMMAND_DEFINITION => {
                    if let Some(c) = SlashCommandDefinition::from_tags(&ev.tags) {
                        info.commands.push(c);
                    }
                }
                _ => {}
            }
        }
        info.found_profile = latest_md.is_some();
        if let Some(ev) = latest_md {
            if let Ok(md) = Metadata::from_json(&ev.content) {
                info.name = md.display_name.clone().or(md.name.clone());
                info.about = md.about.clone();
                info.picture = md.picture.clone();
                info.is_bot = md.custom.get("bot").and_then(|v| v.as_bool()) == Some(true);
            }
        }
        info.commands.sort_by(|a, b| a.command.cmp(&b.command));
        info.commands.dedup_by(|a, b| a.command == b.command);

        let (svc, binding_events) = self.acn_public(pk, FETCH_TIMEOUT).await?;
        info.acn_svc = svc;
        info.acn_binding = acn_core::binding_status(
            &binding_events,
            &pk.to_hex(),
            acn_core::kinds::K_REPORT,
            0,
            None,
            Timestamp::now().as_secs(),
        );
        info.acn_binding_events = binding_events;
        Ok(info)
    }
}
