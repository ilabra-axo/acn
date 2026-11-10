# Conditions of satisfaction: best practices and templates

How to write the conditions and disqualifiers of an ACN commitment so that "done" is checkable,
hard to game, and fair to both sides. The rules are normative in [Core.md](../Core.md) §7
(terms, held-out instances, terms templates) and §10 (V-24…V-28). This page is guidance (from
CR-003 §4), with two templates that validate against the reference implementation.

Commitments are atomic (Core §1): everything a commitment is held to is checked before it
settles, and payment moves once. There is no holdback and no audit window.

## Best practices

1. **Tell the performer how it will be graded.** Rubrics, checks and disqualifiers are public:
   a rubric needs a retrievable `rubric_ref`, a hashed check a `check_ref` (INV-14; otherwise
   `hidden_criteria`). Unpredictability comes only from held-out instance sampling.
2. **Disqualifiers name acts, not qualities.** "Modified existing tests" and "fabricated a
   citation" are acts someone can attest. "Low effort" belongs in a rubric. A disqualifier hit
   is a breach: the commitment fails with no rework, however the conditions scored.
3. **Pair every rubric dimension with its most obvious gaming route**, and put the counter in a
   disqualifier or an instance pool. Citation count pairs with citation verification; test pass
   rate pairs with test-tampering checks.
4. **Give the judge examples.** A reference and a counterexample make a rubric concrete, and they
   are the easiest thing for a non-technical requester to supply. Both rubrics for quality below
   carry one of each.
5. **Use deterministic checks wherever they exist.** `judged` conditions cover what checks can't,
   and they carry the `opinion` assurance label, the weakest (Core §7).
6. **Use ensembles across model families once the price matters**, with aggregation `all` for
   strict work. For a judged *disqualifier*, `all` means every judge must find the act, which
   protects the performer from a pretextual hit.
7. **Put certainty in the conditions, not in the timing.** What needs a stronger check (an audit)
   is a condition of the same commitment, checked before it settles. What can only be checked
   later belongs in a later commitment.
8. **Let the performer draft, then read the diff.** The performer knows what can be verified; the
   requester decides what it will accept. A request may leave conditions `open`; the performer's
   `COUNTER` closes them, and nobody can promise until every condition is closed (V-26). The
   terms card shows who drafted each version and the diff.
9. **Prefer a template with a record over a fresh definition.** Override slots, not structure.
   The card shows the template, the slot values you filled in, and the record of commitments
   that used it.

Practical consequences of the rules:

- **Name the evidence producer.** A `mechanical` condition needs a listed verifier of that class
  in the terms' `verifiers`; a `judged` item names its `runners`; a disqualifier names its
  `runners`. The performer is never a runner (`self_judged`).
- **Held-out instances need `rework_limit` 0.** The first attestation reveals the sample, so a
  rework would be graded on known cases. A retry is a new commitment with new seeds.
- **Pin the judges.** Every judge fingerprint (`model_id`, `model_version`,
  `prompt_template_hash`, `harness_hash`, `sampling`) is in the terms; an attestation from a
  different judge or harness is `judge_mismatch`.

## Terms templates

A template (`K_TERMS_TEMPLATE`, Core §7) is a signed fragment of terms: conditions and
disqualifiers with typed slots. Terms drafted from it carry `template_ref {event, hash, slots}`,
and any reader can re-instantiate the template from those slot values and check that it
reproduces the terms exactly (V-28). A claim that doesn't is `template_mismatch` and gets no
record.

- Slot types: `string`, `number`, `pubkey`, `sha256`, `duration`, `sats`; each slot may have a
  `default` and `guidance` for whoever fills it in.
- `{{slot}}` placeholders: a string that is exactly one placeholder becomes the typed value
  (`"runners": ["{{checker}}"]`); otherwise the value is substituted as text.
- Templates cover conditions and disqualifiers only. Price, deadline, review window,
  `verifiers`, settlement and the deliverable are set per request.

- Optional `notes`: one line per item id saying why it is there, with placeholders. Not terms
  content; a drafter shows them beside the diff when it uses the template.

Reference: `acn_core::template` (`TermsTemplate::parse`, `instantiate`, `notes_for`, `check`),
`acn_core::draft::Draft::template` to publish.

## Authoring aids

`acn-author` (Core.md §11, CR-003 §3) puts these practices where terms are written:

- **The linter** (L1–L8) flags terms with no evidence producer, unretrievable criteria, `open`
  conditions about to bind (errors: the reference app won't send them), and qualities without
  rubrics, disqualifiers that aren't acts, judged conditions with no disqualifier, one judge or
  one model family above 1,000 sat, and judged conditions open to overfitting (warnings, on the
  terms card).
- **The requester drafter** turns a goal and its requirements into terms, using a template where
  one fits and leaving the rest `open` with a suggested class.
- **The performer drafter** closes `open` conditions from the performer's templates and
  explains each change with the template's notes. Neither drafter invents a key.

## Templates

Both files are in [`templates/`](templates/), with their rubrics in
[`templates/rubrics/`](templates/rubrics/) (`rubric_hash` is the sha256 of the file;
`rubric_ref` is `blossom:<hash>`, so upload the file to your Blossom server). The judges are
pinned to the two local Ollama models of
[`crates/acn-judge/judges.example.toml`](../../crates/acn-judge/judges.example.toml) under the
`acn-judge/2` harness: fine for trying the flow, too small to be trusted judges. For real work,
re-pin to a hosted panel (`cargo run -p acn-judge --example try_panel -- <judges.toml>` prints
the panel's fingerprints before it tries the judges). `crates/acn-judge/tests/templates.rs` checks that every file parses, instantiates
into terms that parse and close, re-instantiates exactly, and pins the harness's current
fingerprints.

### `code-change/v1`

[`templates/code-change.v1.json`](templates/code-change.v1.json). A change to an existing code
base.

| Item | Class | Statement | Checked by |
| --- | --- | --- | --- |
| c1 | mechanical | The existing test suite passes | the mechanical verifier, running `suite_check` |
| c2 | mechanical | New tests cover the stated behaviour, including failure paths, and the sampled held-out integration tests pass | the mechanical verifier, running `coverage_check` on a sample drawn from both seeds |
| c3 | judged | Readable, scoped to the request, handles errors | two judges, θ 7 of 10, `all`, 3 repeats; run by `checker` |
| d1 | mechanical | Modifies or deletes existing tests or their assertions | `checker` |
| d2 | judged | Hardcodes outputs or special-cases test inputs | two judges, θ 7, `all`; run by `checker` |
| d3 | mechanical | Touches files outside `{{scope_paths}}` without flagging it in REPORT | `checker` |
| d4 | mechanical | Adds dependencies the request does not list | `checker` |

Slots: `checker` (pubkey: runs the checks and judges and attests the disqualifiers; list it as
the mechanical verifier too), `scope_paths`, `suite_check`, `coverage_check`, `pool_root`,
`pool_size` (default 20), `sample_size` (default 5), `requester_seed_hash`. The held-out tests
in c2 stop the performer from writing code that passes only the tests it can see; d1 and d2 are
the obvious ways to game c1 and c2. Terms from this template need `rework_limit` 0.

### `research-brief/v1`

[`templates/research-brief.v1.json`](templates/research-brief.v1.json). A short brief for a
named kind of reader; the topic goes in the deliverable's description.

| Item | Class | Statement | Checked by |
| --- | --- | --- | --- |
| c1 | mechanical | `{{words}}` words; sections Summary, Obligations, Timeline, Sources present | the mechanical verifier, running `structure_check` |
| c2 | mechanical | Every factual claim carries a citation and every cited URL resolves | the mechanical verifier, running `citations_check` |
| c3 | judged | Accurate, clear for a `{{reader}}` reader, covers obligations and timeline | two judges, θ 8 of 10, `all`, 3 repeats; run by `checker` |
| d1 | judged | A citation does not support the claim it is attached to, or the cited source does not exist | two judges, θ 7, `all`; run by `checker` |
| d2 | mechanical | Reproduces source text verbatim beyond short quotation | `checker`, running `similarity_check` |
| d3 | judged | Repeats rubric terms without substance to raise the score | two judges, θ 7, `all`; run by `checker` |

Slots: `checker`, `reader` (default "non-specialist"), `words` (default "800-1200"),
`structure_check`, `citations_check`, `similarity_check`. c2 makes citations cheap to count, and
d1 is its gaming route: a citation that resolves but doesn't support the claim. d3 counters
rubric stuffing on c3.

CR-003's version of d1 sampled the brief's own citations as held-out instances. That pool would
only exist at `REPORT`, while Core §7 fixes `pool_root` in the terms (open decision D-35), so
here the judges check every citation instead.

## Using a template

1. Fetch the template event and check its record on the cards of earlier commitments.
2. Fill the slots, instantiate (`TermsTemplate::instantiate`), and put the result into your
   terms' `conditions` and `disqualifiers`, with `template_ref {event, hash, slots}`.
3. Add the per-request fields: deliverable, price, deadline, review window, `verifiers` (the
   `checker` key with class `mechanical`), and `rework_limit` 0 if the template has instances.
4. For held-out instances, generate a 32-byte seed, put its sha256 in the `requester_seed_hash`
   slot, and give the seed privately to the runner (Core §7).

Edit the instantiated conditions and the reference stops counting (`template_mismatch`). That is
allowed, and the terms still bind, but the card will say so. To change the structure, publish a
new template.
