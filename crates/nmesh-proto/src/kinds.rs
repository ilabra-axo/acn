//! Event kinds used by nmesh.
//!
//! **The nmesh-specific numbers below are PLACEHOLDERS.** They were chosen from ranges that are
//! unused in the NIPs README / kind registry at the time of writing, but they are not registered.
//! Before any public release they must be checked again and, ideally, proposed as a NIP.
//! See `docs/spec/0001-kinds-and-tags.md`.

use nostr::Kind;

/// Regular event: a structured payload bound to a data contract (normally sent as a NIP-59 rumor).
pub const STRUCTURED_MESSAGE: u16 = 4930;
/// Regular event: a user/renderer action on a previous structured message (button, form submit).
pub const ACTION: u16 = 4931;
/// Ephemeral event: agent progress / status ("thinking", "calling tool X"). Reserved, not yet used.
pub const AGENT_STATUS: u16 = 24930;
/// Addressable event: a data contract definition (JSON Schema + metadata). `d` = contract id.
pub const CONTRACT_DEFINITION: u16 = 34930;
/// Addressable event: a renderer manifest. `d` = renderer id.
pub const RENDERER_MANIFEST: u16 = 34931;
/// Addressable event: an agent manifest (capabilities, operator, data handling). `d` = agent id.
pub const AGENT_MANIFEST: u16 = 34932;

// --- Standard kinds nmesh reuses (not placeholders) ---

/// NIP-17 chat message (used as the rumor kind for plain text DMs).
pub const PRIVATE_DIRECT_MESSAGE: u16 = 14;
/// NIP-17 DM inbox relay list.
pub const INBOX_RELAYS: u16 = 10050;
/// NIP-CD slash command definition.
pub const SLASH_COMMAND_DEFINITION: u16 = 31992;

// --- ACN (docs/Core.md; provisional, D-1). Constants live in `acn-core::kinds`; mirrored here
// so transport code can route without depending on the validator. ---

/// Rumor kind carrying one fully signed ACN act (Core.md §5.5 private mode).
pub const ACN_PRIVATE_ACT: u16 = 7739;
/// ACN service advertisement (addressable).
pub const ACN_SVC: u16 = 37700;

#[inline]
pub fn kind(k: u16) -> Kind {
    Kind::from_u16(k)
}
