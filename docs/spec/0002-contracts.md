# Spec 0002: Data contracts (draft)

A **contract** is a versioned JSON Schema (draft 2020-12) plus metadata that defines what a
structured payload means.

## Identity and versioning
- ID format: `<namespace>.<name>/v<major>`, e.g. `nmesh.card/v1`, `org.acme.invoice/v2`.
- Namespaces: `nmesh.*` is reserved for built-ins. `mcp.<server>.<tool>/v1` is used for contracts
  derived automatically from MCP `outputSchema`. Everyone else uses reverse-DNS or their own
  npub prefix.
- **Minor changes must be additive** (new optional properties). Anything else gets a new major
  ID.
- **Pinning:** `schema_hash = sha256(canonical JSON of schema)`, where canonical means object
  keys sorted with no insignificant whitespace. A message MAY pin a hash. A host MUST reject a
  pinned message whose hash doesn't match the schema it holds for that ID, and fall back to alt.

## Definition (kind 34930 content, and `contracts/registry/*.json`)
```json
{
  "id": "nmesh.card/v1",
  "title": "Card",
  "description": "...",
  "capabilities": ["interactive"],
  "schema": { "$schema": "https://json-schema.org/draft/2020-12/schema", "...": "..." }
}
```
`capabilities` are hints for renderer selection: `interactive`, `spatial`, `streaming`.

## Host rules
1. Look up the contract. If it is unknown, try to fetch the definition from trusted publishers.
2. Check the pin, if present.
3. Validate the payload. **Invalid payloads never reach a renderer**; show alt text.
4. Resolve a renderer (spec 0003).

## Built-ins
| ID | Purpose |
|---|---|
| `nmesh.card/v1` | title, body, fields, image, buttons |
| `nmesh.table/v1` | typed columns and rows |
| `nmesh.form/v1` | input request (the Data Forms equivalent) |
| `nmesh.form.submit/v1` | action payload: `action_id` and/or `values` |
| `nmesh.location/v1` | point, optional `spatial` anchor (spec 0005) |
