//! Data contracts, renderer resolution and trust policy.
//!
//! - [`ContractRegistry`] holds contract definitions (built-ins + any loaded from relays) and
//!   validates payloads against them.
//! - [`resolve_renderer`] picks a renderer for a message following the order in
//!   `docs/spec/0003-renderers.md`.
//! - [`TrustPolicy`] decides which publishers' manifests are acceptable. Today it is an explicit
//!   allow-list; it is designed to grow into web-of-trust (`docs/adr/0005-trust.md`).

pub mod builtin;
mod registry;
mod resolve;
mod trust;

pub use registry::{schema_hash, ContractError, ContractRegistry, Validated};
pub use resolve::{resolve_renderer, Resolution};
pub use trust::{TrustDecision, TrustPolicy};
