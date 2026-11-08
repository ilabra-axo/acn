use std::collections::HashMap;

use jsonschema::Validator;
use nmesh_proto::manifest::ContractDefinition;
use nmesh_proto::{ContractRef, StructuredMessage};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("unknown contract `{0}`")]
    Unknown(String),
    #[error("schema hash mismatch for `{id}`: message pins {pinned}, registry has {actual}")]
    HashMismatch {
        id: String,
        pinned: String,
        actual: String,
    },
    #[error("invalid schema for `{id}`: {reason}")]
    InvalidSchema { id: String, reason: String },
    #[error("payload does not satisfy `{id}`: {errors:?}")]
    Invalid { id: String, errors: Vec<String> },
}

/// SHA-256 over the canonical (key-sorted, compact) JSON encoding of a schema.
///
/// `serde_json::Value` without `preserve_order` stores object keys sorted, so `to_string` is
/// canonical enough for pinning within the nmesh ecosystem.
pub fn schema_hash(schema: &Value) -> String {
    hex::encode(Sha256::digest(schema.to_string().as_bytes()))
}

struct Entry {
    def: ContractDefinition,
    hash: String,
    validator: Validator,
}

/// Result of a successful validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validated {
    pub contract_id: String,
    pub schema_hash: String,
}

#[derive(Default)]
pub struct ContractRegistry {
    entries: HashMap<String, Entry>,
}

impl ContractRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registry pre-loaded with the built-in nmesh contracts.
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        for def in crate::builtin::contracts() {
            r.insert(def).expect("built-in contract schemas are valid");
        }
        r
    }

    pub fn insert(&mut self, def: ContractDefinition) -> Result<String, ContractError> {
        let validator =
            jsonschema::validator_for(&def.schema).map_err(|e| ContractError::InvalidSchema {
                id: def.id.clone(),
                reason: e.to_string(),
            })?;
        let hash = schema_hash(&def.schema);
        self.entries.insert(
            def.id.clone(),
            Entry {
                def,
                hash: hash.clone(),
                validator,
            },
        );
        Ok(hash)
    }

    pub fn get(&self, id: &str) -> Option<&ContractDefinition> {
        self.entries.get(id).map(|e| &e.def)
    }

    pub fn hash_of(&self, id: &str) -> Option<&str> {
        self.entries.get(id).map(|e| e.hash.as_str())
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// A `ContractRef` pinned to the schema currently in the registry.
    pub fn pinned_ref(&self, id: &str) -> Option<ContractRef> {
        self.hash_of(id).map(|h| ContractRef::pinned(id, h))
    }

    pub fn validate(
        &self,
        contract: &ContractRef,
        payload: &Value,
    ) -> Result<Validated, ContractError> {
        let entry = self
            .entries
            .get(&contract.id)
            .ok_or_else(|| ContractError::Unknown(contract.id.clone()))?;
        if let Some(pinned) = &contract.schema_hash {
            if pinned != &entry.hash {
                return Err(ContractError::HashMismatch {
                    id: contract.id.clone(),
                    pinned: pinned.clone(),
                    actual: entry.hash.clone(),
                });
            }
        }
        let errors: Vec<String> = entry
            .validator
            .iter_errors(payload)
            .map(|e| format!("{}: {}", e.instance_path(), e))
            .collect();
        if !errors.is_empty() {
            return Err(ContractError::Invalid {
                id: contract.id.clone(),
                errors,
            });
        }
        Ok(Validated {
            contract_id: contract.id.clone(),
            schema_hash: entry.hash.clone(),
        })
    }

    pub fn validate_message(&self, msg: &StructuredMessage) -> Result<Validated, ContractError> {
        self.validate(&msg.contract, &msg.payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builtins_validate() {
        let r = ContractRegistry::with_builtins();
        let card = r.pinned_ref("nmesh.card/v1").unwrap();
        assert!(r.validate(&card, &json!({"title": "Hi"})).is_ok());
        assert!(matches!(
            r.validate(&card, &json!({"body": "no title"})),
            Err(ContractError::Invalid { .. })
        ));
    }

    #[test]
    fn hash_pinning() {
        let r = ContractRegistry::with_builtins();
        let wrong = ContractRef::pinned("nmesh.card/v1", "deadbeef");
        assert!(matches!(
            r.validate(&wrong, &json!({"title": "x"})),
            Err(ContractError::HashMismatch { .. })
        ));
    }

    #[test]
    fn hash_is_key_order_independent() {
        let a: Value = serde_json::from_str(r#"{"type":"object","title":"x"}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"title":"x","type":"object"}"#).unwrap();
        assert_eq!(schema_hash(&a), schema_hash(&b));
    }
}
