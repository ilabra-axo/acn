//! Trust policy for third-party manifests (renderers, agents, contracts).
//!
//! **v1 (now):** explicit allow-list of publisher pubkeys, plus "self". Simple, predictable.
//!
//! **Goal (docs/adr/0005-trust.md):** web of trust, built from existing NIPs rather than new ones:
//! - NIP-51 lists (follow sets / curation sets) of trusted publishers;
//! - NIP-89-style recommendations (kind 31989) of renderers/agents by people you follow;
//! - NIP-85 Trusted Assertions (`30382:rank`, `30384:rank`) from providers the user picks in
//!   their kind 10040 list, for scores that need heavy computation.
//!
//! The `TrustDecision` enum already has room for those sources so callers don't need to change.

use std::collections::HashSet;

use nostr::PublicKey;

#[derive(Debug, Clone, PartialEq)]
pub enum TrustDecision {
    /// Published by the local user.
    Own,
    /// Publisher is on the explicit allow-list.
    AllowListed,
    /// Recommended by someone the user trusts (future: NIP-51 / NIP-89 graph).
    Recommended {
        by: Vec<PublicKey>,
    },
    /// Scored by a NIP-85 provider at or above the threshold (future).
    Scored {
        rank: u8,
        provider: PublicKey,
    },
    Untrusted,
}

impl TrustDecision {
    pub fn is_trusted(&self) -> bool {
        !matches!(self, TrustDecision::Untrusted)
    }
}

#[derive(Debug, Clone, Default)]
pub struct TrustPolicy {
    me: Option<PublicKey>,
    allow: HashSet<PublicKey>,
}

impl TrustPolicy {
    pub fn new(me: PublicKey) -> Self {
        Self {
            me: Some(me),
            allow: HashSet::new(),
        }
    }

    pub fn allow(&mut self, pk: PublicKey) -> &mut Self {
        self.allow.insert(pk);
        self
    }

    pub fn revoke(&mut self, pk: &PublicKey) {
        self.allow.remove(pk);
    }

    pub fn allowed(&self) -> impl Iterator<Item = &PublicKey> {
        self.allow.iter()
    }

    pub fn evaluate(&self, publisher: &PublicKey) -> TrustDecision {
        if self.me.as_ref() == Some(publisher) {
            TrustDecision::Own
        } else if self.allow.contains(publisher) {
            TrustDecision::AllowListed
        } else {
            TrustDecision::Untrusted
        }
    }
}
