use std::path::Path;

use nostr_sdk::prelude::*;

use crate::CoreError;

/// Whether an identity is a human or an agent. Agents publish `"bot": true` in their kind 0
/// (NIP-24); clients use this to show the "this participant sees plaintext" warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityKind {
    Human,
    Agent,
}

/// Load an `nsec`/hex secret key from `path`, or create one (0600) if it doesn't exist.
///
/// This is a development convenience. Real clients should prefer an external signer
/// (NIP-46 remote signer, NIP-55 Android signer, NIP-07 in browsers) and never hold the key.
pub fn load_or_create_keys(path: &Path) -> Result<Keys, CoreError> {
    if path.exists() {
        let s = std::fs::read_to_string(path)?;
        return Ok(Keys::parse(s.trim())?);
    }
    let keys = Keys::generate();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|e| CoreError::Other(e.to_string()))?;
    write_secret(path, &nsec)?;
    Ok(keys)
}

#[cfg(unix)]
fn write_secret(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(contents.as_bytes())
}

#[cfg(not(unix))]
fn write_secret(path: &Path, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)
}
