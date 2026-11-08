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

## Screencast

[`docs/demo/acn-demo.mp4`](demo/acn-demo.mp4) (2 min 48 s, 1440×900, captioned, no audio) walks
through the consent dialog and seven scenarios: negotiation, evidence classes, paid call,
dispute, governance, lapse and rogue agent. It was recorded before the wallet existed, so it
doesn't show balances or refunds yet.

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

Paste Atlas's npub into "Start a chat", accept, and open **🎬 Director**. `--pace 0.5` doubles
the speed. The cast keeps its keys in `.nmesh/sim/`, so npubs stay the same across runs.

**Wallet.** The cast hosts a Cashu test mint (CDK, fake Lightning) on `http://127.0.0.1:3338`,
and the desktop app's wallet points there by default (`NMESH_MINT`; set it to `none` to disable).
Click the **₿** chip in the sidebar, then **Top up 100 test sats** before running *Paid call*
or *Dispute*. Without enough funds the request is refused before anything is signed. To use a
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
| Review | `Reported`; a report waiting for a governance sign-off stays in progress |
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
| **Paid call** (T1, 21 sat) | Satisfy & pay | 21 sat leave your balance when you request (HTLC lock); Atlas checks the token with the mint before working (§8.2); SATISFY reveals the preimage and Atlas redeems: *your token: redeemed by Atlas*. **Closing is paying** (INV-6) |
| **Paid call, rejected** (presenter) | Reject… on the card, then fast-forward ≈ 26 h, then ↩ Refund | No preimage, no payment: *refundable*; the refund button stays disabled until the locktime; afterwards the 21 sat come back |
| **Negotiation** (N) | Accept counter-offer | Counter diff (deadline, revisions) bound to a new terms hash (INV-4); PROGRESS is an annotation and never moves the head |
| **Rework loop** | Reject "sections", then Accept | REJECT must name a condition; Reworking; report **v2**; rework limit |
| **Verified work** | Reject "tests", then Accept | Verity's mechanical attestation is **counted**; Muse's opinions are **not counted** (INV-8 / V-11); your rejection is flagged `contradicted_rejection` (V-16) but still applies (INV-5) |
| **Dispute & ruling** (50 sat) | Reject | The arbiter was named in the terms; Atlas disputes; Solon rules with the **escrowed preimage**, and Atlas redeems the 50 sat without you (§8.4) |
| **Governance gate** | Accept | Atlas's REPORT shows as *pending* until **Gate** endorses it (V-14) |
| **Silent performer** | Fast-forward, then Withdraw | **Lapsed** is derived by `status_at`, never announced |
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
