import type { AppEvent, BoardView, CastWallet, Clock, Commitment, Contact, Hint, Identity, Message, Wallet } from "./types";

export interface Backend {
  /** `tauri`: the real app. `presenter`: in-browser demo (mock chat + real ACN via WASM). */
  readonly kind: "tauri" | "presenter";
  identity(): Promise<Identity>;
  contacts(): Promise<Contact[]>;
  openConversation(pubkey: string): Promise<Message[]>;
  addContact(input: string): Promise<Contact>;
  acceptAgent(pubkey: string): Promise<Contact>;
  sendText(pubkey: string, text: string): Promise<Message>;
  sendAction(
    pubkey: string,
    target: string,
    actionId: string | null,
    values: Record<string, unknown> | null,
    label: string | null,
  ): Promise<Message>;
  onEvent(cb: (ev: AppEvent) => void): Promise<() => void>;
  openUrl(url: string): Promise<void>;
  // ACN commitments
  commitments(pubkey: string): Promise<Commitment[]>;
  acnRequest(pubkey: string, text: string): Promise<Commitment>;
  acnSatisfy(pubkey: string, commitment: string): Promise<Commitment>;
  acnReject(pubkey: string, commitment: string, condition: string | null, note: string): Promise<Commitment>;
  acnWithdraw(pubkey: string, commitment: string): Promise<Commitment>;
  acnAcceptCounter(pubkey: string, commitment: string): Promise<Commitment>;
  acnDispute(pubkey: string, commitment: string, reason: string): Promise<Commitment>;
  acnHint(pubkey: string, commitment: string): Promise<Hint | null>;
  acnRunHint(pubkey: string, commitment: string): Promise<Commitment | null>;
  clock(): Promise<Clock>;
  setClockOffset(offset: number): Promise<Clock>;
  /** Find the ACN demo cast (Atlas first). Empty if it isn't running. */
  discoverDemo(): Promise<Contact[]>;
  // Wallet (Cashu). `null`: no wallet configured.
  wallet(): Promise<Wallet | null>;
  walletTopUp(amount: number): Promise<Wallet>;
  /** Reclaim the lock of a commitment that closed unsatisfied, after its locktime. */
  walletRefund(pubkey: string, commitment: string): Promise<Wallet>;
  /** Wallets of the simulated cast, when this backend can see them (presenter mode). */
  castWallets(): Promise<CastWallet[]>;
  /** Forget all commitments so far and reset the clock (presenter: also the simulated mint). */
  resetDemo(): Promise<void>;
  /** Commitments across all conversations and roles; closed ones newest first, paged. */
  board(closedLimit: number): Promise<BoardView>;
}

const EVENT = "nmesh://event";

async function createTauriBackend(): Promise<Backend> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  return {
    kind: "tauri",
    identity: () => invoke("identity"),
    contacts: () => invoke("contacts"),
    openConversation: (pubkey) => invoke("open_conversation", { pubkey }),
    addContact: (input) => invoke("add_contact", { input }),
    acceptAgent: (pubkey) => invoke("accept_agent", { pubkey }),
    sendText: (pubkey, text) => invoke("send_text", { pubkey, text }),
    sendAction: (pubkey, target, actionId, values, label) =>
      invoke("send_action", { pubkey, target, actionId, values, label }),
    onEvent: async (cb) => listen<AppEvent>(EVENT, (e) => cb(e.payload)),
    openUrl: (url) => openUrl(url),
    commitments: (pubkey) => invoke("commitments", { pubkey }),
    acnRequest: (pubkey, text) => invoke("acn_request", { pubkey, text }),
    acnSatisfy: (pubkey, commitment) => invoke("acn_satisfy", { pubkey, commitment }),
    acnReject: (pubkey, commitment, condition, note) => invoke("acn_reject", { pubkey, commitment, condition, note }),
    acnWithdraw: (pubkey, commitment) => invoke("acn_withdraw", { pubkey, commitment }),
    acnAcceptCounter: (pubkey, commitment) => invoke("acn_accept_counter", { pubkey, commitment }),
    acnDispute: (pubkey, commitment, reason) => invoke("acn_dispute", { pubkey, commitment, reason }),
    acnHint: (pubkey, commitment) => invoke("acn_hint", { pubkey, commitment }),
    acnRunHint: (pubkey, commitment) => invoke("acn_run_hint", { pubkey, commitment }),
    clock: () => invoke("clock"),
    setClockOffset: (offset) => invoke("set_clock_offset", { offset }),
    discoverDemo: () => invoke("discover_demo"),
    wallet: () => invoke("wallet"),
    walletTopUp: (amount) => invoke("wallet_top_up", { amount }),
    walletRefund: (pubkey, commitment) => invoke("wallet_refund", { pubkey, commitment }),
    // Atlas runs in another process (nmesh-acn-sim); it reports redemptions by DM.
    castWallets: async () => [],
    resetDemo: () => invoke("reset_demo"),
    board: (closedLimit) => invoke("board", { closedLimit }),
  };
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function createBackend(): Promise<Backend> {
  if (isTauri()) return createTauriBackend();
  const { createPresenterBackend } = await import("./presenter/backend");
  return createPresenterBackend();
}
