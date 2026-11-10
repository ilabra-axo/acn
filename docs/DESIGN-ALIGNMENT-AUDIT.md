# Design-alignment audit

**Date:** 2026-11-10  
**Source snapshot:** `825b82ea8263ca5106c67df1a245c235c6d78f33`, plus the pre-existing untracked Hinkal evaluation. Source line references refer to this snapshot.  
**Method:** Read-only source review and bounded existing tests, with parallel reviews of protocol/bindings, wallets, app/renderers and judges. One frontend conversation race was reproduced with an in-memory probe against the compiled store. No application source or configuration was changed. This report is not a comprehensive security certification, cryptographic audit or independent validation of external providers.

## 0. Remediation status (2026-11-11)

Every finding has a fix and a regression test on branch `audit-remediation` (merged to `master`). Each test was written to fail on the unfixed code; where a fix needed a new API, the test exercises the new path. Full workspace tests, clippy, the desktop type check, the Tauri build and the browser suites pass. Spec and docs changes: Core.md Appendix B row "Design-alignment audit"; [harness-binding-API.md](harness-binding-API.md) rules 8–15; [ARCHITECTURE.md](ARCHITECTURE.md) §5.5, §6.2, §8, §9, §10; specs 0002–0004.

| ID | Fix | Regression |
| --- | --- | --- |
| A01 | `acn_binding_nostr::run_with`: one admission policy (allowlist, `acn.enabled`) for history and live; `Performer::with_admission` | `startup_applies_allowlist_and_acn_switch`, `admission_applies_to_replayed_history` |
| A02 | `acn_binding::journal`: intent recorded before the executor runs; an interrupted intent is never re-run (`execution_uncertain` unless our act is in history); replayed plain commands not re-executed; by-reference terms resolved during replay | `restart_does_not_reexecute`, `interrupted_run_is_not_rerun`, bridge `restart_does_not_repeat_work` (MCP calls counted) |
| A03 | `Store` verifies before inserting and quarantines rejects; `nmesh_app::commitments::admit` likewise | `corrupt_copy_does_not_replace_a_verified_report`, `a_corrupt_copy_never_replaces_a_verified_act` |
| A04 | Binding and revocation events in every performer fold, refreshed; `Performer::authority` before work and signing; a revoked binding isn't re-issued | `request_above_operator_scope_is_not_worked` (declined `out_of_scope`, no work; also another mint), `revoked_binding_stops_work` |
| A05 | V-8 against the terms the act commits to; `DECLINE`/`REVOKE` carry no value; V-8 stays performer-only (Core §10) | SC-1…SC-7b |
| §3 qualification | Candidate folded and required to be applied before it is sent; `governance_unsupported`; priced tools refuse direct commands | `governance_is_declined_before_work`, `paid_tools_refuse_direct_commands` |
| A06 | `lock_policy` reads every proof's NUT-10 tags; `check_lock` requires the complete §8.1 policy (one receiver, one refund key = requester, `n_sigs` 1, `SIG_INPUTS`, locktime); `check_payment_from` used by the driver, the cast (request and `FUND` tokens) and wasm; named keysets enforced | `inspect_rejects_hostile_tokens`, `every_policy_deviation_is_rejected`, `keysets_named_by_the_terms_are_enforced` |
| A07 | Repeated Y or secret rejected before summing; checked arithmetic; state reply must match the proofs one-to-one | `inspect_rejects_hostile_tokens` (duplicates in one and in separate groups) |
| A08 | Signed act journaled before send, stored only after send, resent verbatim; redemption recorded after success, retried with backoff, ambiguous outcomes marked for reconciliation; the cast clears its in-flight mark on failure | `failed_send_is_retried_with_the_same_act`, `failed_redemption_is_retried` |
| A09 | Refund in prepare / submit / finish with NUT-13 outputs and a persisted intent; `recover_refunds` on open (NUT-09) | `refund_recovers_after_interruption` |
| A10 | `App::recover_delivery_keys` reads `k` from the mint using the wallet's own `FUND` token; persisted; projection checks it against the hashlock | `withheld_claim_key_is_recovered_from_the_mint` (incl. restart) |
| A11 | Stale loads dropped; renderer hosts bound to their message's conversation; backend refuses a target outside that conversation; send results only inserted into the conversation they belong to | `app_flow` (stray target refused); store changes in `store.svelte.ts` |
| A12 | `Discovery` pending / resolved / unresolved; send runs discovery first; no profile = needs acceptance; agents never downgraded | `discovery_tests`, `app_flow` (no-profile key gated) |
| A13 | Fetch only after V-1 and only in accepted conversations; `FetchPolicy` (public addresses with pinned DNS, no redirects, time and size bounds, one at a time); configured servers trusted | `untrusted_fetches_are_bounded` |
| A14 | `acn-judge/2`: exactly the pinned `model_id` requested, a different reported model fails the run; all request parameters fingerprinted; strict typed replies, injection needs a span; honest descriptor; templates re-pinned | `replies_are_parsed_strictly_and_bounded`, `descriptor_reply_schema_matches_the_parser`, `provider_model_must_be_the_pinned_one`, `every_request_parameter_is_in_the_fingerprint`, `malformed_replies_and_other_models_fail_closed` |
| A15 | Evidence counts only up to the deciding act's `created_at` (`RELEASE`, `REJECT`, `premature_fund`) | C-6g, S-5b, S-7c |
| A16 | Evidence `private` by default, uploaded only after gate and fold when `public`; card images load on click, hash-pinned, natively, bounded | `gate_denial_uploads_nothing`, `private_disclosure_uploads_nothing`; `Card.svelte` / `App::fetch_image` |
| A17 | Host confirmation (`ActionConfirm.svelte`) with the actual command, values and recipient for every renderer action | `screenshots.mjs` (05a) |
| A18 | Paid by-reference L0 case fully checked; `payment_invalid` required; execution counts; L0 not claimed without public evidence and an operator binding. Lint of unparseable terms is an `L0` error. Contractless data marked not validated; `alt` required. App stops before publishing if it can't save `s`; orphan locks refundable by lock id; refunds net of fees; mint capabilities observed (NUT-06) before locking. ARCHITECTURE claims corrected | conformance suites, `unparseable_terms_are_an_error_not_silence`, `alt_is_required`, `refund_pays_input_fees` |

**Residuals (by design or acknowledged):** the judge provider is trusted to run the named model and `model_version` is a claim; the demo cast keeps no journal; the action prompt can't know what a command does; refund-key comparison needs `check_payment_from` (all reference callers use it); real mints, backup and encrypted secrets remain roadmap items. This remediation does not certify the system for real value: the §1 recommendation still applies until those items are done and independently reviewed.

## 1. Executive conclusion

**The architecture is aligned; several implemented safety guarantees are not.** The separation between pure protocol semantics, transport adapters and presentation is present. Signed ACN acts, shared projections, schema/hash validation, encrypted DMs and the canonical test-mint payment flow are implemented and exercised by tests.

However, authorization, effect idempotence, incoming-payment verification, recovery and privacy boundaries have material gaps. Passing the current happy-path and conformance suites does not establish the stronger guarantees in the design documents.

**Recommendation:** continue as a test-sats prototype. Do not expose unrestricted side-effecting MCP tools to untrusted callers or authorize real-value wallet use on the strength of current conformance claims. Hinkal remains an unfunded evaluation until the relevant authorization, replay, evidence and recovery findings are resolved or independently contained by an enforcing adapter.

### Authority and evidence

- Intended behavior: [DESIGN](DESIGN.md), [Core v0.5](Core.md), [Binding API](harness-binding-API.md), [wire specifications](spec/), and accepted [ADRs](adr/).
- [ARCHITECTURE](ARCHITECTURE.md) describes implemented behavior; [ROADMAP](ROADMAP.md) distinguishes acknowledged future work.
- **P1:** can invalidate a claimed authorization/payment/privacy guarantee or cause significant duplicate effects, data disclosure or lost access to funds.
- **P2:** material hardening, recovery, interoperability or assurance gap with narrower prerequisites.
- Unless explicitly marked reproduced, findings are **source-derived**. Tests passing below do not reproduce or disprove the failure sequences. Some findings need dedicated adversarial regression tests before exact operational reachability is established.

## 2. Priority summary

| ID | Priority | Mismatch | Evidence |
| --- | --- | --- | --- |
| A01 | P1 | Startup ACN bypasses sender allowlist and enablement checks | Source-derived |
| A02 | P1 | Replay reconstruction is not effect-idempotent | Source-derived |
| A03 | P1 | Unverified same-ID acts overwrite valid binding history | Source-derived |
| A04 | P1 | Performer folds omit operator binding/revocation context | Source-derived |
| A05 | P1 | Negotiated terms are checked against the original price/mint for V-8 | Source-derived |
| A06 | P1 | Incoming HTLC verification does not establish the required full spending policy | Source-derived |
| A07 | P1 | Payment amount accounting does not reject duplicate proofs | Source/dependency review; dedicated regression needed |
| A08 | P1 | Publication/redemption completion is recorded before success | Source-derived |
| A09 | P1 | Manual refund has an in-flight recovery-material loss window | Source-derived |
| A10 | P1 | Requester does not recover contingent delivery keys without CLAIM | Source-derived |
| A11 | P1 | Conversation-switch race misroutes actions | Frontend store probe reproduced |
| A12 | P1 | Consent fails open during incomplete discovery | Source-derived |
| A13 | P1 | Sender-controlled terms hints trigger unrestricted native fetches | Source-derived |
| A14 | P1 | Judge identity and response handling do not enforce their advertised pins/schema | Source-derived; permissive parser is covered by an existing passing test |
| A15 | P1 | Late disqualifier evidence can affect an earlier RELEASE decision | Source-derived; dedicated protocol vector needed |
| A16 | P2 | Raw tool evidence and remote images cross privacy boundaries | Source-derived |
| A17 | P2 | Renderer actions lack the promised host confirmation boundary | Source-derived |
| A18 | P2 | Conformance, authoring and financial recovery checks overstate their evidence | Source-derived |

## 3. Authorization and effect execution

### A01 — Startup bypasses authorization switches

**Code:** [bridge](../crates/nmesh-agent-bridge/src/lib.rs), lines 243–254 versus 276–290.

The startup loop ingests every historical ACN act and unconditionally drives stored commitments. It does not apply the sender allowlist or `acn.enabled` checks used by live handling. An otherwise valid historical request can therefore reach execution despite either restriction.

**Intent:** ARCHITECTURE §5 describes allowlist enforcement and ACN enablement; the Binding API lifecycle requires a validated request before execution.

**Alignment work:** apply the same pre-execution authorization policy to startup and live routes. Preserve legitimate own history without treating it as new executable authority. Test non-owner startup requests and disabled ACN explicitly.

### A02 — Restart reconstruction is not effect-idempotent

**Code:** [bridge](../crates/nmesh-agent-bridge/src/lib.rs), lines 243–254 and 345–369; [session](../crates/nmesh-core/src/session.rs), lines 277–306; [binding run loop](../crates/acn-binding-nostr/src/lib.rs), lines 194–206.

Historical non-ACN messages are sent through normal command/action handling and can invoke MCP again. Inbox deduplication is per subscription, not durable. For ACN, a 1.5-second idle interval is treated as sufficient history: an earlier request may execute before its historical report arrives. A process-local busy set does not protect restarts or multiple instances.

**Intent:** ARCHITECTURE §5 claims restart/replay idempotence. Relay-only history is acknowledged debt, but does not supply effect idempotence.

**Alignment work:** durable execution identities and an execution journal independent of report arrival. Record intent before executing; reconcile ambiguous results. Replay must reconstruct state without automatically repeating historical commands. Also resolve by-reference terms on startup rather than merely ingesting their events.

### A03 — Invalid copies overwrite valid history before validation

**Code:** [binding store](../crates/acn-binding/src/lib.rs), lines 222–237 and 342–349; [embedded-act decoding](../crates/nmesh-core/src/message.rs), lines 81–84.

`Store::add()` replaces entries using the incoming act's claimed ID before verifying its hash/signature. A corrupted copy of a known report can replace that valid report; the fold then rejects the corrupted copy and no longer sees the valid original. If the request is again actionable, execution can repeat. This requires a sender able to deliver a private act; it does not require forging the original signature.

The app has a related replace-before-validation pattern in [on_act](../crates/nmesh-app/src/lib.rs), lines 1449–1454, although its effects are projection/state corruption rather than performer execution.

**Intent:** Core INV-11/V-1: invalid events are not applied; forged input must not remove previously valid state.

**Alignment work:** verify identity/signature before inserting into the authoritative ID-indexed store; quarantine rejected input separately. Test valid report → corrupt same-ID input → unchanged effective state and execution count.

### A04 — Performer folds omit operator authority context

**Code:** [binding](../crates/acn-binding/src/lib.rs), lines 222–247, 423–424 and 680–683; [Nostr binding setup](../crates/acn-binding-nostr/src/lib.rs), lines 124–159.

Operator binding events are fetched/published/pinned, but normal performer folds receive only per-commitment acts. Standalone binding/revocation events are not included in that context. An absent binding becomes an `unbound` flag rather than enforcing known limits or revocations. `SessionBinding::sign()` adds no independent scope enforcement.

**Intent:** Core V-8 and Binding API §1 require scope enforcement locally and validation before publication. For example, a configured operator maximum must not be defeated by accepting a request above the advertised minimum price.

**Alignment work:** supply authenticated binding/revocation context to every relevant fold and perform a local authority check before work and signing. Test scoped limits, expiry and revocation using the same metadata seen by an independent validator.

### A05 — Negotiation retains original scope values

**Code:** [fold](../crates/acn-core/src/fold.rs), lines 443–444, 553–561, 1438–1453 and 1784–1792.

The fold captures price/mint from REQUEST and passes those fixed values to later steps even after ACCEPT changes the terms in force. A later performer act can therefore be scope-checked against the original lower price or different mint rather than the binding terms it is actually accepting or performing.

**Intent:** Core V-8 requires scope to cover the act's value. Negotiation is implemented core functionality even though L0 shims decline profile N.

**Alignment work:** evaluate scope against the terms the act commits to, including pending terms for ACCEPT. Add vectors for counters crossing a value cap or changing to an excluded mint. Separately review V-8 role coverage: the current final check applies only to performer advancing acts, not all agent roles described by the specification.

### Execution-policy qualification

The optional Gate is explicitly a **pre-signing** API hook, not a general pre-execution authorization system. Its placement after tool execution is not itself a violation of that narrow interface. It cannot prevent a completed tool side effect or evidence upload.

However, [sign_and_send](../crates/acn-binding/src/lib.rs), lines 675–697, signs before folding and checks only Reject-severity anomalies. A governance-pending report can be stored/sent without advancing the phase, leaving work actionable again. L0 should decline governance it cannot satisfy, and candidate applicability must be checked rather than equating absence of a reject anomaly with successful application. Direct commands also bypass ACN funding/pricing checks entirely; paying for a service is not authorization for arbitrary wallet effects.

## 4. Payment validity and recovery

### A06 — Canonical construction is stronger than incoming-lock verification

**Code:** [wallet inspect](../crates/nmesh-wallet/src/lib.rs), lines 321–362; [verified facts](../crates/acn-wallet/src/lib.rs), lines 104–112; [payment check](../crates/acn-wallet/src/settlement.rs), lines 24–48.

The wallet constructs locks with performer/refund keys, but inspection compares only hashlock, first beneficiary and locktime. Additional beneficiary keys, thresholds, refund authority and signature flags are not established. The reduced `Verified` type prevents downstream checking from repairing the omission.

Extra permitted beneficiaries can weaken exclusive redemption; additional required signatures can prevent the performer redeeming alone. Missing or substituted refund policy can defeat the intended requester escape path. These are not exercised by canonical wallet-generated tokens.

**Intent:** Core §8.1 specifies the effective spending policy, not just the first key.

**Alignment work:** validate every proof's complete effective policy against the agreed terms. Reject unsupported variants explicitly. Include mixed-policy proofs and hostile thresholds/refund keys in tests.

### A07 — Duplicate inputs can inflate inspected value

**Code:** [wallet inspect](../crates/nmesh-wallet/src/lib.rs), lines 321–362.

The verifier sums every proof without rejecting duplicate secrets/proof Ys. Review of the pinned CDK/Cashu implementation found no uniqueness check in this parsing/DLEQ/state-query path; the mint's swap verifier rejects duplicate input Ys. Repeated valid evidence can therefore be counted more than once before later redemption fails. This finding is source-derived, not a completed adversarial token reproduction.

**Intent:** Core §8.2 requires verified funding before work; valid DLEQ and unspent state are not enough to establish the value of distinct spendable inputs.

**Alignment work:** reject duplicates before accounting; use checked amount arithmetic and require complete state-query correspondence. Test repeated proofs inside one token separately from spending a valid token twice.

### A08 — Failed delivery or redemption suppresses subsequent recovery

**Code:** [binding](../crates/acn-binding/src/lib.rs), lines 694–697 and 702–729.

The outgoing act enters the store before `Inbox.send()` succeeds. After send failure, the local phase can already be Reported/Declined, so another drive does not resend. Conversely, after restart an unpublished report may be absent and the work repeated.

The commitment is also inserted into `redeemed` before the asynchronous wallet call succeeds. A transient failure suppresses further redemption attempts for that performer instance.

**Intent:** Binding API lifecycle and ARCHITECTURE replay/recovery claims.

**Alignment work:** durable outbox and operation state separating prepared, submitted, confirmed and retryable failure. Preserve signed act IDs for delivery retry. Do not mark redemption successful before success; reconcile uncertain wallet outcomes rather than assuming failure means no spend.

### A09 — Manual refunds lack durable in-flight recovery material

**Code:** [wallet refund](../crates/nmesh-wallet/src/lib.rs), lines 416–435.

Refund generates random replacement secrets/blinding factors in memory, posts the swap, then stores the returned proofs. If the mint commits but the response or process is lost, the original proofs are spent without persisted recovery material for the new outputs.

**Intent:** Core §8's timeout refund escape. This is distinct from the acknowledged missing backup/encryption work.

**Alignment work:** use a recoverable wallet operation or persist swap intent and restoration material before submission. Add interruption tests around submission, response receipt and proof persistence.

### A10 — Silent-claim fallback is not wired into the requester

**Code:** [wallet preimage access](../crates/nmesh-wallet/src/lib.rs), lines 460–470; [LearnKey advice](../crates/acn-wallet/src/settlement.rs), lines 145–159; [app projection](../crates/nmesh-app/src/commitments.rs), lines 83–94; [delivery opening](../crates/acn-view/src/lib.rs), lines 652–668.

The wallet can retrieve the redemption preimage and settlement advice can request it, but no production app caller consumes that fallback. The projection derives the delivery key from CLAIM only. A performer can redeem FUND and withhold CLAIM, leaving the requester UI unable to open the deliverable despite the mint exposing the key.

**Intent:** Core §8.2 says withholding CLAIM gains nothing; Binding API requester lifecycle explicitly supports CLAIM **or** wallet preimage recovery.

**Alignment work:** requester reconciliation of spent FUND tokens, verified key recovery and safe projection/decryption input independent of receiving CLAIM. Test redeem-without-CLAIM end to end, including restart.

## 5. Messaging, consent and privacy

### A11 — Conversation races can send form values to the wrong peer

**Code:** [store](../apps/desktop/src/lib/store.svelte.ts), lines 91–100 and 139–148; [conversation renderer host](../apps/desktop/src/lib/components/Conversation.svelte), lines 24–30; [backend action](../crates/nmesh-app/src/lib.rs), lines 471–503.

Conversation loads update shared state without checking that the requested conversation is still active. Renderer hosts bind the message target but actions use the current active recipient. The backend does not verify that the target belongs to that recipient/conversation.

**Reproduced:** open A then B, resolve A's history last, submit A's displayed form: recipient B, target A-form, values from A. The probe used the compiled store in memory, not native Tauri or a real message delivery.

**Intent:** renderer context includes conversation/sender; actions address the originating message. Encryption to the wrong peer still discloses data.

**Alignment work:** discard stale loads, bind action context immutably and validate target ownership/conversation in the backend. Also guard asynchronous send-result insertion after a conversation switch.

### A12 — Discovery uncertainty bypasses consent

**Code:** [contact defaults/refresh](../crates/nmesh-app/src/view.rs), lines 166–198; [consent gate](../crates/nmesh-app/src/lib.rs), lines 1531–1538; background discovery/persistence at lines 1581–1603 and 1615–1654.

New contacts initially look human and consented until asynchronous discovery completes. A refresh with incomplete but successful results can also downgrade a previously known agent to human and set consent true. The gate accepts both cases.

**Intent:** ADR 0004 and spec 0004 require disclosure before first contact and a persistent agent indicator; ROADMAP marks the enforced gate complete.

**Alignment work:** represent discovery uncertainty explicitly, preserve established agent classification across incomplete fetches, and define a deliberate policy for contacting unresolved identities. Missing advertisements are not affirmative evidence of a human.

### A13 — Terms references initiate unrestricted native requests

**Code:** [incoming acts](../crates/nmesh-app/src/lib.rs), lines 1457–1462; hints at lines 937–947; [blob fetch](../crates/nmesh-blossom/src/lib.rs), lines 282–300. Binding `resolve_terms()` also fetches before validating the incoming act.

A fresh REQUEST/COUNTER can trigger fetching its sender-supplied server hint without prior consent or semantic acceptance. The native client lacks a destination/redirect policy, explicit request deadline and response-size limit on this path; it buffers bytes before checking the hash.

**Impact:** tracking/online-presence disclosure, loopback/LAN requests and resource exhaustion. Hash verification protects accepted document integrity after retrieval, not request privacy or destination safety. The webview CSP does not govern native reqwest requests.

**Alignment work:** validate the incoming event first, require an explicit retrieval policy, restrict destinations including redirects and resolved addresses, and bound bytes/time/concurrency. Preserve intentionally configured local dev servers through explicit policy rather than trusting remote hints.

### A16 — Existing evidence and images leak beyond encrypted messaging

**Code:** [MCP artifact](../crates/nmesh-agent-bridge/src/shim.rs), lines 60–65; [evidence upload](../crates/acn-binding/src/lib.rs), lines 622–646; [Blossom adapter](../crates/acn-binding-nostr/src/lib.rs), lines 91–98; [card image](../apps/desktop/src/lib/renderers/Card.svelte), lines 14–15.

The executor serializes the entire MCP result as a tool artifact and uploads it before the signing gate. These bytes are not encrypted by the generic evidence adapter. This is especially dangerous for private payment adapters and secret-bearing tool responses; a later gate denial cannot retract an upload.

Built-in cards also load sender-controlled image URLs directly. A schema-valid image can contact a tracking endpoint when viewed; the optional image hash is not enforced there.

**Intent:** private-by-default messaging and ADR 0006's Tier 0 non-exfiltration claim. Evidence storage may be intentionally public for some tasks, but arbitrary private tool results must not become public implicitly.

**Alignment work:** explicit evidence disclosure policy, minimized receipt/artifact schemas and encryption/access control where needed. Block or ask before remote image loading and define image integrity behavior. Test actual uploaded bytes and network requests.

### A17 — Side-effecting renderer actions lack host confirmation

**Code:** [card](../apps/desktop/src/lib/renderers/Card.svelte), lines 7–9 and 30–40; [form](../apps/desktop/src/lib/renderers/Form.svelte), lines 32–42; [send_action](../crates/nmesh-app/src/lib.rs), lines 476–503.

A sender-selected label is the visible description of a sender-selected invocation. The host validates the envelope shape but neither classifies side effects nor previews the actual invocation. A benign label can therefore conceal a destructive tool command behind a click.

**Intent:** renderer spec §host responsibilities and ADR 0006 require host-controlled confirmation where appropriate.

**Alignment work:** trustworthy previews and policy-based confirmation. Do not infer operation safety from labels, schema validity or a prior data-handling consent grant.

## 6. Outcome evidence and pure protocol semantics

### A14 — Judge pins and reply schema do not constrain actual execution sufficiently

**Code:** [judge config/request](../crates/acn-judge/src/lib.rs), lines 156–205 and 259–294; fingerprints at lines 335–344; reply parser at lines 596–626.

Two distinct gaps:

1. Fingerprints pin configured `model_id`/`model_version`, but the request uses a separately configurable model override. The version is neither selected nor verified from response metadata; JSON mode is also outside the fingerprint. Changing actual routing can retain the same advertised identity. Provider execution remains trusted even after this is corrected; matching labels is not proof a model ran.
2. A score-only reply can parse successfully. Missing or mistyped injection evidence defaults to false and rationale defaults to empty. This converts absent required assessment into apparently negative injection evidence; the retry/failure path does not run. An existing test explicitly accepts permissive score-only parsing.

**Intent:** Core judged-condition model/prompt/harness/sampling pins and the reference harness's advertised reply schema.

**Alignment work:** bind actual request configuration to the fingerprint, enforce provider-pinned identifiers where supported and disclose unverifiable versions. Validate typed required fields; fail closed or retry malformed responses. Injection-positive responses without required supporting spans must not silently turn into negative votes.

The harness hash is a digest of a hand-maintained descriptor, not an implementation/build identity. The descriptor and permissive parser already disagree. Either specify it honestly as a semantic descriptor plus versioning discipline or bind a reproducible implementation identity; an unchanged descriptor alone does not establish unchanged execution semantics.

### A15 — RELEASE validation does not cut off later disqualifier evidence

**Code:** [fold](../crates/acn-core/src/fold.rs), lines 1007–1023, 1267–1288 and 1696–1707.

When evaluating RELEASE, `verdicts_at()` creates a temporary snapshot containing only the current report state and evaluates all attestations. The late-disqualifier check needs a settled/satisfied snapshot, absent from that temporary set. A disqualifier hit timestamped after RELEASE can consequently affect the earlier release decision and potentially turn an already-settled projection back into `bad_release`.

**Intent:** Core V-25 explicitly says evidence after the act settling the report is late and not counted. Recomputing from an expanded event set must respect that cutoff even if the payment preimage has already become public.

**Evidence qualification:** source-derived; existing vectors cover a hit before RELEASE and a hit after SATISFY, not this post-RELEASE sequence. Add the missing signed vector and permutation coverage before choosing the fix. Apply the same temporal review to condition evidence used for RELEASE.

## 7. Additional alignment and assurance gaps (A18)

These are material follow-ups, not proof that every documented demo path fails:

- **Financial recovery depends on commitment history.** [app](../crates/nmesh-app/src/lib.rs), lines 807–873 and 1201–1212, locks funds before fallible upload/sign/persist/publish steps. Refund at lines 1117–1133 depends on fold advice, while reset clears acts at lines 1147–1161. Keep wallet-indexed orphan locks recoverable even when publication fails or history is removed. Persistence failures must be actionable; do not log and continue after losing payment secrets.
- **Real-mint fees:** [refund](../crates/nmesh-wallet/src/lib.rs), lines 400–425, constructs outputs for the full input amount without subtracting input spending fees. The bundled zero-input-fee mint passes; fee-bearing mint compatibility is not established. This lies within acknowledged real-mint work.
- **Capabilities/keysets:** bridge capabilities are configured claims, not NUT-06-verified guarantees. Current payment verification does not establish correspondence to the agreed keyset IDs. Core §8.6 requires both capability checks and named keysets. Clearly distinguish declared, observed and tested capabilities.
- **Conformance scope:** [L0 suite](../crates/acn-conformance/src/lib.rs), lines 612–625, accepts a paid by-reference REPORT without all the advertised fold/evidence/settlement checks; wrong-hash decline at lines 648–656 does not require the specific payment failure. Add execution-count assertions proving rejected requests caused no work. The MCP bridge's evidence-based level calculation can also claim L0 without the operator binding required by discovery; the CLI shim suppresses that claim.
- **Authoring fails open diagnostically:** [drafters](../crates/acn-author/src/draft.rs), lines 378–385 and 587–603, turn terms-parse failures into empty lint findings. Invalid terms are still rejected downstream, but an empty finding list must not mean both valid and unparseable.
- **Validation indicators:** contractless JSON is marked `valid: true` in [commitment rendering](../crates/nmesh-app/src/commitments.rs), lines 120–127. Distinguish raw-displayable from schema-validated. Missing required `alt` tags are also decoded as empty rather than rejected in [structured decoding](../crates/nmesh-proto/src/structured.rs).
- **Documentation drift:** ARCHITECTURE line 595 says the bridge is T0/free-only despite implemented T1 support. Its claims of replay idempotence and complete lock verification need qualification until corrected. The safe raw-data inspector also needs an explicit exception to the spec's absolute statement that invalid payloads never reach a renderer; this review did not establish an XSS path through that escaped inspector.

## 8. What is aligned, and what is intentionally unfinished

### Supported by inspection and existing tests

- Pure `acn-core` semantics are separated from network execution; the shared Rust validator/projections are used across hosts.
- Canonical Cashu locks enforce wrong-key, wrong-preimage, early-refund and double-spend rejection against the local test mint.
- Rust structured-message handling checks schemas and contract pins before ordinary renderer resolution.
- NIP-17 transport and independently signed embedded ACN acts preserve separate transport and act identities. Selective disclosure intentionally permits these identities to differ; authorization must define which principal it governs rather than assuming equality.
- L0 reference bindings deliberately decline unsupported profiles/triggers rather than pretending to implement negotiated/verifier/contingent execution.
- Both current reference bindings pass the existing L0 suite under its present coverage.

### Acknowledged roadmap work, not newly discovered regressions

Persistent local messages, real/multi-mint policy, wallet caps, encrypted proofs/preimages, backup, external signers, NIP-17 gossip routing, complete MLS groups, third-party registry/WoT, Tier 1 sandboxed renderers and L1–L3 binding suites are explicitly unfinished. None is counted as a defect merely for being absent.

Nevertheless, incomplete persistence and wallet policy must constrain deployment claims. They cannot be treated as production-ready simply because the demo path works.

## 9. Validation performed

Commands were bounded; no external judge credentials, live-value financial transactions or package installations were used. Integration tests start local test relays/mints/mock endpoints. Cargo `--offline` prevents dependency fetching, not network activity within tests.

| Validation | Result |
| --- | --- |
| `cargo test --workspace --locked --offline` (180-second bound) | Cached workspace build succeeded; run timed out in `nmesh-acn-sim/tests/live.rs`. No failure was shown in captured output, but the suite did not finish and is **not a pass**. |
| Pure-crate subset: `acn-core`, `acn-author`, `acn-view`, `acn-wallet`, `acn-sim`, excluding `fold_is_order_independent` and `cr003_cos_vectors_under_every_arrival_order` | 56 passed, 0 failed, 2 filtered out. The CR-003 permutation test passed in separate targeted runs; the general fold-order property did not complete within the baseline subset's bound. |
| `acn-core` + `acn-binding` library tests | 13 passed. Targeted binding-scope/governance scenario tests also passed. |
| Both reference bindings' L0 integration suites | Passed for priced/free configurations. |
| `acn-wallet` + `nmesh-wallet` tests | 5 passed, including real local-mint HTLC and preimage visibility tests. |
| Scoped app/proto/contracts library tests and desktop-to-MCP integration | Passed; successful-discovery consent coverage does not establish incomplete-discovery behavior. |
| `cargo test --locked --offline -p nmesh-core -p nmesh-proto -p nmesh-contracts -p acn-judge --lib` | Passed. `nmesh-core` has zero library unit tests in this run. |
| Desktop `pnpm check` | Passed with zero errors/warnings. |
| In-memory compiled frontend store probe | Reproduced A11's stale-conversation/action-recipient mismatch. |

**Limitations:** no native Tauri build/runtime audit, full browser smoke suite, fresh clean build, independent cryptographic review, deployed Hinkal compatibility test, real judge-provider identity verification or exhaustive module-by-module proof. The local protoc path is missing, so the successful cached build does not establish clean protobuf build setup. Additional adversarial probe attempts did not complete; they are not counted as reproductions. Counts from separate runs overlap and must not be summed into a unique total.

## 10. Remediation order and acceptance gates

1. **Contain effects and data disclosure:** A01–A04, A11–A13, A16–A17. Unified authorization, immutable conversation/action context, validated history insertion, bounded retrieval and explicit evidence policy.
2. **Establish payment validity and recoverability:** A06–A10 and A18's financial items. Full lock-policy validation, unique-proof accounting, durable execution/outbox/swap recovery, orphan-lock refund and silent-claim fallback.
3. **Repair protocol/evidence semantics:** A05, A14–A15 and governance applicability. Pin actual judge configuration, fail closed on malformed evidence, use correct terms for scope and respect settlement-time cutoffs.
4. **Make assurance match reality:** extend conformance with negative execution assertions and fault injection; correct implementation claims; run the full suites with agreed longer bounds and native/browser checks.

For each fix, add a focused regression **first or alongside the change**, verify the intended failure on the unfixed implementation, then run adjacent tests. Do not repair every issue with a single broad rewrite. A real-value deployment decision requires the relevant gates to pass, not just completion of this audit report.
