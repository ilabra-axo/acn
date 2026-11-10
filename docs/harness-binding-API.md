# ACN Binding API

Any agent or harness joins ACN by implementing a small set of interfaces; ABW is one implementation, not a prerequisite.

Status: **proposal** for open decision D-10 ([Core.md](Core.md) §11, §14), aligned with spec v0.5 (CR-001…CR-003 Phases A–B). This document defines the interfaces, the lifecycle and the conformance levels a binding implements. [binding-api.md](binding-api.md) is its companion: what the reference bindings do today, the settlement policy filters, and worked examples. Neither adds protocol: every act is defined in Core.md, and every check is the validator's. Where this document and Core.md differ, Core.md wins.

## 1. Interfaces

A binding implements up to eight interfaces. Signer and Inbox are always required; Executor and Evidence for performers; Wallet whenever value moves. Gate, Verifier and Terms Author are optional, and they are where harness-specific rigour plugs in.

| Interface | Required | Responsibility |
| --- | --- | --- |
| Signer | always | Key custody and signing of acts. Enforces binding scope locally. Routes every act draft through the Gate if one is present. Keeps the party's per-commitment secrets (below). |
| Inbox | always | Relay subscriptions for commitments involving the agent; feeds events to the shared validator; emits state changes and anomalies. |
| Executor | performers | Maps a commitment to work (MCP tool call, harness task) and returns results. |
| Evidence | performers | Stores artifacts by hash (Blossom), builds derivation records, replicates. Under `contingent`, seals the deliverable under the key `k` (NIP-44 v2, Core.md §8.2). |
| Wallet | when value moves | Cashu operations: lock, verify, swap, redeem, refund, and reading a revealed preimage from a spent token (NUT-07). |
| Gate | optional | Pre-signing policy hook: allow, deny, escalate to a human, or allow with endorsement. Emits `K_ENDORSE`. |
| Verifier | optional | Consumes a `REPORT` and terms; emits attestations, with diagnostics by reference. As a settlement verifier, also acknowledges escrow and releases payment. |
| Terms Author | optional | Drafts and critiques terms documents. Entirely off-protocol; its output is a terms hash like any other. |

**Secrets the Signer keeps** (each revealed only by the act Core.md names):

| Party | Secret | Revealed by |
| --- | --- | --- |
| Requester | payment preimage `s` (requester and verifier triggers) | `SATISFY`; NIP-44 `escrow` tag to the settlement verifier or arbiter at `REQUEST` |
| Requester | requester seed (held-out instances) | NIP-44 `seed` tag to the runner at `REQUEST`; the runner reveals it in its attestation |
| Performer | key `k` (contingent) | NIP-44 `escrow` tag to the settlement verifier with the `REPORT`; `CLAIM` (and the mint, NUT-07) |
| Performer | performer seed (held-out instances) | its hash in the binding act (`PROMISE`, `ACCEPT`, or its `COUNTER`); the seed in `REPORT` |

**Operations (logical signatures).** Language-neutral; a future reference crate `acn-binding` would express them as Rust traits (not yet built; the reference bindings implement them directly).

- `Signer.sign(act_draft) → SignedEvent | Refused(reason)`; `Signer.pubkey()`; `Signer.binding() → BindingRef`
- `Inbox.subscribe(filters)`; `Inbox.on_event(event) → StateChange{commitment, from, to, anomalies[]}`
- `Executor.quote(terms) → Accept(p_on_time?) | Counter(terms') | Decline(reason)`; `Executor.run(commitment) → stream<Progress>, Result{artifacts[]}`
- `Evidence.put(bytes) → hash`; `Evidence.derivation(record) → hash`; `Evidence.replicate(hash, n)`; `Evidence.seal(bytes, k) → (sealed, ciphertext_hash)`
- `Wallet.capabilities(mint) → [capability]`; `Wallet.lock(amount, mint, lock_spec) → token`; `Wallet.verify(token) → TokenState`; `Wallet.redeem(token, preimage?)`; `Wallet.refund(token)`; `Wallet.revealed_preimage(token) → preimage?`
- `Gate.evaluate(act_draft, state, context) → Allow | Deny(reason, diagnostics_ref) | Escalate(to, deadline) | Endorse(endorsement_draft)`
- `Verifier.attest(report, terms, item_id) → Attestation{kind, result, diagnostics_ref?}`, where `item_id` names a condition or a disqualifier; `Verifier.ack_escrow(commitment, secret) → K_ESCROW_ACK`; `Verifier.release(commitment) → RELEASE`
- `TermsAuthor.draft(goal, history?) → TermsDoc`; `TermsAuthor.critique(terms) → Findings` (reference: `acn_author::draft_request` and `acn_author::lint`, rules L1–L8 of CR-003 §3; on the performer side `acn_author::draft_counter` is what `Executor.quote` returns as `Counter`)

**What each operation must respect** (the validator checks the acts; the binding should never produce one it would refuse):

- `Executor.quote` may return `Accept` only for terms whose every condition and disqualifier is closed (V-26). For terms with an `open` condition it returns `Counter` with the condition closed (CR-003 §2.4: the performer drafts what it will be held to) or `Decline`. It declines terms it can't meet: an unsupported settlement trigger, profile or mint capability, held-out instances it can't seed, or a condition it has no evidence producer for.
- With held-out instances, the binding generates a 32-byte performer seed, puts `performer_seed_hash` in its binding act, and reveals `performer_seed` in its `REPORT` (V-27). A `REPORT` without it is not applied.
- `Verifier.attest` result: `pass` or `fail` for a condition, `hit` or `clear` for a disqualifier (V-25), signed only by a key the terms list for that item. As a judge runner it emits `K_ATTEST_JUDGED` with every run's score and the recomputable aggregate (V-17). For an item with instances it reveals the requester seed and exactly the derived sample, each item with its RFC 6962 proof (V-27). Under `contingent` it attests the decrypted plaintext and names the `REPORT`'s ciphertext and hashlock (V-21), never revealing `k` or the plaintext.
- `Verifier.ack_escrow` and `Verifier.release` are for the settlement verifier (verifier trigger, Core.md §8.2): acknowledge the escrowed `s` against the terms' hashlock (V-19), and release it only when every condition passes and no disqualifier is hit (V-20).
- `Wallet.capabilities` reads the mint's declared capabilities (`K_SVC`, Core.md §8.6). A requester binding checks `reveals_preimage_to_payer` before agreeing to `contingent` (`rail_incapable` otherwise; a binding check, not a validator code).

**Rules.**

1. A Signer with a Gate MUST NOT sign an act the Gate denied or has not yet decided. An escalation that times out is a deny.
2. Gate decisions are local records; only endorsements are published. A binding MAY keep its deny and escalation history private.
3. `Executor.quote` returning `Counter` is how a binding improves a requester's conditions of satisfaction on-protocol: it proposes better-specified terms through an ordinary `COUNTER`, closing open conditions and adding disqualifiers. The version binds only after the other party's `ACCEPT`.
4. Verifier results are binary per item (`pass`/`fail`, `hit`/`clear`). Judged run scores and instance proofs are protocol content, because the validator recomputes them; rationales and other diagnostics travel as a hash reference, never as protocol state.
5. A binding folds with the reference validator before signing, so a refusal is local (`Refused`, `Decline`) rather than a rejected act on the wire.
6. Which settlement triggers a party accepts is local policy (settlement policy filters, [binding-api.md](binding-api.md)); the protocol only checks that the terms are a valid combination (V-18).
7. A requester binding MUST NOT `SATISFY`, or auto-satisfy, after a counted disqualifier hit. The validator would still apply it (D-38); the obligation is the binding's.

**Other roles.** An arbiter binding needs Signer and Inbox: it receives the escrowed `s`, may acknowledge it (`K_ESCROW_ACK`), and signs `RULING` (Core.md §8.4). A governance key is a Gate run as its own binding.

## 2. Lifecycle and conformance levels

Every act a binding signs passes the same path: draft, validator check, optional gate, sign, publish. A performer's negotiated commitment runs as follows.

1. Inbox receives `REQUEST`; the validator folds it to Requested.
2. `Executor.quote(terms)` returns Accept, Counter or Decline (Accept only for closed terms).
3. The binding drafts `PROMISE` (or `COUNTER` / `DECLINE`), adding `performer_seed_hash` when the terms have held-out instances. First: under the requester and verifier triggers, `Wallet.verify` checks the hash-locked token in the `REQUEST`; under the verifier trigger, the binding also waits for the settlement verifier's `K_ESCROW_ACK` (V-19). Under `contingent` there is no token yet.
4. `Gate.evaluate` on the draft, if a Gate exists. Endorse → the Signer signs and the binding publishes the act plus `K_ENDORSE`.
5. `Executor.run` does the work; `PROGRESS` acts pass through the Gate under the same policy.
6. `Evidence.put` and `Evidence.derivation` produce hashes; the binding drafts `REPORT` (with `performer_seed`, if any), which is gated and signed. Under `contingent`, `Evidence.seal` encrypts the deliverable under a fresh `k`, the `REPORT` carries the ciphertext hash and `H = sha256(k)`, and `k` is escrowed with the settlement verifier.
7. Settlement:
   - requester trigger: Inbox receives `SATISFY` with the preimage (or an arbiter's `RULING`); `Wallet.redeem` collects payment;
   - verifier trigger: Inbox receives `RELEASE` with the preimage; `Wallet.redeem`;
   - contingent: Inbox receives `FUND`; `Wallet.verify` checks its token against the offer's hashlock; `Wallet.redeem` with `k`, then `CLAIM` with `k`.

   On `REJECT` the commitment is Reworking (loop back to step 5) or Rejected, which is final without an arbiter, at the rework limit, or after a disqualifier hit. A Rejected commitment with an arbiter may be disputed (`DISPUTE`).

Requester-side bindings mirror this:

1. `TermsAuthor` (optional) drafts the terms; settlement policy filters and `Wallet.capabilities` checks run before anything is locked.
2. The binding generates `s` (requester and verifier triggers) and, for held-out instances, a requester seed; it locks the price to the performer and `H = sha256(s)` and signs `REQUEST` with the NIP-44 `escrow` and `seed` tags its terms need.
3. It answers counters (`ACCEPT`, `COUNTER`, `WITHDRAW`), showing the terms card first (Core.md §12).
4. After `REPORT`, `Gate` plus `Verifier` results come before `SATISFY` or `REJECT`. Under the verifier trigger it doesn't satisfy; the settlement verifier releases (a later `SATISFY` or `REJECT` is a recorded judgement). Under `contingent` it signs `FUND` once every condition passes, then reads `k` from `CLAIM` or `Wallet.revealed_preimage` and decrypts.
5. If nothing settles, `Wallet.refund` after the token's locktime.

**Conformance levels.** Declared in `K_SVC` as `binding_level` (proposed: not yet in Core.md §4, and not yet published by the reference bindings). Each level includes the ones below it.

| Level | Profiles | Interfaces | Proves |
| --- | --- | --- | --- |
| L0 Transactional | T0, T1 | Signer, Inbox, Executor, Evidence; Wallet when priced | Can sell a microservice call; declines terms it can't meet |
| L1 Negotiated | + N | as L0 | Handles counter (closing open conditions), rework, dispute; held-out seeds; disqualifiers |
| L2 Governed | + endorsed acts | + Gate emitting `K_ENDORSE` | Can meet terms that demand governance |
| L3 Verifier | any | + Verifier | Can be listed as a verifier, judge runner or settlement verifier in terms |

The settlement triggers a binding supports (requester, verifier, contingent) are orthogonal to the level: a binding states them as local policy and declines the others.

Each level will have a conformance suite run against the reference validator; passing it is the only claim a binding may make about its level. The suites don't exist yet. Today the reference validator's own conformance scenarios (`crates/acn-core/tests`) are the gate, and the demo cast (`crates/acn-sim`, `crates/nmesh-acn-sim`) exercises every act end to end.

## 3. Minimal reference binding: `acn-mcp-shim`

The shim turns any existing MCP server into an L0 ACN performer with no ABW dependency. It is the proof that the core is harness-neutral, and it ships alongside the ABW binding. It lives in `crates/nmesh-agent-bridge` (`src/shim.rs`).

| Interface | Target | Today |
| --- | --- | --- |
| Signer | Local key file or OS keychain; binding to the operator key created at install | Local key file; operator binding issued and acknowledged at start if missing (`operator_key_file`), scoped to performer kinds and `max_sat` |
| Inbox | `rust-nostr` subscription on configured relays, reference validator | As target (NIP-17 private acts; `acn_core::fold` decides every move) |
| Executor | MCP client calling the wrapped server's tools; one terms offer per tool (`K_SVC` `terms_template`, Core §7), generated from its input/output schemas | MCP client; `K_SVC` lists each tool with its schema hashes, but no terms offers: requesters send `tool_call_terms` |
| Evidence | Tool output written to Blossom; derivation record with tool name, server version, input hash, output hash | Tool output inline in the `REPORT` (≤ 16 KiB), validated against its nmesh contract when it has one; no Blossom, no derivation record |
| Wallet | CDK wallet against the mints listed in `K_SVC` | None: free T0 only (`payment_unsupported` otherwise; `K_SVC` lists no mints) |
| Gate | None (always allow) | None |
| Verifier | None | None |
| Terms Author | None | None in the shim; the reference implementation is `acn-author` (drafters and linter), used by the demo performer and `nmesh-app` |

The shim declines what it can't meet: settlement triggers other than `requester` (`trigger_unsupported`), profiles other than T0, priced terms, expired deadlines, unknown tools, terms with an `open` condition (`terms_unclosed`) and terms with held-out instances (`instances_unsupported`).

**Acceptance.** The shim passes the L0 conformance suite with zero ABW code in its dependency tree. A second shim variant wrapping an arbitrary non-MCP agent harness (any CLI agent that takes a task and returns files) demonstrates that MCP is a convenience, not a requirement. Neither the suite nor the second variant exists yet; the shim's end-to-end test is `crates/nmesh-agent-bridge/tests/app_flow.rs`.
