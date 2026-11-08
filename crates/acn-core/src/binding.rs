//! Operator → agent key bindings (spec §4, V-8).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::act::Parsed;
use crate::kinds::{K_BIND, K_BIND_ACK, K_BIND_REVOKE};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BindingStatus {
    Bound { operator: String, binding: String },
    Unbound,
    OutOfScope { reason: String },
    Revoked { binding: String },
}

#[derive(Debug, Clone)]
struct Rec {
    id: String,
    operator: String,
    kinds: Option<BTreeSet<u16>>,
    max_sat: Option<u64>,
    mints: BTreeSet<String>,
    expires: u64,
    acked: bool,
    revoked_at: Option<u64>,
}

/// Binding status of `agent` from a set of `K_BIND` / `K_BIND_ACK` / `K_BIND_REVOKE` events.
/// Signatures are verified; anything else in `events` is ignored. Pure.
pub fn binding_status(
    events: &[crate::Event],
    agent: &str,
    kind: u16,
    sat: u64,
    mint: Option<&str>,
    at: u64,
) -> BindingStatus {
    let parsed: Vec<Parsed<'_>> = events
        .iter()
        .filter(|e| matches!(e.kind, K_BIND | K_BIND_ACK | K_BIND_REVOKE) && e.verify().is_ok())
        .filter_map(|e| crate::act::parse(e).ok().flatten())
        .collect();
    Bindings::build(&parsed).status(agent, kind, sat, mint, at)
}

#[derive(Debug, Default)]
pub struct Bindings {
    by_agent: BTreeMap<String, Vec<Rec>>,
}

impl Bindings {
    pub fn build(parsed: &[Parsed<'_>]) -> Self {
        let mut recs: BTreeMap<String, (String, Rec)> = BTreeMap::new();
        for p in parsed.iter().filter(|p| p.kind() == K_BIND) {
            let agent = p.ev.values("p")[0].to_string();
            let mut kinds = None;
            let mut max_sat = None;
            let mut mints = BTreeSet::new();
            for t in p.ev.tags_named("scope") {
                match (t.get(1).map(String::as_str), t.get(2)) {
                    (Some("kinds"), Some(v)) => {
                        kinds = Some(v.split(',').filter_map(|k| k.trim().parse().ok()).collect())
                    }
                    (Some("max_sat"), Some(v)) => max_sat = v.parse().ok(),
                    (Some("mint"), Some(v)) => {
                        mints.insert(v.clone());
                    }
                    _ => {}
                }
            }
            let expires =
                p.ev.single("expires")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            recs.insert(
                p.id().to_string(),
                (
                    agent,
                    Rec {
                        id: p.id().to_string(),
                        operator: p.ev.pubkey.clone(),
                        kinds,
                        max_sat,
                        mints,
                        expires,
                        acked: false,
                        revoked_at: None,
                    },
                ),
            );
        }
        for p in parsed.iter().filter(|p| p.kind() == K_BIND_ACK) {
            let target = p.ev.values("e")[0];
            if let Some((agent, rec)) = recs.get_mut(target) {
                if &p.ev.pubkey == agent {
                    rec.acked = true;
                }
            }
        }
        for p in parsed.iter().filter(|p| p.kind() == K_BIND_REVOKE) {
            let target = p.ev.values("e")[0];
            if let Some((_, rec)) = recs.get_mut(target) {
                if p.ev.pubkey == rec.operator {
                    let at = p.at();
                    rec.revoked_at = Some(rec.revoked_at.map_or(at, |r| r.min(at)));
                }
            }
        }
        let mut by_agent: BTreeMap<String, Vec<Rec>> = BTreeMap::new();
        for (_, (agent, rec)) in recs {
            if rec.acked {
                by_agent.entry(agent).or_default().push(rec);
            }
        }
        Self { by_agent }
    }

    /// Binding status of `agent` for an act of `kind`, value `sat`, at `at` (V-8).
    pub fn status(
        &self,
        agent: &str,
        kind: u16,
        sat: u64,
        mint: Option<&str>,
        at: u64,
    ) -> BindingStatus {
        let Some(recs) = self.by_agent.get(agent) else {
            return BindingStatus::Unbound;
        };
        let mut reasons = Vec::new();
        let mut revoked = None;
        for r in recs {
            if r.revoked_at.is_some_and(|ra| at >= ra) {
                revoked.get_or_insert(r.id.clone());
                continue;
            }
            if at >= r.expires {
                reasons.push("binding expired".to_string());
                continue;
            }
            if r.kinds.as_ref().is_some_and(|k| !k.contains(&kind)) {
                reasons.push(format!("kind {kind} not in scope"));
                continue;
            }
            if r.max_sat.is_some_and(|m| sat > m) {
                reasons.push(format!("value {sat} sat exceeds scope"));
                continue;
            }
            if let Some(m) = mint {
                if !r.mints.is_empty() && !r.mints.contains(m) {
                    reasons.push(format!("mint {m} not in scope"));
                    continue;
                }
            }
            return BindingStatus::Bound {
                operator: r.operator.clone(),
                binding: r.id.clone(),
            };
        }
        if let Some(b) = revoked {
            if reasons.is_empty() {
                return BindingStatus::Revoked { binding: b };
            }
        }
        reasons.sort();
        BindingStatus::OutOfScope {
            reason: reasons.join("; "),
        }
    }
}
