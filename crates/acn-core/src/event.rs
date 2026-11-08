//! Minimal NIP-01 event with pure-Rust id + BIP-340 verification.
//!
//! Wire-compatible with rust-nostr / nostr-tools JSON (`tests/nostr_compat.rs`).

use k256::schnorr::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::jcs::sha256_hex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u16,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("id does not match event hash")]
    BadId,
    #[error("invalid public key")]
    BadPubkey,
    #[error("invalid signature encoding")]
    BadSigEncoding,
    #[error("signature does not verify")]
    BadSignature,
}

/// NIP-01 event id: sha256 of `[0, pubkey, created_at, kind, tags, content]` (serde_json, as rust-nostr).
pub fn compute_id(
    pubkey: &str,
    created_at: u64,
    kind: u16,
    tags: &[Vec<String>],
    content: &str,
) -> String {
    let ser = serde_json::to_string(&(0u8, pubkey, created_at, kind, tags, content))
        .expect("serializing plain data cannot fail");
    sha256_hex(ser.as_bytes())
}

impl Event {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("serializing plain data cannot fail")
    }

    /// V-1: id and signature.
    pub fn verify(&self) -> Result<(), VerifyError> {
        let id = compute_id(
            &self.pubkey,
            self.created_at,
            self.kind,
            &self.tags,
            &self.content,
        );
        if id != self.id {
            return Err(VerifyError::BadId);
        }
        let pk = hex::decode(&self.pubkey).map_err(|_| VerifyError::BadPubkey)?;
        let vk = VerifyingKey::from_bytes(&pk).map_err(|_| VerifyError::BadPubkey)?;
        let sig = hex::decode(&self.sig).map_err(|_| VerifyError::BadSigEncoding)?;
        let sig = Signature::try_from(sig.as_slice()).map_err(|_| VerifyError::BadSigEncoding)?;
        let msg = hex::decode(&self.id).map_err(|_| VerifyError::BadId)?;
        vk.verify_raw(&msg, &sig)
            .map_err(|_| VerifyError::BadSignature)
    }

    /// All tags named `name`.
    pub fn tags_named(&self, name: &str) -> impl Iterator<Item = &Vec<String>> + '_ {
        let name = name.to_owned();
        self.tags
            .iter()
            .filter(move |t| t.first().map(String::as_str) == Some(name.as_str()))
    }

    /// Value (index 1) of the only tag named `name`. `Err(n)` if the cardinality is not 1.
    pub fn single(&self, name: &str) -> Result<&str, usize> {
        let vals: Vec<&str> = self
            .tags_named(name)
            .filter_map(|t| t.get(1).map(String::as_str))
            .collect();
        match vals.as_slice() {
            [v] => Ok(v),
            other => Err(other.len()),
        }
    }

    /// Optional single tag: `Ok(None)` if absent, `Err` if repeated.
    pub fn optional(&self, name: &str) -> Result<Option<&str>, usize> {
        match self.single(name) {
            Ok(v) => Ok(Some(v)),
            Err(0) => Ok(None),
            Err(n) => Err(n),
        }
    }

    /// `e` tags carrying the given marker (index 3), e.g. `prev`, `report`, `endorses`.
    pub fn e_marked(&self, marker: &str) -> Vec<&str> {
        self.tags_named("e")
            .filter(|t| t.get(3).map(String::as_str) == Some(marker))
            .filter_map(|t| t.get(1).map(String::as_str))
            .collect()
    }

    pub fn values(&self, name: &str) -> Vec<&str> {
        self.tags_named(name)
            .filter_map(|t| t.get(1).map(String::as_str))
            .collect()
    }
}
