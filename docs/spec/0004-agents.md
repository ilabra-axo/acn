# Spec 0004: Agents (draft)

## Identity
- An agent has its own keypair and publishes:
  - kind 0 with `"bot": true` (NIP-24);
  - kind 10050 inbox relays (NIP-17);
  - kind 34932 **agent manifest**;
  - kind 31992 **slash command definition** per tool (NIP-CD), scoped to `kind:14`.

## Agent manifest (kind 34932 content)
```json
{
  "id": "echo",
  "name": "Echo agent",
  "description": "...",
  "hosting": "self_hosted",
  "operator": "npub1... or org",
  "data_handling": {
    "sees_plaintext": true,
    "retention": "none",
    "forwards_to": ["mcp:echo", "api.llm.example"]
  },
  "tools": [
    { "name": "card", "description": "...", "output_contract": "nmesh.card/v1" }
  ]
}
```
Clients MUST show `hosting`, `operator` and `data_handling` before first contact, and show a
persistent "agent present: sees messages" indicator in any conversation that includes one.
The gate fails closed. Missing advertisements are not evidence of a human: a client MUST NOT
send to a key before discovery has run, MUST treat a key with no profile at all as possibly an
agent (asking the user before first contact, with what is unknown), and MUST NOT downgrade a key
it once classified as an agent because a later fetch came back incomplete.

## Invocation
- NIP-CD: `/<command>[@nostr:<npub>] <args>` as the content of a DM (kind 14 rumor).
- **nmesh extension:** if the argument text starts with `{`, it is parsed as a JSON object of
  named arguments.
- MCP tool → NIP-CD args mapping: required properties first, then at most one optional
  property. A trailing string becomes `text`. Enums map to `enum`, numbers to `number`, booleans
  to `bool`.

## Results
- If the tool has `outputSchema` and returns `structuredContent` that validates, the agent
  replies with a structured message (kind 4930) with an `e` tag pointing at the invocation. Its
  `alt` comes from the text content.
  - `outputSchema["x-nmesh-contract"] = "<id>"` maps the result onto a known contract.
  - Otherwise the contract is `mcp.<server>.<tool>/v1`, derived from the schema.
- Otherwise the agent replies with plain text.
- Errors are plain text: `/<command> failed: …`.

## Actions
- A kind 4931 Action whose payload `action_id` begins with `/` is treated as an invocation of
  that command. If `values` is present it supplies **named arguments**; otherwise the text after
  the command is bound positionally.
- Card buttons put the command in `actions[].id` (e.g. `/echo hi`). Forms put it in
  `submit_action` (e.g. `/card`), and the host sends it as `action_id` together with the values.
- Other actions are acknowledged as text.

## Reserved
- Kind 24930 (ephemeral) agent status: `{ "state": "working" | "done" | "error", "detail": "..." }`
  with an `e` tag for the invocation. To be implemented in Phase 3.
