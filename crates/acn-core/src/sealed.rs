//! Sealed deliverables for contingent settlement (spec §8.2, CR-002): the NIP-44 v2 payload
//! construction (ChaCha20, HMAC-SHA256, HKDF, padding) keyed directly by the performer's 32-byte
//! key `k` as the conversation key, instead of an ECDH-derived one. Pure Rust, no randomness:
//! callers pass the 32-byte nonce. Interoperates with any NIP-44 v2 implementation given the
//! same conversation key (see `tests/nostr_compat.rs`).

use base64::Engine as _;
use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use sha2::{Digest, Sha256};

const VERSION: u8 = 2;
const MIN_LEN: usize = 1;
const MAX_LEN: usize = 65_535;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SealError {
    #[error("plaintext must be 1..=65535 bytes")]
    Length,
    #[error("not a NIP-44 v2 payload")]
    Payload,
    #[error("authentication failed: wrong key or tampered payload")]
    Mac,
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    for p in parts {
        inner.update(p);
    }
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// HKDF-Expand(PRK = k, info = nonce, L = 76) → (chacha key, chacha nonce, hmac key).
fn message_keys(k: &[u8; 32], nonce: &[u8; 32]) -> ([u8; 32], [u8; 12], [u8; 32]) {
    let mut okm = Vec::with_capacity(96);
    let mut t: Vec<u8> = Vec::new();
    for i in 1u8..=3 {
        t = hmac(k, &[&t, nonce, &[i]]).to_vec();
        okm.extend_from_slice(&t);
    }
    let mut key = [0u8; 32];
    let mut iv = [0u8; 12];
    let mut auth = [0u8; 32];
    key.copy_from_slice(&okm[..32]);
    iv.copy_from_slice(&okm[32..44]);
    auth.copy_from_slice(&okm[44..76]);
    (key, iv, auth)
}

fn padded_len(len: usize) -> usize {
    if len <= 32 {
        return 32;
    }
    let next_power = 1usize << (usize::BITS - (len - 1).leading_zeros());
    let chunk = if next_power <= 256 {
        32
    } else {
        next_power / 8
    };
    chunk * ((len - 1) / chunk + 1)
}

/// Encrypt `plaintext` under `k` with the caller's random `nonce`; base64 payload.
pub fn seal(k: &[u8; 32], nonce: &[u8; 32], plaintext: &[u8]) -> Result<String, SealError> {
    if !(MIN_LEN..=MAX_LEN).contains(&plaintext.len()) {
        return Err(SealError::Length);
    }
    let mut buf = Vec::with_capacity(2 + padded_len(plaintext.len()));
    buf.extend_from_slice(&(plaintext.len() as u16).to_be_bytes());
    buf.extend_from_slice(plaintext);
    buf.resize(2 + padded_len(plaintext.len()), 0);
    let (key, iv, auth) = message_keys(k, nonce);
    ChaCha20::new(&key.into(), &iv.into()).apply_keystream(&mut buf);
    let mac = hmac(&auth, &[nonce, &buf]);
    let mut payload = vec![VERSION];
    payload.extend_from_slice(nonce);
    payload.extend_from_slice(&buf);
    payload.extend_from_slice(&mac);
    Ok(base64::engine::general_purpose::STANDARD.encode(payload))
}

/// Decrypt a payload made by [`seal`] (or any NIP-44 v2 encryption with conversation key `k`).
pub fn open(k: &[u8; 32], payload: &str) -> Result<Vec<u8>, SealError> {
    let p = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|_| SealError::Payload)?;
    if p.len() < 1 + 32 + 2 + 32 + 32 || p[0] != VERSION {
        return Err(SealError::Payload);
    }
    let nonce: [u8; 32] = p[1..33].try_into().expect("length checked");
    let (ct, mac) = p[33..].split_at(p.len() - 33 - 32);
    let (key, iv, auth) = message_keys(k, &nonce);
    let want = hmac(&auth, &[&nonce, ct]);
    // Constant-time comparison.
    if want.iter().zip(mac).fold(0u8, |acc, (a, b)| acc | (a ^ b)) != 0 {
        return Err(SealError::Mac);
    }
    let mut buf = ct.to_vec();
    ChaCha20::new(&key.into(), &iv.into()).apply_keystream(&mut buf);
    let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    if len < MIN_LEN || buf.len() != 2 + padded_len(len) {
        return Err(SealError::Payload);
    }
    Ok(buf[2..2 + len].to_vec())
}

/// sha256 hex of a sealed payload's bytes: the deliverable's ciphertext hash (`ciphertext`).
pub fn ciphertext_hash(payload: &str) -> Option<String> {
    let p = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    Some(crate::jcs::sha256_hex(&p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper() {
        let k = [7u8; 32];
        for len in [1usize, 31, 32, 33, 200, 257, 4000] {
            let msg = vec![b'a'; len];
            let s = seal(&k, &[9; 32], &msg).unwrap();
            assert_eq!(open(&k, &s).unwrap(), msg);
        }
        let s = seal(&k, &[9; 32], b"hello").unwrap();
        assert_eq!(open(&[8; 32], &s), Err(SealError::Mac));
        assert_eq!(seal(&k, &[9; 32], b""), Err(SealError::Length));
    }

    #[test]
    fn padding_matches_nip44() {
        for (l, p) in [
            (1, 32),
            (32, 32),
            (33, 64),
            (37, 64),
            (45, 64),
            (49, 64),
            (64, 64),
            (65, 96),
            (100, 128),
            (111, 128),
            (200, 224),
            (250, 256),
            (320, 320),
            (383, 384),
            (384, 384),
            (400, 448),
            (500, 512),
            (512, 512),
            (515, 640),
            (700, 768),
            (800, 896),
            (900, 1024),
            (1020, 1024),
            (65536, 65536),
        ] {
            assert_eq!(padded_len(l), p, "len {l}");
        }
    }
}
