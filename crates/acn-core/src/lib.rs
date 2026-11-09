//! ACN core — Agent Commitments over Nostr (spec: `docs/Core.md`, v0.5).
//!
//! A validator is a **pure function** from an event set (plus content-addressed documents) to
//! commitment states and anomalies (spec §10). This crate has no I/O, no clock, and no C
//! dependencies, so it compiles to `wasm32-unknown-unknown` and every client/binding computes
//! the same state from the same events (INV-2, INV-12).
//!
//! ```text
//! fold(events, documents) -> Output { commitments, anomalies }   // deterministic, time-free
//! status_at(&commitment, now) -> Status                          // lapse etc. (spec §6.5)
//! ```
//!
//! Building acts: use [`draft::Draft`] (act schemas, spec §6.2), then sign with any Nostr signer
//! (or [`draft::Draft::sign`] for tests / headless agents).

pub mod anomaly;
pub mod draft;
pub mod event;
pub mod instances;
pub mod jcs;
pub mod kinds;
pub mod sealed;
pub mod template;
pub mod terms;

mod act;
mod binding;
mod fold;
mod status;

pub use anomaly::{Anomaly, Code, Severity};
pub use binding::{binding_status, BindingStatus};
pub use event::Event;
pub use fold::{fold, Commitment, ConditionStatus, Output, Pending, Phase, Profile, State};
pub use status::{status_at, Party, Status};
pub use terms::{Assurance, DisqualifierStatus, Terms};
