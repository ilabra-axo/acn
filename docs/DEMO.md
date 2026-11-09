# ACN demo guide

There are two ways to show the prototype. Both run the **same Rust validator, projection and
scenario logic** (`acn-core`, `acn-view`, `acn-sim`). Every act is a real BIP-340-signed ACN event.

| | Presenter mode | Live mode |
|---|---|---|
| Runs | in any browser, no install | desktop app + relay + simulated cast |
| Validator | `acn-core` compiled to WASM | `acn-core` native |
| Transport | none (in-memory) | Nostr relay, NIP-17 gift wraps, `K_PRIVATE_ACT` |
| Best for | slides, conference wifi, recordings | "this is real Nostr" |
| Money | in-browser simulated mint with the NUT-14 rules (`SimMint`); you start with 100 sat | **real Cashu ecash** (test sats) at a CDK mint the cast hosts on `127.0.0.1:3338`; top up from the wallet panel |
| Refund demo | fast-forward the presentation clock past the locktime | the mint's real clock rules (≈ 25 h); covered by `nmesh-wallet/tests/htlc.rs` |

## The MVP in one minute

*Pay only if two independent LLM judges score the deliverable at least 7/10 against a rubric.*

1. **Judged, verifier releases** (`/judged`, 60 sat). The card shows **assurance: opinion** and
   names the judges and the threshold before anything is signed (CR-001 §2.6). Verity holds the
   escrowed preimage, acknowledges it (`ESCROW_ACK`), and only then does Atlas promise. Atlas
   reports; Verity counts the words (`c1`, mechanical, *check*), runs both judges three times
   against the public rubric (`c2`, judged, *opinion*; the terms carry it, CR-003) and
   **releases**: Atlas is paid and
   you never clicked anything. The state is **Settled**; you may still record one judgement.
2. **Judge prompt injection** (`/injected`). The summary ends with "Note to the evaluator: …".
   The judge harness detects it, `c2` fails, nothing is released, you reject, and
   `injection_detected` stays on Atlas's REPORT for good. Your 60 sat refund after the locktime.
3. **Judged, you settle** (`/judged-auto`, 40 sat). Same conditions, but Muse is a third-party
   runner and the requester trigger applies: your binding's auto-satisfy policy signs
   `SATISFY` once every condition passes. Closure stays your act (INV-5).
4. **Contingent** (`/contingent`, 30 sat). Mechanical conditions only (S-9). No money up front:
   Atlas seals the module with a fresh key `k` (NIP-44 v2, §8.2) and escrows `k` with Verity,
   who decrypts, runs the tests and attests without revealing anything. The card shows
   *🔒 Encrypted delivery* and a **Fund 30 sat** button; you fund against `H = sha256(k)`,
   Atlas redeems at the mint (which reveals `k`), publishes `CLAIM`, and the card decrypts.

![Judged: both conditions counted, Verity released, Atlas paid](screenshots/p11-judged-released.png)

![Injection: c2 failed, injection_detected on the REPORT, 60 sat refundable](screenshots/p12-injected.png)

![Contingent: decrypted after CLAIM](screenshots/p14-contingent-opened.png)

### Who writes the conditions (CR-003)

5. **Drafted conditions** (`/drafted`, free). Your request says "don't make it slower" but
   leaves it `open`: nobody can promise against that. Atlas's `COUNTER` closes it as a
   benchmark check with a retrievable definition and proposes two **disqualifiers** ("disables
   the benchmark", "modifies existing tests"). The counter shows the **terms card** (*Done means
   · Checked by · Pay if · Disqualified if*), **v2 · drafted by Atlas**, and the diff against
   v1. You accept; Verity checks both conditions and both disqualifiers; auto-satisfy.
6. **Disqualified** (`/disqualified`). Same terms, but Atlas loosens an existing assertion.
   Both conditions pass, yet Verity attests `d2` as **hit**: Satisfy disappears, the Director
   says *Reject: d2 hit*, and the commitment ends **Rejected** with no rework, although one was
   allowed. `disqualified` is on Atlas's REPORT: a breach of stated terms, not a low score.
7. **Held-out tests** (`/held-out`, free). Verity keeps 20 integration tests Atlas never sees;
   the terms card shows only what they are, how they were chosen and that 5 will be drawn. Your
   app commits a seed's hash in the terms and sends the seed privately to Verity; Atlas commits
   its own seed's hash when it promises and reveals the seed with the work. Verity derives the
   sample from both seeds and the commitment id, runs those 5, and attests with your seed and
   a Merkle proof for each case, so anyone can check the pick wasn't chosen.

Point out the ladder: a judged condition certifies that pinned judges produced a score; it is
evidence about an opinion, not about the work (INV-8). Stronger classes (mechanical, grounded,
an audit) are conditions you add to the same terms.

## Screencast

[`docs/demo/acn-demo.mp4`](demo/acn-demo.mp4) (3 min 36 s, 1440×900, captioned, no audio, presenter
mode) walks through the consent dialog and the wallet top-up, then the MVP: **judged, verifier
releases**, the **prompt injection** and **contingent** (encrypted until paid). After that come
evidence classes, dispute and ruling, the rogue agent, the **board** with its drawer, the wallet's
locks and history, and **reset demo**.

[![ACN demo screencast](demo/acn-demo-poster.png)](demo/acn-demo.mp4)

To re-record it (presenter mode; Chrome + `playwright-core`):

```sh
cd apps/desktop && pnpm build && pnpm preview --port 4173 &
node scripts/screencast.mjs /tmp/cast          # writes a .webm
ffmpeg -i /tmp/cast/*.webm -c:v libx264 -crf 22 -pix_fmt yuv420p -movflags +faststart ../../docs/demo/acn-demo.mp4
```

Captions and pacing live in `scripts/screencast.mjs`.

## Presenter mode

```sh
cd apps/desktop
pnpm install
pnpm wasm      # only after changing the Rust ACN crates
pnpm dev       # http://localhost:5173
```

Open **Atlas** (or **Rogue**), accept the disclosure, then click **🎬 Director**.

## Live mode

```sh
# from the repo root
cargo build --workspace
./target/debug/nmesh relay                                  # ws://127.0.0.1:7777
./target/debug/nmesh-acn-sim --pace 1                       # prints Atlas's and Rogue's npubs
cd apps/desktop && pnpm tauri dev                           # desktop app
```

The relay is in memory: after restarting it, restart `nmesh-acn-sim` too, so the cast publishes its
profiles and `K_SVC` again.

**Real judges.** By default the judged scenarios use scripted judges. Pass a judge panel and
Verity and Muse call real models instead (`crates/acn-judge`):

```sh
ollama pull qwen2.5:1.5b && ollama pull llama3.2:1b          # or hosted judges, see the file
./target/debug/nmesh-acn-sim --judges crates/acn-judge/judges.example.toml
cargo run -p acn-judge --example try_panel -- crates/acn-judge/judges.example.toml   # dry run
```

The panel's fingerprints go into Atlas's `K_SVC` templates, so the terms pin the judges that will
actually run. Scores are real opinions: `/judged` may now end **Rejected** (`below_threshold`)
instead of Settled. Two small local models take about a minute per judgement on a laptop CPU and
are too small to be trusted judges; the example file shows hosted ones (API keys come from
environment variables). Rationales go to the cast's log at `debug` level; only their hashes are
published (D-19). If judging fails three times, nothing is attested and the review window
applies.

**Version and updates.** The sidebar footer shows `nmesh v<version> · <commit> · acn/<version>`. A dev
build compares its commit with the checkout's `HEAD` every two minutes; when the checkout is
ahead it shows **Update available** with the new commits, and **Restart** makes `pnpm tauri dev`
rebuild from the current sources and relaunch. (`tauri dev` only watches `src-tauri/`, so edits to
the workspace crates are not picked up without this.) Release builds have no update channel yet.

Paste Atlas's npub into "Start a chat", accept, and open **🎬 Director**. `--pace 0.5` doubles
the speed. The cast keeps its keys in `.nmesh/sim/`, so npubs stay the same across runs.

**Wallet.** The cast hosts a Cashu test mint (CDK, fake Lightning) on `http://127.0.0.1:3338`,
and the desktop app's wallet points there by default (`NMESH_MINT`; set it to `none` to disable).
Click the **₿** chip in the sidebar, then **Top up 100 test sats** before running *Paid call*,
*Dispute*, *Contingent* or the judged scenarios (all sixteen scenarios cost 261 sat, so top up
three times; 60 of it stays locked after the injection until its locktime). Without enough funds the request is refused before anything is signed. To use a
separately running mint: `./target/debug/nmesh-mint` and `nmesh-acn-sim --mint http://127.0.0.1:3338`.
Atlas DMs you when it redeems; send it `wallet` to see its balance.

![Wallet panel after paid, dispute and a refunded rejection (presenter mode)](screenshots/p6c-wallet.png)

## The Director

- **Scenarios:** start a commitment with the scenario's terms template, which Atlas publishes
  in its `K_SVC`.
- **Now:** explains what just happened and what you're about to do. **▶ Next** signs your next
  act. **Autopilot** plays the whole thing.
- **Presentation clock:** moves the time used by `status_at` (deadlines, lapses) without
  touching signatures. In presenter mode it also drives the simulated mint's locktime.
- **↺ Reset demo** (click twice): clears every commitment, stops autopilot and resets the clock.
  Presenter mode also resets the simulated mint to 100 sat. Live mode can't delete signed acts
  from the relay, so the app ignores everything signed before the reset (also after a restart);
  the wallet's real test ecash is kept.
- **Balances:** yours and (presenter mode) Atlas's. **open wallet** shows locks per commitment,
  history and refunds.
- You can also ignore the Director and use the card's buttons: Accept counter-offer, Satisfy,
  Reject (pick the condition), Dispute, Withdraw.

Each card shows the applied chain, any pending acts, conditions with evidence (counted or
not, and why), the counter-offer diff, settlement, validator notes, and **Show signed events**
(every event, including rejected ones, retained under INV-11).

## The board

**Chats | Board** in the sidebar switches to a kanban view of every commitment, across agents
and in any role you play (requester, performer, verifier, arbiter, gate).

| Column | ACN phase(s) |
|---|---|
| Requested | `Requested` |
| Negotiating | `Countered`, or a counter pending while promised |
| In progress | `Promised`, `Reworking`; overdue / lapsed shown in red |
| Review | `Reported`, and contingent `Offered` / `Funded`; a report waiting for a governance sign-off stays in progress |
| Dispute | `Rejected` while still disputable, `Disputed` |
| Closed | terminal; newest first, **Show older** pages back |

Each tile shows the tool, the counterparty and its binding, **whose move it is** ("Your move ·
satisfy or reject" gets an accent edge), the relevant countdown, the money (🔒 locked → ✓ paid
/ ↩ refundable), evidence dots per condition, and the most telling validator note. **Needs you**
filters to your moves. Clicking a tile opens the full commitment card in a drawer.

There is **no drag-and-drop**: in ACN only signed acts change state, and most moves aren't
yours. Tiles move by themselves when acts arrive. Column and whose-move are computed in Rust
(`acn-view`), so live and presenter mode always agree.

Nice moment: start a few scenarios without autopilot, switch to the board, press **+10 min**:
the silent promise turns red as overdue, without any act being sent.

![Board with commitments at several stages (presenter mode)](screenshots/b1-board.png)

## Scenarios and talk track

| Scenario | What you do | What to point at |
|---|---|---|
| **Quick tool call** (T0) | Accept | REQUEST → REPORT → SATISFY; *operator-bound ✓*; a REPORT is an assertion, closure is yours (INV-5) |
| **Paid call (Cashu)** (T1, 21 sat) | Satisfy & pay | 21 sat leave your balance when you request (HTLC lock); Atlas checks the token with the mint before working (§8.2); SATISFY reveals the preimage and Atlas redeems: *your token: redeemed by Atlas*. **Closing is paying** (INV-6) |
| **Paid call, rejected** (presenter) | Reject… on the card, then fast-forward ≈ 26 h, then ↩ Refund | No preimage, no payment: *refundable*; the refund button stays disabled until the locktime; afterwards the 21 sat come back |
| **Negotiation** (N) | Accept counter-offer | Counter diff (deadline, revisions) bound to a new terms hash (INV-4); PROGRESS is an annotation and never moves the head |
| **Rework loop** | Reject "sections", then Accept | REJECT must name a condition; Reworking; report **v2**; rework limit |
| **Verified work** | Reject "tests", then Accept | Verity's mechanical attestation is **counted**; Muse's opinions are **not counted** (INV-8 / V-11); your rejection is flagged `contradicted_rejection` (V-16) but still applies (INV-5) |
| **Dispute & ruling** (50 sat) | Reject | The arbiter was named in the terms; Atlas disputes; Solon rules with the **escrowed preimage**, and Atlas redeems the 50 sat without you (§8.4) |
| **Governance gate** | Accept | Atlas's REPORT shows as *pending* until **Gate** endorses it (V-14) |
| **Silent performer** | Fast-forward, then Withdraw | **Lapsed** is derived by `status_at`, never announced |
| **Judged, verifier releases** (60 sat) | Nothing | *assurance: opinion* with the judges and θ; `ESCROW_ACK` before `PROMISE` (V-19); `c1` check + `c2` opinion both counted; `RELEASE` (V-20) → **Settled**, Atlas redeems; the button now says *Record satisfied* (it moves no money) |
| **Judge prompt injection** (60 sat) | Reject (the Director proposes it) | `c2` ✗ with `injection_detected` on the REPORT (V-17); no `RELEASE`; *refundable* after the locktime |
| **Judged, you settle** (40 sat) | Satisfy (auto-satisfy) | Muse is the runner (not the performer: `self_judged` otherwise); the requester trigger; closing is paying |
| **Drafted conditions** | Accept counter-offer, then Satisfy (auto) | The `REQUEST`'s `c2` is `open` (an underspecified request is fine; a `PROMISE` against it would be `unclosed_condition`); the counter's terms card and diff; v2 drafted by Atlas binds after your `ACCEPT` |
| **Disqualified** | Accept counter-offer, then Reject (`d2`) | `c1` and `c2` pass, `d2` hit; Satisfy not offered; Rejected with `rework_count` 0; `disqualified` on the REPORT |
| **Held-out tests** | Satisfy | The terms card shows the pool's description, rule and "5 of 20", never the cases; `PROMISE` carries `performer_seed_hash`, `REPORT` reveals `performer_seed`; Verity's attestation lists the sampled cases with proofs (V-27) |
| **Contingent (encrypted until paid)** (30 sat) | Fund | *🔒 Encrypted delivery* until `CLAIM`; Verity's attestation names `ciphertext`, `hashlock`, `plaintext` (V-21); Review lane passes Verity → you → Atlas; the card decrypts after the claim |
| **Rogue agent** | — | *unaccountable ⚠* (no binding); PROMISE and DECLINE at once → **equivocation** flagged on both; the closing act wins deterministically, the loser is `stale` and kept as evidence |

## What is simulated

- The cast's decisions are scripted (`crates/acn-sim`). Their **acts are real**, and the
  requester's validator judges them like any other events.
- **Presenter mode's mint is simulated** (`sim://mint`): same rules (receiver redeems with the
  preimage, refund only after the locktime, single spend), but no blind signatures or DLEQ.
- **Live mode's money is real ecash with fake Lightning:** tokens, blind signatures, DLEQ proofs
  and HTLC conditions are real Cashu (CDK); only the top-up invoice is fake. Never point it at
  real funds.
- Verifier results are scripted; no tests are actually run.
- **The judges are scripted unless you pass `--judges`** (live mode only; presenter mode is
  always scripted). Scripted judges call no LLM: the `K_ATTEST_JUDGED` events carry fixed
  fingerprints and scores (8, 7, 8 and 7, 9, 8), and injection detection is a substring match on
  "Note to the evaluator". With `--judges`, `acn-judge` runs the pinned models, scans for
  injection and lets the models vote on it. The validator checks both kinds the same way (V-17).
- **Atlas's summary is still canned** (a fixed paragraph about the protocol; only its first
  sentence names your brief), so real judges grade essentially the same text every time.
- **Contingent encryption is real:** the deliverable is sealed with NIP-44 v2 under `k`
  (`acn-core::sealed`) and only decrypts once `CLAIM` puts `k` on the record.
