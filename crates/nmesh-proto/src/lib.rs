//! nmesh wire format.
//!
//! This crate is deliberately small and dependency-light (only `nostr` + serde) so it can be
//! shared by every client (CLI, desktop, Android/ARCore via FFI, WASM) and by agent bridges.
//!
//! Everything here is documented normatively in `docs/spec/`.

pub mod command;
pub mod kinds;
pub mod manifest;
pub mod structured;

mod error;

pub use error::ProtoError;
pub use structured::{Action, ContractRef, StructuredMessage};
