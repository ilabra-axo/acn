//! nmesh client core.
//!
//! One [`Session`] = one Nostr identity connected to a set of relays. It sends and receives
//! NIP-17 private messages (plain text and nmesh structured payloads, both gift-wrapped) and
//! surfaces them as a single stream of [`Inbound`] items.
//!
//! This crate is UI-agnostic on purpose: the CLI, a future desktop/web shell and the
//! Android/ARCore client (via `nmesh-ffi` / UniFFI) all sit on top of it.

mod acn;
mod error;
mod identity;
mod message;
mod peer;
mod session;

#[cfg(feature = "mls")]
pub mod mls;

pub use error::CoreError;
pub use identity::{load_or_create_keys, IdentityKind};
pub use message::{Body, Inbound};
pub use peer::PeerInfo;
pub use session::{InboxOptions, Session, SessionConfig};

pub use nostr_sdk::prelude::{EventId, Keys, PublicKey, RelayUrl, Timestamp, ToBech32};
