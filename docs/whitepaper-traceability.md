# Whitepaper ↔ spec ↔ prototype traceability

How each claim in [Agents Accountable for Outcomes](Agents%20Accountable%20for%20Outcomes.md)
maps to the spec ([Core.md](Core.md) v0.5) and to the code. Status as of Nov 11, 2026 (after the [design-alignment audit](DESIGN-ALIGNMENT-AUDIT.md) remediation). A row is ✅
only when the code path exists and a test exercises it (named in the row or in
[ARCHITECTURE.md](ARCHITECTURE.md) §6); claims rest on the invariants (INV-n) the row cites.

**✅ built** · **🟡 partial or simulated** · **⬜ specified, not built** · **🧭 aspiration**

## Claims

| § | Whitepaper claim | Spec | Prototype | Status |
|---|---|---|---|---|
| 3 | Every participant has a verifiable identity | §4, V-1 | Nostr keys; BIP-340 checked in `acn-core/src/event.rs` | ✅ |
| 3 | Agents can be tied to an accountable owner; unowned agents are shown as such | §4, V-8, INV-10 | `K_BIND` + `K_BIND_ACK` (`acn-core/src/binding.rs`); the bridge and demo cast publish bindings; "operator-bound ✓ / unaccountable ⚠" in the consent dialog and on cards | ✅ |
| 3 | The requester writes the terms: deliverable, deadline, price, how the result is checked | §7 | `Terms` (`acn-core/src/terms.rs`); per-tool terms offers in `K_SVC`, instantiated by `nmesh-app`; conditions may come from a signed terms template (`K_TERMS_TEMPLATE`, CR-003); drafters and a linter help write them (`acn-author`, CR-003 §3: lint errors block a `REQUEST`, warnings on the card; the performer drafts its counter from its templates). In the app the requester drafts terms on the drafting screen (intent → drafter → terms editor with live lint → send) and can counter the performer's counter-offer with its own version (`App::{request_terms, counter}`); Atlas accepts a closed, same-price counter or declines `counter_unacceptable`. Tests: `requester_counters_and_atlas_answers` (playthrough), `drafted_terms_by_reference_and_requester_counters` (`live`), `presenter.mjs` | ✅ |
| 3 | Payment is locked at request time | §8.1–8.2 | `H` in the terms; `nmesh-wallet` locks the price as a NUT-14 HTLC token before the REQUEST is signed and sends it NIP-44-encrypted in `content.token`; the performer verifies it with the mint (DLEQ + NUT-07) and checks every proof's complete spending policy, distinct proofs and named keysets (§8.6 "Received locks"; `inspect_rejects_hostile_tokens`) before working, or declines `payment_invalid`. The requester checks the mint's own NUT-06 capabilities before locking and won't publish if it can't save `s`. Under the contingent trigger (CR-002) nothing is locked up front: the requester `FUND`s after the attestation | ✅ test mint (fake Lightning) |
| 3 | The agent signs those exact terms | INV-4, V-9 | `PROMISE`/`ACCEPT` must carry the terms hash in force; `terms_mismatch` otherwise | ✅ |
| 3 | Report with evidence | §6.2, §9 | `REPORT` with an inline result and `x` artifact hashes; attestations (`K_ATTEST_*`). The reference bindings (`acn_binding::Performer`) and the demo cast (`with_evidence`) store every artifact and a derivation record on Blossom (`nmesh-blossom`); the L0 suite and the `live` test fetch and hash-check them for every `REPORT`. The card shows what each attester signed (method, recipe, referent, judges, instances) and links the artifacts | ✅ dev Blossom server |
| 3 | Checked by tests, real data, or a person | §7 condition classes | `mechanical` / `grounding` / `judged` / `interpretive` / `human`, shown in the card's Conditions panel | 🟡 verifier actors are scripted in the demo; judges are real only with `--judges` |
| 3 | Satisfied → payment released | INV-6, V-10 | `SATISFY` carries the preimage; `bad_preimage` if wrong; Atlas redeems at the mint; the card shows "redeemed by Atlas" | ✅ test sats |
| 3 | Not satisfied → rework | §6.2 `REJECT` | `Reworking` below `rework_limit`, then `Rejected`; rework demo scenario | ✅ |
| 3 | Never delivers → money returns | §6.5, §8.2 | Lapse derived by `status_at`; settlement "refundable"; NUT-14 refund after the locktime (`CashuWallet::refund`, wallet panel / card button). A refund interrupted mid-swap is recovered on reopen (`refund_recovers_after_interruption`), refunds pay a fee-bearing mint's fees (`refund_pays_input_fees`), and a lock whose request was never published stays refundable from the wallet's record | ✅ real mint in tests; in the live demo the real ≈ 25 h locktime applies, presenter mode fast-forwards |
| 3 | Every step is a signed message that cannot be altered | INV-1, INV-11 | Regular events; rejected events are kept and listed under "Show signed events"; a corrupted copy of a known act never replaces the verified original, in the app or the bindings (`a_corrupt_copy_never_replaces_a_verified_act`, `corrupt_copy_does_not_replace_a_verified_report`) | ✅ |
| 4 | Only the requester can say the job is done | INV-5, V-5 | `SATISFY` is requester-only; a `REPORT` never closes. Under the verifier and contingent triggers (CR-002) `RELEASE` / `CLAIM` settle and the requester's later `SATISFY` or `REJECT` is a recorded judgement only | ✅ requester trigger; see the CR-002 row |
| 4 | Saying "done" is what pays; no escrow company | INV-6, §8 | Preimage in `SATISFY` (*paid call* demo); the mint enforces the lock, nobody holds the funds | ✅ test sats |
| 4 | Evidence and opinion are kept apart | INV-8, V-11 | Interpretive reports never count (`wrong_evidence_class`); a judged attestation counts only for a `judged` condition, labelled `opinion`; *verified work* demo | ✅ |
| 4 | (added) Rejecting work that passed its tests is flagged | V-16 | `contradicted_rejection`; for passed judged conditions, `overruled_judgement` (D-28) | ✅ |
| 4 | (added, CR-001) An LLM judge may decide payment, labelled as an opinion | INV-8, §7, V-17 | `judged` conditions, `K_ATTEST_JUDGED`, derived assurance label shown before signing; *judged*, *injected* and *judged-auto* demos; real judges with `acn-judge/2` (`nmesh-acn-sim --judges`): the harness requests exactly the pinned model, fails a run whose provider reports another, and treats a malformed reply as a failed run, never as "no injection" (`malformed_replies_and_other_models_fail_closed`) | ✅ live mode with real judges; presenter mode scripted; the provider is trusted to run the named model, and `model_version` is a claim |
| 4 | (added, CR-002) Payment without the requester's act, for checkable work | §8.2, V-19…V-23 | Verifier trigger (`RELEASE`); contingent trigger (sealed delivery, `FUND`, `CLAIM`); *judged* and *contingent* demos. A `RELEASE` is judged only on evidence signed before it, so later evidence can't undo a settlement (C-6g, S-5b, S-7c). Withholding `CLAIM` gains nothing: the requester's app reads `k` from the mint and opens the delivery (`withheld_claim_key_is_recovered_from_the_mint`) | ✅ test sats |
| 4 | (added, CR-003) No hidden criteria: everything the work is judged against is readable before the agent signs; `open` conditions are closed before `PROMISE`; disqualifiers; held-out instances drawn by a two-party seed | INV-14, §7, V-24…V-28 | `hidden_criteria`, `unclosed_condition`, `disqualified`, `seed_mismatch` / `seed_withheld` / `instance_mismatch`; the terms card; *drafted*, *disqualified* and *held-out* demos. Held-out cases for tests and for judges: `acn-judge` shows the judges only the sample both seeds select and reveals it with proofs (`real_panel_judges_held_out_cases`; real Ollama judges in `live -- --ignored`) | ✅ (the requester as its own runner: seed kept, runner not built, P3) |
| 5 | Not a platform; anyone can implement | INV-13 | Open spec; the validator is a standalone pure crate | ✅ |
| 5 | Not a judge of quality; parties choose verifiers | §7 `verifiers` | Verifiers listed per class in the terms; unlisted ones flagged `unlisted_verifier` | ✅ |
| 5 | Not a reputation authority; many scorers | §11 `K_SCORE` | Kind reserved; no scorer built | ⬜ |
| 5 | Any framework can join via small interfaces; a thin tool-server adapter is enough | §11, D-10 | The Binding API's interfaces are traits (`acn-binding`, [harness-binding-API.md](harness-binding-API.md)); two bindings run on them, `acn-mcp-shim` (any MCP server) and `acn-cli-shim` (any command-line harness, no MCP), and both pass the L0 conformance suite (`acn-conformance`) with paid T1 calls and Blossom evidence. The driver applies the same admission to history and live events, journals each execution so a restart never repeats work, checks its operator scope before working and signing, resends the same act after a failed send, and keeps evidence private unless configured public (harness-binding-API rules 8–15; bridge `startup.rs`) | ✅ L0; 🟡 L1–L3 (negotiation, governance, verifiers) have no suites yet |
| 6 | Nostr for messages; signed; no owner | §5 | `nostr-sdk`; relays | ✅ |
| 6 | (added) Commitments can travel privately, each act still verifiable alone | §5.5 | `K_PRIVATE_ACT` rumor inside NIP-17 gift wraps (`nmesh-core/src/acn.rs`). Privacy holds around them too: evidence is private unless a binding is configured public, card images load only on click, and other parties' URLs are fetched only to public addresses under bounds (`untrusted_fetches_are_bounded`) | ✅ |
| 3 | (added) Agents are disclosed before first contact, and the user approves what is sent to them | ADR 0004, spec 0003–0004 | The consent gate fails closed: unknown keys are discovered first, a key with no profile needs the same acceptance as an agent, an agent is never downgraded (`discovery_tests`, `app_flow`); every renderer action shows the command, values and recipient before it is sent, and goes only to the conversation it came from (`ActionConfirm.svelte`, `app_flow`) | ✅ |
| 6 | (added, P2) Terms too large to inline travel by hash, so the signed act still binds exactly one document | §7, §6.2, INV-4 | `terms` tag `["terms", <sha256>, <server hint>]`; the app, the demo cast and the binding driver fetch, hash-check and fold with the document, and the act stays pending until they have it (`PendingTerms`). Tests: `acn-core` `ref_tests`, L0 cases `t0-terms-by-reference` / `t1-terms-by-reference`, `live` | ✅ dev Blossom server |
| 6 | Cashu for payment, private by design | §8 | CDK 0.18 wallet (`nmesh-wallet`) and test mint (`nmesh-mint`); tokens encrypted to the performer, so relays see neither amount nor mint | 🟡 test mint only; no real mint, NIP-60 backup or NWC yet |
| 7 | Checkpoint with optional human sign-off publishes endorsements | §9 `K_ENDORSE`, V-14 | Endorsement validation; "Gate" in the *governance* demo | 🟡 no real gate product |
| 7 | Requesters can require endorsement in the terms | §7 `governance` | `governance.keys/kinds/window`; `pending_endorsement` / `unendorsed` | ✅ |
| 7 | Overseers build reputations | §11, §13 | Endorsements are signed records; no scoring yet | ⬜ |
| 8 | Hireable agents, track records, a verification industry… | §7 (terms templates), §12 | A first piece: records attached to a definition of done, not a party. Terms templates (`acn-core/src/template.rs`) and per-template records computed by each reader (`acn_view::template_records`, on the card) | 🧭 mostly; 🟡 template records over the commitments a reader can see |
| 9 | Mint trust; amounts capped | §8.6, binding `max_sat` | `max_sat` scope enforced by the validator against the terms each act commits to, so a counter can't raise the price past the cap (`out_of_scope`, SC-1…SC-7b); the performer's binding declines over-scope requests before any work; the app only pays at its configured mint, after checking its NUT-06 capabilities | 🟡 one local test mint; no multi-mint policy; the requester's wallet doesn't apply `max_sat` caps |
| 9 | (added) Arbiter holds the escrowed secret, so it could collude | §8.4 | NIP-44 escrow to the arbiter; *dispute* demo, where the RULING releases `s` and Atlas redeems | ✅ |
| 9 | (added) Clocks are not trusted | INV-3, V-15 | `time_regress`; deadlines compared under monotonicity | ✅ |
| 10 | Conversation for action: request → promise → report → satisfaction | §6 | The 16-act state machine (`acn-core/src/fold.rs`; v0.2's 13 plus `RELEASE`, `FUND`, `CLAIM`) | ✅ |
| 10 | Agents forecast their own deadlines (scored with Brier) | §6.2 `p_on_time` | `p_on_time` in `PROMISE`, range-checked when present (the validator doesn't require it); Brier scoring not built | 🟡 |

## Changes made to the whitepaper (Nov 8)

1. **§3:** the missing "five steps, one loop" figure replaced with a diagram; "every agent is tied" → "can be bound", with unbound agents flagged (matches INV-10).
2. **§3:** rework is no longer unconditional. A rejection names a condition, rework depends on the terms' limit, and arbitration is possible.
3. **§3 / §4:** the money returns after the payment's **time lock**, not at the deadline (spec §8.2: `L > deadline + review_window + margin`).
4. **§4:** added that a requester's rejection of verified work is flagged (V-16), while still honouring INV-5.
5. **§5:** interfaces are "being specified" (D-10 is open), not yet "published"; the MCP adapter exists.
6. **§5 / §8:** "every commitment is on the record" qualified. The prototype sends acts privately, so track records come from public or disclosed commitments.
7. **§6:** added private transport with individually verifiable acts (spec §5.5).
8. **§7:** "overseers build reputations" → "can build" (no scorer yet).
9. **§9:** "specification stage, v0.1" → an accurate built / not-built list; added the arbiter-collusion and clock limitations from v0.2.
10. **§10:** made explicit that agents submit `p_on_time` forecasts, which scorers can grade with Brier.

## Whitepaper text that predates spec v0.3–v0.5

The whitepaper was last revised against v0.2. These passages no longer describe every mode:

- Intro and §4 ("released only when the person who asked is satisfied", "only the requester's
  declaration of satisfaction can release" the payment): under the verifier and contingent
  triggers (CR-002) a settlement verifier's `RELEASE` or the performer's `CLAIM` releases it, and
  an arbiter's `RULING` already could under v0.2.
- §3 ("The payment is locked at that moment"): not under the contingent trigger.
- §4 ("never lets an opinion stand in for a check"): still true per condition, but a `judged`
  condition (CR-001) lets pinned LLM judges' scores decide payment, labelled `opinion`.
- §6 ("back to the requester after the deadline"): after the locktime, which is later than the
  deadline plus the review window (§3 says this correctly).
- §9 ("A specification (v0.2 logical design)"): the spec is v0.5; the demonstration also
  covers judged conditions, settlement triggers, drafted conditions, disqualifiers and held-out
  tests.
- §10 ("When an agent commits, it also states how likely it thinks it is to meet the deadline"):
  `p_on_time` is optional in the validator.

## Gaps the whitepaper depends on (to build or keep caveated)

- Cashu against real mints (the CDK path works against a test mint, including paid calls to the reference bindings): NIP-60
  wallet backup, NWC top-up, multi-mint policy.
- Binding API beyond L0 (D-10): the L1–L3 suites; profile N, Gate and Verifier in the driver; the
  requester side and the demo cast on the traits; a requester-side runner.
- `K_SCORE` scorers, including Brier scoring of `p_on_time`.
- Blossom beyond the dev server: deletion, quotas, a mirroring policy.
- Public (non-private) commitments and selective disclosure to third parties (D-2 remainder), which track records need.
- **Reputation under private commitments: open decision D-13 in Core.md.** Until it is
  resolved, the whitepaper's track-record claims (§5, §8) describe intent rather than mechanism.
