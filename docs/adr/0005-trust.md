# ADR 0005: Trust: allow-list now, web of trust as the goal

**Status:** accepted

## Context
Third-party renderers execute code (Tier 1/2) and third-party agents read plaintext. We need
a trust decision for every manifest, without inventing a new trust protocol.

## Decision
**v1 (implemented):** `TrustPolicy`, an explicit allow-list of publisher pubkeys plus the user
themself. Built-in renderers and contracts are always trusted. Anything else falls back to the
generic renderer or alt text.

**Target (Phase 4),** layered on existing NIPs:
1. **Curation:** NIP-51 sets (follow sets / curation sets) of trusted publishers or manifests.
2. **Recommendations:** NIP-89-style kind 31989 events ("I use renderer X for contract Y")
   from people the user follows. One hop by default, configurable.
3. **Scores:** NIP-85 Trusted Assertions (`30382:rank` for publishers, `30384:rank` for
   addressable manifests) from providers the user selects in their kind 10040 list.
4. **Pinning:** once accepted, a renderer is pinned by artifact SHA-256. Updates require
   re-acceptance if permissions grow.

`TrustDecision` already has `Recommended` and `Scored` variants, so callers won't change.

## Consequences
- The simple start is predictable and auditable.
- WoT is opt-in per source, and users can see *why* something is trusted.
- Our only new requirement is that manifests be addressable events, which keeps them
  compatible with the NIP-51, NIP-89 and NIP-85 tooling that already exists.
