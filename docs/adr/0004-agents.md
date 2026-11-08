# ADR 0004: Agents are peers; three hosting models; explicit plaintext visibility

**Status:** accepted

## Context
Agents should be addable "via an MCP endpoint". Some will run on the user's machine, some
will be run by us (the operator), and some by third parties. In an E2E conversation an agent
necessarily reads plaintext.

## Decision
1. **An agent is a Nostr peer** with its own keypair, run by `nmesh-agent` (the bridge). It never
   holds a human's key.
2. **All three hosting models are supported** and declared in the agent manifest:
   `self_hosted | operator | third_party`, plus a free-text `operator` identity.
3. **Data handling is declared, not implied.** The manifest's `data_handling` contains
   `sees_plaintext: true`, `retention`, and `forwards_to` (MCP servers, LLM providers). Clients
   **must** show this before an agent is invited or DM'd for the first time, and show a
   persistent indicator in any conversation that includes an agent.
4. **Agents mark themselves** with `"bot": true` in kind 0 (NIP-24).
5. **Invocation uses NIP-CD slash commands**, so commands work from any Nostr client. The bridge
   publishes one kind 31992 definition per MCP tool. JSON-object args (`/tool {"a":1}`) are an
   nmesh extension for tools that don't fit positional args.
6. **Results use contracts.** When an MCP tool declares `outputSchema`, its `structuredContent`
   is validated and sent as a structured message. `x-nmesh-contract` in the schema maps a tool
   onto an existing contract (e.g. `nmesh.card/v1`). Otherwise a contract
   `mcp.<server>.<tool>/v1` is derived.

## Not chosen
- **NIP-90 DVMs:** now unrecommended upstream; its job market model is broader than we need.
- **Running MCP inside the client:** this ties agents to one device, they can't be shared in a
  group, and it blurs the trust boundary.

## Consequences
- Adding an agent means writing a config file. A hosted "paste an MCP URL" flow comes later
  (Phase 3).
- Agent identity and reputation build up on the agent's own npub.
- ContextVM can be added as another bridge transport without changing clients.
