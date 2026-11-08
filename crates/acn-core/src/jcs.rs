//! RFC 8785 (JCS) canonical JSON and content hashes (spec §5.3).

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Canonical JSON string (RFC 8785).
pub fn canonical<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json_canonicalizer::to_string(value)
}

/// Lowercase hex sha256 of the JCS encoding. Used for terms, derivation records, templates.
pub fn hash<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    Ok(sha256_hex(canonical(value)?.as_bytes()))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_and_number_form_are_canonical() {
        let a = json!({"b": 1.0, "a": [1, "x"], "c": {"z": true, "y": null}});
        let b: serde_json::Value =
            serde_json::from_str(r#"{"c":{"y":null,"z":true},"a":[1,"x"],"b":1}"#).unwrap();
        assert_eq!(
            canonical(&a).unwrap(),
            r#"{"a":[1,"x"],"b":1,"c":{"y":null,"z":true}}"#
        );
        assert_eq!(hash(&a).unwrap(), hash(&b).unwrap());
    }
}
