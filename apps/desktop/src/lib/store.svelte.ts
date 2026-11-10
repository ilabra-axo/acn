import type { Backend } from "./api";
import { createBackend } from "./api";
import type { AppEvent, BoardView, BuildInfo, CastWallet, Clock, Commitment, Contact, Hint, Identity, Message, UpdateStatus, Wallet } from "./types";

function upsertSorted(list: Message[], m: Message): Message[] {
  if (list.some((x) => x.id === m.id)) return list;
  const next = [...list, m];
  next.sort((a, b) => a.created_at - b.created_at);
  return next;
}

function sortContacts(list: Contact[]): Contact[] {
  return [...list].sort(
    (a, b) => (b.last_at ?? 0) - (a.last_at ?? 0) || (a.name ?? a.npub).localeCompare(b.name ?? b.npub),
  );
}

export class AppStore {
  backend: Backend | null = null;
  me = $state<Identity | null>(null);
  build = $state<BuildInfo | null>(null);
  update = $state<UpdateStatus | null>(null);
  restarting = $state(false);
  contacts = $state<Contact[]>([]);
  active = $state<string | null>(null);
  messages = $state<Message[]>([]);
  /** ACN commitments of the active conversation, by id (projections from the Rust fold). */
  commitments = $state<Record<string, Commitment>>({});
  error = $state<string | null>(null);
  ready = $state(false);

  activeContact = $derived(this.contacts.find((c) => c.pubkey === this.active) ?? null);

  async init() {
    try {
      this.backend = await createBackend();
      await this.backend.onEvent((ev) => this.onEvent(ev));
      this.me = await this.backend.identity();
      this.contacts = sortContacts(await this.backend.contacts());
      this.ready = true;
      this.loadWallet();
      this.loadBoard();
      this.build = await this.backend.buildInfo();
      this.checkUpdate();
      window.setInterval(() => this.checkUpdate(), 120_000);
    } catch (e) {
      this.fail(e);
    }
  }

  async checkUpdate() {
    if (!this.backend || this.restarting) return;
    try {
      this.update = await this.backend.updateCheck();
    } catch {
      this.update = null;
    }
  }

  /** Dev: `pnpm tauri dev` rebuilds from the current sources and relaunches the app. */
  async restartToUpdate() {
    if (!this.backend) return;
    this.restarting = true;
    try {
      await this.backend.restartToUpdate();
    } catch (e) {
      this.restarting = false;
      this.fail(e);
    }
  }

  private onEvent(ev: AppEvent) {
    if (ev.type === "contact") {
      const c = ev.contact;
      if (c.pubkey === this.active) c.unread = 0;
      const rest = this.contacts.filter((x) => x.pubkey !== c.pubkey);
      this.contacts = sortContacts([...rest, c]);
    } else if (ev.type === "commitment") {
      if (ev.commitment.conversation === this.active) {
        this.commitments = { ...this.commitments, [ev.commitment.id]: ev.commitment };
      }
      this.scheduleBoard();
    } else if (ev.type === "wallet") {
      this.wallet = ev.wallet;
      this.loadCastWallets();
    } else if (ev.message.conversation === this.active) {
      this.messages = upsertSorted(this.messages, ev.message);
    }
  }

  async open(pubkey: string) {
    if (!this.backend) return;
    this.active = pubkey;
    this.messages = [];
    this.commitments = {};
    try {
      this.messages = await this.backend.openConversation(pubkey);
      const list = await this.backend.commitments(pubkey);
      this.commitments = Object.fromEntries(list.map((c) => [c.id, c]));
      this.contacts = this.contacts.map((c) => (c.pubkey === pubkey ? { ...c, unread: 0 } : c));
    } catch (e) {
      this.fail(e);
    }
  }

  async addContact(input: string): Promise<boolean> {
    if (!this.backend || !input.trim()) return false;
    try {
      const c = await this.backend.addContact(input.trim());
      this.onEvent({ type: "contact", contact: c });
      await this.open(c.pubkey);
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  async acceptAgent(pubkey: string) {
    if (!this.backend) return;
    try {
      const c = await this.backend.acceptAgent(pubkey);
      this.onEvent({ type: "contact", contact: c });
    } catch (e) {
      this.fail(e);
    }
  }

  async sendText(text: string) {
    if (!this.backend || !this.active) return;
    try {
      const m = await this.backend.sendText(this.active, text);
      this.messages = upsertSorted(this.messages, m);
    } catch (e) {
      this.fail(e);
    }
  }

  async sendAction(
    target: string,
    actionId: string | null,
    values: Record<string, unknown> | null,
    label: string | null,
  ) {
    if (!this.backend || !this.active) return;
    try {
      const m = await this.backend.sendAction(this.active, target, actionId, values, label);
      this.messages = upsertSorted(this.messages, m);
    } catch (e) {
      this.fail(e);
    }
  }

  // --- ACN ---

  private putCommitment(c: Commitment) {
    this.commitments = { ...this.commitments, [c.id]: c };
  }

  /** Request a commitment. Returns its id. */
  async request(text: string): Promise<string | null> {
    if (!this.backend || !this.active) return null;
    try {
      const c = await this.backend.acnRequest(this.active, text);
      this.putRequest(c);
      return c.id;
    } catch (e) {
      this.fail(e);
      return null;
    }
  }

  /** A REQUEST we just sent: its card, and its place in the conversation. */
  private putRequest(c: Commitment) {
    this.putCommitment(c);
    this.messages = upsertSorted(this.messages, {
      id: c.id,
      conversation: c.conversation,
      sender: this.me?.pubkey ?? "",
      mine: true,
      created_at: c.created_at,
      body: { kind: "commitment", commitment: c.id },
    });
  }

  /** Requester acts. `arg` is the reject note / dispute reason. */
  async commitmentAct(
    kind: "satisfy" | "reject" | "withdraw" | "accept_counter" | "dispute" | "fund",
    id: string,
    arg = "",
    condition: string | null = null,
    conversation: string | null = null,
  ) {
    const p = conversation ?? this.active;
    if (!this.backend || !p) return;
    try {
      const b = this.backend;
      const c =
        kind === "satisfy" ? await b.acnSatisfy(p, id)
        : kind === "reject" ? await b.acnReject(p, id, condition, arg)
        : kind === "withdraw" ? await b.acnWithdraw(p, id)
        : kind === "accept_counter" ? await b.acnAcceptCounter(p, id)
        : kind === "fund" ? await b.acnFund(p, id)
        : await b.acnDispute(p, id, arg);
      if (c.conversation === this.active) this.putCommitment(c);
      if (this.view === "board") await this.loadBoard();
    } catch (e) {
      this.fail(e);
    }
  }

  /** Counter-offer with edited terms (profile N). Returns false on error (shown as a toast). */
  async counter(id: string, terms: any, reason: string, conversation: string | null = null): Promise<boolean> {
    const p = conversation ?? this.active;
    if (!this.backend || !p) return false;
    try {
      const c = await this.backend.acnCounter(p, id, terms, reason);
      if (c.conversation === this.active) this.putCommitment(c);
      if (this.view === "board") await this.loadBoard();
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  /** Send drafted terms as a REQUEST to the active conversation. */
  async requestTerms(profile: string, terms: any): Promise<boolean> {
    const p = this.active;
    if (!this.backend || !p) return false;
    try {
      const c = await this.backend.acnRequestTerms(p, profile, terms);
      this.putRequest(c);
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  // --- board (queue view, Core.md §12) ---

  view = $state<"chats" | "board">("chats");
  board = $state<BoardView | null>(null);
  /** How many closed commitments to show; "Show older" raises it. */
  closedLimit = $state(6);
  needsMeOnly = $state(false);
  /** Commitment open in the board's drawer. */
  boardOpen = $state<string | null>(null);
  private boardTimer: number | null = null;

  showView(v: "chats" | "board") {
    this.view = v;
    if (v === "board") this.loadBoard();
    else this.boardOpen = null;
  }

  async loadBoard() {
    if (!this.backend) return;
    try {
      this.board = await this.backend.board(this.closedLimit);
    } catch (e) {
      this.fail(e);
    }
  }

  /** Coalesce bursts of commitment events into one reload. */
  private scheduleBoard() {
    if (this.boardTimer !== null) return;
    this.boardTimer = window.setTimeout(() => {
      this.boardTimer = null;
      this.loadBoard();
    }, 250);
  }

  showOlder() {
    this.closedLimit += 10;
    this.loadBoard();
  }

  /** Leave the board for the conversation a commitment lives in. */
  async openInChat(id: string) {
    const c = this.board?.items.find((x) => x.id === id);
    if (!c) return;
    this.showView("chats");
    await this.open(c.conversation);
  }

  // --- wallet ---

  /** The built-in Cashu wallet; `null` when none is configured. */
  wallet = $state<Wallet | null>(null);
  castWallets = $state<CastWallet[]>([]);
  walletOpen = $state(false);
  walletBusy = $state(false);

  /** My lock for a commitment, if I paid for it. */
  lockFor(commitment: string) {
    return this.wallet?.locks.find((l) => l.commitment === commitment) ?? null;
  }

  async loadWallet() {
    if (!this.backend) return;
    try {
      this.wallet = await this.backend.wallet();
      await this.loadCastWallets();
    } catch (e) {
      this.fail(e);
    }
  }

  private async loadCastWallets() {
    if (!this.backend) return;
    try {
      this.castWallets = await this.backend.castWallets();
    } catch {
      /* optional */
    }
  }

  async topUp(amount: number) {
    if (!this.backend) return;
    this.walletBusy = true;
    try {
      this.wallet = await this.backend.walletTopUp(amount);
    } catch (e) {
      this.fail(e);
    } finally {
      this.walletBusy = false;
    }
  }

  async refund(conversation: string, commitment: string) {
    if (!this.backend) return;
    this.walletBusy = true;
    try {
      this.wallet = await this.backend.walletRefund(conversation, commitment);
    } catch (e) {
      this.fail(e);
    } finally {
      this.walletBusy = false;
    }
  }

  // --- presenter ---

  clock = $state<Clock>({ now: Math.floor(Date.now() / 1000), offset: 0 });
  /** Commitment the Director is following. */
  focus = $state<string | null>(null);
  hint = $state<Hint | null>(null);
  autopilot = $state(false);
  directorOpen = $state(false);
  private ticker: number | null = null;

  startTicker() {
    if (this.ticker !== null) return;
    this.ticker = window.setInterval(() => this.tick(), 900);
  }

  private busyTick = false;
  private walletTicks = 0;
  private boardTicks = 0;
  private async tick() {
    if (!this.backend || this.busyTick) return;
    this.busyTick = true;
    try {
      this.clock = await this.backend.clock();
      // Live wallets learn about redemptions from the mint (NUT-07); poll while it is on screen.
      if (this.walletOpen && this.backend.kind === "tauri" && ++this.walletTicks % 5 === 0) {
        this.wallet = await this.backend.wallet();
      }
      // Countdowns and lapses are time-derived: re-project now and then.
      if (this.view === "board" && ++this.boardTicks % 3 === 0) await this.loadBoard();
      if (this.focus && this.active) {
        this.hint = await this.backend.acnHint(this.active, this.focus);
        const t = this.hint?.action.type;
        if (this.autopilot && t && t !== "wait" && t !== "done") await this.next();
      } else {
        this.hint = null;
      }
    } catch {
      /* the next tick retries */
    } finally {
      this.busyTick = false;
    }
  }

  demoSearching = $state(false);

  /** Find the ACN demo cast, open Atlas and the Director. */
  async openDemo(): Promise<boolean> {
    if (!this.backend) return false;
    this.demoSearching = true;
    try {
      const cast = await this.backend.discoverDemo();
      if (!cast.length) {
        this.fail(
          "ACN demo cast not found on your relay. Run ./target/debug/nmesh-acn-sim in the repo root, then try again.",
        );
        return false;
      }
      for (const c of cast) this.onEvent({ type: "contact", contact: c });
      await this.open(cast[0].pubkey);
      this.directorOpen = true;
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    } finally {
      this.demoSearching = false;
    }
  }

  async startScenario(id: string, brief: string) {
    this.autopilot = false;
    const cid = await this.request(`/${id}${brief ? ` ${brief}` : ""}`);
    if (cid) this.focus = cid;
  }

  async next() {
    if (!this.backend || !this.active || !this.focus) return;
    try {
      const c = await this.backend.acnRunHint(this.active, this.focus);
      if (c) this.putCommitment(c);
      this.hint = await this.backend.acnHint(this.active, this.focus);
      this.clock = await this.backend.clock();
    } catch (e) {
      this.fail(e);
    }
  }

  async advanceClock(secs: number) {
    if (!this.backend) return;
    this.clock = await this.backend.setClockOffset(this.clock.offset + secs);
    if (this.view === "board") await this.loadBoard();
    if (this.active) {
      const list = await this.backend.commitments(this.active);
      this.commitments = Object.fromEntries(list.map((c) => [c.id, c]));
    }
  }

  /** Clear every commitment, stop autopilot and reset the clock. */
  async resetDemo() {
    if (!this.backend) return;
    try {
      this.autopilot = false;
      this.focus = null;
      this.hint = null;
      await this.backend.resetDemo();
      this.commitments = {};
      this.clock = await this.backend.clock();
      if (this.active) this.messages = await this.backend.openConversation(this.active);
      this.boardOpen = null;
      this.closedLimit = 6;
      await this.loadBoard();
      await this.loadWallet();
    } catch (e) {
      this.fail(e);
    }
  }

  async resetClock() {
    if (!this.backend) return;
    this.clock = await this.backend.setClockOffset(0);
    await this.advanceClock(0);
  }

  openUrl(url: string) {
    this.backend?.openUrl(url).catch((e) => this.fail(e));
  }

  fail(e: unknown) {
    this.error = typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
    setTimeout(() => (this.error = null), 9000);
  }
}

export const store = new AppStore();
