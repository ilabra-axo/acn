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
  /** The host escrows `secret` with `to` (contingent key k → the settlement verifier). */
  escrow?: { to: Actor; secret: string };
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
  /** Requester seeds the runner holds (held-out instances); every role is local here. */
  private seeds: Record<string, string> = {};
  /** Event ids of the cast's published terms templates, by name (acn-sim `Cast.templates`). */
  private templateIds: Record<string, string> = {};
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
  /** Contingent (CR-002): Atlas's delivery key k per commitment. */
  private keys: Record<string, string> = {};
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
    // The cast's public terms templates (CR-003 §2.5), signed as the live cast signs them.
    const templates = acn<{ name: string; actor: Actor; draft: Draft }[]>("templates").map((x) => {
      const e = this.sign(this.secrets[x.actor], x.draft, t + 2);
      this.templateIds[x.name] = e.id;
      return e;
    });
    this.extra = [bind, ack, ...templates];
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

  /** The cast as acn-sim takes it: the keys plus the published template ids. */
  private castArg() {
    return { ...this.cast, templates: this.templateIds };
  }

  template(scenario: string): any {
    return acn("template", { scenario, cast: this.castArg() });
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
    // The secret's holder: the arbiter (Core.md §8.4) or the settlement verifier (verifier trigger, §8.2).
    const terms = template.terms ?? {};
    const holder = terms.arbiter ?? (terms.settlement_trigger === "verifier" ? terms.settlement_verifier : undefined);
    // Held-out instances (CR-003 §2.3): our seed's hash goes in the terms; the seed goes to the runner.
    const seed = template.requester_seed_to ? randomHex32() : undefined;
    const draft = acn<Draft>("request", {
      requester_seed_hash: seed ? acn<string>("sha256", { bytes: seed }) : null,
      seed: seed ? "presenter-seed" : undefined,
      template,
      tool: scenario,
      now: this.now(),
      performer,
      hashlock,
      input: { brief },
      // Presenter mode plays every role locally, so the escrow is a placeholder; the live app
      // NIP-44-encrypts s to its holder.
      escrow: holder && s ? "presenter-escrow" : undefined,
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
      if (holder) this.escrow[e.id] = s;
    }
    // Atlas's per-commitment secret (k when contingent; its performer seed is derived from it).
    this.keys[e.id] = randomHex32();
    if (seed) this.seeds[e.id] = seed;
    if (token) {
      this.tokens[e.id] = token;
      this.mint = acn<any>("mint_tag", { mint: this.mint, token, commitment: e.id }).mint;
      this.onWallet();
    }
    this.add(e);
    return e.id;
  }

  /** The requester drafter (acn-author) over the cast's published templates. */
  draft(intent: any): any {
    const templates = this.extra
      .filter((e) => e.kind === 7727)
      .map((e) => ({ event: e.id, content: JSON.parse(e.content) }));
    return acn("draft_request", { intent, templates, config: {} });
  }

  /** Requester: send terms we drafted (free, or priced at the simulated mint). */
  requestTerms(performer: string, profile: string, terms: any): string {
    let s: string | undefined;
    let hashlock: string | null = null;
    const price = terms?.price?.amount ?? 0;
    if (price > 0 && terms.payment) {
      s = randomHex32();
      hashlock = acn<string>("sha256", { bytes: s });
      terms = { ...terms, payment: { ...terms.payment, hashlock, locktime: terms.deadline + terms.review_window + 24 * 3600 } };
    }
    const draft = acn<Draft>("request_terms", { performer, profile, terms });
    const content = JSON.parse(draft.content);
    if (s && price > 0) {
      const r = acn<any>("mint_lock", {
        mint: this.mint, owner: this.me, amount: price, hashlock, receiver: performer,
        locktime: terms.payment.locktime, now: this.now(),
      });
      this.mint = r.mint;
      content.token = r.token;
      draft.content = JSON.stringify(content);
    }
    const e = this.sign(this.mySecret, draft, this.now());
    if (s) this.preimages[e.id] = s;
    this.keys[e.id] = randomHex32();
    if (content.token) {
      this.tokens[e.id] = content.token;
      this.mint = acn<any>("mint_tag", { mint: this.mint, token: content.token, commitment: e.id }).mint;
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
    if (action.type === "fund") {
      // Contingent: lock the price to the offer's hashlock H = sha256(k), payable to the performer.
      const content = JSON.parse(draft.content);
      const performer = this.project().find((p) => p.id === commitment)?.performer ?? this.cast.performer;
      const r = acn<any>("mint_lock", {
        mint: this.mint, owner: this.me, amount: content.amount, hashlock: content.hashlock,
        receiver: performer, locktime: content.locktime, commitment, now: this.now(),
      });
      this.mint = r.mint;
      this.tokens[commitment] = r.token;
      content.token = r.token;
      draft.content = JSON.stringify(content);
      this.onWallet();
    }
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
      // Contingent requests carry no token: the requester pays later, by FUND.
      if (!terms?.payment || !(terms.price?.amount > 0) || terms.settlement_trigger === "contingent") continue;
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
    const moves = acn<Move[]>("react", { cast: this.castArg(), events: this.events, escrow: this.escrow, payments: this.payments, keys: this.keys, seeds: this.seeds });
    for (const m of moves) {
      if (this.scheduled.has(m.key)) continue;
      this.scheduled.add(m.key);
      const t = window.setTimeout(() => {
        const c = m.draft.tags.find((t) => t[0] === "c")?.[1] ?? "";
        if (m.escrow) {
          // Placeholder ciphertext (every role is local); the holder learns the key in-process.
          m.draft.tags.push(["escrow", "presenter-escrow", this.cast[m.escrow.to]]);
          this.escrow[c] = m.escrow.secret;
        }
        if (m.draft.kind === 7715) {
          // CLAIM: Atlas redeems the FUND token with k at the mint, then signs (revealing k).
          const fund = this.events.find((e) => e.kind === 7714 && commitmentOf(e) === c);
          const token = fund && JSON.parse(fund.content).token;
          this.redeemed.add(c);
          try {
            const r = acn<any>("mint_redeem", { mint: this.mint, token, by: this.cast.performer, preimage: this.keys[c], now: this.now() });
            this.mint = r.mint;
            this.onWallet();
          } catch (err) {
            this.narration[c] = `Atlas could not claim: ${err}`;
            this.onChange(c);
            return;
          }
        }
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
    this.seeds = {};
    this.preimages = {};
    this.scheduled.clear();
    this.narration = {};
    this.offset = 0;
    this.payments = {};
    this.tokens = {};
    this.redeemed.clear();
    this.keys = {};
    this.resetMint();
  }
}

export function commitmentOf(e: AcnEvent): string {
  if (e.kind === 7700) return e.id;
  return e.tags.find((t) => t[0] === "c")?.[1] ?? "";
}
