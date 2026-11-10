//! The performer's durable execution journal: what it was about to do, what it signed and whether
//! that was delivered, what it redeemed, and which plain (non-ACN) messages it already handled.
//!
//! Relay history alone can't make effects idempotent: a `REPORT` may arrive after its `REQUEST`
//! or not at all, and inbox de-duplication lasts one subscription. The journal records **intent
//! before effect**, so a restart never runs work twice:
//!
//! - before the executor runs, the commitment (keyed by commitment id and terms hash) is recorded
//!   as [`Step::Executing`];
//! - the signed act is recorded as [`Step::Prepared`] *before* it is sent, and as
//!   [`Step::Sent`] once [`crate::Inbox::send`] succeeds. A prepared act is re-sent as is (same
//!   id) on the next drive, never re-signed or re-derived;
//! - `Executing` without an act after a restart is **ambiguous**: the work may or may not have
//!   run. It is never re-run automatically. If our act for the commitment turns up in history the
//!   entry is completed with it; otherwise the driver declines with `execution_uncertain`.
//!
//! The journal is a small JSON file, rewritten atomically (temporary file + rename) on every
//! change. A journal that exists but can't be parsed is an error: the binding refuses to start
//! rather than forget what it did.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use acn_core::Event;
use serde::{Deserialize, Serialize};

/// Where a commitment's move stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "step")]
pub enum Step {
    /// The executor is (or was) running. Recorded before it starts.
    Executing,
    /// The act is signed and recorded, not yet confirmed delivered.
    Prepared,
    /// Delivered.
    Sent,
    /// The binding will send nothing more on its own (e.g. the gate denied every move, or the
    /// validator would apply none). An operator may remove the entry to retry.
    Halted { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub commitment: String,
    /// Terms hash in force when the move was decided.
    pub terms: String,
    /// The counterparty the act goes to.
    pub to: String,
    pub step: Step,
    /// The signed act (from `Prepared` on).
    #[serde(default)]
    pub act: Option<Event>,
    /// Whether the executor was started for this entry.
    #[serde(default)]
    pub executed: bool,
    pub at: u64,
}

/// Redemption of a paid commitment's token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum Redemption {
    /// Failed so far; retried from `next_at`.
    Pending {
        attempts: u32,
        next_at: u64,
        last_error: String,
    },
    Redeemed {
        sat: u64,
        at: u64,
    },
    /// The outcome is uncertain (e.g. the mint says the token is already spent, which may be our
    /// own earlier redemption or the requester's refund): not retried, needs an operator.
    Reconcile {
        error: String,
        at: u64,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    v: u32,
    /// `commitment:terms` -> entry.
    #[serde(default)]
    entries: BTreeMap<String, Entry>,
    /// Commitment -> redemption.
    #[serde(default)]
    redemptions: BTreeMap<String, Redemption>,
    /// Plain messages already handled: rumor id -> its `created_at`.
    #[serde(default)]
    messages: BTreeMap<String, u64>,
}

/// How long handled message ids are remembered. Hosts must not act on messages older than this.
pub const MESSAGE_MEMORY_SECS: u64 = 7 * 24 * 60 * 60;

pub struct Journal {
    path: Option<PathBuf>,
    state: Mutex<State>,
}

impl Default for Journal {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl Journal {
    /// A journal that lives as long as the process (tests, ephemeral bindings): it prevents
    /// re-execution within the process only.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            state: Mutex::new(State::default()),
        }
    }

    /// Open (or create on first write) the journal at `path`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let state = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b)
                .map_err(|e| format!("execution journal {} is unreadable: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => return Err(format!("execution journal {}: {e}", path.display())),
        };
        Ok(Self {
            path: Some(path),
            state: Mutex::new(state),
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn key(commitment: &str, terms: &str) -> String {
        format!("{commitment}:{terms}")
    }

    pub fn entry(&self, key: &str) -> Option<Entry> {
        self.state.lock().unwrap().entries.get(key).cloned()
    }

    /// Entries of `commitment` (any terms).
    pub fn entries_for(&self, commitment: &str) -> Vec<(String, Entry)> {
        let prefix = format!("{commitment}:");
        self.state
            .lock()
            .unwrap()
            .entries
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(k, e)| (k.clone(), e.clone()))
            .collect()
    }

    /// Record `entry` under `key`, durably, before returning.
    pub fn put(&self, key: &str, entry: Entry) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        let old = st.entries.insert(key.to_string(), entry);
        if let Err(e) = self.persist(&st) {
            match old {
                Some(o) => st.entries.insert(key.to_string(), o),
                None => st.entries.remove(key),
            };
            return Err(e);
        }
        Ok(())
    }

    pub fn redemption(&self, commitment: &str) -> Option<Redemption> {
        self.state
            .lock()
            .unwrap()
            .redemptions
            .get(commitment)
            .cloned()
    }

    pub fn set_redemption(&self, commitment: &str, r: Redemption) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        st.redemptions.insert(commitment.to_string(), r);
        self.persist(&st)
    }

    /// Record a plain message as handled. `Ok(true)` the first time (act on it), `Ok(false)` if
    /// it was handled before. Recorded before the host acts: at most once.
    pub fn first_seen(&self, id: &str, created_at: u64, now: u64) -> Result<bool, String> {
        let mut st = self.state.lock().unwrap();
        if st.messages.contains_key(id) {
            return Ok(false);
        }
        let horizon = now.saturating_sub(MESSAGE_MEMORY_SECS);
        st.messages.retain(|_, at| *at >= horizon);
        st.messages.insert(id.to_string(), created_at);
        if let Err(e) = self.persist(&st) {
            st.messages.remove(id);
            return Err(e);
        }
        Ok(true)
    }

    /// Commitments with work due at `now`: an act to (re)send, or a redemption to retry.
    pub fn due(&self, now: u64) -> Vec<String> {
        let st = self.state.lock().unwrap();
        let mut v: Vec<String> = st
            .entries
            .values()
            .filter(|e| e.step == Step::Prepared)
            .map(|e| e.commitment.clone())
            .collect();
        v.extend(st.redemptions.iter().filter_map(|(c, r)| match r {
            Redemption::Pending { next_at, .. } if *next_at <= now => Some(c.clone()),
            _ => None,
        }));
        v.sort();
        v.dedup();
        v
    }

    fn persist(&self, st: &State) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(st).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("tmp");
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)
        };
        write().map_err(|e| format!("writing execution journal {}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survives_reopen_and_rejects_garbage() {
        let dir = std::env::temp_dir().join(format!("acn-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("j.json");
        let j = Journal::open(&path).unwrap();
        let e = Entry {
            commitment: "c1".into(),
            terms: "t".into(),
            to: "r".into(),
            step: Step::Executing,
            act: None,
            executed: true,
            at: 1,
        };
        j.put(&Journal::key("c1", "t"), e.clone()).unwrap();
        assert!(j.first_seen("m1", 10, 10).unwrap());
        let j2 = Journal::open(&path).unwrap();
        assert_eq!(j2.entries_for("c1"), vec![("c1:t".to_string(), e)]);
        assert!(j2.entries_for("c").is_empty());
        assert!(!j2.first_seen("m1", 10, 10).unwrap());
        std::fs::write(&path, b"{not json").unwrap();
        assert!(Journal::open(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
