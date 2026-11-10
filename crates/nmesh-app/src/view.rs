//! Serializable view models handed to UIs (Tauri webview today, Kotlin via FFI later).
//!
//! Field names are snake_case; `apps/desktop/src/lib/types.ts` mirrors these types.

use nmesh_contracts::{resolve_renderer, ContractRegistry, Resolution, TrustPolicy};
use nmesh_core::{Body, Inbound, PeerInfo, PublicKey, ToBech32};
use nmesh_proto::command::SlashCommandDefinition;
use nmesh_proto::manifest::{AgentManifest, RendererManifest};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Identity {
    pub pubkey: String,
    pub npub: String,
    pub name: Option<String>,
    pub relays: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArgView {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub required: bool,
    pub label: String,
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandView {
    pub command: String,
    pub title: String,
    pub description: String,
    pub args: Vec<ArgView>,
}

impl From<&SlashCommandDefinition> for CommandView {
    fn from(c: &SlashCommandDefinition) -> Self {
        Self {
            command: c.command.clone(),
            title: c.title.clone(),
            description: c.description.clone(),
            args: c
                .args
                .iter()
                .map(|a| ArgView {
                    name: a.name.clone(),
                    ty: a.ty.as_str().to_string(),
                    required: a.required,
                    label: a.label.clone(),
                    choices: a.choices.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ContactView {
    pub pubkey: String,
    pub npub: String,
    pub name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    pub is_agent: bool,
    /// How sure we are of `is_agent` (ADR 0004: disclosure before first contact).
    pub discovery: Discovery,
    /// Agent manifest including hosting + data handling. UIs MUST show it before first contact.
    pub agent: Option<AgentManifest>,
    pub commands: Vec<CommandView>,
    /// User has acknowledged the agent's data handling, or the contact is a known human. False
    /// while discovery is pending, and for unresolved identities until the user accepts.
    pub consented: bool,
    pub last_message: Option<String>,
    pub last_at: Option<u64>,
    pub unread: u32,
    /// Present if the peer offers ACN commitments (`K_SVC`).
    pub acn: Option<AcnPeerView>,
}

/// Whether a contact's kind (human or agent) is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Discovery {
    /// Not fetched yet: nothing may be sent until it is.
    Pending,
    /// A profile was found (or the key is a known agent); `is_agent` is reliable.
    Resolved,
    /// No profile found: possibly an agent that published nowhere we look. Treated like an
    /// agent with no disclosure, so the user must accept before anything is sent.
    Unresolved,
}

#[derive(Debug, Clone, Serialize)]
pub struct AcnPeerView {
    pub profiles: Vec<String>,
    pub min_price_sat: u64,
    pub deadline_secs: u64,
    pub review_window_secs: u64,
    pub binding: crate::commitments::BindingView,
    /// Tools offered as commitments, with their terms template summary (if any).
    pub tools: Vec<AcnToolView>,
    /// The agent says it is a simulation (demo cast).
    pub simulated: bool,
    /// The Binding API conformance level the agent claims (`K_SVC` `binding_level`, Core §4).
    /// A claim, shown as such: only the level's suite can check it.
    pub binding_level: Option<String>,
    /// Where its artifacts and derivation records are (Blossom servers, Core §9).
    pub blossom: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AcnToolView {
    pub name: String,
    pub title: String,
    pub description: String,
    pub profile: String,
    pub shows: Vec<String>,
    pub price_sat: u64,
}

impl AcnPeerView {
    fn from_svc(svc: &Value, binding: &acn_core::BindingStatus) -> Self {
        let defaults = &svc["terms_defaults"];
        Self {
            profiles: svc["profiles"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            min_price_sat: svc["price"]["min"].as_u64().unwrap_or(0),
            deadline_secs: defaults["deadline_secs"].as_u64().unwrap_or(600),
            review_window_secs: defaults["review_window_secs"].as_u64().unwrap_or(86_400),
            binding: binding.into(),
            simulated: svc["simulated"].as_bool().unwrap_or(false),
            binding_level: svc["binding_level"]
                .as_str()
                .filter(|l| matches!(*l, "L0" | "L1" | "L2" | "L3"))
                .map(str::to_owned),
            blossom: svc["blossom"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect(),
            tools: svc["tools"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|t| {
                    let s = |k: &str| t[k].as_str().unwrap_or_default().to_string();
                    let tpl = &t["terms_template"];
                    AcnToolView {
                        name: s("name"),
                        title: if s("title").is_empty() {
                            s("name")
                        } else {
                            s("title")
                        },
                        description: s("description"),
                        profile: tpl["profile"].as_str().unwrap_or("T0").to_string(),
                        shows: t["shows"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        price_sat: tpl["terms"]["price"]["amount"].as_u64().unwrap_or(0),
                    }
                })
                .collect(),
        }
    }
}

impl ContactView {
    pub fn new(pk: PublicKey) -> Self {
        Self {
            pubkey: pk.to_hex(),
            npub: pk.to_bech32().unwrap_or_default(),
            name: None,
            about: None,
            picture: None,
            is_agent: false,
            discovery: Discovery::Pending,
            agent: None,
            commands: vec![],
            consented: false,
            last_message: None,
            last_at: None,
            unread: 0,
            acn: None,
        }
    }

    /// Apply a discovery result. An established agent stays an agent: a later fetch that misses
    /// its advertisements is not evidence of a human. `accepted` is the user's consent record.
    pub fn apply_peer_info(&mut self, info: PeerInfo, accepted: bool) {
        self.is_agent = self.is_agent || info.is_agent();
        self.discovery = if self.is_agent || info.found_profile {
            Discovery::Resolved
        } else {
            Discovery::Unresolved
        };
        self.name = info
            .name
            .or_else(|| info.agent.as_ref().map(|a| a.name.clone()));
        self.about = info.about;
        self.picture = info.picture;
        self.commands = info.commands.iter().map(CommandView::from).collect();
        self.acn = info
            .acn_svc
            .as_ref()
            .map(|svc| AcnPeerView::from_svc(svc, &info.acn_binding));
        if info.agent.is_some() {
            self.agent = info.agent;
        }
        self.consented = accepted || (self.discovery == Discovery::Resolved && !self.is_agent);
    }

    /// The user must accept a disclosure before anything is sent.
    pub fn needs_consent(&self) -> bool {
        !self.consented && (self.is_agent || self.discovery == Discovery::Unresolved)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "tier", rename_all = "snake_case")]
pub enum RendererView {
    /// A renderer the host ships (`id` names the component, e.g. `nmesh.card`).
    Installed {
        id: String,
    },
    /// Third-party renderer accepted by the trust policy (Phase 4; not rendered yet).
    Trusted {
        id: String,
        publisher: Option<String>,
    },
    Generic,
    Alt,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BodyView {
    Text {
        text: String,
    },
    Structured {
        contract: String,
        schema_hash: Option<String>,
        payload: Value,
        alt: String,
        valid: bool,
        error: Option<String>,
        renderer: RendererView,
        reply_to: Option<String>,
    },
    Action {
        target: String,
        contract: String,
        payload: Value,
        alt: String,
    },
    Unknown {
        event_kind: u16,
        alt: Option<String>,
    },
    /// Placeholder for an ACN commitment in the thread; the UI renders the live
    /// `CommitmentView` with this id (docs/Core.md §12 thread view).
    Commitment {
        commitment: String,
    },
}

impl BodyView {
    pub fn summary(&self) -> String {
        match self {
            BodyView::Text { text } => text.clone(),
            BodyView::Structured { alt, .. } => alt.clone(),
            BodyView::Action { alt, .. } => format!("↳ {alt}"),
            BodyView::Unknown { alt, .. } => alt.clone().unwrap_or_else(|| "(unsupported)".into()),
            BodyView::Commitment { .. } => "Commitment".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageView {
    pub id: String,
    pub conversation: String,
    pub sender: String,
    pub mine: bool,
    pub created_at: u64,
    pub body: BodyView,
}

/// Context needed to turn a raw body into a view (validation + renderer resolution).
pub struct RenderContext<'a> {
    pub registry: &'a ContractRegistry,
    pub installed: &'a [RendererManifest],
    pub trust: &'a TrustPolicy,
    pub target: &'a str,
}

impl RenderContext<'_> {
    pub fn body(&self, body: &Body) -> BodyView {
        match body {
            Body::Text(t) => BodyView::Text { text: t.clone() },
            Body::Structured(m) => {
                let (valid, error) = match self.registry.validate_message(m) {
                    Ok(_) => (true, None),
                    Err(e) => (false, Some(e.to_string())),
                };
                let renderer = match resolve_renderer(
                    &m.contract.id,
                    self.target,
                    valid,
                    self.installed,
                    &[],
                    self.trust,
                ) {
                    Resolution::Installed(r) => RendererView::Installed { id: r.id.clone() },
                    Resolution::Trusted(r, _) => RendererView::Trusted {
                        id: r.id.clone(),
                        publisher: None,
                    },
                    Resolution::Generic => RendererView::Generic,
                    Resolution::AltText => RendererView::Alt,
                };
                BodyView::Structured {
                    contract: m.contract.id.clone(),
                    schema_hash: m.contract.schema_hash.clone(),
                    payload: m.payload.clone(),
                    alt: m.alt.clone(),
                    valid,
                    error,
                    renderer,
                    reply_to: m.reply_to.map(|id| id.to_hex()),
                }
            }
            Body::Action(a) => BodyView::Action {
                target: a.target.to_hex(),
                contract: a.contract.id.clone(),
                payload: a.payload.clone(),
                alt: a.alt.clone(),
            },
            Body::Unknown { kind, alt } => BodyView::Unknown {
                event_kind: *kind,
                alt: alt.clone(),
            },
            Body::Act(e) => BodyView::Commitment {
                commitment: crate::commitments::commitment_of(e),
            },
        }
    }

    pub fn message(&self, me: &PublicKey, m: &Inbound) -> MessageView {
        MessageView {
            id: m.id.to_hex(),
            conversation: m.counterparty(me).to_hex(),
            sender: m.sender.to_hex(),
            mine: &m.sender == me,
            created_at: m.created_at.as_secs(),
            body: self.body(&m.body),
        }
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    fn pk() -> PublicKey {
        nmesh_core::Keys::generate().public_key()
    }

    #[test]
    fn an_unknown_contact_is_not_consented() {
        let c = ContactView::new(pk());
        assert_eq!(c.discovery, Discovery::Pending);
        assert!(!c.consented);
    }

    #[test]
    fn no_profile_is_unresolved_and_needs_consent() {
        let mut c = ContactView::new(pk());
        c.apply_peer_info(PeerInfo::default(), false);
        assert_eq!(c.discovery, Discovery::Unresolved);
        assert!(c.needs_consent());
        c.apply_peer_info(PeerInfo::default(), true);
        assert!(!c.needs_consent(), "the user accepted");
    }

    #[test]
    fn a_human_profile_needs_no_consent() {
        let mut c = ContactView::new(pk());
        c.apply_peer_info(
            PeerInfo {
                found_profile: true,
                ..PeerInfo::default()
            },
            false,
        );
        assert_eq!(c.discovery, Discovery::Resolved);
        assert!(!c.is_agent && c.consented && !c.needs_consent());
    }

    #[test]
    fn an_incomplete_fetch_never_downgrades_an_agent() {
        let mut c = ContactView::new(pk());
        c.apply_peer_info(
            PeerInfo {
                found_profile: true,
                is_bot: true,
                ..PeerInfo::default()
            },
            false,
        );
        assert!(c.is_agent && c.needs_consent());
        // A later fetch that finds a profile without the bot flag, or nothing at all.
        for info in [
            PeerInfo {
                found_profile: true,
                ..PeerInfo::default()
            },
            PeerInfo::default(),
        ] {
            c.apply_peer_info(info, false);
            assert!(c.is_agent, "still an agent");
            assert!(c.needs_consent(), "still gated");
        }
    }
}
