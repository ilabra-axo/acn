# Spec 0003: Renderers (draft)

## Manifest (kind 34931 content)
```json
{
  "id": "acme.chart",
  "title": "Acme charts",
  "contracts": ["acme.timeseries/v1"],
  "targets": ["web"],
  "runtime": "sandboxed_web",
  "entry": { "url": "https://blossom.example/<sha256>", "sha256": "<sha256>" },
  "actions": ["nmesh.form.submit/v1"],
  "permissions": []
}
```
- `runtime`: `declarative` (Tier 0), `sandboxed_web` (Tier 1), `native` (Tier 2).
- `targets`: `web`, `android`, `ar`, `tui`, …
- `entry` is required for non-built-ins. The host MUST verify `sha256` before loading.
  Blossom-hosted artifacts are recommended.
- `permissions` default to none. The host prompts before granting any.

## Resolution
For each validated structured message, the host picks a renderer in this order:
1. **Installed** renderers supporting `(contract, target)`. Built-ins are always installed.
2. **Trusted** third-party renderers (per the trust policy, ADR 0005).
3. **Generic** renderer (key/value tree).
4. **Alt text.**

Within a tier, prefer the lowest runtime tier, then the fewest permissions.
Reference implementation: `nmesh_contracts::resolve_renderer`.

## Host ↔ renderer contract
- Input: `{ contract, payload, context: { conversation, sender, theme, locale, target } }`.
- Output: requests only, e.g. `emitAction(contract, payload, alt)`, `openLink(url)`,
  `resize(h)`.
- The host validates every `emitAction` payload against the action contract and signs it
  itself, asking the user to confirm actions that have side effects. The host can't tell from a
  label or a schema whether a command has side effects, so the reference host confirms every
  action, showing the actual command and values and the recipient (the sender's label is shown
  as the sender's words); the user may skip the prompt for one command in one conversation for
  the session.
- An action answers the message it was rendered from: its recipient is that message's
  conversation, fixed when the renderer is mounted, and the host refuses a target that isn't a
  message from that peer.
- Renderers load nothing from the network by themselves. Images are hash-pinned and fetched by
  the host on the user's request (bounded, public addresses only), shown only if the hash
  matches.
- **Tier 1** uses the MCP Apps (2026-01-26) host↔view messages. nmesh adds only the
  `emitAction` mapping.

## Tier 0 declarative DSL
To be specified in Phase 1. Constraints: a small fixed set of components (text, image, stack,
grid, button, input, map, model3d); data binding by JSON Pointer into the payload; no scripting.
