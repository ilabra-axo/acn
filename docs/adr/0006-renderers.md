# ADR 0006: Three renderer tiers; Tier 1 compatible with MCP Apps

**Status:** accepted

## Context
Renderers must work in chat, on the web, on Android and in AR. Arbitrary code is powerful
but dangerous and not portable. MCP Apps (`ui://` resources in sandboxed iframes, with a
postMessage bridge) is now a stable MCP extension supported by several hosts.

## Decision
- **Tier 0: Declarative (default).** A JSON UI tree the host interprets. It runs everywhere
  (DOM, Compose, AR panels, terminal), can't exfiltrate, and is cheap. Built-in renderers are
  Tier 0.
- **Tier 1: Sandboxed web.** An iframe with a strict CSP, no network unless granted, and no key
  access. The host↔view protocol is **MCP Apps-compatible**, so an MCP server that already ships
  a `ui://` app can be used directly. Web and desktop only.
- **Tier 2: Native.** Platform modules or scene templates (ARCore + glTF). Installed
  explicitly; never auto-loaded from the network.
- **Renderers never sign.** They ask the host to emit an Action. The host validates it against
  the declared action contract and signs, after user confirmation where appropriate.
- **Resolution order:** installed → trusted third-party → generic → alt. Within a tier the least
  privileged runtime wins.

## Consequences
- AR support comes mostly from Tier 0 plus the spatial contract block, not from shipping code.
- The MCP Apps ecosystem is reusable on day one for web.
