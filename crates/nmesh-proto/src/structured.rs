//! Structured messages and actions: JSON payloads bound to a data contract.
//!
//! ```text
//! kind:    4930 (placeholder)
//! content: <JSON payload>
//! tags:    ["contract", "<contract-id>", "<schema-sha256?>"]
//!          ["alt", "<human readable fallback>"]
//!          ["e", "<event-id>"]                   (optional, reply)
//! ```
//!
//! In DMs and MLS groups this event is never signed or published directly: it is the inner
//! *rumor* (NIP-59 / Marmot), so the payload stays end-to-end encrypted.

use nostr::{EventBuilder, EventId, Kind, Tag, TagKind, Tags};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kinds::{self, kind};
use crate::ProtoError;

pub const TAG_CONTRACT: &str = "contract";

/// Reference from a message to the contract its payload conforms to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContractRef {
    /// e.g. `nmesh.card/v1`. Major version is part of the id; minor changes must be additive.
    pub id: String,
    /// Optional SHA-256 (hex) of the canonical JSON Schema, for pinning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_hash: Option<String>,
}

impl ContractRef {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            schema_hash: None,
        }
    }

    pub fn pinned(id: impl Into<String>, schema_hash: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            schema_hash: Some(schema_hash.into()),
        }
    }

    fn to_tag(&self) -> Tag {
        let mut values = vec![self.id.clone()];
        if let Some(h) = &self.schema_hash {
            values.push(h.clone());
        }
        Tag::custom(TagKind::custom(TAG_CONTRACT), values)
    }

    fn from_tags(tags: &Tags) -> Result<Self, ProtoError> {
        let tag = tags
            .find(TagKind::custom(TAG_CONTRACT))
            .ok_or(ProtoError::MissingTag(TAG_CONTRACT))?;
        let s = tag.as_slice();
        let id = s
            .get(1)
            .filter(|v| !v.is_empty())
            .ok_or(ProtoError::InvalidTag {
                tag: TAG_CONTRACT,
                reason: "empty contract id".into(),
            })?
            .clone();
        let schema_hash = s.get(2).filter(|v| !v.is_empty()).cloned();
        Ok(Self { id, schema_hash })
    }
}

/// The required NIP-31 `alt` tag: a missing or empty one is malformed, not an empty fallback.
fn alt_from_tags(tags: &Tags) -> Result<String, ProtoError> {
    tags.find(TagKind::Alt)
        .and_then(|t| t.content())
        .filter(|a| !a.trim().is_empty())
        .map(str::to_string)
        .ok_or(ProtoError::MissingTag("alt"))
}

fn first_e_tag(tags: &Tags) -> Option<EventId> {
    tags.event_ids().next().copied()
}

fn expect_kind(found: Kind, expected: u16) -> Result<(), ProtoError> {
    if found.as_u16() != expected {
        return Err(ProtoError::UnexpectedKind {
            expected,
            found: found.as_u16(),
        });
    }
    Ok(())
}

/// A JSON payload bound to a data contract, with a mandatory human-readable fallback.
#[derive(Debug, Clone, PartialEq)]
pub struct StructuredMessage {
    pub contract: ContractRef,
    pub payload: Value,
    /// Plain-text fallback shown by clients that cannot render the contract. Always required.
    pub alt: String,
    pub reply_to: Option<EventId>,
}

impl StructuredMessage {
    pub fn new(contract: ContractRef, payload: Value, alt: impl Into<String>) -> Self {
        Self {
            contract,
            payload,
            alt: alt.into(),
            reply_to: None,
        }
    }

    pub fn reply_to(mut self, id: EventId) -> Self {
        self.reply_to = Some(id);
        self
    }

    /// Build the (unsigned) event. Callers either gift-wrap it as a rumor or sign it.
    pub fn to_builder(&self) -> EventBuilder {
        let mut tags = vec![self.contract.to_tag(), Tag::alt(&self.alt)];
        if let Some(id) = self.reply_to {
            tags.push(Tag::event(id));
        }
        EventBuilder::new(kind(kinds::STRUCTURED_MESSAGE), self.payload.to_string()).tags(tags)
    }

    /// Parse from the parts of an `Event` or `UnsignedEvent` (rumor).
    pub fn from_parts(kind: Kind, tags: &Tags, content: &str) -> Result<Self, ProtoError> {
        expect_kind(kind, kinds::STRUCTURED_MESSAGE)?;
        Ok(Self {
            contract: ContractRef::from_tags(tags)?,
            payload: serde_json::from_str(content)?,
            alt: alt_from_tags(tags)?,
            reply_to: first_e_tag(tags),
        })
    }
}

/// An interaction with a previous structured message (button press, form submit, AR placement...).
///
/// Renderers never sign anything: they ask the host to emit an `Action`, and the host signs it
/// (after user confirmation where appropriate).
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    /// The structured message being acted on.
    pub target: EventId,
    /// Contract of the action payload (e.g. `nmesh.form.submit/v1`).
    pub contract: ContractRef,
    pub payload: Value,
    pub alt: String,
}

impl Action {
    pub fn to_builder(&self) -> EventBuilder {
        let tags = vec![
            self.contract.to_tag(),
            Tag::alt(&self.alt),
            Tag::event(self.target),
        ];
        EventBuilder::new(kind(kinds::ACTION), self.payload.to_string()).tags(tags)
    }

    pub fn from_parts(kind: Kind, tags: &Tags, content: &str) -> Result<Self, ProtoError> {
        expect_kind(kind, kinds::ACTION)?;
        Ok(Self {
            target: first_e_tag(tags).ok_or(ProtoError::MissingTag("e"))?,
            contract: ContractRef::from_tags(tags)?,
            payload: serde_json::from_str(content)?,
            alt: alt_from_tags(tags)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;
    use serde_json::json;

    #[test]
    fn structured_roundtrip() {
        let keys = Keys::generate();
        let msg = StructuredMessage::new(
            ContractRef::pinned("nmesh.card/v1", "abc123"),
            json!({"title": "Hello", "body": "World"}),
            "Hello: World",
        );
        let event = msg.to_builder().sign_with_keys(&keys).unwrap();
        let parsed =
            StructuredMessage::from_parts(event.kind, &event.tags, &event.content).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn alt_is_required() {
        let keys = Keys::generate();
        let msg = StructuredMessage::new(
            ContractRef::new("nmesh.card/v1"),
            json!({"title": "Hello"}),
            "Hello",
        );
        let event = msg.to_builder().sign_with_keys(&keys).unwrap();
        for tags in [
            Tags::from_list(
                event
                    .tags
                    .iter()
                    .filter(|t| t.kind() != TagKind::Alt)
                    .cloned()
                    .collect(),
            ),
            Tags::from_list(
                event
                    .tags
                    .iter()
                    .map(|t| {
                        if t.kind() == TagKind::Alt {
                            Tag::alt(" ")
                        } else {
                            t.clone()
                        }
                    })
                    .collect(),
            ),
        ] {
            assert!(matches!(
                StructuredMessage::from_parts(event.kind, &tags, &event.content),
                Err(ProtoError::MissingTag("alt"))
            ));
        }
    }

    #[test]
    fn action_requires_target() {
        let keys = Keys::generate();
        let event = EventBuilder::new(kind(kinds::ACTION), "{}")
            .tags([ContractRef::new("x/v1").to_tag()])
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            Action::from_parts(event.kind, &event.tags, &event.content),
            Err(ProtoError::MissingTag("e"))
        ));
    }

    #[test]
    fn rejects_wrong_kind() {
        let keys = Keys::generate();
        let event = EventBuilder::text_note("hi").sign_with_keys(&keys).unwrap();
        assert!(StructuredMessage::from_parts(event.kind, &event.tags, &event.content).is_err());
    }
}
