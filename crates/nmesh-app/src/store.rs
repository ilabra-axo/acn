//! Tiny JSON persistence for local-only state (contacts + agent consent).
//!
//! Message history is not stored locally yet: it is re-fetched from relays (gift wraps) on
//! start. A proper database is roadmap Phase 1.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredContact {
    pub pubkey: String,
    /// Last known agent status, so consent is enforced before the profile re-fetch completes.
    #[serde(default)]
    pub is_agent: bool,
    /// User accepted this agent's data-handling disclosure.
    #[serde(default)]
    pub accepted: bool,
    /// Discovery found a profile (or an agent): `is_agent` was established, not assumed.
    #[serde(default)]
    pub resolved: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Stored {
    #[serde(default)]
    pub contacts: Vec<StoredContact>,
    /// Payment preimages `s` for conditional commitments we requested (plaintext; test sats only).
    #[serde(default)]
    pub secrets: std::collections::HashMap<String, String>,
    /// Our seeds for held-out instances (CR-003 §2.3), per commitment we requested: kept so we
    /// can be our own runner, or check a runner's reveal.
    #[serde(default)]
    pub seeds: std::collections::HashMap<String, String>,
    /// "Reset demo": ACN acts signed before this unix time are ignored (they stay on relays).
    #[serde(default)]
    pub demo_epoch: u64,
    /// Commitments cleared by the reset that had acts in the same second as `demo_epoch`.
    #[serde(default)]
    pub cleared: Vec<String>,
    /// Delivery keys `k` recovered from the mint for contingent commitments whose performer
    /// redeemed without a `CLAIM` (commitment id → hex), so the deliverable stays readable.
    #[serde(default)]
    pub keys: std::collections::BTreeMap<String, String>,
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("contacts.json"),
        }
    }

    pub fn load(&self) -> Stored {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data: &Stored) -> std::io::Result<()> {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(data)?)?;
            std::fs::rename(tmp, &self.path)
        };
        write().inspect_err(|e| {
            tracing::warn!(error = %e, path = %self.path.display(), "failed to save contacts");
        })
    }
}
