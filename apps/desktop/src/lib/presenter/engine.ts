// In-browser ACN world for presenter mode.
//
// Every act is a real BIP-340-signed ACN event. State, legality, anomalies, hints and the
// simulated cast's reactions all come from the Rust crates via WASM (acn-core fold, acn-view
// projection, acn-sim scenarios) — the same code the live simulator and desktop app run.
// This file only keeps the event list, timers and keys.
//
// Money: an in-browser simulated mint (acn-wallet's SimMint, via WASM) enforces the NUT-14 rules
// (preimage redeem, refund only after the locktime, single spend). The live app uses real Cashu
// ecash at a CDK test mint instead.

export const START_BALANCE = 100;

import { acn, randomHex32 } from "./wasm";
import type { Hint, HintAction } from "../types";

export type Actor = "performer" | "rogue" | "verifier" | "judge" | "arbiter" | "gatekeeper";

export interface AcnEvent {
  id: string;
  pubkey: string;
  created_at: number;
  kind: number;
  tags: string[][];
  content: string;
  sig: string;
}

interface Draft {
  kind: number;
  tags: string[][];
  content: string;
}

interface Move {
  actor: Actor;
  key: string;
  delay_ms: number;
  narration: string;
  draft: Draft;
  not_before: number;
}

const ACTORS: Actor[] = ["performer", "rogue", "verifier", "judge", "arbiter", "gatekeeper"];
// Fixed demo keys (presenter mode only), so npubs are stable between runs.
const secretFor = (i: number) => (i + 0x21).toString(16).padStart(2, "0").repeat(32);

export const ACTOR_NAMES: Record<Actor, string> = {
  performer: "Atlas",
  rogue: "Rogue",
  verifier: "Verity",
  judge: "Muse",
  arbiter: "Solon",
  gatekeeper: "Gate",
};

export class AcnWorld {
  readonly me: string;
  readonly cast: Record<Actor, string>;
  private readonly mySecret = secretFor(0x40);
  private readonly secrets: Record<Actor, string>;
  private events: AcnEvent[] = [];
  private extra: AcnEvent[] = [];
  private escrow: Record<string, string> = {};
  private preimages: Record<string, string> = {};
  private scheduled = new Set<string>();
  private timers: number[] = [];
  /** Simulated mint state (JSON owned by WASM). */
  private mint: any;
  /** Atlas's verdict per priced commitment (Core.md §8.2 step 3). */
  private payments: Record<string, any> = {};
  /** commitment -> token I locked. */
  private tokens: Record<string, string> = {};
  private redeemed = new Set<string>();
  /** Called when any wallet balance changes. */
  onWallet: () => void = () => {};
  /** Narration of the most recent simulated move, per commitment. */
  narration: Record<string, string> = {};
  offset = 0;
  pace = 1;

  constructor(private readonly onChange: (commitment: string) => void) {
    this.secrets = Object.fromEntries(ACTORS.map((a, i) => [a, secretFor(i)])) as Record<Actor, string>;
    this.cast = Object.fromEntries(ACTORS.map((a) => [a, acn<string>("pubkey", { secret: this.secrets[a] })])) as Record<Actor, string>;
    this.me = acn<string>("pubkey", { secret: this.mySecret });
    // Operator binding for Atlas (Rogue stays unbound).
    const op = secretFor(0x50);
    const t = this.realNow() - 3600;
    const bind = this.sign(op, acn<Draft>("bind", { agent: this.cast.performer, expires: t + 30 * 86_400 }), t);
    const ack = this.sign(this.secrets.performer, acn<Draft>("bind_ack", { binding: bind.id }), t + 1);
    this.extra = [bind, ack];
    this.resetMint();
  }

  private resetMint() {
    this.mint = acn("mint_new");
    this.topUp(START_BALANCE);
  }

  topUp(amount: number) {
    this.mint = acn<any>("mint_issue", { mint: this.mint, owner: this.me, amount, now: this.now() }).mint;
    this.onWallet();
  }

  wallet(owner: string = this.me): any {
    return acn("mint_view", { mint: this.mint, owner });
  }

  /** Requester: reclaim an unsatisfied commitment's lock. The presentation clock counts here. */
  refund(commitment: string) {
    const advice = acn<any>("advise", { events: this.events, commitment, me: this.me, now: this.now() });
    if (advice?.type !== "refund") throw "nothing to refund: the commitment is open or was satisfied";
    if (!advice.ready) throw `refundable after the locktime (${new Date(advice.locktime * 1000).toLocaleString()}); fast-forward the clock to get there`;
    this.mint = acn<any>("mint_refund", { mint: this.mint, token: advice.token, by: this.me, now: this.now() }).mint;
    this.onWallet();
  }

  realNow() {
    return Math.floor(Date.now() / 1000);
  }

  now() {
    return this.realNow() + this.offset;
  }

  scenarios(): any[] {
    return acn("scenarios");
  }

  template(scenario: string): any {
    return acn("template", { scenario, cast: this.cast });
  }

  private sign(secret: string, draft: Draft, at: number): AcnEvent {
    return acn<AcnEvent>("sign", { draft, secret, created_at: at, aux: randomHex32() });
  }

  private add(e: AcnEvent) {
    if (this.events.some((x) => x.id === e.id)) return;
    this.events.push(e);
    this.onChange(commitmentOf(e));
    this.react();
  }

  /** Projections of every commitment (from `me`'s point of view). */
  project(): any[] {
    return acn("project", { events: [...this.events, ...this.extra], me: this.me, now: this.now() });
  }

  /** Requester: start a scenario with `performer`. Returns the commitment id. */
  request(performer: string, scenario: string, brief: string): string {
    const template = this.template(scenario);
    let s: string | undefined;
    let hashlock: string | null = null;
    if (template.hashlock) {
      s = randomHex32();
      hashlock = acn<string>("sha256", { bytes: s });
    }
    const arbiter = template.terms?.arbiter;
    const draft = acn<Draft>("request", {
      template,
      tool: scenario,
      now: this.now(),
      performer,
      hashlock,
      input: { brief },
      // Presenter mode plays every role locally, so the escrow is a placeholder; the live app
      // NIP-44-encrypts s to the arbiter (Core.md §8.4).
      escrow: arbiter ? "presenter-escrow" : undefined,
    });
    // Lock the price to the performer before signing (Core.md §8.2 steps 1-2). Presenter mode
    // carries the token in the clear; the live app NIP-44-encrypts it to the performer.
    const content = JSON.parse(draft.content);
    const pay = content.terms?.payment;
    const price = content.terms?.price?.amount ?? 0;
    let token: string | undefined;
    if (s && pay && price > 0) {
      const r = acn<any>("mint_lock", {
        mint: this.mint, owner: this.me, amount: price, hashlock, receiver: performer,
        locktime: pay.locktime, now: this.now(),
      });
      this.mint = r.mint;
      token = r.token;
      content.token = token;
      draft.content = JSON.stringify(content);
    }
    const e = this.sign(this.mySecret, draft, this.now());
    if (s) {
      this.preimages[e.id] = s;
      if (arbiter) this.escrow[e.id] = s;
    }
    if (token) {
      this.tokens[e.id] = token;
      this.mint = acn<any>("mint_tag", { mint: this.mint, token, commitment: e.id }).mint;
      this.onWallet();
    }
    this.add(e);
    return e.id;
  }

  /** Requester act through the act schemas (acn-sim's requester_draft). */
  act(commitment: string, action: HintAction) {
    const { draft, not_before } = acn<{ draft: Draft; not_before: number }>("requester_draft", {
      events: this.events,
      commitment,
      action,
      preimage: this.preimages[commitment] ?? null,
    });
    this.add(this.sign(this.mySecret, draft, Math.max(this.now(), not_before)));
  }

  hint(commitment: string): Hint | null {
    return acn("hint", { events: this.events, commitment, now: this.now() });
  }

  setOffset(offset: number) {
    this.offset = Math.max(0, offset);
    for (const c of new Set(this.events.map(commitmentOf))) this.onChange(c);
  }

  /** Atlas checks each new priced REQUEST's token with the (simulated) mint. */
  private checkPayments() {
    for (const e of this.events) {
      if (e.kind !== 7700 || e.id in this.payments) continue;
      const terms = JSON.parse(e.content).terms;
      if (!terms?.payment || !(terms.price?.amount > 0)) continue;
      this.payments[e.id] = acn("payment_check", { mint: this.mint, events: this.events, commitment: e.id, me: this.cast.performer });
    }
  }

  /** Atlas redeems once the preimage is public (SATISFY or a performer-side RULING). */
  private redeem() {
    for (const c of new Set(this.events.map(commitmentOf))) {
      if (this.redeemed.has(c)) continue;
      const advice = acn<any>("advise", { events: this.events, commitment: c, me: this.cast.performer, now: this.now() });
      if (advice?.type !== "redeem") continue;
      this.redeemed.add(c);
      const t = window.setTimeout(() => {
        try {
          const r = acn<any>("mint_redeem", { mint: this.mint, token: advice.token, by: this.cast.performer, preimage: advice.preimage, now: this.now() });
          this.mint = r.mint;
          this.narration[c] = `Atlas redeems ${r.amount} sat at the mint with the revealed preimage.`;
        } catch (err) {
          this.narration[c] = `Atlas could not redeem: ${err}`;
        }
        this.onWallet();
        this.onChange(c);
      }, 900 * this.pace);
      this.timers.push(t);
    }
  }

  /** Run the simulated cast (acn-sim::react) and schedule their moves. */
  private react() {
    this.checkPayments();
    this.redeem();
    const moves = acn<Move[]>("react", { cast: this.cast, events: this.events, escrow: this.escrow, payments: this.payments });
    for (const m of moves) {
      if (this.scheduled.has(m.key)) continue;
      this.scheduled.add(m.key);
      const t = window.setTimeout(() => {
        const e = this.sign(this.secrets[m.actor], m.draft, Math.max(this.now(), m.not_before));
        // Narrations already name the actor ("Atlas counters…").
        this.narration[commitmentOf(e)] = m.narration;
        this.add(e);
      }, m.delay_ms * this.pace);
      this.timers.push(t);
    }
  }

  reset() {
    this.timers.forEach((t) => clearTimeout(t));
    this.timers = [];
    this.events = [];
    this.escrow = {};
    this.preimages = {};
    this.scheduled.clear();
    this.narration = {};
    this.offset = 0;
    this.payments = {};
    this.tokens = {};
    this.redeemed.clear();
    this.resetMint();
  }
}

export function commitmentOf(e: AcnEvent): string {
  if (e.kind === 7700) return e.id;
  return e.tags.find((t) => t[0] === "c")?.[1] ?? "";
}
