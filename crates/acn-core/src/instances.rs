//! Held-out instances (spec §7 "Instances", CR-003 §2.3): which pool items get tested is drawn
//! from two seeds neither party controls alone, and each drawn item is proved against the pool's
//! Merkle root. Pure functions; the fold checks attestations with them (V-27).
//!
//! - **Merkle tree:** RFC 6962 §2.1 (as in Certificate Transparency): leaf hash
//!   `sha256(0x00 ‖ d)`, node hash `sha256(0x01 ‖ left ‖ right)`, splitting at the largest power
//!   of two below the size. A pool item `d` is the JCS encoding of its JSON value. Audit paths
//!   are verified as in RFC 9162 §2.1.3.2.
//! - **Sample:** `seed = sha256(requester_seed ‖ performer_seed ‖ commitment_id)` over the raw
//!   32-byte values; each index `i` of `0..pool_size` ranks by `sha256(seed ‖ u32_be(i))`; the
//!   `sample_size` lowest ranks are the sample.

use serde_json::Value;
use sha2::{Digest, Sha256};

pub type Hash = [u8; 32];

fn sha(parts: &[&[u8]]) -> Hash {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

pub fn leaf_hash(data: &[u8]) -> Hash {
    sha(&[&[0x00], data])
}

fn node_hash(l: &Hash, r: &Hash) -> Hash {
    sha(&[&[0x01], l, r])
}

/// The bytes a pool item contributes to the tree: its JCS encoding.
pub fn item_bytes(item: &Value) -> Vec<u8> {
    crate::jcs::canonical(item).unwrap_or_default().into_bytes()
}

fn split(n: usize) -> usize {
    let mut k = 1;
    while k << 1 < n {
        k <<= 1;
    }
    k
}

/// RFC 6962 Merkle tree hash over leaf hashes (`n ≥ 1`).
fn mth(leaves: &[Hash]) -> Hash {
    match leaves.len() {
        0 => sha(&[]),
        1 => leaves[0],
        n => {
            let k = split(n);
            node_hash(&mth(&leaves[..k]), &mth(&leaves[k..]))
        }
    }
}

/// Root over raw pool items (each item's bytes are leaf data).
pub fn root(items: &[Vec<u8>]) -> Hash {
    let leaves: Vec<Hash> = items.iter().map(|d| leaf_hash(d)).collect();
    mth(&leaves)
}

/// Audit path for leaf `index` (RFC 6962 §2.1.1), bottom-up.
pub fn proof(items: &[Vec<u8>], index: usize) -> Vec<Hash> {
    fn path(leaves: &[Hash], m: usize) -> Vec<Hash> {
        let n = leaves.len();
        if n <= 1 {
            return vec![];
        }
        let k = split(n);
        if m < k {
            let mut p = path(&leaves[..k], m);
            p.push(mth(&leaves[k..]));
            p
        } else {
            let mut p = path(&leaves[k..], m - k);
            p.push(mth(&leaves[..k]));
            p
        }
    }
    let leaves: Vec<Hash> = items.iter().map(|d| leaf_hash(d)).collect();
    path(&leaves, index)
}

/// RFC 9162 §2.1.3.2: does `leaf` sit at `index` in a tree of `size` leaves with `root`?
pub fn verify(index: u64, size: u64, leaf: &Hash, path: &[Hash], root: &Hash) -> bool {
    if index >= size {
        return false;
    }
    let (mut fnode, mut snode) = (index, size - 1);
    let mut r = *leaf;
    for p in path {
        if snode == 0 {
            return false;
        }
        if fnode & 1 == 1 || fnode == snode {
            r = node_hash(p, &r);
            if fnode & 1 == 0 {
                while fnode & 1 == 0 && fnode != 0 {
                    fnode >>= 1;
                    snode >>= 1;
                }
            }
        } else {
            r = node_hash(&r, p);
        }
        fnode >>= 1;
        snode >>= 1;
    }
    snode == 0 && r == *root
}

/// The sample: `sample_size` pool indices, ordered by rank.
pub fn sample(
    requester_seed: &Hash,
    performer_seed: &Hash,
    commitment: &Hash,
    pool_size: u32,
    sample_size: u32,
) -> Vec<u32> {
    let seed = sha(&[requester_seed, performer_seed, commitment]);
    let mut ranked: Vec<(Hash, u32)> = (0..pool_size)
        .map(|i| (sha(&[&seed, &i.to_be_bytes()]), i))
        .collect();
    ranked.sort();
    ranked
        .into_iter()
        .take(sample_size as usize)
        .map(|(_, i)| i)
        .collect()
}

pub fn from_hex(s: &str) -> Option<Hash> {
    hex::decode(s).ok()?.try_into().ok()
}

/// What a runner holds to attest an item with instances: its whole pool, the requester's seed it
/// was given, the performer's seed the `REPORT` revealed, and the commitment id (all hex).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HeldOut {
    pub pool: Vec<Value>,
    pub requester_seed: String,
    pub performer_seed: String,
    pub commitment: String,
}

impl HeldOut {
    /// Check the pool and seed against the item's terms, then [`reveal`]: what V-27 will check,
    /// so a runner never signs an attestation that doesn't count.
    pub fn reveal_for(
        &self,
        terms: &crate::terms::Instances,
    ) -> Result<(Vec<u32>, Vec<Value>), String> {
        if self.pool.len() != terms.pool_size as usize {
            return Err(format!(
                "the pool holds {} items, the terms say {}",
                self.pool.len(),
                terms.pool_size
            ));
        }
        if pool_root(&self.pool) != terms.pool_root {
            return Err("the pool doesn't hash to the terms' pool_root".into());
        }
        let rs = from_hex(&self.requester_seed).ok_or("requester_seed is not 32-byte hex")?;
        if crate::jcs::sha256_hex(&rs) != terms.requester_seed_hash {
            return Err("requester_seed doesn't match requester_seed_hash".into());
        }
        reveal(
            &self.pool,
            &self.requester_seed,
            &self.performer_seed,
            &self.commitment,
            terms.sample_size,
        )
        .ok_or_else(|| "the performer seed or commitment id is not 32-byte hex".into())
    }
}

/// `pool_root` for a pool of JSON items, as hex.
pub fn pool_root(pool: &[Value]) -> String {
    let b: Vec<Vec<u8>> = pool.iter().map(item_bytes).collect();
    hex::encode(root(&b))
}

/// What a runner reveals in its attestation (V-27): the sample both seeds select from `pool`, in
/// rank order, and `[{index, item, proof}]` for the `instances` field. Seeds and the commitment
/// id are 32-byte hex. `None` if one doesn't decode or the sample is larger than the pool.
pub fn reveal(
    pool: &[Value],
    requester_seed: &str,
    performer_seed: &str,
    commitment: &str,
    sample_size: u32,
) -> Option<(Vec<u32>, Vec<Value>)> {
    let (rs, ps, cid) = (
        from_hex(requester_seed)?,
        from_hex(performer_seed)?,
        from_hex(commitment)?,
    );
    if sample_size as usize > pool.len() {
        return None;
    }
    let b: Vec<Vec<u8>> = pool.iter().map(item_bytes).collect();
    let picked = sample(&rs, &ps, &cid, pool.len() as u32, sample_size);
    let revealed = picked
        .iter()
        .map(|&i| {
            let proof: Vec<String> = proof(&b, i as usize).iter().map(hex::encode).collect();
            serde_json::json!({"index": i, "item": pool[i as usize], "proof": proof})
        })
        .collect();
    Some((picked, revealed))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6962 test vectors (Certificate Transparency's merkle tree tests).
    #[test]
    fn rfc6962_vectors() {
        let leaves: Vec<Vec<u8>> = vec![
            vec![],
            vec![0x00],
            vec![0x10],
            vec![0x20, 0x21],
            vec![0x30, 0x31],
            vec![0x40, 0x41, 0x42, 0x43],
            vec![0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57],
            (0x60..=0x6f).collect(),
        ];
        assert_eq!(
            hex::encode(root(&leaves[..1])),
            "6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d"
        );
        assert_eq!(
            hex::encode(root(&leaves)),
            "5dc9da79a70659a9ad559cb701ded9a2ab9d823aad2f4960cfe370eff4604328"
        );
    }

    #[test]
    fn every_proof_verifies_and_tampering_fails() {
        for n in 1..=17usize {
            let items: Vec<Vec<u8>> = (0..n).map(|i| format!("item {i}").into_bytes()).collect();
            let r = root(&items);
            for i in 0..n {
                let p = proof(&items, i);
                let leaf = leaf_hash(&items[i]);
                assert!(verify(i as u64, n as u64, &leaf, &p, &r), "n={n} i={i}");
                assert!(!verify(i as u64, n as u64, &leaf_hash(b"other"), &p, &r));
                if n > 1 {
                    let j = (i + 1) % n;
                    assert!(
                        !verify(j as u64, n as u64, &leaf, &p, &r),
                        "n={n} i={i} as {j}"
                    );
                }
            }
        }
    }

    #[test]
    fn reveal_proves_against_the_pool_root() {
        let pool: Vec<Value> = (0..12)
            .map(|i| serde_json::json!(format!("case {i}")))
            .collect();
        let r = from_hex(&pool_root(&pool)).unwrap();
        let (s, items) = reveal(
            &pool,
            &"01".repeat(32),
            &"02".repeat(32),
            &"03".repeat(32),
            3,
        )
        .unwrap();
        assert_eq!(s, sample(&[1; 32], &[2; 32], &[3; 32], 12, 3));
        for x in &items {
            let path: Vec<Hash> = x["proof"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| from_hex(h.as_str().unwrap()).unwrap())
                .collect();
            let leaf = leaf_hash(&item_bytes(&x["item"]));
            assert!(verify(x["index"].as_u64().unwrap(), 12, &leaf, &path, &r));
        }
        assert!(reveal(
            &pool,
            &"01".repeat(32),
            &"02".repeat(32),
            &"03".repeat(32),
            13
        )
        .is_none());
    }

    #[test]
    fn sample_depends_on_both_seeds() {
        let (a, b, c) = ([1u8; 32], [2u8; 32], [3u8; 32]);
        let s = sample(&a, &b, &c, 20, 5);
        assert_eq!(s.len(), 5);
        let mut u = s.clone();
        u.sort();
        u.dedup();
        assert_eq!(u.len(), 5);
        assert_eq!(s, sample(&a, &b, &c, 20, 5));
        assert_ne!(s, sample(&[9; 32], &b, &c, 20, 5));
        assert_ne!(s, sample(&a, &[9; 32], &c, 20, 5));
    }
}
