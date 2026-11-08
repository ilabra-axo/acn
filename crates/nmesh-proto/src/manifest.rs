//! Registry manifests: contracts, renderers and agents.
//!
//! All three are **addressable events** (`d` tag = stable id) so publishers can update them in
//! place, and so other people can recommend / curate them by `a`-coordinate. This mirrors the
//! NIP-89 handler pattern (and the trust model in `docs/adr/0005-trust.md`).
//!
//! Body format: `content` is the JSON-serialised manifest; tags carry only what relays or
//! naive clients need (`d`, `alt`, and one `contract` tag per declared contract).

use nostr::{EventBuilder, Kind, Tag, TagKind, Tags};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::kinds::{self, kind};
use crate::structured::TAG_CONTRACT;
use crate::ProtoError;

/// A data contract: a versioned JSON Schema with metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContractDefinition {
    /// e.g. `nmesh.card/v1`.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// JSON Schema (draft 2020-12).
    pub schema: Value,
    /// Optional capability hints, e.g. `spatial`, `interactive`, `streaming`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

/// Renderer runtime tier. See `docs/spec/0003-renderers.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RendererRuntime {
    /// Tier 0: declarative UI tree interpreted by the host. Portable (web, Compose, AR panels).
    Declarative,
    /// Tier 1: sandboxed web view (iframe). Wire-compatible with MCP Apps (`ui://` resources).
    SandboxedWeb,
    /// Tier 2: native module or scene template (e.g. ARCore + glTF).
    Native,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RendererEntry {
    pub url: String,
    /// SHA-256 (hex) of the artifact. Hosts MUST verify before loading.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RendererManifest {
    pub id: String,
    pub title: String,
    /// Contract ids this renderer handles.
    pub contracts: Vec<String>,
    /// Targets: `web`, `android`, `ar`, `tui`, ...
    pub targets: Vec<String>,
    pub runtime: RendererRuntime,
    /// Absent for built-in renderers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<RendererEntry>,
    /// Action contracts this renderer may ask the host to emit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    /// Requested permissions (`network`, `location`, `camera`, ...). Default: none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
}

/// Who runs the agent bridge. All three are supported; the UI must show which one applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hosting {
    /// Run by the user themself (local or their own server).
    SelfHosted,
    /// Run by the nmesh deployment operator.
    Operator,
    /// Run by an independent third party.
    ThirdParty,
}

/// What happens to data an agent receives. An agent in an E2E room necessarily sees plaintext;
/// clients MUST surface this before inviting an agent (docs/adr/0004-agents.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataHandling {
    /// Always true for agents today; kept explicit so the UI never has to assume.
    pub sees_plaintext: bool,
    /// Free text, e.g. "none", "30 days", "until conversation is deleted".
    pub retention: String,
    /// Third parties the plaintext may be forwarded to (LLM providers, MCP servers...).
    #[serde(default)]
    pub forwards_to: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSummary {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Contract emitted by this tool's structured output, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub hosting: Hosting,
    /// Human-readable operator identity (npub, NIP-05, org name).
    pub operator: String,
    pub data_handling: DataHandling,
    #[serde(default)]
    pub tools: Vec<ToolSummary>,
}

impl AgentManifest {
    pub fn emitted_contracts(&self) -> impl Iterator<Item = &str> {
        self.tools
            .iter()
            .filter_map(|t| t.output_contract.as_deref())
    }
}

// --- Event encoding ---

fn addressable<T: Serialize>(
    k: u16,
    id: &str,
    alt: String,
    contracts: impl IntoIterator<Item = String>,
    body: &T,
) -> Result<EventBuilder, ProtoError> {
    let mut tags = vec![Tag::identifier(id), Tag::alt(alt)];
    tags.extend(
        contracts
            .into_iter()
            .map(|c| Tag::custom(TagKind::custom(TAG_CONTRACT), [c])),
    );
    Ok(EventBuilder::new(kind(k), serde_json::to_string(body)?).tags(tags))
}

fn decode<T: DeserializeOwned>(
    found: Kind,
    expected: u16,
    tags: &Tags,
    content: &str,
    id_of: impl Fn(&T) -> &str,
) -> Result<T, ProtoError> {
    if found.as_u16() != expected {
        return Err(ProtoError::UnexpectedKind {
            expected,
            found: found.as_u16(),
        });
    }
    let body: T = serde_json::from_str(content)?;
    let d = tags.identifier().ok_or(ProtoError::MissingTag("d"))?;
    if d != id_of(&body) {
        return Err(ProtoError::InvalidTag {
            tag: "d",
            reason: format!(
                "`d` tag `{d}` does not match manifest id `{}`",
                id_of(&body)
            ),
        });
    }
    Ok(body)
}

impl ContractDefinition {
    pub fn to_builder(&self) -> Result<EventBuilder, ProtoError> {
        addressable(
            kinds::CONTRACT_DEFINITION,
            &self.id,
            format!("nmesh data contract: {}", self.title),
            [],
            self,
        )
    }
    pub fn from_parts(kind: Kind, tags: &Tags, content: &str) -> Result<Self, ProtoError> {
        decode(
            kind,
            kinds::CONTRACT_DEFINITION,
            tags,
            content,
            |c: &Self| &c.id,
        )
    }
}

impl RendererManifest {
    pub fn to_builder(&self) -> Result<EventBuilder, ProtoError> {
        addressable(
            kinds::RENDERER_MANIFEST,
            &self.id,
            format!("nmesh renderer: {}", self.title),
            self.contracts.clone(),
            self,
        )
    }
    pub fn from_parts(kind: Kind, tags: &Tags, content: &str) -> Result<Self, ProtoError> {
        decode(kind, kinds::RENDERER_MANIFEST, tags, content, |r: &Self| {
            &r.id
        })
    }
}

impl AgentManifest {
    pub fn to_builder(&self) -> Result<EventBuilder, ProtoError> {
        addressable(
            kinds::AGENT_MANIFEST,
            &self.id,
            format!("nmesh agent: {}", self.name),
            self.emitted_contracts()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            self,
        )
    }
    pub fn from_parts(kind: Kind, tags: &Tags, content: &str) -> Result<Self, ProtoError> {
        decode(kind, kinds::AGENT_MANIFEST, tags, content, |a: &Self| &a.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[test]
    fn agent_manifest_roundtrip() {
        let m = AgentManifest {
            id: "echo".into(),
            name: "Echo".into(),
            description: "Echoes".into(),
            hosting: Hosting::SelfHosted,
            operator: "me".into(),
            data_handling: DataHandling {
                sees_plaintext: true,
                retention: "none".into(),
                forwards_to: vec![],
            },
            tools: vec![ToolSummary {
                name: "echo".into(),
                description: String::new(),
                output_contract: Some("nmesh.card/v1".into()),
            }],
        };
        let ev = m
            .to_builder()
            .unwrap()
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert_eq!(
            AgentManifest::from_parts(ev.kind, &ev.tags, &ev.content).unwrap(),
            m
        );
    }
}
