# Whitepaper ↔ spec ↔ prototype traceability

How each claim in [Agents Accountable for Outcomes](Agents%20Accountable%20for%20Outcomes.md)
maps to the spec ([Core.md](Core.md) v0.5) and to the code. Status as of Nov 11, 2026, after the
[design-alignment audit](DESIGN-ALIGNMENT-AUDIT.md) remediation.

**Rules for this matrix.** A row is ✅ only when the code path exists **and** a named test
exercises it (in the row, or in [ARCHITECTURE.md](ARCHITECTURE.md) §6 and §9). Each claim rests
on the invariants (INV-n) and validation rules (V-n) it cites. A claim about money is ✅ only
against a real CDK mint in tests; the demo's Lightning backend is fake throughout. Rows follow
the whitepaper's section order; "(added)" marks a guarantee the prototype makes that the
whitepaper doesn't state.

**✅ built and tested** · **🟡 partial, simulated, or only enforced by our own side** · **⬜ specified, not built** · **🧭 aspiration**

## Claims

### §3 What ACN is

| Whitepaper claim | Spec | Prototype and tests | Status |
|---|---|---|---|
| Every participant has a verifiable identity | §4, V-1 | Nostr keys; BIP-340 and NIP-01 ids checked in `acn-core/src/event.rs`, interoperable with rust-nostr both ways (`acn-core/tests/nostr_compat.rs`); unsigned or altered acts are `bad_sig` | ✅ |
| Every agent can be bound to an accountable owner; an unowned agent is shown as unaccountable | §4, V-8, INV-10 | `K_BIND` + `K_BIND_ACK` (`acn-core/src/binding.rs`), revocation and scope; the bridge and demo cast publish bindings; "operator-bound ✓ / unaccountable ⚠" in the consent dialog and on cards. Tests: `bindings`, `revoked_binding` (scenarios), `app_flow`, `live` (Atlas bound, Rogue not) | ✅ |
| (added) Agents are disclosed before first contact, and the user approves what is sent to them | ADR 0004, specs 0003–0004 | The consent gate fails closed: unknown keys are discovered first, a key with no profile needs the same acceptance as an agent, and an agent is never downgraded (`discovery_tests`, `app_flow`). Every renderer action shows the command, values and recipient before it is sent, and goes only to the conversation it came from (`ActionConfirm.svelte`, `app_flow`, `screenshots.mjs`) | ✅ |
| The requester writes what success means: deliverable, deadline, price, how the result is checked | §7 | `Terms` (`acn-core/src/terms.rs`); per-tool terms offers in `K_SVC`, instantiated by `nmesh-app`; signed terms templates (`K_TERMS_TEMPLATE`, CR-003); drafters and a linter (`acn-author`: lint errors block a `REQUEST`, unparseable terms are an error). In the app: the drafting screen (intent → drafter → editor with live lint → send) and the requester's own counter-offer (`App::{request_terms, counter}`). Tests: `acn-author` unit, `requester_counters_and_atlas_answers` (playthrough), `drafted_terms_by_reference_and_requester_counters` (`live`), `presenter.mjs` | ✅ |
| The payment is locked when the job is commissioned | §8.1–8.2 | `H` in the terms; `nmesh-wallet` locks the price as a NUT-14 HTLC token before the `REQUEST` is signed, NIP-44-encrypted in `content.token`. The requester first checks the mint's own NUT-06 capabilities and won't publish if it can't save `s`. The performer verifies the token with the mint (DLEQ, NUT-07) and checks every proof's complete spending policy, distinct proofs and named keysets (§8.6 "Received locks") before working, or declines `payment_invalid`. Tests: `htlc` (incl. `inspect_rejects_hostile_tokens`), `acn-wallet` unit, `performer_waits_for_and_obeys_the_payment_check`, `live`. Not under the contingent trigger: see §4 | ✅ test mint |
| An agent that accepts signs those exact terms | INV-4, V-9 | `PROMISE` / `ACCEPT` carry the hash of the terms in force; `terms_mismatch` otherwise; terms too large to inline are bound by hash (§6 row). Tests: `n_counter_accept`, `counter_while_promised` (scenarios) | ✅ |
| It reports the work along with its evidence | §6.2, §9 | `REPORT` with an inline result and `x` artifact hashes; the reference bindings (`acn_binding::Performer`) and the demo cast (`with_evidence`) store artifacts and a derivation record on Blossom (`nmesh-blossom`); the card shows what each attester signed and links the artifacts. Tests: the L0 suite and `live` fetch and hash-check every artifact and record | ✅ dev Blossom server |
| The result is checked by tests, by comparison with real data, or by a person | §7 condition classes, INV-8 | Classes `mechanical`, `grounding`, `judged`, `interpretive`, `human`, with evidence rules per class (V-11, V-17); shown in the card's Conditions panel. Tests: `evidence`, J-1…J-18, C-vectors | 🟡 the verifiers are scripted cast members (Verity, Muse); real checkers exist only for judged conditions (`acn-judge`, with `--judges`); no grounding verifier, no requester-side runner |
| If the requester is satisfied, saying so releases the payment | INV-6, V-10 | `SATISFY` carries the preimage (`bad_preimage` if wrong); the performer redeems at the mint. Tests: `t1_preimage` (scenarios), `htlc`, `live` (balances move) | ✅ test mint |
| Otherwise the requester names the failed condition; rework if the terms allow, else Rejected; a named arbiter can rule | §6.2 `REJECT`, §8.4 | `REJECT` names a condition (gap); `Reworking` below `rework_limit`, then `Rejected`; `DISPUTE` / `RULING` with the arbiter holding the escrowed secret. Any L1 binding reworks with the rejection's gaps and disputes rejections the evidence contradicts. Tests: *rework* and *dispute* in `playthrough` and `live` (Atlas redeems after the ruling); L1 cases `n-rework`, `n-contradicted-rejection-disputed` (both bindings, paid redemption after the ruling) | ✅ |
| If the agent never delivers, the requester reclaims the money once the time lock expires | §6.5, §8.2 | Lapse derived by `status_at`; settlement "refundable"; NUT-14 refund after the locktime (`CashuWallet::refund`). An interrupted refund is recovered on reopen; refunds pay a fee-bearing mint's fees; a lock whose request was never published stays refundable. Tests: `time_status` (scenarios), `htlc`, `refund_recovers_after_interruption`, `refund_pays_input_fees` | ✅ real mint in tests; the live demo waits the real ≈ 25 h locktime, presenter mode fast-forwards |
| The time lock is set safely after the deadline and the review period (also §4) | §8.2 timing constraint (MUST) | The reference requester sets `L = deadline + review_window + 24 h` (`nmesh-app`, `acn_core::draft`); performer bindings decline paid terms whose locktime doesn't exceed `deadline + review_window` plus their redeem margin (`locktime_too_early`, harness-binding-API rule 16). Tests: `paid_terms_need_a_locktime_after_the_review_window`, L0 case `decline-locktime-too-early` (both bindings) | ✅ in the reference bindings; the validator can't check it (no clock) |
| Each step is a signed message that cannot be altered afterwards | INV-1, INV-11 | Regular signed events; rejected events are kept and listed under "Show signed events"; a corrupted copy of a known act never replaces the verified original, in the app or the bindings. Tests: `every_anomaly_code_is_exercised`, `a_corrupt_copy_never_replaces_a_verified_act`, `corrupt_copy_does_not_replace_a_verified_report` | ✅ |

### §4 Three design choices

| Whitepaper claim | Spec | Prototype and tests | Status |
|---|---|---|---|
| Only the person who asked can say the job is done; the performer can only report | INV-5, V-5 | `SATISFY` is requester-only (`unauthorized` otherwise); a `REPORT` never closes. Tests: `every_act_kind_is_exercised`, scenarios for `unauthorized`. Under the verifier and contingent triggers (next rows) a `RELEASE` or `CLAIM` settles, and the requester's later act is a recorded judgement | ✅ under the requester trigger |
| Saying "done" is what pays; no escrow company holds the money | INV-6, §8 | The preimage in `SATISFY` is the payment; the mint enforces the lock and nobody else holds funds. Tests: `t1_preimage`, `htlc`, `live` (*paid*) | ✅ test mint |
| Evidence and opinion are kept apart; an opinion is not proof | INV-8, V-11 | Interpretive reports never count (`wrong_evidence_class`); a judged attestation counts only for a `judged` condition and is labelled `opinion`. Tests: `evidence`, J-2, `cr001_judged_condition_cannot_be_satisfied_by_the_wrong_evidence`, *verified* in `playthrough` | ✅ |
| A rejection of work that passed its tests is visibly flagged | V-16 | `contradicted_rejection`; for passed judged conditions, `overruled_judgement` (D-28). Tests: `evidence`, J-15…J-17, *verified* | ✅ |
| (added, CR-001) An LLM judge's score may decide payment, labelled as an opinion | INV-8, §7, V-17 | `judged` conditions, `K_ATTEST_JUDGED`, derived assurance label shown before signing; real judges with `acn-judge/2`, which requests exactly the pinned model, fails a run whose provider reports another, and treats a malformed reply as a failed run. Tests: J-vectors, `acn-judge` unit and `end_to_end` (incl. `malformed_replies_and_other_models_fail_closed`), `live -- --ignored` with real Ollama judges | ✅ live mode; presenter mode scripted. The provider is trusted to run the named model; `model_version` is a claim |
| (added, CR-002) Payment without the requester's act, for checkable work | §8.2, V-19…V-23 | Verifier trigger (`RELEASE`); contingent trigger (sealed delivery, `FUND`, `CLAIM`). A `RELEASE` counts only evidence signed before it, so later evidence can't undo a settlement; withholding `CLAIM` gains nothing, because the requester's app reads `k` from the mint. Tests: S-vectors incl. S-5b, S-7c, C-6g; `withheld_claim_key_is_recovered_from_the_mint`; *judged* and *contingent* in `live` | ✅ test mint |
| (added, CR-003) No hidden criteria: everything the work is judged against is readable before the agent signs; open conditions closed before a promise; disqualifiers; held-out cases drawn by both parties | INV-14, §7, V-24…V-28 | `hidden_criteria`, `unclosed_condition`, `disqualified`, `seed_mismatch` / `seed_withheld` / `instance_mismatch`; the terms card; judges see only the sample both seeds select, revealed with proofs. Tests: C-vectors, `real_panel_judges_held_out_cases`, *drafted*, *disqualified*, *held-out* | ✅; 🟡 the requester can't be its own runner yet (seed kept, runner not built) |

### §5 What ACN deliberately is not

| Whitepaper claim | Spec | Prototype and tests | Status |
|---|---|---|---|
| Not a platform or marketplace; anyone can implement it | INV-13 | Open spec; the validator is a pure crate with no I/O, built natively and for the browser (`acn-wasm`); two independent bindings pass the same suite | ✅ |
| Not an AI: it records and settles | §10 | The validator is a pure function of signed events and documents; judges run outside it, and it only counts the scores a listed runner signed (V-17). Test: `fold_is_order_independent` | ✅ |
| Not a judge of quality: the parties choose who checks | §7 `verifiers`, `runners` | Verifiers listed per class in the terms; attestations from anyone else are `unlisted_verifier` and not counted. Tests: scenarios, `live` (*verified*) | ✅ |
| Not a reputation authority: anyone can compute a track record from public or disclosed commitments; many scorers | §11 `K_SCORE`, D-13 | Kind reserved; no scorer; commitments are private by default and there is no disclosure mechanism (D-13 open). Per-template records are computed by each reader over what it can see (`acn_view::template_records`) | ⬜ |
| Not tied to any framework: a small set of interfaces; a thin adapter makes a tool server accountable | §11, D-10 | The Binding API as traits (`acn-binding`) with a performer driver; `acn-mcp-shim` (any MCP server) and `acn-cli-shim` (any CLI harness) pass the L0 conformance suite (`acn-conformance`), priced and free, terms inline or by reference. The driver journals execution, applies one admission policy to history and live events, checks its operator scope before working, and keeps evidence private unless configured public (harness-binding-API rules 8–15). Tests: both L0 runs, `acn-binding` unit, bridge `startup.rs` | ✅ L0 (T0, T1) and L1 (profile N: counters, rework, disputes, held-out seeds, disqualifiers; `acn_conformance::l1`, passed by both bindings); ⬜ L2–L3 (governance, verifiers) and requester bindings on the traits |

### §6 Built on open rails

| Whitepaper claim | Spec | Prototype and tests | Status |
|---|---|---|---|
| Nostr for messages: own identity, signed messages, independent relays, no owner | §5 | `nostr-sdk`; validation never depends on the relay; sessions republish their public events after a relay restart. Tests: `dm_roundtrip`, `relay_restart` | ✅ dev relay; 🟡 single relay, no outbox / inbox-relay routing |
| Commitments can travel privately, each step still individually signed so it can be shown to a verifier or arbiter | §5.5 | `K_PRIVATE_ACT` rumor inside NIP-17 gift wraps; each act verifies alone. Evidence is private unless a binding is configured public, card images load only on click, and other parties' URLs are fetched only to public addresses under bounds. Tests: `app_flow`, `live`, `untrusted_fetches_are_bounded`, `private_disclosure_uploads_nothing` | ✅ |
| (added, P2) Terms too large to inline travel by hash, so the signed act still binds exactly one document | §7, §6.2, INV-4 | `terms` tag `["terms", <sha256>, <server hint>]`; fetched, hash-checked, folded; the act stays pending until the document arrives (`PendingTerms`). Tests: `ref_tests`, L0 cases `t0-terms-by-reference` / `t1-terms-by-reference`, `live` | ✅ dev Blossom server |
| Cashu for payment: locked with conditions, no escrow agent | §8 | CDK 0.18 wallet (`nmesh-wallet`) and test mint (`nmesh-mint`): NUT-14 HTLC with the preimage path for the performer and the refund path for the requester. Tests: `htlc` | ✅ test mint |
| Cashu is private by design | §8 | Tokens are NIP-44-encrypted to the performer, so relays see neither amount nor mint | 🟡 test mint only; no real mint, NIP-60 backup or NWC; preimages and proofs in plaintext on disk |

### §7 Rigour as a market term

| Whitepaper claim | Spec | Prototype and tests | Status |
|---|---|---|---|
| A checkpoint in front of an agent's commitments runs checks, asks a human where needed, and publishes a signed endorsement | §9 `K_ENDORSE`, V-14 | Endorsement validation; the Gate interface (`acn-binding`, rule 1); the *governance* demo's scripted Gate. Tests: `governance` (scenarios), *governed* in `playthrough` | 🟡 no gate product, no human sign-off UI; the reference bindings decline governed terms at L0 (`governance_unsupported`, `governance_is_declined_before_work`) |
| A requester can require endorsement from named overseers in its terms | §7 `governance` | `governance.keys / kinds / window`; `pending_endorsement` / `unendorsed`. Tests: `governance` | ✅ validator; 🟡 no reference performer can meet such terms yet (L2) |
| Overseers build reputations; one that approves everything is visible | §11, §13 | Endorsements are signed records; no scoring | ⬜ |

### §8 Aspirations

| Whitepaper claim | Spec | Prototype and tests | Status |
|---|---|---|---|
| Agents become hireable: advertise a service, take work from strangers, be paid on delivery, without a platform | §4 `K_SVC`, §8 | `K_SVC` with per-tool terms offers and mint capabilities; discovery by service advertisement (`discover_demo`); paid T1 calls to any binding. Tests: `live`, both L0 runs | 🧭; 🟡 the mechanism exists at test-mint scale |
| Track records replace marketing, including how well an agent predicts its deadlines | §11, §6.2 `p_on_time`, D-13 | Template records over commitments a reader can see; `p_on_time` recorded, not scored | 🧭; ⬜ scorers, Brier scoring, disclosure |
| Verification becomes an industry | §7 `verifiers`, `runners`; harness-binding-API L3 | The verifier and runner roles are in the protocol; the judge harness is a reference runner | 🧭; ⬜ the L3 suite and a verifier binding |
| Better contracts get written | §7 templates, CR-003 §3 | Terms templates with notes, the linter (L1–L8), requester and performer drafters, the drafting screen. Tests: `acn-author` unit, template vectors C-15…C-16e, `templates.rs` | 🧭; ✅ the authoring aids |
| Humans stay in charge where it matters | §7 `governance`, §9 | See §7: validated, not productised | 🧭; 🟡 |

### §9 Where it stands

| Whitepaper claim | Spec | Prototype and tests | Status |
|---|---|---|---|
| A reference validator for the full cycle: the same verdict in any order and on any relay, natively and in a browser | §10, INV-2 | `acn_core::fold`; the WASM build drives presenter mode. Tests: `fold_is_order_independent` (every permutation up to 6 events, a fixed sample above), `every_act_kind_is_exercised`, `presenter.mjs` | ✅ |
| A reference adapter makes a tool server accountable, bound to its owner's key | §11 | Two bindings at L0 (§5 row); the operator binding is issued at install and never re-issued after revocation. Tests: both L0 runs, `revoked_binding_stops_work` | ✅ (the whitepaper understates this; see below) |
| A messaging app with a built-in Cashu wallet: lock at request, the agent checks with the mint, accepting lets it collect, unclaimed money returns | §8.2 | `nmesh-app` and the desktop app; wallet panel. Tests: `live`, `htlc`, `presenter.mjs` | ✅ test mint |
| A demonstration of the full range, live over Nostr or in a browser | — | Sixteen scenarios (`acn_sim::SCENARIOS`). Tests: `playthrough`, `live`, `presenter.mjs`, `board.mjs` | ✅ |
| Not built: real money; public track records and scoring; an arbitration service | §8, §11, §8.4 | Arbiters exist only as the demo's scripted Solon | ⬜ |
| Limitation: mints are trusted; exposure reduced by naming the mint and capping amounts | §8.6, binding `max_sat` | The mint is named in the terms; the performer's `max_sat` is enforced by the validator against the terms each act commits to (`out_of_scope`, SC-1…SC-7b), and the binding declines over-scope requests before working (`request_above_operator_scope_is_not_worked`); the app pays only at its configured mint | 🟡 one test mint; no multi-mint policy; the requester's wallet applies no caps |
| Limitation: an arbiter holds the payment secret and could collude | §8.4 | NIP-44 escrow to the arbiter; the *dispute* demo, where the ruling releases `s`. Test: `live` | ✅ (as a stated limitation) |
| Limitation: clocks are not trusted; each step is dated no earlier than the one before | INV-3, V-15 | `time_regress`; deadlines compared under monotonicity; evidence counted only up to the deciding act. Tests: `time_regress`, `late`, C-6g | ✅ |

### §10 Foundations

| Whitepaper claim | Spec | Prototype and tests | Status |
|---|---|---|---|
| Conversation for action: request, promise, report, declaration of satisfaction, each a signed machine-checkable act | §6 | The 16-act state machine (`acn-core/src/fold.rs`: v0.2's 13 plus `RELEASE`, `FUND`, `CLAIM`). Test: `every_act_kind_is_exercised` | ✅ |
| When an agent commits, it states how likely it is to meet the deadline; scorers grade it with the Brier score | §6.2 `p_on_time` | `p_on_time` in `PROMISE`, range-checked when present | 🟡 optional, not required; no Brier scoring |

## Whitepaper text that no longer matches (revise on request)

The whitepaper was last revised against spec v0.2 (Nov 8). These passages are now out of date:

- **Intro and §4** ("released only when the person who asked is satisfied"; "only the
  requester's declaration of satisfaction can release" the payment): under the verifier and
  contingent triggers (CR-002) a settlement verifier's `RELEASE` or the performer's `CLAIM`
  releases it, and an arbiter's `RULING` already could under v0.2.
- **§3** ("The payment is locked at that moment"): not under the contingent trigger, where the
  requester pays (`FUND`) after the work is attested.
- **§4** ("never lets an opinion stand in for a check"): still true per condition, but a `judged`
  condition (CR-001) lets pinned LLM judges' scores decide payment, labelled `opinion`.
- **§5** ("a small set of interfaces, which are being specified now… the reference adapter
  already lets an MCP tool server take on simple commitments"): the interfaces are published as
  traits with L0 and L1 conformance suites, and two independent bindings (MCP and any CLI
  harness) pass both, including paid and negotiated commitments. L2–L3 are still being specified.
- **§6** ("or back to the requester after the deadline"): after the locktime, which is later
  than the deadline plus the review window (§3 says this correctly).
- **§9** ("A specification (v0.2 logical design)"): the spec is v0.5.
- **§9** ("A reference adapter… for simple, free calls"): also paid (hash-locked) calls, with
  evidence on Blossom, and a second binding for command-line harnesses.
- **§9** (the app bullet): the app also drafts terms (drafter and linter), counters an agent's
  counter-offer, and shows what each verifier signed.
- **§9** (the demonstration bullet): it also covers judged conditions, settlement triggers,
  drafted conditions, disqualifiers and held-out tests.
- **§9** ("What is not built yet: … the published interface for connecting other agent
  systems"): built at L0 and L1 (negotiated commitments); L2 and L3 are not.
- **§10** ("When an agent commits, it also states how likely it thinks it is to meet the
  deadline"): `p_on_time` is optional in the validator.

## Gaps the whitepaper depends on (to build or keep caveated)

- Real money: real mints, NIP-60 wallet backup, NWC top-up, a multi-mint policy, the
  requester's own per-mint caps, encrypted preimages and proofs.
- Binding API beyond L1 (D-10): the L2–L3 suites; Gate and Verifier in the driver;
  the requester side and the demo cast on the traits; a requester-side runner.
- Real verifiers: a grounding verifier, a verifier binding (L3), a gate with human sign-off.
- An arbitration service (arbiters are scripted in the demo).
- `K_SCORE` scorers, including Brier scoring of `p_on_time`.
- Public (non-private) commitments and selective disclosure to third parties (D-2 remainder),
  which track records need.
- **Reputation under private commitments: open decision D-13 in Core.md.** Until it is
  resolved, the whitepaper's track-record claims (§5, §8) describe intent rather than mechanism.
- Blossom beyond the dev server (deletion, quotas, a mirroring policy); relay routing beyond a
  single relay.

## Revision history of the whitepaper (Nov 8)

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
