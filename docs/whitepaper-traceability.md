# Whitepaper ↔ spec ↔ prototype traceability

How each claim in [Agents Accountable for Outcomes](Agents%20Accountable%20for%20Outcomes.md)
maps to the spec ([Core.md](Core.md) v0.5) and to the code. Status as of Nov 10, 2026. A row is ✅
only when the code path exists and a test exercises it (named in the row or in
[ARCHITECTURE.md](ARCHITECTURE.md) §6); claims rest on the invariants (INV-n) the row cites.

**✅ built** · **🟡 partial or simulated** · **⬜ specified, not built** · **🧭 aspiration**

## Claims

| § | Whitepaper claim | Spec | Prototype | Status |
|---|---|---|---|---|
| 3 | Every participant has a verifiable identity | §4, V-1 | Nostr keys; BIP-340 checked in `acn-core/src/event.rs` | ✅ |
| 3 | Agents can be tied to an accountable owner; unowned agents are shown as such | §4, V-8, INV-10 | `K_BIND` + `K_BIND_ACK` (`acn-core/src/binding.rs`); the bridge and demo cast publish bindings; "operator-bound ✓ / unaccountable ⚠" in the consent dialog and on cards | ✅ |
| 3 | The requester writes the terms: deliverable, deadline, price, how the result is checked | §7 | `Terms` (`acn-core/src/terms.rs`); per-tool terms offers in `K_SVC`, instantiated by `nmesh-app`; conditions may come from a signed terms template (`K_TERMS_TEMPLATE`, CR-003); drafters and a linter help write them (`acn-author`, CR-003 §3: lint errors block a `REQUEST`, warnings on the card; the performer drafts its counter from its templates) | ✅ (drafting screen: P2) |
| 3 | Payment is locked at request time | §8.1–8.2 | `H` in the terms; `nmesh-wallet` locks the price as a NUT-14 HTLC token before the REQUEST is signed and sends it NIP-44-encrypted in `content.token`; the performer verifies it with the mint (DLEQ + NUT-07) before working, or declines `payment_invalid`. Under the contingent trigger (CR-002) nothing is locked up front: the requester `FUND`s after the attestation | ✅ test mint (fake Lightning) |
| 3 | The agent signs those exact terms | INV-4, V-9 | `PROMISE`/`ACCEPT` must carry the terms hash in force; `terms_mismatch` otherwise | ✅ |
| 3 | Report with evidence | §6.2, §9 | `REPORT` with an inline result and `x` artifact hashes; attestations (`K_ATTEST_*`) | 🟡 Blossom artifacts and derivation records not built |
| 3 | Checked by tests, real data, or a person | §7 condition classes | `mechanical` / `grounding` / `judged` / `interpretive` / `human`, shown in the card's Conditions panel | 🟡 verifier actors are scripted in the demo; judges are real only with `--judges` |
| 3 | Satisfied → payment released | INV-6, V-10 | `SATISFY` carries the preimage; `bad_preimage` if wrong; Atlas redeems at the mint; the card shows "redeemed by Atlas" | ✅ test sats |
| 3 | Not satisfied → rework | §6.2 `REJECT` | `Reworking` below `rework_limit`, then `Rejected`; rework demo scenario | ✅ |
| 3 | Never delivers → money returns | §6.5, §8.2 | Lapse derived by `status_at`; settlement "refundable"; NUT-14 refund after the locktime (`CashuWallet::refund`, wallet panel / card button) | ✅ real mint in tests; in the live demo the real ≈ 25 h locktime applies, presenter mode fast-forwards |
| 3 | Every step is a signed message that cannot be altered | INV-1, INV-11 | Regular events; rejected events are kept and listed under "Show signed events" | ✅ |
| 4 | Only the requester can say the job is done | INV-5, V-5 | `SATISFY` is requester-only; a `REPORT` never closes. Under the verifier and contingent triggers (CR-002) `RELEASE` / `CLAIM` settle and the requester's later `SATISFY` or `REJECT` is a recorded judgement only | ✅ requester trigger; see the CR-002 row |
| 4 | Saying "done" is what pays; no escrow company | INV-6, §8 | Preimage in `SATISFY` (*paid call* demo); the mint enforces the lock, nobody holds the funds | ✅ test sats |
| 4 | Evidence and opinion are kept apart | INV-8, V-11 | Interpretive reports never count (`wrong_evidence_class`); a judged attestation counts only for a `judged` condition, labelled `opinion`; *verified work* demo | ✅ |
| 4 | (added) Rejecting work that passed its tests is flagged | V-16 | `contradicted_rejection`; for passed judged conditions, `overruled_judgement` (D-28) | ✅ |
| 4 | (added, CR-001) An LLM judge may decide payment, labelled as an opinion | INV-8, §7, V-17 | `judged` conditions, `K_ATTEST_JUDGED`, derived assurance label shown before signing; *judged*, *injected* and *judged-auto* demos; real judges with `acn-judge` (`nmesh-acn-sim --judges`) | ✅ live mode with real judges; presenter mode scripted |
| 4 | (added, CR-002) Payment without the requester's act, for checkable work | §8.2, V-19…V-23 | Verifier trigger (`RELEASE`); contingent trigger (sealed delivery, `FUND`, `CLAIM`); *judged* and *contingent* demos | ✅ test sats |
| 4 | (added, CR-003) No hidden criteria: everything the work is judged against is readable before the agent signs; `open` conditions are closed before `PROMISE`; disqualifiers; held-out instances drawn by a two-party seed | INV-14, §7, V-24…V-28 | `hidden_criteria`, `unclosed_condition`, `disqualified`, `seed_mismatch` / `seed_withheld` / `instance_mismatch`; the terms card; *drafted*, *disqualified* and *held-out* demos. Held-out cases for tests and for judges: `acn-judge` shows the judges only the sample both seeds select and reveals it with proofs (`real_panel_judges_held_out_cases`; real Ollama judges in `live -- --ignored`) | ✅ (the requester as its own runner: seed kept, runner not built, P3) |
| 5 | Not a platform; anyone can implement | INV-13 | Open spec; the validator is a standalone pure crate | ✅ |
| 5 | Not a judge of quality; parties choose verifiers | §7 `verifiers` | Verifiers listed per class in the terms; unlisted ones flagged `unlisted_verifier` | ✅ |
| 5 | Not a reputation authority; many scorers | §11 `K_SCORE` | Kind reserved; no scorer built | ⬜ |
| 5 | Any framework can join via small interfaces; a thin tool-server adapter is enough | §11, D-10 | `acn-mcp-shim` in `nmesh-agent-bridge` (T0, free calls); the Binding API is a proposal ([harness-binding-API.md](harness-binding-API.md); reference behaviour in [binding-api.md](binding-api.md)), D-10 open | 🟡 |
| 6 | Nostr for messages; signed; no owner | §5 | `nostr-sdk`; relays | ✅ |
| 6 | (added) Commitments can travel privately, each act still verifiable alone | §5.5 | `K_PRIVATE_ACT` rumor inside NIP-17 gift wraps (`nmesh-core/src/acn.rs`) | ✅ |
| 6 | Cashu for payment, private by design | §8 | CDK 0.18 wallet (`nmesh-wallet`) and test mint (`nmesh-mint`); tokens encrypted to the performer, so relays see neither amount nor mint | 🟡 test mint only; no real mint, NIP-60 backup or NWC yet |
| 7 | Checkpoint with optional human sign-off publishes endorsements | §9 `K_ENDORSE`, V-14 | Endorsement validation; "Gate" in the *governance* demo | 🟡 no real gate product |
| 7 | Requesters can require endorsement in the terms | §7 `governance` | `governance.keys/kinds/window`; `pending_endorsement` / `unendorsed` | ✅ |
| 7 | Overseers build reputations | §11, §13 | Endorsements are signed records; no scoring yet | ⬜ |
| 8 | Hireable agents, track records, a verification industry… | §7 (terms templates), §12 | A first piece: records attached to a definition of done, not a party. Terms templates (`acn-core/src/template.rs`) and per-template records computed by each reader (`acn_view::template_records`, on the card) | 🧭 mostly; 🟡 template records over the commitments a reader can see |
| 9 | Mint trust; amounts capped | §8.6, binding `max_sat` | `max_sat` scope enforced by the validator (`out_of_scope`); the app only pays at its configured mint | 🟡 one local test mint; no multi-mint policy; the wallet doesn't apply `max_sat` caps |
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

- Cashu against real mints (the CDK path works against a test mint): T0 paid tokens, NIP-60
  wallet backup, NWC top-up, multi-mint policy.
- Binding API for external harnesses (D-10): P3.
- `K_SCORE` scorers, including Brier scoring of `p_on_time`.
- Blossom artifacts and derivation records: P2.
- Public (non-private) commitments and selective disclosure to third parties (D-2 remainder), which track records need.
- **Reputation under private commitments: open decision D-13 in Core.md.** Until it is
  resolved, the whitepaper's track-record claims (§5, §8) describe intent rather than mechanism.
