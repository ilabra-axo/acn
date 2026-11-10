# ACN CR-003 — Conditions of Satisfaction authorship

Nov 8, 2026 · @ivan Labra

> **Status (Nov 10, 2026): Phases A, B, C and D applied, with changes.** This is the change request as written; the normative text is
> [Core.md](../Core.md) (v0.5), and where they differ, Core.md wins. Record: Core.md Appendix B,
> rows "CR-003".
>
> - **Corrected in this text (Nov 9):** as first written, this CR assumed CR-001's holdback and
>   audit windows, which were dropped: commitments are atomic, and nothing is held back or
>   audited after settlement. The disqualifier `window` field (`until_satisfy` /
>   `until_audit_end`), the audit-window effects, the holdback in the card, the L9 lint rule and
>   the `audit` / `holdback` / `window` lines of the §4 templates have been removed below, and
>   vector C-8 rewritten. A disqualifier is checked before settlement; an audit is a condition
>   of the same commitment.
> - **Changed by decision:** terms versions, `prev` and `drafted_by` are derived from the signed
>   chain, not terms fields. A disqualifier hit makes `REJECT` legal under every trigger, and
>   while it stands any `REJECT` ends Rejected without rework; the requester's silence after a
>   hit is no lapse; the validator still applies a `SATISFY` or `FUND` signed after a hit (D-38).
>   `human`, `grounding` and `interpretive` disqualifiers are not allowed yet (D-29, D-30).
>   Held-out instances need `rework_limit` 0 (D-39); the encodings the CR left open are fixed
>   in Core.md §7 (RFC 6962 pool, sample ranking). Only `acn-terms/0.5` is accepted.
>   Templates (Phase C): `template_ref` carries the slot values themselves, not their hash; a
>   template covers conditions and disqualifiers only, so the §4 examples' `intent`, `price` and
>   `deadline` aren't template content; an unavailable template isn't flagged; records are per
>   template, not yet per slot range. The §4 examples are published as
>   [docs/acn/templates/](../acn/templates/) with §4.1 as
>   [docs/acn/cos-templates.md](../acn/cos-templates.md); `research-brief/v1` drops d1's
>   deliverable-derived pool (D-35).
>   Authoring aids (Phase D) are the `acn-author` crate: deterministic drafters that never
>   invent a key and the linter L1–L8 (L2 only when retrievability is known; L4/L5 by quality
>   words). Templates gain optional `notes`, one line per item, for the drafter's explanations.
>   See Core.md Appendix B, row "CR-003 Phase D".
> - **Open decisions:** OD-1 → D-33, OD-2 → D-34, OD-3 → D-32, OD-4 → D-35, OD-5 → D-29,
>   OD-6 → D-36, OD-7 → D-37 (all open); added D-30, D-31 (closed), D-38 (closed), D-39.
> - **Reference:** `acn-core` (V-24…V-28, `instances`, `template`), `acn-view` (terms card,
>   template records), demo `/drafted`, `/disqualified` (both from Atlas's `retry-change/v1`),
>   `/held-out` (mechanical and judged held-out cases; `acn-judge` feeds the sample to the judges).

CR-003 makes conditions of satisfaction (CoS) authorable inside the negotiation cycle. Either party may draft them, the performer typically in its counter-offer, and no commitment can form until every condition is closed and readable by the performer. It adds explicit disqualifiers, held-out evaluation instances with a two-party seed, and shareable terms templates. It assumes CR-001 and CR-002 are applied.

CR-003 reverses one CR-001 choice: `commit_reveal` rubrics are removed. Grading against criteria the performer cannot read before promising is not a commitment, so anti-gaming moves from hiding the criteria to hiding which instances get tested. The rule is public criteria, unpredictable instances, explicit disqualifiers.

## 1. Agent prompt

Paste the block below into the repo agent with this whole document attached. It applies on top of CR-001 and CR-002.

```text
You are applying change request CR-003 (conditions-of-satisfaction authorship) to ACN.
The repository holds the authoritative ACN spec and reference code, kept in sync.
CR-003 is attached: section 2 is normative for ACN Core; section 3 specifies
authoring aids that live in bindings, not Core; section 4 holds templates and best
practices for the docs; section 5 holds the demo, conformance vectors and open
decisions.

Task
1. Read the current spec and code first. CR-003's section, invariant, rule and kind
   numbers are NOT authoritative; locate each target by meaning and use the repo's
   numbering. Assume CR-001 and CR-002 are applied.
2. Work in phases, in order:
   Phase A: 2.1 (no hidden criteria), 2.2 (disqualifiers), 2.4 (closure before
            promise, drafting in COUNTER), 2.7 client obligations.
   Phase B: 2.3 (held-out instances and the two-party seed).
   Phase C: 2.5 (terms templates).
   Phase D: section 3 authoring aids, in the reference binding only.
3. For each phase: apply the spec changes, the matching code (terms schema, act
   schemas, validator rules and anomaly codes from 2.6, the fold if affected,
   client display), and that phase's conformance vectors from section 5, each also
   run under every arrival-order permutation.
4. Put section 4 into the docs as docs/acn/cos-templates.md, with both worked
   examples as template files that validate against the Phase C schema.
5. Add CR-003's open decisions to the spec's open-decisions list.

Authorized invariant change
Section 2.1 removes `commit_reveal` from CR-001's `rubric_disclosure` and adds the
no-hidden-criteria invariant. Both are authorized by the spec owner; apply them
exactly as written, including the CR-001 threat-table and vector updates in 2.1.
Any OTHER change that would touch an invariant: stop and ask.

Rules
- Default behaviour must not change for commitments that use none of the new
  fields, except that terms declaring `commit_reveal` are now rejected.
- Spec and code must agree at the end. Conflicts with the current spec or code are
  listed with a proposed resolution, never resolved silently.
- Use symbolic kind names; allocate provisional numbers from the existing block.
- Do not invent patterns, crates or abstractions not named here or already present.
- Keep ACN Core harness-neutral. Drafters and linters (section 3) never enter Core
  or the validator; Core only validates the terms they produce.

Deliverables
- One branch per phase, separate commits per CR-003 subsection, changelog entries.
- A short report per phase: what changed where, conflicts, open questions.

Done when (per phase)
- All existing and new conformance vectors for the phase pass, with permutations.
- The validator rejects: terms with non-public rubrics; a PROMISE or ACCEPT against
  terms with an unclosed condition; a satisfied commitment with a disqualifier hit;
  a judged attestation whose sampled instances do not derive from both seeds.
```

## 2. Change specification (normative)

Seven changes, each naming the part of the spec it modifies. Together they make authorship part of negotiation and replace hidden criteria with three mechanisms: disqualifiers, held-out instances and templates with a record.

### 2.1 No hidden criteria (amends CR-001)

**Target:** the invariants list; CR-001's `judged` condition fields, judged-evidence rule, threat table, demo and vectors.

**New invariant, No hidden criteria:**

> Every criterion a deliverable is evaluated against MUST be readable by the performer before it signs PROMISE or ACCEPT. Criteria are: rubric text, mechanical check definitions, disqualifiers, and the description and generation rule of any instance pool. Only the selection of evaluation instances may be unpredictable, and only by the mechanism in 2.3.

**Changes to CR-001:**

- `rubric_disclosure` is removed. Terms carry `rubric_hash` plus `rubric_ref`, a retrievable reference (Blossom) to the rubric text. Terms that declare `commit_reveal` or omit `rubric_ref` are rejected with `hidden_criteria`.
- Mechanical checks referenced only by hash also get a retrievable reference. A check definition the performer cannot fetch is a hidden criterion.
- Judged-evidence check 4 becomes: the rubric text at `rubric_ref` hashes to `rubric_hash`. Code `rubric_mismatch` is kept.
- Threat "Judge hill-climbing": mitigation becomes held-out instances (2.3), judge ensembles across model families, audit conditions of a stronger class, and a disqualifier for rubric-term stuffing.
- Vector J-5 becomes: terms declare `commit_reveal`; expected: rejected, `hidden_criteria`.
- CR-001 demo 3.1 uses a public rubric. CR-001 OD-3 (rubric reveal timing) is closed by this change.

### 2.2 Disqualifiers

**Target:** the terms document section. New top-level list `disqualifiers[]`, beside `conditions[]`.

A disqualifier is a public predicate about the deliverable or the way it was produced. When attested as hit, the commitment fails regardless of how its conditions scored. Because the performer accepted it as a term, a hit is breach, not a quality dispute.

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | Unique within the terms, e.g. `d1` |
| `statement` | string | The disqualifying act in plain language, e.g. "modifies or deletes existing tests" |
| `class` | enum | `mechanical`, `judged` or `human`; the same fields as a condition of that class (runners, judges, rubric, declarer) |
| `instances` | object or null | Optional sampling, as in 2.3, e.g. citations sampled for source verification |

A disqualifier is checked before the commitment settles; commitments are atomic, so there is no window after settlement.

**Attestation.** A disqualifier is checked with the attestation kind of its class, referencing the disqualifier id, with `result` of `hit` or `clear`. Only keys named in the disqualifier may sign it; any other signer gives `unsupported_disqualification`, recorded on the signer.

**Effects.**

- A hit before SATISFY: the requester MUST NOT issue SATISFY, auto-satisfy is blocked, and the commitment takes the spec's performer-breach path. Code `disqualified` is recorded on the performer with the disqualifier id.
- Withholding payment on an attested `clear` result, or on no attestation, is a requester fault, as for any review lapse.
- Reputation projections SHOULD weight `disqualified` as breach and `below_threshold` as a quality miss; the codes are distinct for this reason.

**Assurance.** Disqualifiers carry their own derived assurance label, displayed beside the commitment's label. They do not enter the commitment's overall label, which still describes how satisfaction is established.

### 2.3 Held-out instances and the two-party seed

**Target:** CR-001's `judged` condition, `mechanical` conditions that take test inputs, and disqualifiers (2.2).

The criteria are public; the cases tested against them are drawn at evaluation time from a fixed pool. This is the syllabus-versus-exam-questions distinction, and it is what makes overfitting to known cases unprofitable without hiding any criterion.

Optional `instances` object:

| Field | Type | Meaning |
| --- | --- | --- |
| `pool_description` | string | Public: what kind of cases the pool holds |
| `pool_rule` | string | Public: how the pool was generated or curated |
| `pool_root` | sha256 | Merkle root over the pool items, fixed in the terms version |
| `pool_size`, `sample_size` | integer | Sample size ≤ pool size |
| `requester_seed_hash` | sha256 | Committed in the terms version |
| `performer_seed_hash` | sha256 | Committed in the act that binds the performer: its PROMISE or ACCEPT, or the COUNTER the requester then accepts |

**Derivation.** The sample seed is the SHA-256 of `requester_seed ‖ performer_seed ‖ commitment_id`. Pool indices are ordered by the SHA-256 of `seed ‖ index`, and the first `sample_size` indices are the sample. The performer reveals `performer_seed` in REPORT. The runner reveals `requester_seed` and the sampled items with Merkle proofs in the attestation.

Neither party alone controls the sample. The requester cannot grind a seed toward the hardest cases, and the performer cannot know the cases before delivery.

**Failures.** A seed that does not match its hash gives `seed_mismatch`. A sample that does not follow the derivation, or an item that does not prove against `pool_root`, gives `instance_mismatch`; the attestation is not counted. A REPORT without the performer's seed is incomplete and gives `seed_withheld` on the performer. A runner that does not reveal within the review window falls under the spec's existing review-timeout rule.

### 2.4 Drafting in the negotiation cycle; closure before promise

**Target:** the act table (REQUEST, COUNTER, PROMISE, ACCEPT) and terms versioning.

**Terms versions.** Every REQUEST or COUNTER that carries terms creates a terms version with `version`, `prev` (hash of the previous version) and `drafted_by` (pubkey). If the spec already versions countered terms, add `drafted_by` only.

**Open conditions.** A condition MAY have class `open`, with `id`, `statement` (the requester's intent in prose) and an optional `suggested_class`. `open` is allowed only in REQUEST and COUNTER. A disqualifier is never `open`.

**Closed condition.** A condition is closed when it has a class other than `open`, every field that class requires, a named evidence producer (runner, verifier or declarer pubkey), and only public criteria (2.1). A disqualifier is closed on the same test.

**Closure rule.** PROMISE or ACCEPT is valid only against a terms version in which every condition and disqualifier is closed. Otherwise the act is not counted and gives `unclosed_condition`. Underspecified requests are allowed; underspecified promises are not.

**Performer drafting.** The performer's COUNTER MAY close open conditions, add or change conditions and disqualifiers, and change any other field. A performer-drafted version binds only after the requester's act on that version, under the existing counter semantics. Requester-side drafting uses the same mechanism: a REQUEST may already carry closed terms.

### 2.5 Terms templates

**Target:** the kind allocation list and the terms schema.

**New kind `K_TERMS_TEMPLATE`.** A signed, parameterized terms fragment: conditions, disqualifiers and instance specs with typed slots (`string`, `number`, `pubkey`, `sha256`, `duration`, `sats`), optional defaults, and optional `guidance` prose per slot. It is versioned by content hash.

**Reference from terms.** Terms MAY carry `template_ref` with the template event id, its content hash and a hash of the slot values. Terms stay self-contained: the validator evaluates the instantiated terms, and `template_ref` is provenance only.

**Check.** If re-instantiating the template with the slot values does not reproduce the referenced conditions and disqualifiers exactly, the reference is not counted and gives `template_mismatch`. The commitment remains valid without it.

Templates exist so that a track record (pass rate, dispute rate, audit agreement) can attach to a definition of done, not only to a party. Those records are reader-computed projections, not Core state.

### 2.6 Validation: new rules and anomaly codes

**Target:** the validation rules section and the anomaly code list.

| Code | Raised when | Recorded on | Effect |
| --- | --- | --- | --- |
| `hidden_criteria` | Terms carry a criterion the performer cannot fetch (2.1) | Terms author | Terms version rejected |
| `unclosed_condition` | PROMISE or ACCEPT against an open or incomplete condition or disqualifier (2.4) | Signer | Act not counted |
| `disqualified` | A valid `hit` attestation for a disqualifier (2.2) | Performer | Commitment fails |
| `unsupported_disqualification` | Disqualifier attestation signed by an unlisted key (2.2) | Signer | Attestation not counted |
| `seed_mismatch` | A revealed seed does not match its hash (2.3) | Revealing party | Attestation or REPORT not counted |
| `seed_withheld` | REPORT omits the performer seed when instances are declared (2.3) | Performer | REPORT incomplete |
| `instance_mismatch` | Sample does not follow the derivation or fails its Merkle proof (2.3) | Runner | Attestation not counted |
| `template_mismatch` | Template re-instantiation differs from the terms (2.5) | Terms author | Reference not counted |

### 2.7 Client obligations and threats

**Target:** the clients and projections section; the adversarial-cases table.

Before PROMISE, ACCEPT or SATISFY, clients MUST show the terms version as a card with four parts: **Done means** (each condition in plain language, with class and assurance label), **Checked by** (runners, judges with fingerprints, declarers), **Pay if** (price and settlement trigger), and **Disqualified if** (each disqualifier with its label). They MUST also show who drafted the version and a diff against the previous version. For instance pools they show the description, rule and sample size, never the contents.

Add to the adversarial-cases table:

| Threat | Vector | Mitigation |
| --- | --- | --- |
| Gaming a public rubric | Performer optimizes for rubric wording, not substance | Held-out instances; reference and counterexample material in the rubric; rubric-stuffing disqualifier; audits |
| Seed grinding | One party picks its seed to steer the sample | Two-party seed committed before delivery (2.3) |
| Self-serving CoS | Performer drafts conditions it can trivially meet | Requester must act on the version; card and diff shown; templates with records as defaults |
| Pretextual disqualification | Requester alleges a hit to withhold payment | Only listed keys attest; unsupported hits recorded; withholding without a valid hit is requester fault |
| Vague disqualifiers | Statements about quality used as a catch-all | Guidance and lint (section 3): disqualifiers name acts, quality goes in rubrics |
| Pool leakage | Pool items become known across commitments | Pool rule published, contents never; burn revealed items (OD-3) |
| Template laundering | Reputable template instantiated with weak slot values | Slot values hashed into `template_ref`; records projected per template and slot range |

## 3. Authoring aids (bindings, not Core)

Core validates terms; it never helps write them. The aids below belong in the reference binding and in harness bindings such as ABW. Their output is an ordinary terms version, judged by Core like any other.

**Requester drafter.** Input: an intent of one or two sentences, optional reference and counterexample material, price and deadline. Output: a terms version that matches a template where one fits, fills its slots, and leaves anything it cannot close as an `open` condition with a `suggested_class`. It never invents a declarer or runner key; those come from the requester's configuration.

**Performer drafter.** Input: a REQUEST with open or weak conditions, plus the performer's capabilities. Output: a COUNTER that closes every open condition and proposes disqualifiers, in effect a quote stating what the performer will be held to. It explains each change in one line, so the requester's client can show it beside the diff.

**Linter.** Runs on any draft, from either side, before it is sent. Errors block sending; warnings are shown on the card.

| Rule | Level | Check |
| --- | --- | --- |
| L1 | Error | A condition or disqualifier has no evidence producer |
| L2 | Error | A rubric or check definition is not retrievable |
| L3 | Error | An `open` condition remains in a version the user is about to promise or accept |
| L4 | Warning | A condition states a quality ("high quality", "clear") with no rubric; convert it to `judged` or name a `human` declarer |
| L5 | Warning | A disqualifier states a quality rather than an act; move it to a rubric |
| L6 | Warning | A `judged` condition has no paired disqualifier for its most obvious gaming route |
| L7 | Warning | A single judge, or all judges from one model family, at a price above a configured threshold |
| L8 | Warning | A `judged` condition with no instances, references or counterexamples, so it is open to overfitting |

L4 to L6 carry the main best-practice rules from section 4, so the guidance is enforced at the point of authoring rather than read once in the docs.

## 4. Templates and best practices

This section goes into the docs as `docs/acn/cos-templates.md`. The two examples become template files that validate against the 2.5 schema; fields abbreviated here (judges, runners and check references on disqualifiers) are filled in the files.

### 4.1 Best practices

1. **Tell the performer how it will be graded.** Rubrics, checks and disqualifiers are public. Unpredictability comes only from instance sampling.
2. **Disqualifiers name acts, not qualities.** "Modified existing tests" and "fabricated a citation" are verifiable acts. "Low effort" belongs in a rubric.
3. **Pair every rubric dimension with its most obvious gaming route**, and put the counter in a disqualifier or an instance pool. Citation count pairs with citation verification; test pass rate pairs with test-tampering checks.
4. **Give the judge examples.** A reference and a counterexample make a rubric concrete, and they are the easiest thing for a non-technical requester to supply.
5. **Use deterministic checks wherever they exist.** Judged conditions cover what checks cannot, and they carry the `opinion` label.
6. **Use ensembles across model families once the price matters**, with aggregation `all` for strict work.
7. **Put certainty in the conditions, not in the timing.** Commitments are atomic: what needs a stronger check (an audit) is a condition of the same commitment, checked before it settles; what can only be checked later belongs in a later commitment.
8. **Let the performer draft, then read the diff.** The performer knows what can be verified; the requester decides what it will accept.
9. **Prefer a template with a record over a fresh definition.** Override slots, not structure.

### 4.2 Example: code change

```yaml
template: code-change/v1
slots:
  intent:      { type: string }
  scope_paths: { type: string, guidance: "Paths the change may touch" }
  price:       { type: sats }
  deadline:    { type: duration, default: 48h }
intent: "{{intent}}"
conditions:
  - id: c1
    class: mechanical
    statement: "The existing test suite passes"
    check_ref: blossom://ci/run-suite@<hash>
    runners: [requester_ci]
  - id: c2
    class: mechanical
    statement: "New tests cover the stated behaviour, including failure paths"
    check_ref: blossom://ci/coverage-diff@<hash>
    runners: [requester_ci]
    instances:
      pool_description: "Integration tests exercising the changed behaviour"
      pool_rule: "Written by the requester before REQUEST; not in the repo"
      pool_size: 20
      sample_size: 5
  - id: c3
    class: judged
    statement: "Readable, scoped to the request, handles errors"
    rubric_ref: blossom://rubrics/code-change-v2
    judges: [<family A, pinned>, <family B, pinned>]
    scale: { min: 0, max: 10 }
    threshold: 7
    aggregation: all
    repeats: 3
    runners: [requester_binding]
disqualifiers:
  - id: d1
    class: mechanical
    statement: "Modifies or deletes existing tests or their assertions"
  - id: d2
    class: judged
    statement: "Hardcodes outputs or special-cases test inputs"
  - id: d3
    class: mechanical
    statement: "Touches files outside {{scope_paths}} without flagging it in REPORT"
  - id: d4
    class: mechanical
    statement: "Adds dependencies not listed in the terms"
price: "{{price}}"
deadline: "{{deadline}}"
```

The held-out integration tests in `c2` stop the performer from writing code that passes only the tests it can see.

### 4.3 Example: research brief

```yaml
template: research-brief/v1
slots:
  topic:    { type: string }
  reader:   { type: string, default: "non-specialist" }
  words:    { type: string, default: "800-1200" }
  price:    { type: sats }
intent: "A brief on {{topic}} for a {{reader}} reader"
conditions:
  - id: c1
    class: mechanical
    statement: "{{words}} words; sections Summary, Obligations, Timeline, Sources present"
    check_ref: blossom://checks/brief-structure@<hash>
    runners: [requester_binding]
  - id: c2
    class: mechanical
    statement: "Every factual claim carries a citation and every cited URL resolves"
    check_ref: blossom://checks/citations-resolve@<hash>
    runners: [requester_binding]
  - id: c3
    class: judged
    statement: "Accurate, clear for the stated reader, covers obligations and timeline"
    rubric_ref: blossom://rubrics/brief-v1   # includes one reference and one padded counterexample
    judges: [<family A, pinned>, <family B, pinned>]
    scale: { min: 0, max: 10 }
    threshold: 8
    aggregation: all
    repeats: 3
    runners: [requester_binding]
disqualifiers:
  - id: d1
    class: judged
    statement: "A sampled citation does not support the claim it is attached to, or does not exist"
    instances:
      pool_description: "The brief's own citations"
      pool_rule: "All citations in the delivered brief, in order of appearance"
      sample_size: 5
  - id: d2
    class: mechanical
    statement: "Reproduces source text verbatim beyond short quotation"
  - id: d3
    class: judged
    statement: "Repeats rubric terms without substance to raise the score"
price: "{{price}}"
```

The pool in `d1` is the delivered brief itself, so `pool_root` is fixed at REPORT rather than in the terms. This needs a small extension to 2.3, recorded as OD-4.

## 5. Demo scenario, conformance vectors, open decisions

### 5.1 Demo: both drafting paths in one commitment

A requester commissions a retry-with-backoff change to a relay client for 20,000 sat. The requester's drafter starts the terms; the performer's drafter finishes them; neither side sees criteria it cannot read.

1. The requester types the intent and adds "and don't make it slower." The drafter matches `code-change/v1`, fills its slots, and leaves `c4` as an `open` condition: "no performance regression", suggested class `mechanical`.
2. REQUEST carries terms v1, drafted by the requester. The card shows `c4` as open and notes that v1 cannot be promised yet (L3).
3. The performer's drafter sends COUNTER with v2. It closes `c4` as a benchmark check (p95 latency within 5% of baseline, retrievable `check_ref`) and adds `d5`, "disables or skips the benchmark", each with a one-line rationale. The COUNTER commits the performer's seed hash.
4. The requester's card shows v2 drafted by the performer, with the diff against v1. The requester accepts; the commitment binds.
5. The performer delivers and signs REPORT with the artifact hash and its seed.
6. The requester's binding runs the checks. Five of twenty held-out integration tests are sampled from both seeds and pass. Both judges score at least 7 on three runs each. All disqualifiers are clear.
7. Auto-SATISFY releases the 20,000 sat. The commitment is closed; nothing is held back.

A second run has the performer loosen one existing assertion. Every condition passes, `d1` is attested as hit, auto-satisfy is blocked, and `disqualified` is recorded. The point for the audience is that a breach of stated terms and a low score are different events with different consequences.

### 5.2 Conformance vectors

| Id | Phase | Setup | Expected |
| --- | --- | --- | --- |
| C-1 | A | Terms declare `commit_reveal` | Rejected; `hidden_criteria` |
| C-2 | A | `judged` condition without `rubric_ref` | Rejected; `hidden_criteria` |
| C-3 | A | PROMISE against a version with an `open` condition | Not counted; `unclosed_condition` |
| C-4 | A | PROMISE against a condition with no evidence producer | Not counted; `unclosed_condition` |
| C-5 | A | Performer COUNTER closes all open conditions; requester accepts | Binds; `drafted_by` is the performer |
| C-6 | A | Conditions pass; mechanical disqualifier hit before SATISFY | Auto-satisfy blocked; `disqualified`; breach path |
| C-7 | A | Disqualifier hit signed by an unlisted key; requester withholds | Hit not counted; `unsupported_disqualification`; requester fault |
| C-8 | A | A disqualifier carries an audit window (`window`) | Terms don't parse; `malformed` (commitments are atomic) |
| C-9 | A | Commitment uses none of the new fields | Behaves exactly as before CR-003 |
| C-10 | B | Valid seeds, derivation and Merkle proofs | Attestation counted |
| C-11 | B | Revealed requester seed does not match its hash | Not counted; `seed_mismatch` |
| C-12 | B | REPORT omits the performer seed | REPORT incomplete; `seed_withheld` |
| C-13 | B | Sampled indices do not follow the derivation | Not counted; `instance_mismatch` |
| C-14 | B | A sampled item fails its proof against `pool_root` | Not counted; `instance_mismatch` |
| C-15 | C | `template_ref` re-instantiates exactly | Reference counted |
| C-16 | C | `template_ref` re-instantiation differs | Reference not counted; `template_mismatch`; commitment valid |

Each vector also runs under every arrival-order permutation.

### 5.3 Open decisions

- [ ] **OD-1 Performer bond.** Whether a disqualification should cost more than the lost payment, through an optional performer bond forfeited on `disqualified`, and how that sits with CR-002 settlement.
- [ ] **OD-2 Counter auto-accept.** Whether a requester may pre-commit to accept counters that only close open conditions from a named template, without a separate act.
- [ ] **OD-3 Pool burning.** Whether revealed pool items are marked spent per `pool_root`, and how readers detect reuse across commitments.
- [ ] **OD-4 Deliverable-derived pools.** Allowing `pool_root` to be fixed at REPORT when the pool is the deliverable's own content, such as its citations.
- [ ] **OD-5 Disqualifier classes.** Whether `grounding` disqualifiers are allowed, and whether `interpretive` stays excluded.
- [ ] **OD-6 Judge rationales.** CR-001 OD-6, now sharper with public rubrics: publishing rationales to the performer is consistent with no hidden criteria, but makes gaming cheaper.
- [ ] **OD-7 Template governance.** Who may publish templates, how they are deprecated, and whether third parties may sign endorsements of a template's record.
