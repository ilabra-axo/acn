# Roadmap

Each phase ends with something usable. Dates are intentionally absent; each phase is a few weeks
of focused work for 1–2 people.

## Phase 0: Foundations ✅ (this scaffold)

- [x] Rust workspace: `proto`, `contracts`, `core`, `agent-bridge`, `cli`
- [x] NIP-17 DMs carrying text and structured messages; self-copies for multi-device
- [x] Contract registry: JSON Schema validation, SHA-256 pinning, 5 built-in contracts
- [x] Renderer resolution and trust policy (allow-list)
- [x] Agent bridge: MCP stdio and HTTP → NIP-CD commands → structured replies
- [x] Agent manifest with hosting and data-handling disclosure; kind 0 `bot: true`
- [x] Dev relay and CLI; end-to-end test against an in-process relay
- [x] ADRs and draft specs

## Phase 0.5: Desktop client ✅

- [x] `nmesh-app`: contacts, history (from relays), unread counts, view models, renderer decisions
- [x] Agent discovery (kind 0 `bot`, agent manifest, NIP-CD commands) and an enforced consent gate
- [x] Tauri 2 shell + Svelte 5 UI: sidebar, chat, agent banner, slash autocomplete
- [x] Tier 0 web renderers: card, table, form, location (with AR-anchor badge), generic, alt
- [x] Actions: card buttons and form submits → kind 4931 → bridge routes `/command` to MCP tools
- [x] Browser preview with a stand-in backend (now presenter mode), plus a screenshot script

## ACN track (docs/Core.md)

- [x] Core.md v0.2: time model, advancing/annotating acts, NIP-59-compliant private mode, NUT-14 lock, escrowed-preimage arbitration, V-15/V-16
- [x] **P0** `acn-core`: all 13 v0.2 acts (CR-002 added `RELEASE`, `FUND`, `CLAIM`), terms, JCS, sibling resolution, bindings, endorsements, attestations; scenarios for every act and anomaly code; permutation property test; `wasm32-unknown-unknown` build
- [x] P1 (partial): `K_SVC`, operator binding, T0 at price 0 end to end (desktop ↔ relay ↔ shim ↔ MCP), private-mode transport
- [x] Demo: 9 scenarios (T0, T1, N, rework, attestations, dispute/ruling, governance, lapse, equivocation) in live mode (`nmesh-acn-sim`) and presenter mode (WASM), with the Director and presentation clock ([DEMO.md](DEMO.md))
- [x] P1 (T1 / N part): Cashu via CDK against a local test mint (`nmesh-mint`, fake Lightning): NUT-14 HTLC lock at request time, DLEQ + NUT-07 check before work, redeem on SATISFY / RULING, refund after the locktime, double spend rejected; integrated wallet (panel, balances, per-commitment token state); presenter mode uses a simulated mint with the same rules
- [x] **MVP demo:** pay only if two LLM judges score the rubric ≥ θ (CR-001), settled by the verifier trigger (CR-002): `/judged`, `/injected`, `/judged-auto`, plus `/contingent` (encrypted until paid, mechanical only)
- [x] Real judge harness (`acn-judge`): pinned judges on OpenAI-compatible endpoints (local Ollama or hosted), repeats and medians, injection scan plus model vote, pinned by `harness_hash`; `nmesh-acn-sim --judges`. Presenter mode keeps scripted judges. Next: Atlas does the work for real (an LLM performer), so the judges grade what was asked
- [x] D-28: `overruled_judgement` flags rejections of passed judged conditions (V-16)
- [x] CR-003 Phase A (`acn-terms/0.5`): no hidden criteria (public rubrics and checks), disqualifiers, `open` conditions drafted in `COUNTER` with closure before promise, the terms card; `/drafted`, `/disqualified`
- [x] CR-003 Phase B: held-out instances and the two-party seed (RFC 6962 pool roots, seeds committed before delivery, sample derived from both, proofs in the attestation); `/held-out`
- [x] Held-out instances for judged conditions: `acn-judge` gives the judges the sampled cases and reveals the requester seed and proofs in `K_ATTEST_JUDGED`; `/held-out` adds a judged condition on a second pool. The app keeps its requester seed and assembles a runner's `HeldOut` (`App::held_out`)
- [ ] A requester-side runner in the reference binding (the app running checks or judges itself): P3
- [x] CR-003 Phase C: terms templates (`K_TERMS_TEMPLATE`, typed slots, `template_ref` checked by re-instantiation, V-28), template and record on the terms card; best practices and two templates in `docs/acn/cos-templates.md`; Atlas's `/drafted` and `/disqualified` counters come from its published `retry-change/v1`
- [x] CR-003 Phase D: `acn-author`, the terms linter (L1–L8; errors block a `REQUEST`, warnings on the card) and the requester and performer drafters (Atlas's counters come from it; `App::draft_request`); template `notes` explain each change
- [x] Drafting UI (intent → drafted terms → editor with linter findings → send), in the app and presenter mode: P2
- [ ] A model-backed drafter behind the same signatures as `acn-author`'s
- [ ] Template records split by slot range (the template-laundering residual, Core §13)
- [x] Board (queue view, §12): columns by ACN stage across agents and roles, whose-move from `acn-view`, needs-you filter, paged Closed column, drawer with the full card; no drag-and-drop. Reset demo (Director)
- [x] P1 remainder: paid calls in the MCP shim, as T1 with a hash-locked token (verified before work, redeemed after `SATISFY`); unlocked-token T0 is not offered
- [ ] Wallet: NIP-60 backup, NWC top-up, real mints and a multi-mint policy, binding `max_sat` caps applied to the wallet, encrypted preimage/proof storage; the demo cast's execution journal
- [x] P2: the requester's counter-offer (terms editor on the card; Atlas accepts a closed, same-price counter or declines `counter_unacceptable`), terms by reference on Blossom (`terms` tag server hint; the app, the cast and the binding driver fetch and fold with them; conformance `t0/t1-terms-by-reference`), attestation details and evidence links on the card, the demo cast hosting Blossom and storing every `REPORT`'s artifacts and derivation record
- [x] P3 at L0 ([harness-binding-API.md](harness-binding-API.md), D-10): `acn-binding` traits and the performer driver, `acn-binding-nostr` adapters, `binding_level` in `K_SVC`, the L0 suite (`acn-conformance`), `acn-mcp-shim` at L0 (Blossom evidence, derivation records, CDK wallet), `acn-cli-shim` as the second independent binding, `nmesh blossom` dev server
- [x] Design-alignment audit remediation ([DESIGN-ALIGNMENT-AUDIT.md](DESIGN-ALIGNMENT-AUDIT.md) A01–A18): one admission policy for history and live; execution journal and outbox (no re-execution, same act on retry); verify-before-store; operator scope checked before work; V-8 against the terms each act commits to; evidence cutoff at the deciding act; full received-lock policy, distinct proofs, keysets, observed mint capabilities; recoverable refunds with fees; contingent key recovery without `CLAIM`; consent fails closed; host action confirmation; bounded fetches; private evidence by default; strict judge replies and pinned requests (`acn-judge/2`)
- [x] P3 at L1: profile N in the driver (promise, counters closing open conditions, the requester's counters, rework, disputes of contradicted rejections, held-out seeds, disqualifiers), the §8.2 locktime check in every binding, the L1 suite (`acn_conformance::l1`); both reference bindings at L1
- [ ] P3 beyond L1: L2 (Gate emitting endorsements) and L3 (Verifier) suites and driver support; the requester side (`nmesh-app`) and the demo cast on the traits; a requester-side runner; templates in the shims' configs
- [ ] P4: arbiter path, anchoring (D-11), adversarial scenarios

## Phase 1: Usable messaging

- [ ] Persistent storage (nostr-sdk LMDB/SQLite database) and history sync on reconnect
- [ ] NIP-65 / NIP-17 relay discovery: send to the recipient's 10050 inbox relays (gossip)
- [ ] External signers: NIP-46 (remote), NIP-55 (Android); keep keys out of the app
- [ ] Contacts and profiles (kind 0, NIP-05 verification)
- [ ] Multi-party NIP-17 DMs (small rooms)
- [ ] Local message store (history currently re-fetched from relays; 30-day window)
- [ ] Tier 0 declarative renderer DSL v1, so new renderers need no client release
- [ ] Desktop: notifications, search, settings (relays, name), multiple identities

## Phase 2: E2E groups (Marmot)

- [ ] `mls` feature: create group, add members from KeyPackages, process welcomes and messages
- [ ] SQLite MDK storage; key rotation; member removal
- [ ] Structured messages and actions inside groups (same rumor path as DMs)
- [ ] UI: an "agent in this group can read all messages" banner and per-agent consent
- [ ] Interop test against White Noise

## Phase 3: Agents, properly

- [x] Actions routed back to tools (`action_id` = `/command`, form values → named args)
- [ ] Agent status (ephemeral 24930): "working…", tool progress (MCP progress notifications)
- [ ] Conversational mode: LLM loop over MCP tools (provider-pluggable; disclosed in `forwards_to`)
- [ ] "Add agent from MCP URL" flow: provision a bridge with a fresh key, show the manifest, invite
- [ ] Hosted bridge service for operator-run and third-party agents (multi-tenant `nmesh-agent`)
- [ ] ContextVM transport for MCP servers that live on Nostr
- [ ] Per-agent allow-lists and rate limits; optional payments (NIP-57 zaps / Cashu NIP-61)

## Phase 4: Registry, trust and third-party renderers

- [ ] Publish and fetch contract, renderer and agent manifests (kinds 34930–34932)
- [ ] Recommendations: NIP-89-style `31989` for nmesh manifests; NIP-51 curation sets
- [ ] Web-of-trust policy: recommendations from follows, then NIP-85 `rank` from user-chosen providers
- [ ] Tier 1 sandboxed web renderers: MCP Apps-compatible host bridge, CSP, permission prompts, hash pinning
- [ ] `nmesh-ffi` (UniFFI) and an Android client (Compose) with Tier 0 renderers
- [ ] SDK and templates: `create-renderer`, `create-agent`

## Phase 5: Spatial / ARCore

- [ ] `spatial` block (spec 0005): geospatial anchors, cloud anchors, glTF on Blossom
- [ ] ARCore client: Geospatial API and SceneView; render `nmesh.location/v1` and spatial cards
- [ ] AR interactions emitted as actions (place, move, select)
- [ ] Anchor refresh (cloud anchors expire); shared anchors inside MLS groups

## Phase 6: Ecosystem

- [ ] Write up the kinds and contract/renderer tags as a NIP proposal; reserve kind numbers
- [ ] Moderation tooling (NIP-56 reports, NIP-32 labels) for public registries
- [ ] Contract governance: versioning rules, deprecation, and a canonical registry repo
