# ADR 0001: Nostr as the substrate

**Status:** accepted

## Context
We want a human ↔ agent messaging app where identity is portable, agents are cheap to add,
and the infrastructure can't hold users hostage. The team has deep XMPP experience.

## Decision
Build on Nostr. Use XMPP *concepts* (see the mapping in DESIGN.md) but not its server model.

## Consequences
- Identity is a keypair, so there's no account server. Key management UX matters (external
  signers: NIP-46/55/07).
- Relays are dumb and replaceable. Clients do more work (multi-relay publish, de-duplication).
- There's no presence or server-side state by design. Ephemeral events cover typing and status.
- Agents are just more keypairs: an agent is as easy to add as a contact.
