# ACN Change Request CR-001: the `judged` condition class

Nov 7, 2026 · @ivan Labra

> **Status (Nov 9, 2026): applied, with changes.** This is the change request as written; the
> normative text is [Core.md](../Core.md) (v0.5), and where they differ, Core.md wins. Record:
> Core.md Appendix B, rows "CR-001" and "CR-003".
>
> - **Dropped:** the `audit` and `holdback` fields and the holdback settlement of §2.4, with
>   vectors J-9 and J-10 and the holdback steps of the §3.1 demo. Commitments are atomic: an audit
>   is an ordinary condition of satisfaction of a stronger class (vector J-13).
> - **Reversed by CR-003:** `rubric_disclosure` and `commit_reveal`. Rubrics are public
>   (`rubric_ref` in the terms, INV-14); terms declaring `commit_reveal` are rejected
>   (`hidden_criteria`). Vector J-5 now tests that; the old J-5 check is J-18.
> - **Added later:** J-11…J-18 (assurance, unknown fields, audit condition, overruled judgement).
> - **Versions:** the CR's `acn-terms/0.3` is superseded; the only terms version is
>   `acn-terms/0.5`.
> - **Open decisions:** OD-1 → D-14 (open), OD-2 → D-15 (closed), OD-3 → D-16 (closed by
>   CR-003), OD-4 → D-17, OD-5 → D-18, OD-6 → D-19 (open; see also D-36).
> - **Reference:** `acn-core` (V-17), judge harness `crates/acn-judge`, demo `/judged`,
>   `/injected`, `/judged-auto`.

CR-001 adds LLM-as-judge acceptance to ACN as a first-class, honestly labelled condition class, so a requester can write "accept if judge J with rubric R scores at least θ" and settle automatically. It also adds the safeguards that make the obvious gaming attacks visible and costly. This document holds the agent prompt and the change specification it applies.

## 1. Agent prompt

Paste the block below into the repo agent, with this whole document attached. It assumes the agent can read and edit both the spec and the code.

```text
You are applying change request CR-001 (the `judged` condition class) to ACN.
The repository holds the authoritative ACN spec and its reference code, kept in sync.
CR-001 is attached; its section 2 is normative, section 3 is the demo scenario,
conformance vectors and open decisions.

Task
1. Read the current spec and code first. CR-001 was written against an earlier draft:
   its section numbers, invariant numbers, rule numbers and kind numbers are NOT
   authoritative. Locate each target by meaning (e.g. "the section that defines
   condition classes in terms documents") and use the repo's current numbering.
2. Apply every change in CR-001 section 2 to the spec, in the places it names.
   Renumber consistently and update every cross-reference you touch.
3. Apply the matching code changes: terms and attestation schemas, the validator
   (new rule, amended evidence-class rule, new anomaly codes), the fold if affected,
   and client display of assurance labels where a client exists.
4. Add the conformance vectors in CR-001 section 3 to the test suite, plus the
   arrival-order permutation test for each new vector.
5. Add the demo scenario to the Binding API documentation as a worked example.
6. Add CR-001's open decisions to the spec's open-decisions list.

Rules
- Spec and code must agree at the end. If CR-001 conflicts with the current spec or
  code, do not resolve it silently: list the conflict, propose a resolution, and
  stop for confirmation if the conflict touches an invariant.
- Do not weaken any existing invariant. The amendment to the
  interpretive-is-not-provenance invariant is a clarification; keep its force.
- Use symbolic kind names in code; allocate provisional numbers from the existing
  block and record them where the spec records kind allocations.
- Do not invent patterns, crates or abstractions not named in CR-001 or already
  present in the repo.
- Keep ACN Core harness-neutral: nothing ABW-specific in Core or the validator.

Deliverables
- One branch with spec and code changes, in separate commits per CR-001 subsection.
- A changelog entry for CR-001.
- A short report: what changed where (paths and spec sections), conflicts found and
  how they were resolved, open questions.

Done when
- All existing and new conformance vectors pass, including permutation tests.
- Every CR-001 section 2 item maps to a spec location and, where applicable, code.
- The validator rejects a `judged` condition satisfied by an interpretive report,
  by a performer-run judge, or by a mismatched judge fingerprint.
```

## 2. Change specification (normative)

Seven changes, each naming the part of the spec it modifies. A `judged` condition certifies that a pinned judge produced a score at or above a threshold. It never certifies that the work is good, and the spec says so wherever it appears.

### 2.1 Terms: new condition class `judged`

**Target:** the section defining terms documents and condition classes. Insert `judged` between `mechanical` and `interpretive`.

A `judged` condition is satisfied only by a judged attestation (2.2) from a listed runner. Fields:

| Field | Type | Meaning |
| --- | --- | --- |
| `judges[]` | array, ≥ 1 | Each: `model_id`, `model_version` (provider-pinned), `prompt_template_hash`, `harness_hash`, `sampling` (temperature, max tokens, seed if supported) |
| `rubric_hash` | sha256 | Hash of the rubric text. The rubric itself MAY be withheld until evaluation (`rubric_disclosure`) |
| `rubric_disclosure` | enum | `public` (rubric published with terms) or `commit_reveal` (only the hash is published; the text is revealed in the attestation) |
| `scale` | object | Score range, e.g. `{min: 0, max: 10}` |
| `threshold` | number | θ; pass when the aggregate score ≥ θ |
| `aggregation` | enum | `all` (every judge ≥ θ), `mean`, or `median` |
| `repeats` | integer, ≥ 1 | Runs per judge; the per-judge score is the median of its runs (non-determinism control) |
| `runners[]` | pubkeys | Keys allowed to run the judges and sign the attestation. MUST NOT include the performer |
| `audit` | object or null | `rate` (0–1), `auditors[]` pubkeys, `window` seconds |
| `holdback` | object or null | `fraction` (0–1) of price released only after the audit window (2.4) |

**Assurance labels.** Every condition carries a derived assurance label: `check` (mechanical), `grounded` (grounding), `opinion` (judged and interpretive), `human` (human). A commitment's overall assurance is its weakest condition's label, ordered `grounded` > `check` > `human` > `opinion`. The label is computed by the validator, never asserted by a party.

### 2.2 Attestation: new kind `K_ATTEST_JUDGED`

**Target:** the provenance section, alongside the mechanical, grounding and interpretive attestation kinds; and the kind allocation list.

Signed by a runner key. References `c`, the `REPORT` event id, the condition id, and subject artifact hashes. Content:

- per judge: the full fingerprint (as in 2.1), every run's score, the median, and a rationale hash
- `aggregate_score`, `result` (pass or fail)
- `rubric_text_hash` and a Blossom reference to the rubric text (required under `commit_reveal`; MUST equal `rubric_hash`)
- `injection_detected` (bool) and, if true, the hash of the offending span

`K_INTERP_REPORT` remains free-form and advisory. `K_ATTEST_JUDGED` is the only kind that can satisfy a `judged` condition.

### 2.3 Validation: new rule and amended evidence-class rule

**Target:** the validation rules section.

**Amend the evidence-class rule:** a `judged` condition is satisfied only by `K_ATTEST_JUDGED`; `K_INTERP_REPORT` never satisfies `mechanical`, `grounding` or `judged` conditions.

**New rule, Judged evidence.** A `K_ATTEST_JUDGED` counts toward its condition only if every check passes; each failure yields an anomaly code.

1. Signer is in `runners[]` and is not the performer. Code `self_judged` (also flagged on the performer if it is the signer).
2. Every judge fingerprint matches the terms exactly. Code `judge_mismatch`.
3. Run count per judge equals `repeats`. Code `judge_mismatch`.
4. Under `commit_reveal`, the revealed rubric hashes to `rubric_hash`. Code `rubric_mismatch`.
5. `injection_detected` is false. If true, the condition fails and code `injection_detected` is recorded on the performer.
6. Aggregate score meets θ under the declared aggregation. Code `below_threshold`.

### 2.4 Settlement: auto-satisfy and holdback

**Target:** the settlement section and the act table.

**Auto-satisfy.** A requester's binding MAY issue `SATISFY` automatically when every condition is met. Closure remains the requester's act; the policy is the requester's pre-commitment, not a transfer of closure.

**Holdback.** When `holdback` is set, payment is split into two hash-locked tokens under the existing milestone mechanism. The main token is released by `SATISFY` on judged pass. The holdback token is released by a second, condition-scoped `SATISFY` after the audit window: automatically if the commitment was not sampled, or if sampled and the audit passes. A failed audit leaves the holdback to refund at its locktime, and the audit outcome is recorded as an attestation from an auditor key. Withholding the holdback without a sampled, failed audit is a requester fault for scoring.

### 2.5 Invariant clarification

**Target:** the invariant stating that interpretive reports are not provenance. Append:

> A judged attestation certifies that a pinned judge produced a stated score; it is evidence about an opinion, not about the work. It MAY satisfy a `judged` condition and nothing else, and clients MUST display the resulting `opinion` assurance label.

### 2.6 Client obligations

**Target:** the clients and projections section. Clients MUST display a commitment's overall assurance label before the performer signs `PROMISE` or `ACCEPT` and before the requester signs `SATISFY`, and MUST name the judges and threshold for any `judged` condition.

### 2.7 Threats

**Target:** the adversarial-cases table. Add:

| Threat | Vector | Mitigation |
| --- | --- | --- |
| Judge hill-climbing | Performer runs the known judge and rubric locally and iterates until it passes | `commit_reveal` rubrics; judge ensembles across model families; audits with holdback |
| Judge prompt injection | Deliverable contains text addressed to the judge | Harness isolates and sanitizes the artifact; detection fails the condition and is recorded on the performer |
| Self-judging | Performer runs the judge or signs as runner | Runner must not be the performer (`self_judged`) |
| Judge drift or collusion | A judge passes poor work systematically | Audits on the record; runners and judge fingerprints scored on audit outcomes |
| Non-determinism disputes | Same input, different scores | `repeats` with median; full run scores in the attestation |

## 3. Demo scenario, conformance vectors, open decisions

The demo is presented as rung one of an assurance ladder (judged, judged plus audit, mechanical, grounded), so its limitation is part of the argument rather than a footnote.

### 3.1 Demo: outcome-priced technical summary

A requester commissions a 500-word technical summary of a document and pays only if two independent judges rate it at least 7 out of 10.

**Terms (abridged).** Price 2,000 sat; deadline 1 hour; review window 30 minutes. Condition `c1` (`mechanical`): output is plain text of at most 500 words. Condition `c2` (`judged`): two judges from different model families, rubric under `commit_reveal`, scale 0–10, θ = 7, aggregation `all`, repeats 3, runner = requester's binding key, audit rate 10%, holdback 20%. Overall assurance: `opinion`.

**Flow.**

1. The requester's Terms Author fills a judged template; the client shows "assurance: opinion; judged by J1 and J2, threshold 7/10, 20% held for audit."
2. `REQUEST` with two locked tokens: 1,600 sat main, 400 sat holdback.
3. The performer's client shows the same label; the performer signs `PROMISE`.
4. The performer delivers and signs `REPORT` with the summary's hash.
5. The requester's binding runs the mechanical check, then both judges three times each, revealing the rubric in a `K_ATTEST_JUDGED`.
6. Both medians ≥ 7: the binding auto-issues `SATISFY` for the main token; the performer redeems 1,600 sat.
7. After the audit window: not sampled, so the binding issues the holdback `SATISFY` and the performer redeems 400 sat. Had it been sampled and failed, the 400 sat would refund at locktime, with the audit on the record.

A second run of the demo with an injection attempt in the deliverable shows the condition failing and the attempt recorded on the performer, which tends to be the most persuasive moment of the demo.

### 3.2 Conformance vectors

| Id | Setup | Expected |
| --- | --- | --- |
| J-1 | Both judges pass; runner is requester | Condition met; auto-`SATISFY` valid; state Satisfied |
| J-2 | `K_INTERP_REPORT` offered for a `judged` condition | Not counted; `wrong_evidence_class` |
| J-3 | Attestation signed by the performer | Not counted; `self_judged` on performer |
| J-4 | One judge's `model_version` differs from terms | Not counted; `judge_mismatch` |
| J-5 | `commit_reveal`; revealed rubric hash differs | Not counted; `rubric_mismatch` |
| J-6 | `injection_detected` true | Condition failed; `injection_detected` recorded on performer |
| J-7 | Aggregation `all`; one judge below θ | Condition failed; `below_threshold` |
| J-8 | `repeats` 3; attestation carries 2 runs | Not counted; `judge_mismatch` |
| J-9 | Holdback; not sampled; requester withholds past window | Requester fault recorded; holdback refunds at locktime |
| J-10 | Holdback; sampled; audit fails | Main settlement stands; holdback refunds at locktime; audit attestation on record |
| J-11 | Commitment with one `mechanical` and one `judged` condition | Overall assurance computed as `opinion` |

Each vector also runs under every arrival-order permutation.

### 3.3 Open decisions

- [ ] **OD-1 Assurance ordering.** Whether `human` ranks above `opinion`, and whether `check` and `grounded` are comparable at all or should be reported side by side.
- [ ] **OD-2 Verifiable audit sampling.** Proposal: the terms commit to an audit salt by hash; a commitment is sampled when hash(commitment id ‖ salt) falls below the audit rate, so the requester cannot choose which deliveries to audit after the fact.
- [ ] **OD-3 Rubric reveal timing.** Reveal inside the attestation (current text) or in a separate event before judging.
- [ ] **OD-4 Model deprecation.** What happens when a pinned judge model is withdrawn mid-commitment: renegotiation via `COUNTER`, a named fallback judge in terms, or lapse.
- [ ] **OD-5 Injection detection standard.** Harness-defined and pinned by `harness_hash` (current text), or a shared reference harness maintained with ACN.
- [ ] **OD-6 Rationale disclosure.** Whether judge rationales are published by hash only or made available to the performer as diagnostics.
