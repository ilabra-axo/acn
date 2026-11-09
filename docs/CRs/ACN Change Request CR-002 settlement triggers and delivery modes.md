# ACN Change Request CR-002: settlement triggers

Nov 8, 2026 · @ivan Labra

> **Status (Nov 9, 2026): applied (Phases A, B, C), with changes.** This is the change request as
> written; the normative text is [Core.md](../Core.md) (v0.5), and where they differ, Core.md
> wins. Record: Core.md Appendix B, rows "CR-002".
>
> - **Removed from this text (Nov 9):** the `delivery` field (`clear`, `staged`,
>   `encrypted_until_paid`) and staged delivery, which this CR first proposed (hence the file
>   name). Commitments are atomic: a staged job is a sequence of commitments, and the exposure of
>   the deliverable follows from the trigger (encrypted until paid only under `contingent`).
>   Vector S-2 was rewritten for the remaining invalid combination, S-3 retired, OD-4 closed.
> - **Deferred until a second value exists:** `fulfillment_proof` (`verifier_attested` is
>   implied) and `rail` (Cashu is the only rail).
> - **Renamed:** `VERIFIER_ACK` is the shared `K_ESCROW_ACK`, code `missing_escrow_ack`.
>   `rail_incapable` is a requester-binding check, not a validator code.
> - **Tightened:** the verifier trigger excludes `human`/`interpretive` conditions and arbiters;
>   `RELEASE` needs every condition passing (and, since CR-003, no disqualifier hit).
> - **Versions:** the CR's `acn-terms/0.4` is superseded; the only terms version is
>   `acn-terms/0.5`.
> - **Open decisions:** OD-1 → D-20, OD-2 → D-21, OD-3 → D-22, OD-5 → D-24, OD-6 → D-25 (open);
>   OD-4 → D-23 (closed: atomic commitments), OD-7 → D-26 (closed by the
>   [spike](../acn/spikes/cashu-preimage-visibility.md)).
> - **Reference:** `acn-core` (V-18…V-23), `acn-wallet`, `nmesh-wallet`, demo `/judged`,
>   `/contingent`.

CR-002 separates what a commitment checks from who can trigger payment and when the deliverable is exposed. A new terms field, `settlement_trigger`, lets a performer insist on zero-trust settlement as policy, without a separate protocol; under the contingent trigger the deliverable stays encrypted until paid. The requester and verifier triggers can be built now; the contingent trigger is fully specified but gated on a rail capability that a short spike must confirm.

## 1. Agent prompt

Paste the block below into the repo agent with this whole document attached. It applies on top of CR-001.

```text
You are applying change request CR-002 (settlement triggers) to ACN.
The repository holds the authoritative ACN spec and reference code, kept in sync.
CR-002 is attached: sections 2 and 3 are normative; section 4 holds a spike, phasing,
conformance vectors and open decisions.

Task
1. Read the current spec and code first. CR-002's section, invariant, rule and kind
   numbers are NOT authoritative; locate each target by meaning and use the repo's
   numbering. Assume CR-001 is applied.
2. Work in the phases of section 4.2, in order. Do not start Phase C until the
   spike in section 4.1 is reported and its decision recorded.
3. For each phase: apply the spec changes, the matching code (terms schema, act
   schemas, validator rules and anomaly codes, the fold, settlement and wallet
   integration, Binding API filters), and the conformance vectors for that phase,
   each also run under every arrival-order permutation.
4. Add CR-002's open decisions to the spec's open-decisions list.

Authorized invariant change
Section 3.7 amends the requester-closure and closure-is-settlement invariants. That
amendment is authorized by the spec owner; apply it exactly as written. Any OTHER
change that would touch an invariant: stop and ask.

Rules
- Default behaviour must not change: a commitment with no settlement_trigger behaves
  exactly as today (requester).
- Spec and code must agree at the end. Conflicts with the current spec or code are
  listed with a proposed resolution, never resolved silently.
- Use symbolic kind names; allocate provisional numbers from the existing block.
- Do not invent patterns, crates or abstractions not named here or already present.
  The Lightning integration in Phase C uses whatever Lightning dependency the repo
  already has; if none exists, stop and ask which one to use.
- Keep ACN Core harness-neutral.

Deliverables
- The spike report (section 4.1) as docs/acn/spikes/cashu-preimage-visibility.md.
- One branch per phase, separate commits per CR-002 subsection, changelog entries.
- A short report per phase: what changed where, conflicts, open questions.

Done when (per phase)
- All existing and new conformance vectors for the phase pass, with permutations.
- Commitments without the new fields pass every pre-CR-002 vector unchanged.
```

## 2. Specification A: terms fields and settlement mechanics (normative)

Two terms fields make settlement and exposure explicit. Each trigger is defined by one question: who holds the payment secret, and when it is revealed.

### 2.1 New terms fields

**Target:** the terms-document section.

| Field | Values | Default | Meaning |
| --- | --- | --- | --- |
| `settlement_trigger` | `requester`, `verifier`, `contingent` | `requester` | Who can cause payment to be released |
| `fulfillment_proof` | `verifier_attested`, `zk` | none | Required when `settlement_trigger` is `contingent`; how the encrypted deliverable is shown to pass its conditions. `zk` is reserved and rejected in this version |
| `settlement_verifier` | pubkey | none | Required when `settlement_trigger` is `verifier`, or when `fulfillment_proof` is `verifier_attested` |
| `rail` | `cashu`, `lightning` | `cashu` | Settlement rail. Must have the capabilities its trigger requires (3.4) |

### 2.2 Valid combinations

Each trigger has its own constraints; terms that break them make the `REQUEST` invalid (code `invalid_settlement_combo`). Commitments are atomic: one deliverable, one settlement. The deliverable is in the clear under `requester` and `verifier`, and encrypted until paid under `contingent`.

| `settlement_trigger` | Who must trust whom | Typical use |
| --- | --- | --- |
| `requester` | Performer trusts requester | Today's behaviour; subjective work |
| `verifier` | Both trust a scored, listed verifier | Deterministic or judged conditions without proofs |
| `contingent` | Requester trusts the attesting verifier not to leak the key (v1); no trust with `zk` (later) | Deterministic conditions; zero trust as the target |

Required constraints:

- `contingent` requires every condition to be `mechanical` (deterministic). Code `contingent_nondeterministic`.
- `contingent` requires a rail with `reveals_preimage_to_payer` (3.4). Code `rail_incapable`.

### 2.3 Settlement mechanics per trigger

**`requester` (unchanged).** The requester generates secret `s`, puts `H = sha256(s)` in the terms, locks payment to the performer and `H`, and reveals `s` in `SATISFY`.

**`verifier`.** The secret is handed to the verifier up front, so the requester cannot withhold it once the work passes.

1. The requester generates `s`, puts `H` in the terms, and sends `s` NIP-44-encrypted to `settlement_verifier` with the `REQUEST`.
2. The verifier decrypts `s`, checks `sha256(s) = H`, and publishes `VERIFIER_ACK`. The performer MUST NOT `PROMISE` before a valid `VERIFIER_ACK`.
3. After `REPORT`, the verifier evaluates the conditions. On pass it publishes `RELEASE` containing `s`; the performer redeems. On fail it publishes the failing attestation, and the requester may `REJECT` for rework.

**`contingent` with `verifier_attested` (v1).** The performer holds the secret, and the secret is the key to the deliverable.

1. `REQUEST` carries terms but no payment token. The performer `PROMISE`s.
2. The performer generates key `k`, encrypts the deliverable to ciphertext `C` (stored on Blossom), and computes `H = sha256(k)`.
3. The performer sends `k` NIP-44-encrypted to `settlement_verifier`, and publishes `REPORT` with `C`'s hash and `H`. State becomes Offered.
4. The verifier decrypts `C` with `k`, checks `sha256(k) = H`, runs every mechanical condition on the plaintext, and publishes a mechanical attestation that names `C`'s hash, `H`, the plaintext hash, and the results. It MUST NOT reveal `k` or the plaintext.
5. On a passing attestation, the requester publishes `FUND`: a payment locked to `H` on the declared rail (a hash-locked Cashu token to the performer, or payment of a Lightning invoice the performer issued with payment hash `H`).
6. The performer claims the payment, which reveals `k` to the requester through the rail, and publishes `CLAIM` containing `k`. The requester decrypts `C`.

**Larger work.** To cap exposure, split the work into a sequence of commitments, each settled on its own (any trigger). There is no staging inside a commitment.

## 3. Specification B: acts, states, validation, rails, policy (normative)

Four new acts, three new states, eight validation rules, a rail-capability declaration, and the authorized invariant amendment.

### 3.1 New acts

**Target:** the act-vocabulary table.

| Act | Signer | Legal from | Resulting state | Required content |
| --- | --- | --- | --- | --- |
| `VERIFIER_ACK` | `settlement_verifier` | Requested (verifier trigger) | unchanged; enables `PROMISE` | confirmation that the escrowed secret hashes to `H` |
| `RELEASE` | `settlement_verifier` | Reported (verifier trigger) | Settled (terminal) | secret `s`; reference to the passing attestations |
| `FUND` | requester | Offered (contingent) | Funded | rail, amount, lock reference bound to `H` from the `REPORT` |
| `CLAIM` | performer | Funded (contingent) | Settled (terminal) | key `k` |

Under `contingent`, `REPORT` moves Promised to **Offered** and carries the ciphertext hash and `H`.

### 3.2 New states and lapse rules

**Target:** the state machine and computed-transition rules.

- **Offered** (contingent): encrypted deliverable reported, awaiting attestation and funding.
- **Funded** (contingent): payment locked to `H`, awaiting the performer's claim.
- **Settled** (terminal): payment released by `RELEASE` or `CLAIM`. `Satisfied` remains the terminal state for the requester trigger.

After Settled, `SATISFY` and `REJECT` are legal as recorded judgements with no state change; they feed scoring only. Before `RELEASE`, a failing verifier attestation lets the requester `REJECT` for rework as today.

Computed lapses, each recorded as a fault of the named party:

- Verifier trigger: no `RELEASE` and no failing attestation within the review window → Lapsed; **verifier fault**; the payment refunds at locktime.
- Contingent: no `FUND` within the new terms field `fund_window` after a passing attestation → Lapsed; **requester fault**; the performer loses its effort, and the requester gains nothing, since it never received the key.
- Contingent: no `CLAIM` before the funding lock expires → the payment refunds; **performer fault**.

### 3.3 Validation rules

**Target:** the validation-rules section; add these rules and anomaly codes.

1. The terms combination is one of the five valid combinations in 2.2. Code `invalid_settlement_combo`.
2. Under `contingent`, every condition is `mechanical`. Code `contingent_nondeterministic`.
3. The declared rail has every capability the trigger requires (3.4). Code `rail_incapable`.
4. Under the verifier trigger, `PROMISE` is valid only after a `VERIFIER_ACK` from `settlement_verifier`. Code `missing_verifier_ack`.
5. `RELEASE` is signed by `settlement_verifier`, `sha256(s) = H`, and passing attestations from that verifier exist for every condition. Code `bad_release`.
6. A contingent attestation names the ciphertext hash and `H` exactly as in the `REPORT`. Code `attestation_mismatch`.
7. `FUND` binds to the `REPORT`'s `H`. A `FUND` published before a passing attestation is valid but flagged. Codes `fund_mismatch`, `premature_fund`.
8. `CLAIM` satisfies `sha256(k) = H`. Code `bad_key`.

### 3.4 Rail capabilities

**Target:** the settlement section and the service advertisement. A service advertisement declares, per rail, which capabilities it supports. Terms are valid only if the rail supports what the trigger needs.

| Capability | Needed by | Cashu | Lightning |
| --- | --- | --- | --- |
| `hashlock` | all triggers | yes (NUT-14) | yes (payment hash) |
| `timelock_refund` | all triggers | yes (NUT-14) | yes (HTLC expiry) |
| `p2pk` | requester, verifier | yes (NUT-11) | not needed (invoice is performer-bound) |
| `reveals_preimage_to_payer` | contingent | **unknown; spike 4.1** | yes, by construction |

### 3.5 Policy filtering

**Target:** the Binding API, `Executor.quote` and requester-side bindings. Bindings filter on `settlement_trigger`, `rail` and `settlement_verifier` as local policy. Example policies, configured per binding, not by the protocol:

- **Zero-trust performer:** accept only `contingent`.
- **Cautious requester:** refuse `contingent` and `verifier` unless `settlement_verifier` is in its trusted set.
- **Default commercial:** accept `requester` below a value threshold; require `verifier` above it, or split the work into smaller commitments.

### 3.6 Threats

**Target:** the adversarial-cases table. Add:

| Threat | Vector | Mitigation |
| --- | --- | --- |
| Key leak (contingent v1) | Verifier passes `k` to the requester, who decrypts without funding | Verifier scored and value-capped; residual risk until `zk` proofs replace verifier attestation |
| Withheld release | Verifier refuses `RELEASE` despite passing work | Mechanical results are reproducible, so the refusal is provable; verifier fault, arbiter path |
| Unfunded offer | Requester sees a passing attestation and never funds | Requester gains nothing (no key); requester fault recorded; optional requester bond (open decision) |
| Silent claim | Performer claims payment without revealing `k` | Contingent allowed only on rails with `reveals_preimage_to_payer` |
| Ciphertext swap | Ciphertext does not match the attested plaintext | Verifier decrypts `C` with `k` and attests the plaintext hash |
| False pass | Verifier and performer collude to pass bad work | Deterministic conditions make the false pass provable once the requester decrypts; verifier reputation destroyed |

### 3.7 Invariant amendment (authorized)

**Target:** the requester-closure and closure-is-settlement invariants. Replace them with:

> **Settlement authority.** Under `settlement_trigger = requester`, only the requester can declare satisfaction, and that declaration releases payment. Under `verifier` or `contingent`, payment is released by the designated trigger, and the requester's `SATISFY` or `REJECT` after release is a recorded judgement with no settlement effect. In every mode, a performer can never release payment to itself without the trigger's secret.
>
> **Settlement is one act.** Settlement is always a single signed act that reveals the payment secret: `SATISFY` (requester), `RELEASE` (verifier) or `CLAIM` (contingent).

## 4. Spike, phasing, conformance vectors, open decisions

One empirical question gates the contingent trigger on Cashu; everything else can be built immediately.

### 4.1 Spike: does Cashu reveal the preimage to the payer?

**Question.** When a performer redeems a hash-locked Cashu token with the preimage, can the payer reliably learn that preimage afterwards, without relying on the performer to publish it?

**Method** (timebox: two working days).

1. Read NUT-07 (token state checks), NUT-11 and NUT-14 as currently published, and record what the spec says about returning a spent proof's witness.
2. Against at least two mint implementations (a CDK mint and a Nutshell mint, both run locally), lock a token to performer pubkey + hash `H` with a timelock refund.
3. Redeem it as the performer with the preimage.
4. As the payer, check the state of the spent proofs and record whether the response includes the witness containing the preimage.
5. Note any mint configuration that changes the result.

**Report** at `docs/acn/spikes/cashu-preimage-visibility.md`: spec text with citation, per-implementation result with request and response samples, and a recommendation.

**Decision rule.**

- Guaranteed by the spec → Cashu gains `reveals_preimage_to_payer`; contingent is allowed on Cashu.
- Implementation-dependent → the capability is declared per mint in the service advertisement; contingent is allowed only with mints that declare it, and validators check the declaration.
- Not available → contingent is restricted to Lightning in this version.

### 4.2 Phasing

1. **Phase A, no rail dependency.** New terms fields and defaults; combination validation; verifier trigger (`VERIFIER_ACK`, `RELEASE`, Settled, verifier lapse); rail-capability declaration; policy filters; invariant amendment. Gate: vectors S-1 to S-8.
2. **Phase B, spike.** Section 4.1, report and recorded decision.
3. **Phase C, contingent.** `verifier_attested` contingent settlement on the rails Phase B allows: Offered and Funded states, `FUND`, `CLAIM`, `fund_window`, contingent attestation fields. Gate: vectors S-9 to S-15.
4. **Later, out of scope.** `zk` fulfillment proofs, which remove the verifier from the contingent path.

### 4.3 Conformance vectors

| Id | Setup | Expected |
| --- | --- | --- |
| S-1 | Commitment with no new fields | Behaves exactly as before; all pre-CR-002 vectors pass |
| S-2 | `verifier` with a `human` condition | `REQUEST` invalid; `invalid_settlement_combo` |
| S-3 | *(retired with staged delivery)* | — |
| S-4 | Verifier trigger; `PROMISE` without `VERIFIER_ACK` | Not applied; `missing_verifier_ack` |
| S-5 | Verifier trigger; passing attestations; valid `RELEASE`; later requester `SATISFY` | Settled; `SATISFY` recorded as judgement, no state change |
| S-6 | `RELEASE` with a secret that does not hash to `H` | Not applied; `bad_release` |
| S-7 | `RELEASE` signed by a key other than `settlement_verifier` | Not applied; `bad_release` |
| S-8 | Verifier silent past the review window | Lapsed; verifier fault recorded |
| S-9 | `contingent` with a `judged` condition | `REQUEST` invalid; `contingent_nondeterministic` |
| S-10 | `contingent` on a rail lacking `reveals_preimage_to_payer` | `REQUEST` invalid; `rail_incapable` |
| S-11 | Contingent happy path | Requested → Promised → Offered → (attestation) → Funded → Settled |
| S-12 | Contingent attestation names a different ciphertext hash | Not counted; `attestation_mismatch` |
| S-13 | `CLAIM` with a key that does not hash to `H` | Not applied; `bad_key` |
| S-14 | Passing attestation; no `FUND` within `fund_window` | Lapsed; requester fault recorded |
| S-15 | `FUND` published before a passing attestation | Valid; `premature_fund` flagged |

Each vector also runs under every arrival-order permutation.

### 4.4 Open decisions

- [ ] **OD-1 Zero-knowledge fulfillment proofs.** Proof system and zkVM choice, and which predicate classes are practical to prove.
- [ ] **OD-2 Requester bond for contingent.** Whether requesters post a small bond, forfeited on an unfunded passing offer, to compensate performer effort.
- [ ] **OD-3 Verifier with encrypted delivery.** Whether the `verifier` trigger should also keep the deliverable encrypted until paid, with the verifier holding both secrets.
- [x] **OD-4 Staged contingent delivery.** Closed: commitments are atomic; a staged job is a sequence of commitments, each of which can be contingent.
- [ ] **OD-5 Verifier payment.** Whether the settlement verifier is paid through its own ACN commitment with the requester or through a fee embedded in the terms.
- [ ] **OD-6 Lightning dependency.** Which Lightning integration to use if the repo has none.
- [ ] **OD-7 Cashu outcome.** Recorded result of the spike and the resulting capability rule.
