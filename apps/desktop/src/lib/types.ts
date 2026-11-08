// Mirrors crates/nmesh-app/src/view.rs. Keep in sync.

export interface Identity {
  pubkey: string;
  npub: string;
  name: string | null;
  relays: string[];
}

export interface ArgView {
  name: string;
  type: string;
  required: boolean;
  label: string;
  choices: string[];
}

export interface CommandView {
  command: string;
  title: string;
  description: string;
  args: ArgView[];
}

export type Hosting = "self_hosted" | "operator" | "third_party";

export interface AgentManifest {
  id: string;
  name: string;
  description: string;
  hosting: Hosting;
  operator: string;
  data_handling: {
    sees_plaintext: boolean;
    retention: string;
    forwards_to: string[];
  };
  tools: { name: string; description: string; output_contract?: string | null }[];
}

export interface BindingView {
  bound: boolean;
  operator: string | null;
  detail: string;
}

export interface AcnTool {
  name: string;
  title: string;
  description: string;
  profile: string;
  shows: string[];
  price_sat: number;
}

export interface AcnPeer {
  profiles: string[];
  min_price_sat: number;
  deadline_secs: number;
  review_window_secs: number;
  binding: BindingView;
  tools: AcnTool[];
  simulated: boolean;
}

export interface Contact {
  pubkey: string;
  npub: string;
  name: string | null;
  about: string | null;
  picture: string | null;
  is_agent: boolean;
  agent: AgentManifest | null;
  commands: CommandView[];
  consented: boolean;
  last_message: string | null;
  last_at: number | null;
  unread: number;
  acn: AcnPeer | null;
}

// --- ACN commitments (docs/Core.md). Mirrors crates/acn-view (Projection) plus the nmesh-app
// additions (conversation, result, performer_binding). ---

export type Phase =
  | "requested" | "countered" | "promised" | "reported" | "reworking"
  | "satisfied" | "rejected" | "disputed" | "declined" | "withdrawn" | "revoked";

export type Party = "requester" | "performer" | "arbiter";
export type Role = "requester" | "performer" | "arbiter" | "verifier" | "governance" | "other";
export type Severity = "reject" | "flag" | "pending";

export interface Note {
  code: string;
  severity: Severity;
  detail: string;
}

export interface Entry {
  id: string;
  act: string;
  kind: number;
  by: string;
  role: Role;
  mine: boolean;
  created_at: number;
  status: "applied" | "annotation" | "rejected" | "pending";
  summary: string;
  notes: Note[];
}

export interface AnomalyView {
  event: string;
  act: string;
  code: string;
  severity: Severity;
  detail: string;
}

export type ConditionStatus =
  | { status: "pass" } | { status: "fail" } | { status: "unverified" } | { status: "human" }
  | { status: "advisory"; verdicts: string[] };

export interface Evidence {
  id: string;
  act: string;
  by: string;
  role: Role;
  verdict: string;
  counted: boolean;
  why: string | null;
}

export interface ConditionView {
  id: string;
  class: "mechanical" | "grounding" | "interpretive" | "human";
  description: string;
  status: ConditionStatus | null;
  evidence: Evidence[];
}

export interface CounterView {
  by: Role;
  mine: boolean;
  reason: string;
  terms_hash: string;
  deadline: number;
  current_deadline: number;
  price_sat: number;
  current_price_sat: number;
  rework_limit: number;
  current_rework_limit: number;
}

export interface SettlementView {
  amount_sat: number;
  mint: string;
  simulated: boolean;
  status: "locked" | "paid" | "refundable";
  detail: string;
  locktime: number | null;
}

export interface ReportView {
  id: string;
  payload: any;
  contract: string | null;
  note: string | null;
  artifacts: string[];
  version: number;
}

export interface Allowed {
  satisfy: boolean;
  reject: boolean;
  withdraw: boolean;
  accept_counter: boolean;
  dispute: boolean;
}

export interface Commitment {
  id: string;
  conversation: string;
  requester: string;
  performer: string;
  mine: boolean;
  profile: "T0" | "T1" | "N";
  phase: Phase | null;
  terminal: boolean;
  awaiting: Party | null;
  awaiting_me: boolean;
  lapsed: Party | null;
  deadline_in: number | null;
  tool: string | null;
  input: any;
  description: string;
  price_sat: number;
  created_at: number;
  arbiter: string | null;
  rework_count: number;
  rework_limit: number;
  chain: Entry[];
  timeline: Entry[];
  report: ReportView | null;
  conditions: ConditionView[];
  counter: CounterView | null;
  settlement: SettlementView | null;
  anomalies: AnomalyView[];
  allowed: Allowed;
  result: Body | null;
  performer_binding: BindingView;
  // Board (computed in acn-view)
  lane: Lane;
  my_role: Role;
  next: Role | null;
  needs_me: boolean;
  pending_endorsement: boolean;
  updated_at: number;
}

export type Lane = "requested" | "negotiating" | "in_progress" | "review" | "dispute" | "closed";

export interface BoardView {
  /** Open commitments, then the newest closed ones (up to the requested limit). */
  items: Commitment[];
  closed_total: number;
  now: number;
}

export type HintAction =
  | { type: "accept_counter" } | { type: "satisfy" } | { type: "withdraw" } | { type: "wait" } | { type: "done" }
  | { type: "reject"; condition: string; note: string }
  | { type: "dispute"; reason: string }
  | { type: "advance_clock"; secs: number };

export interface Hint {
  commitment: string;
  action: HintAction;
  label: string;
  narration: string;
}

export interface Clock {
  now: number;
  offset: number;
}

export type RendererView =
  | { tier: "installed"; id: string }
  | { tier: "trusted"; id: string; publisher: string | null }
  | { tier: "generic" }
  | { tier: "alt" };

export type Body =
  | { kind: "text"; text: string }
  | {
      kind: "structured";
      contract: string;
      schema_hash: string | null;
      payload: any;
      alt: string;
      valid: boolean;
      error: string | null;
      renderer: RendererView;
      reply_to: string | null;
    }
  | { kind: "action"; target: string; contract: string; payload: any; alt: string }
  | { kind: "unknown"; event_kind: number; alt: string | null }
  | { kind: "commitment"; commitment: string };

export interface Message {
  id: string;
  conversation: string;
  sender: string;
  mine: boolean;
  created_at: number;
  body: Body;
}

export type AppEvent =
  | { type: "message"; message: Message }
  | { type: "contact"; contact: Contact }
  | { type: "commitment"; commitment: Commitment }
  | { type: "wallet"; wallet: Wallet };

/** One hash-locked payment (a NUT-14 HTLC token). Mirrors `acn_wallet::Lock`. */
export interface WalletLock {
  id: string;
  amount: number;
  hashlock: string;
  receiver: string;
  refund: string;
  locktime: number;
  commitment: string | null;
  state: "locked" | "redeemed" | "refunded";
  created_at: number;
  settled_at: number | null;
}

export interface WalletEntry {
  at: number;
  kind: "top_up" | "lock" | "redeem" | "refund";
  delta: number;
  commitment: string | null;
  note: string;
}

/** Mirrors `acn_wallet::WalletView`. */
export interface Wallet {
  owner: string;
  mint: string;
  /** Presenter mode: an in-browser simulated mint. Otherwise real Cashu ecash (test sats). */
  simulated: boolean;
  available: number;
  locked_out: number;
  incoming: number;
  history: WalletEntry[];
  locks: WalletLock[];
}

export interface CastWallet {
  name: string;
  wallet: Wallet;
}

/** What a renderer may ask the host to do. Renderers never sign or send anything themselves. */
export interface RendererHost {
  /** Button press. `actionId` usually is a slash command, e.g. "/echo hi". */
  action(actionId: string, label: string): Promise<void>;
  /** Form submission. */
  submit(actionId: string | undefined, values: Record<string, unknown>, label?: string): Promise<void>;
  openUrl(url: string): void;
}
