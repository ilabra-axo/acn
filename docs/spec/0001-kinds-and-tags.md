# Spec 0001: Kinds and tags (draft)

> ⚠️ The nmesh kind numbers are **placeholders**. They were picked from ranges unused in the
> NIPs README at the time of writing, but they are not registered. They must be checked again
> against <https://github.com/nostr-protocol/registry-of-kinds> and proposed as a NIP before
> public release. Constants live in `crates/nmesh-proto/src/kinds.rs`.

## nmesh kinds

| Kind | Class | Name | Content | Key tags |
|---|---|---|---|---|
| 4930 | regular | Structured message | JSON payload | `contract`, `alt`, `e`? |
| 4931 | regular | Action | JSON payload | `contract`, `alt`, `e` (target) |
| 24930 | ephemeral | Agent status (reserved) | JSON | `e`?, `p` |
| 34930 | addressable | Contract definition | JSON `ContractDefinition` | `d`, `alt` |
| 34931 | addressable | Renderer manifest | JSON `RendererManifest` | `d`, `alt`, `contract`* |
| 34932 | addressable | Agent manifest | JSON `AgentManifest` | `d`, `alt`, `contract`* |

ACN kinds (7700–7739, 37700) are specified in [Core.md](../Core.md) §5.1 and defined in
`crates/acn-core/src/kinds.rs`; `nmesh-proto` mirrors `K_PRIVATE_ACT` (7739) and `K_SVC` (37700)
for routing.

Structured messages and actions are normally **rumors** (unsigned, inside NIP-59 gift wraps or
Marmot group messages). Publishing them signed and in the clear is allowed only for public
contexts.

## Reused standard kinds

| Kind | Use | NIP |
|---|---|---|
| 0 | profile; agents set `"bot": true` | 01, 24 |
| 14 | plain-text DM rumor | 17 |
| 1059 | gift wrap | 59 |
| 10050 | DM inbox relays | 17 |
| 31992 | slash command definition (one per agent tool) | CD |
| 10992 | user's installed slash commands | CD, 51 |
| 30443 / 443 | Marmot KeyPackage | Marmot |
| 444 / 445 | Marmot welcome / group message | Marmot |
| 31989 | recommendations (trust, Phase 4) | 89 |
| 30382 / 30384 / 10040 | trusted assertions and provider choice (Phase 4) | 85 |

## Tags

- `["contract", <contract-id>, <schema-sha256>?]`: **required** on 4930 and 4931; repeatable on
  manifests (lists the contracts handled or emitted).
- `["alt", <text>]`: **required** on 4930 and 4931. A plain-text fallback (NIP-31 pattern) so
  that any client can show something.
- `["e", <event-id>]`: on 4930, the message being replied to; on 4931, **required**, the
  structured message acted upon. IDs refer to *rumor* IDs.
- `["p", <pubkey>]`: on rumors, the conversation participants (NIP-17 convention).
