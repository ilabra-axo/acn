# Agent Commitments over Nostr — Logical Design Spec v0.1

Nov 7, 2026 · @ivan Labra

## 1. Purpose and scope

This spec defines a commitment protocol in which agent and MCP microservice endpoints, humans, and verifiers act as Nostr peers that make, fulfil, verify and close commitments, with Cashu conditional settlement and verifiable provenance. Working name: **ACN** (Agent Commitments over Nostr).

The conversation is not the protocol. ACN defines an event grammar and a deterministic state machine; any chat thread, board or queue is a projection over it.

**In scope:** identity and key binding, event envelope and causal linking, act vocabulary and state machine, terms documents, Cashu settlement, provenance and attestations, validation rules, governance endorsements, bindings and reputation, implementation phases.

**Two profiles:**

- **Transactional** (T0 unconditional, T1 conditional): single-shot microservice calls. Request carries payment; a report is an implicit promise. This is the default for MCP tool calls.
- **Negotiated** (N): the full commitment automaton with counter-offers, rework, and optional arbitration, for work with real conditions of satisfaction.

**Non-goals:** UI design; NIP-90 compatibility; replacing ABW's internal event store; mandating a transport for the underlying MCP invocation (Nostr-native or direct HTTP are both valid, since ACN wraps the call as a commitment).

NIP and NUT numbers cited here are from memory as of mid-2026 and must be checked against current specs before implementation.

## 2. Design invariants (normative)

These invariants override any section below that appears to conflict with them. MUST, SHOULD and MAY follow RFC 2119.

1. **INV-1 Immutability.** Commitment acts are regular (non-replaceable) events. Commitment state MUST NOT be stored in replaceable or addressable events. State is a fold over the act set.
2. **INV-2 Deterministic fold.** The same set of valid events MUST yield the same commitment state, regardless of arrival order or relay.
3. **INV-3 Causal order from hashes.** Ordering derives from `prev` references between event ids. `created_at` is advisory and MUST NOT decide any state transition.
4. **INV-4 Terms binding.** Every promise or acceptance binds to a content-addressed terms hash. Terms are immutable; changing them means a new hash.
5. **INV-5 Requester closure.** Only the requester can declare satisfaction. A performer's report is an assertion of completion, never closure.
6. **INV-6 Closure is settlement.** In conditional profiles, the satisfaction event reveals the payment preimage. Closing and paying are one signed act.
7. **INV-7 Signatures prove authorship only.** A signature never counts as evidence that a claim is true.
8. **INV-8 Interpretive is not provenance.** Interpretive reports (e.g. LLM-as-judge) use a distinct event kind. A validator MUST NOT satisfy a mechanical or grounding condition with an interpretive report.
9. **INV-9 Artifacts by hash.** Deliverables and evidence are referenced by sha256 and stored off-relay. Inline content is limited to small payloads.
10. **INV-10 Accountable keys.** Agent keys are bound to an operator key by a signed binding. Unbound keys MAY participate but are flagged unaccountable.
11. **INV-11 Evidence retention.** Invalid, stale or conflicting events are retained and reported as anomalies. They are never applied and never silently dropped.
12. **INV-12 Projections carry no semantics.** No client-side state may affect commitment semantics.
13. **INV-13 Protocol neutrality.** The core names no implementation. Verification, governance, scoring and storage are roles filled by keys and bindings.

## 3. Roles and system layers

Five roles sign acts in one grammar; relays, blob stores and mints are untrusted infrastructure underneath it.

&#91;embedded content: roles and layers · 5 roles, 4 layers\]

Trust comes from signatures, hashes and the shared validator, never from the transport. Every binding and client runs the same validator, so all compute the same state from the same events.

## 4. Identity, key binding, service advertisement

Every participant is a secp256k1 keypair. Operators hold long-lived keys; each agent or endpoint holds its own key, bound to an operator by a signed, scoped, expiring binding.

**Key classes.**

- **Operator key:** the accountable legal or organizational party. Kept cold where possible; signs bindings and revocations only.
- **Agent key:** one per agent or endpoint instance. Signs all commitment acts.
- **Verifier key:** signs attestations. May be an agent key with a verifier scope.

**Binding (`K_BIND`, regular).** Signed by the operator. Tags: `p` agent pubkey; `scope` (allowed act kinds, max commitment value in sat, allowed mints, verifier yes/no); `expires` (unix seconds); optional `svc` reference to the service advertisement. NIP-26 delegation is not used.

**Binding acceptance (`K_BIND_ACK`, regular).** Signed by the agent key, referencing the binding by event id. A binding is effective only once acknowledged. This proves key possession and prevents an operator claiming someone else's key.

**Revocation (`K_REVOKE`, regular).** Signed by the operator, referencing the binding. Optional `supersedes` tag names a replacement binding for rotation. Revocation effectiveness rules are in V-8.

**Service advertisement (`K_SVC`, addressable).** Signed by the agent key. Addressable is permitted here because it is metadata, not commitment state (INV-1). Content: offered tools with input/output schema hashes, profiles supported (T0, T1, N), price schedule, accepted mints, terms templates by hash, verifier requirements, and the MCP invocation transport (Nostr-native or HTTPS endpoint). This is how an MCP endpoint becomes a discoverable peer.

## 5. Event envelope, tags, causal linking

ACN events are standard NIP-01 events. One regular kind per act, so relays can filter by kind; the commitment id lives in an indexed single-letter tag so one filter retrieves a whole commitment.

**Kind allocation.** Commitment acts and attestations MUST sit in the regular range (1000–9999). Provisional block: 7700–7739. Check the kind registry for collisions before freezing; code MUST reference symbolic names (`K_REQUEST`, …), never literals.

**Tags.**

| Tag | Cardinality | Meaning |
| --- | --- | --- |
| `c` | 1 (all acts except request) | Commitment id = event id of the root request. Indexed. Letter provisional. |
| `e` … `"prev"` | 1 (all acts except request) | Event id of the immediate causal predecessor. |
| `p` | 1+ | Counterparty pubkeys (requester, performer; arbiter if any). Routing only. |
| `terms` | 0–1 | sha256 of the terms document + optional Blossom URL hint. |
| `x` | 0+ | sha256 of referenced artifacts. |
| `parent` | 0–1 | Parent commitment id (subcontracting). |
| `profile` | 1 on request | `T0`, `T1` or `N`. |
| `v` | 1 | Protocol version, e.g. `acn/0.1`. |

**Content.** JSON, per-act schema (section 6). Hash-relevant documents use RFC 8785 canonical JSON (JCS) before sha256. Inline content limit: 16 KiB; larger material goes to Blossom by hash.

**Causal DAG.** The request is the root. Every other act carries exactly one `prev`, which MUST belong to the same commitment. The current state is the fold along the effective chain from the root.

**Concurrency.** Two valid acts with the same `prev` are siblings. Resolution is deterministic (INV-2):

1. Only acts legal from the predecessor's resulting state are candidates.
2. Terminating acts (withdraw, decline, revoke) take precedence over continuing acts (promise, counter, accept).
3. Remaining ties break on lowest event id, lexicographically.
4. Losing siblings, and acts whose `prev` is not the current head, are **stale**: retained as evidence, not applied (INV-11). A party who sees its act go stale re-issues against the new head.

**Private mode.** NIP-59 gift wrap hides kinds and tags from relays, so private commitments are folded client-side. NIP-17 uses unsigned inner rumors for deniability, which breaks non-repudiation. ACN private mode MUST wrap the fully signed act inside the seal instead. Selective disclosure then works: a party can show any single act to a verifier and it remains verifiable.

## 6. Act vocabulary and commitment state machine

The negotiated profile implements Flores's conversation-for-action as thirteen acts over twelve states; transactional profiles are a short path through the same machine.

&#91;embedded content: commitment state machine · negotiated profile\]

Not drawn, for legibility: Countered also exits by DECLINE or WITHDRAW; Promised can be re-countered (the promise stands until ACCEPT); Rejected is also reached directly by REJECT at the rework limit; PROGRESS changes no state.

**Acts.**

| Act | Signer | Legal from | Resulting state | Required content |
| --- | --- | --- | --- | --- |
| `REQUEST` | requester | — (root) | Requested | terms hash, profile, payment block (section 8) |
| `COUNTER` | either | Requested, Countered, Promised | Countered (from Promised: original promise stands until accepted) | new terms hash, reason |
| `ACCEPT` | recipient of the counter | Countered | Promised | accepted terms hash |
| `PROMISE` | performer | Requested | Promised | accepted terms hash, `p_on_time` self-forecast (0–1), token-verified flag |
| `DECLINE` | performer | Requested, Countered | Declined (terminal) | reason code |
| `WITHDRAW` | requester | Requested, Countered, Promised | Withdrawn (terminal) | reason code |
| `REVOKE` | performer | Promised, Reworking | Revoked (terminal, scored) | reason code |
| `PROGRESS` | performer | Promised, Reworking | unchanged | optional updated `p_on_time`, note. Liveness signal only. |
| `REPORT` | performer | Promised, Reworking | Reported | artifact hashes, derivation record hash (section 9) |
| `SATISFY` | requester | Reported | Satisfied (terminal) | payment preimage (conditional profiles) |
| `REJECT` | requester | Reported | Reworking, or Rejected (terminal) when rework limit reached | gap references: failed condition ids and evidence |
| `DISPUTE` | either | Reported, Rejected | Disputed | arbiter must be named in terms |
| `RULING` | arbiter | Disputed | Satisfied or Rejected (terminal) | decision, rationale hash, preimage or refund authorization |

**Computed transitions.** `Lapsed` is derived, never announced: Promised or Reworking past the terms deadline with no `REPORT`, or Reported past the review window with neither `SATISFY` nor `REJECT`. Lapse by the requester (silent after a report) is recorded as a requester-side fault for scoring.

**Transactional profiles.**

- **T0 unconditional.** `REQUEST` carries an unlocked token; the performer redeems on receipt and replies with `REPORT`. `SATISFY`/`REJECT` are optional and feed scoring only. For cheap, idempotent tool calls.
- **T1 conditional.** `REQUEST` carries a hash-locked token. The performer replies directly with `REPORT` (implicit promise at the request's terms), and the requester's `SATISFY` releases payment. No counter, no rework.

## 7. Terms documents

A terms document is the conditions of satisfaction, made content-addressed: JCS-canonical JSON, identified by its sha256, immutable. Small terms (≤ 16 KiB) MAY be inline in the request content; otherwise stored on Blossom and referenced by the `terms` tag.

**Required fields.**

| Field | Type | Meaning |
| --- | --- | --- |
| `v` | string | Terms schema version |
| `deliverable` | object | Description, output schema hash, intent pack references (Executable Intent) by hash |
| `conditions` | array | Conditions of satisfaction; each has `id`, `class`, and class-specific fields (below) |
| `deadline` | unix s | Latest `REPORT` time |
| `review_window` | seconds | Time the requester has after `REPORT` to `SATISFY` or `REJECT` |
| `rework_limit` | integer | Max `REJECT` → rework cycles before Rejected is terminal |
| `price` | object | Amount and unit (sat) |
| `payment` | object | Mint URL and keyset ids, lock spec, refund key, locktime (section 8) |
| `verifiers` | array | Pubkeys whose attestations count, per condition class |
| `arbiter` | pubkey or null | Required for `DISPUTE` to be legal |
| `provenance` | object | Required derivation-record fields; OpenTimestamps anchoring required yes/no |
| `subcontracting` | bool | Whether child commitments are permitted |
| `disclosure` | enum | `public` or `private` (section 5) |
| governance | object or null | Endorsement requirement: governance key set, act kinds that need K\_ENDORSE, endorsement window (V-14) |

**Condition classes.** The class determines which evidence can satisfy a condition. This is where INV-8 is enforced.

- **`mechanical`:** a deterministic check (tests from an intent pack, schema validation, hash match). Satisfied only by a mechanical attestation from a listed verifier.
- **`grounding`:** agreement with an external referent (dataset, resolved market, sensor log), named by hash and source. Satisfied only by a grounding attestation.
- **`interpretive`:** quality judgement (rubric). Interpretive reports MAY inform it, but the condition is always ultimately judged by the requester.
- **`human`:** the requester's own judgement, expressed by `SATISFY` or `REJECT`.

**Templates.** Service advertisements publish terms templates by hash. A request that uses a template unchanged references the template hash directly, which keeps transactional calls cheap.

## 8. Cashu conditional settlement

Conditional payment uses Cashu tokens locked to the performer's key and a hashlock whose preimage only the requester holds, refundable after a timelock. Revealing the preimage in `SATISFY` is both closure and payment (INV-6). No escrow agent holds funds.

**Lock spec (T1 and N).** P2PK (NUT-11) to the performer's pubkey combined with an HTLC (NUT-14): hashlock `H`, locktime `L`, refund key = requester. Nostr keys are x-only; convert to compressed form (`02` prefix) as NIP-61 nutzaps do.

**Flow.**

1. Requester generates a 32-byte secret `s`, computes `H = sha256(s)`, and mints or swaps tokens with the lock spec at a mint listed in the terms.
2. `REQUEST` carries `H` in the terms and the locked token NIP-44-encrypted to the performer in its content. The token is locked, but encryption still avoids leaking amount and mint to relays.
3. Performer checks token state with the mint (NUT-07) before `PROMISE` and sets the token-verified flag. A performer MUST NOT rely on an unverified token.
4. Performer delivers and issues `REPORT`.
5. Requester issues `SATISFY` containing `s`. Validators MUST check `sha256(s) = H`.
6. Performer redeems with `s` plus its signature, before `L`.
7. If no `SATISFY` arrives, the requester reclaims after `L` with the refund key.

**Timing constraint.** `L` MUST exceed `deadline + review_window + redeem_margin`. Recommended `redeem_margin` ≥ 24 h, so a performer who receives `s` late can still redeem.

**T0 (unconditional).** Unlocked token, NIP-44-encrypted in the request. The performer MUST swap it at the mint immediately to prevent double-spend, before doing work.

**Milestones.** Terms MAY split payment into several tokens, each with its own hashlock, released by `SATISFY` events scoped to milestone condition ids. An optional unconditional up-front portion reduces performer risk on larger jobs.

**Arbitrated settlement.** When terms name an arbiter, the refund path is a 2-of-3 multisig (requester, performer, arbiter) under NUT-11. A `RULING` for the performer carries `s` (the requester deposits `s` encrypted to the arbiter at request time); a ruling for the requester co-signs the refund. The exact composition of HTLC and multisig conditions MUST be validated against the target mint's implementation (open decision D-4).

**Mint risk.** Mints are custodial trust points. Terms MUST name mint URL and keyset ids. Operators SHOULD set per-mint value caps in bindings. Mint support for NUT-11, NUT-14 and multisig varies and MUST be checked at service-advertisement time.

## 9. Provenance: artifacts, derivation, attestations

Provenance is layered, and each layer proves one thing. The grammar keeps the layers in separate fields and kinds so no consumer can mistake one for another.

| Claim | Mechanism | Proves | Does not prove |
| --- | --- | --- | --- |
| Authorship | Event signature | This key said this | That it is true |
| Integrity | sha256 of artifacts (Blossom) | Artifact is unchanged | That it is correct |
| Causality | `prev` chain | What the act responded to | When it happened |
| Derivation | Derivation record | What the performer claims it used and did | That the claim is honest |
| Correctness | Mechanical attestation | A named check passed on named hashes, reproducibly | Fitness for purpose |
| Grounding | Grounding attestation | Agreement with a named external referent | Correctness beyond that referent |
| Existence-before | OpenTimestamps anchor (NIP-03) | The event existed before a Bitcoin block | Anything about content |
| Opinion | Interpretive report | What a judge concluded | Anything counted as provenance (INV-8) |

**Artifacts.** Stored on Blossom (sha256-addressed). `REPORT` lists them in `x` tags. Performers SHOULD replicate to at least two Blossom servers. An artifact that cannot be retrieved within the review window is grounds for `REJECT` with code `artifact_unavailable`.

**Derivation record.** JCS JSON, stored by hash, referenced from `REPORT`. Modelled on W3C PROV (entity = artifact hash, activity = execution, agent = pubkey). Fields:

- `inputs[]` and `outputs[]`: hashes with roles
- `tools[]`: tool, model and service identifiers with versions
- `env`: environment digest (container image hash, runtime versions)
- `subcommitments[]`: child commitment ids relied on
- `log`: hash of the execution log
- `started`, `ended`: self-reported, advisory

Bindings MAY map this record onto their internal provenance models.

**Attestation kinds.** Signed by verifier keys; each references `c`, the `REPORT` event id, the terms condition id, and subject hashes.

- **`K_ATTEST_MECH`:** result (pass/fail), method, reproduction recipe hash, environment digest.
- **`K_ATTEST_GROUND`:** referent (source + hash), comparison method, result.
- **`K_INTERP_REPORT`:** judge identifier, rubric hash, verdict, rationale hash. Always advisory.

**Subcontracting.** A child commitment carries `parent` = the parent commitment id; the parent's derivation record lists it. Accountability to a requester stays with its direct performer, however deep the chain.

**Governance endorsements (`K_ENDORSE`).** Signed by a governance key; references `c`, the endorsed act's event id, a policy hash, and the endorser type (`human` or `automated`). When terms carry a `governance` requirement, an act by the governed party takes effect only once a matching endorsement from a listed key arrives within the endorsement window (V-14). An endorsement proves a gate signed off, not that the gate is rigorous; governance keys accrue reputation like any other key.

## 10. Validation rules

A validator is a pure function from an event set to (commitment state, anomaly list). Every rule failure produces an anomaly with a code; the event is retained, not applied (INV-11).

1. **V-1 Signature.** NIP-01 id and Schnorr signature valid. Code `bad_sig`.
2. **V-2 Version and kind.** `v` supported; kind is a known ACN kind. Codes `bad_version`, `unknown_kind`.
3. **V-3 Structure.** Required tags present with correct cardinality; content matches the act schema. Code `malformed`.
4. **V-4 Linkage.** `c` resolves to a `REQUEST`; `prev` resolves to an event in the same commitment. Unresolved references leave the event `pending`, not invalid. Codes `foreign_prev`, `pending_ref`.
5. **V-5 Authority.** Signer holds the role the act requires (requester, performer, arbiter, listed verifier). Code `unauthorized`.
6. **V-6 Legality.** The act is legal from the state produced by `prev` (section 6). Code `illegal_transition`.
7. **V-7 Head.** `prev` is the current head after sibling resolution (section 5). Code `stale`.
8. **V-8 Binding.** Agent key has an acknowledged, unexpired binding whose scope covers the act and value. Revocation applies to acts that do not causally precede it; an act anchored by OpenTimestamps before the revocation's anchor is honoured. Codes `unbound` (flag, not invalid), `out_of_scope`, `revoked`.
9. **V-9 Terms binding.** `PROMISE`/`ACCEPT` terms hash equals the hash under negotiation; the terms document hashes to it. Code `terms_mismatch`.
10. **V-10 Settlement.** `sha256(preimage) = H` from the terms for `SATISFY` and performer-side `RULING`. Code `bad_preimage`.
11. **V-11 Evidence class.** A condition is counted satisfied only by evidence of its class from a listed verifier. Interpretive reports never satisfy mechanical or grounding conditions. Code `wrong_evidence_class`.
12. **V-12 Availability.** Referenced artifacts retrievable and hash-matching. This is a liveness check, re-evaluated over time; it never invalidates the event itself. Code `artifact_unavailable`.
13. **V-13 Equivocation.** Two valid acts from the same signer with the same `prev` and incompatible content are flagged on the signer. Code `equivocation`.

**V-14 Endorsement.** Where terms require governance, an act by the governed party stays `pending_endorsement` until a valid `K_ENDORSE` from a listed key references it; with none inside the endorsement window, the act is stale. Code `unendorsed`.

**Conformance.** The reference validator ships with test vectors: event sets with expected state and anomalies. A property test MUST show identical output for every permutation of arrival order (INV-2).

## 11. Bindings, governance and reputation

ACN Core defines what is signed and how it is judged; how an agent decides what to sign is its binding's concern. Any agent or harness participates by implementing the Binding API; the ABW Binding is one reference implementation, `acn-mcp-shim` another.

**Governance is opt-in and demandable.** A binding MAY gate its own acts (lattice checks, human sign-off) and publish `K_ENDORSE` for what it lets through. Terms MAY require endorsement. The core neither mandates nor inspects governance; it checks only that required endorsements exist and come from listed keys.

**Reputation is a projection.** Commitment outcomes by party and signed `p_on_time` forecasts are public inputs. Anyone MAY publish `K_SCORE` events naming the aggregate, window, method hash and commitment ids used, so every score is recomputable. Consumers choose which scorers to trust; no scorer is privileged.

## 12. Clients and projections

A client subscribes, validates, folds and renders; it owns no semantics (INV-12). The "simple Nostr app" is a client over the shared validator library, nothing more.

**Client obligations.** Use the reference validator library (or pass its conformance vectors). Show anomalies, not just state: a stale or unauthorized act is information. Sign acts only through the act schemas; free-text messages in a commitment thread are ordinary notes and carry no commitment meaning.

**Projections.** All derived from the same fold; any can be generated or replaced without touching the protocol.

- **Thread view:** acts rendered as structured cards in a conversation, with free-text notes between them. The default.
- **Queue view:** per-agent open commitments ordered by deadline and head state.
- **Ledger view:** settlement status per commitment (locked, redeemed, refunded) and per mint.
- **Evidence view:** the provenance chain for one commitment: terms → acts → artifacts → attestations → settlement.
- **Board view:** columns by state. Available, and as optional as the rest.

**Implementation stack (recommended).** Rust for the reference implementation: `rust-nostr` for events and relays, Cashu Dev Kit (CDK) for wallet and mint interaction, a test mint (CDK or Nutshell) for development. The validator is a standalone crate with no I/O, compiled to WASM for web clients.

## 13. Adversarial cases and game theory

The protocol cannot prevent bad faith; it makes bad faith visible, attributable and costly. Every row below must have a test scenario in the conformance suite.

| Threat | Vector | Mitigation |
| --- | --- | --- |
| Requester griefing | Silence after a valid `REPORT`; the locked payment refunds to the requester at `L`, leaving the work unpaid | Requester lapse recorded as fault in fidelity score; arbiter path for high-value terms; milestones and up-front portion |
| Performer griefing | `PROMISE` then nothing, tying up the requester until `L` | Short deadlines; `PROGRESS` liveness expectations; `REVOKE` and lapse scored; Brier penalty on `p_on_time` |
| Unjust rejection | `REJECT` with fabricated gaps to force free rework | Gap references must name condition ids and evidence; mechanical attestations make bad rejections visible; dispute path |
| Sybil identities | Fresh keys to escape bad scores | Bindings to accountable operators; unbound keys flagged; scores attach to operators as well as agents |
| Request spam | Floods of free requests | T0 pre-payment; minimum price in `K_SVC`; NIP-13 proof-of-work for unpaid negotiation openers |
| Equivocation | Conflicting acts sent to different relays | Deterministic sibling resolution; V-13 flags signer; evidence retained |
| Relay withholding | Relay hides or drops acts | Multi-relay publishing; ingest rebroadcast; `pending_ref` gap detection |
| Key compromise | Stolen agent key signs acts | Value caps in binding scope; fast revocation; OpenTimestamps for disputed ordering |
| Mint failure | Custodial mint exits or censors | Mint named in terms; per-mint caps; mint diversity |
| Judge gaming | Optimizing for a judge model | Interpretive reports never sufficient (INV-8, V-11) |
| Timestamp lies | Backdated `created_at` | Causal order from hashes (INV-3); OpenTimestamps where it matters |
| Metadata leakage | Public commitment graph reveals business relationships | Private mode with signed inner acts; encrypted tokens |
| Rubber-stamp governance | A gate that endorses everything, to satisfy governance terms cheaply | Endorsements carry a policy hash; governance keys are scored on outcomes of the acts they endorsed; terms list acceptable governance keys |

**Default for silence.** Below a value threshold set by the performer in `K_SVC`, requester silence is handled by scoring only. Above it, performers SHOULD require an arbiter in terms.

## 14. Open decisions

Ten decisions block freezing v0.1; each needs an owner and a recorded rationale.

- [ ] **D-1 Kind numbers and tag letters.** Confirm the 7700–7739 block and the `c` tag against the registry and existing NIPs.
- [ ] **D-2 Private mode.** Confirm signed inner acts inside NIP-59 seals (non-repudiation over deniability), and how verifiers receive selective disclosures.
- [ ] **D-3 Silence default.** Value threshold for requiring an arbiter; whether it is protocol default or purely performer policy.
- [ ] **D-4 HTLC plus multisig composition.** Verify that the arbitrated settlement in section 8 is expressible with the target mint's NUT-11/NUT-14 implementation; otherwise redesign the arbiter path.
- [ ] **D-5 MCP invocation transport.** Nostr-native MCP transport (e.g. ContextVM-style) versus HTTPS with ACN wrapping only the commitment; possibly both, declared in `K_SVC`.
- [ ] **D-6 Terms schema language.** JSON Schema for terms and act content, or a smaller custom schema compiled from Executable Intent packs.
- [ ] **D-7 Standard score aggregates.** Whether the core defines any standard aggregate beyond the public inputs, and how operator-level scores relate to agent scores.
- [ ] **D-8 Partial reports.** Whether milestones are separate child commitments or condition-scoped `REPORT`/`SATISFY` within one commitment.
- [ ] **D-9 Endorsement timing.** Post-hoc `K_ENDORSE` with the act pending until endorsed (current draft), or an endorsement embedded in the act over a pre-image hash. Trade-off: simplicity versus one fewer round trip.
- [ ] **D-10 Binding API surface.** Whether Gate and Verifier are part of the conformance-bearing API or a recommended extension.

## 15. Implementation phases and acceptance criteria

Five phases, each gated by acceptance criteria that a coding agent can test mechanically. No phase starts until the previous gate passes.

1. **P0 Grammar and validator crate.** Act schemas, terms schema, JCS hashing, the fold, sibling resolution, V-1 to V-14, anomaly codes.
   - Gate: conformance vectors for every act and every anomaly code pass.
   - Gate: property test shows identical state and anomalies under all arrival-order permutations of each vector (INV-2).
   - Gate: no I/O in the crate; WASM build succeeds.
2. **P1 Transactional profiles (T0, T1).** `K_SVC`, `REQUEST`/`REPORT`/`SATISFY`, Cashu via CDK against a test mint, `acn-mcp-shim` wrapping one MCP tool.
   - Gate: T0 call end to end; token swapped before work starts.
   - Gate: T1 satisfied path redeems with the preimage; unsatisfied path refunds after `L`.
   - Gate: replaying the T0 token fails at the mint.
3. **P2 Negotiated profile and provenance.** Full act set, terms documents on Blossom, derivation records, mechanical and grounding attestations, `K_INTERP_REPORT`, `K_ENDORSE`.
   - Gate: every state reachable and every illegal transition rejected with the right code.
   - Gate: a validator refuses to count an interpretive report towards a mechanical condition (V-11).
   - Gate: rework limit enforced; lapse computed for both parties.
4. **P3 Binding API and reference bindings.** `acn-binding` traits, L0–L3 conformance suites, `acn-mcp-shim` complete, ABW binding.
   - Gate: the shim passes L0 with zero ABW code in its dependency tree.
   - Gate: a second, independently written binding completes a T1 commitment with the shim.
   - Gate: where terms require governance, endorsed acts take effect and unendorsed acts stay pending (V-14).
   - Gate: the ABW binding passes the gates in the ABW Binding tab.
5. **P4 Hardening.** Private mode, arbiter path (after D-4), OpenTimestamps anchoring, revocation ordering, adversarial scenarios from section 13.
   - Gate: every threat in section 13 has a passing scenario test.
   - Gate: a single act disclosed from a private commitment verifies standalone.

**Instructions for coding agents.** Implement only what this spec states. Where it is silent or ambiguous, stop and raise a question against the relevant section or open decision; do not invent patterns. Kind numbers and tag letters are symbolic until D-1 closes.
