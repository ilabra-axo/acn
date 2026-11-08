# ADR 0003: E2E groups via Marmot (MLS); NIP-17 for DMs

**Status:** accepted

## Context
Groups must be end-to-end encrypted. NIP-29 relay-based groups are simpler, but the relay
sees content and enforces membership. NIP-EE (MLS) is now marked unrecommended in the NIPs repo
and has been superseded by the **Marmot protocol**, which has a maintained Rust
implementation (MDK, used by White Noise).

## Decision
- **1:1 and very small ad-hoc conversations:** NIP-17 (NIP-44 + NIP-59 gift wrap). This is
  implemented now.
- **Groups:** Marmot via `mdk-core`. Behind the `mls` cargo feature until Phase 2 completes.
- **No relay-trusted groups by default.** NIP-29 may be added later for *public* rooms only.
- Group application messages are rumors, the same as NIP-17. Structured messages and actions are
  therefore transport-independent: one parser (`Inbound::from_rumor`) serves both.

## Consequences
- We inherit MLS properties: forward secrecy, post-compromise security, efficient membership
  changes.
- KeyPackages (kind 30443, and legacy 443) must be published and rotated.
- **Agents in an E2E group see plaintext.** This is unavoidable and must be explicit in the UI
  (ADR 0004).
- Interop with other Marmot clients is a goal and a test case.
