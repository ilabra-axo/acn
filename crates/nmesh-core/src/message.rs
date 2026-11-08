use nmesh_proto::{Action, StructuredMessage};
use nostr_sdk::prelude::*;

/// What a message carries.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Text(String),
    Structured(StructuredMessage),
    Action(Action),
    /// A fully signed ACN act carried in a `K_PRIVATE_ACT` rumor. Its signature is checked by
    /// the ACN validator, not here; it may be authored by someone other than the sender
    /// (selective disclosure).
    Act(Box<acn_core::Event>),
    /// A kind we don't understand; keep the alt text so it can still be shown.
    Unknown {
        kind: u16,
        alt: Option<String>,
    },
}

impl Body {
    /// Plain-text representation for logs, notifications and dumb clients.
    pub fn summary(&self) -> String {
        match self {
            Body::Text(t) => t.clone(),
            Body::Structured(m) => format!("[{}] {}", m.contract.id, m.alt),
            Body::Action(a) => format!("[action {}] {}", a.contract.id, a.alt),
            Body::Act(e) => format!("[acn {}]", acn_core::kinds::name(e.kind)),
            Body::Unknown { kind, alt } => {
                format!("[kind {kind}] {}", alt.as_deref().unwrap_or("(no alt)"))
            }
        }
    }
}

/// A decrypted inbound message.
#[derive(Debug, Clone)]
pub struct Inbound {
    /// Rumor id (stable across relays; used for replies and actions).
    pub id: EventId,
    /// Authenticated sender (from the NIP-59 seal).
    pub sender: PublicKey,
    pub created_at: Timestamp,
    /// `p` tags on the rumor (NIP-17 participants other than the sender).
    pub recipients: Vec<PublicKey>,
    pub body: Body,
}

impl Inbound {
    /// The other side of a 1:1 conversation from `me`'s point of view.
    pub fn counterparty(&self, me: &PublicKey) -> PublicKey {
        if &self.sender != me {
            return self.sender;
        }
        self.recipients
            .iter()
            .find(|p| *p != me)
            .copied()
            .unwrap_or(*me)
    }
}

impl Inbound {
    pub(crate) fn from_rumor(sender: PublicKey, mut rumor: UnsignedEvent) -> Self {
        let id = rumor.id();
        let k = rumor.kind.as_u16();
        let body = match k {
            nmesh_proto::kinds::PRIVATE_DIRECT_MESSAGE => Body::Text(rumor.content.clone()),
            nmesh_proto::kinds::STRUCTURED_MESSAGE => {
                match StructuredMessage::from_parts(rumor.kind, &rumor.tags, &rumor.content) {
                    Ok(m) => Body::Structured(m),
                    Err(_) => unknown(k, &rumor.tags),
                }
            }
            nmesh_proto::kinds::ACTION => {
                match Action::from_parts(rumor.kind, &rumor.tags, &rumor.content) {
                    Ok(a) => Body::Action(a),
                    Err(_) => unknown(k, &rumor.tags),
                }
            }
            nmesh_proto::kinds::ACN_PRIVATE_ACT => match acn_core::Event::from_json(&rumor.content)
            {
                Ok(e) => Body::Act(Box::new(e)),
                Err(_) => unknown(k, &rumor.tags),
            },
            _ => unknown(k, &rumor.tags),
        };
        Self {
            id,
            sender,
            created_at: rumor.created_at,
            recipients: rumor.tags.public_keys().copied().collect(),
            body,
        }
    }
}

fn unknown(kind: u16, tags: &Tags) -> Body {
    Body::Unknown {
        kind,
        alt: tags
            .find(TagKind::Alt)
            .and_then(|t| t.content())
            .map(str::to_owned),
    }
}
