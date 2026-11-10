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
  /** Conformance level the agent claims (K_SVC binding_level): a claim, not checked by the app. */
  binding_level: string | null;
  blossom: string[];
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
  | "satisfied" | "rejected" | "disputed" | "declined" | "withdrawn" | "revoked" | "settled" | "offered" | "funded";

export type Party = "requester" | "performer" | "arbiter" | "verifier";
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
  /** What the attestation says (protocol content; rationales travel by hash). */
  detail: AttestationDetail;
}

export interface AttestationDetail {
  created_at: number;
  method: string | null;
  recipe: string | null;
  referent: any;
  judges: { model_id: string; model_version: string; runs: number[]; median: number | null }[];
  aggregate_score: number | null;
  rubric_text_hash: string | null;
  injection_detected: boolean;
  instances: number[];
  rationale: string | null;
}

/** Derived by the validator (spec §7); weakest first: opinion < human < check < grounded. */
export type Assurance = "opinion" | "human" | "check" | "grounded";

export interface JudgedView {
  judges: string[];
  threshold: number;
  scale_min: number;
  scale_max: number;
  aggregation: "all" | "mean" | "median";
  repeats: number;
  /** Where the performer reads the rubric (INV-14). */
  rubric_ref: string | null;
  rubric_text: string | null;
  runners: string[];
}

export type ConditionClass = "mechanical" | "grounding" | "judged" | "interpretive" | "human" | "open";

export interface ConditionView {
  id: string;
  class: ConditionClass;
  /** null for an `open` condition (CR-003). */
  assurance: Assurance | null;
  description: string;
  status: ConditionStatus | null;
  evidence: Evidence[];
  judged: JudgedView | null;
}

// ---- CR-003: drafting, disqualifiers, the terms card ----

export type DisqualifierStatus = "hit" | "clear" | "unchecked";

export interface DisqualifierView {
  id: string;
  class: ConditionClass;
  assurance: Assurance | null;
  description: string;
  status: DisqualifierStatus | null;
  evidence: Evidence[];
}

export interface TermsVersionView {
  version: number;
  hash: string;
  act: string;
  drafted_by: string;
  drafted_by_role: Role;
  drafted_by_me: boolean;
  prev: string | null;
  in_force: boolean;
  pending: boolean;
}

export interface TermsCard {
  version: number;
  hash: string;
  drafted_by: string;
  drafted_by_role: Role;
  drafted_by_me: boolean;
  assurance: Assurance | null;
  done_means: {
    id: string;
    class: ConditionClass;
    assurance: Assurance | null;
    description: string;
    suggested_class: ConditionClass | null;
    check_ref: string | null;
    rubric_ref: string | null;
    rubric_text: string | null;
    instances: InstancesView | null;
  }[];
  checked_by: { item: string; keys: string[]; requester: boolean; judges: string[]; rule: string | null }[];
  pay_if: {
    price_sat: number;
    trigger: "requester" | "verifier" | "contingent";
    settlement_verifier: string | null;
    arbiter: string | null;
    deadline: number;
    review_window: number;
    rework_limit: number;
    fund_window: number | null;
  };
  disqualified_if: { id: string; class: ConditionClass; assurance: Assurance | null; description: string; instances: InstancesView | null }[];
  /** Why this version can't be promised or accepted yet. */
  unclosed: string | null;
  /** The template this version was drafted from (V-28), with its reader-computed record. */
  template: TemplateView | null;
  /** The terms linter (CR-003 §3, L1–L8). */
  lint: LintFinding[];
}

export interface LintFinding {
  rule: string;
  level: "error" | "warning";
  item: string | null;
  message: string;
}

export interface TemplateView {
  event: string;
  name: string | null;
  author: string | null;
  status: { status: "counted" } | { status: "mismatch"; reason: string } | { status: "unavailable" };
  slots: Record<string, unknown>;
  record: { commitments: number; paid: number; rejected: number; disqualified: number; disputed: number; open: number };
}

/** Held-out instances: the public part of the pool (never its contents). */
export interface InstancesView {
  pool_description: string;
  pool_rule: string;
  pool_size: number;
  sample_size: number;
  pool_root: string;
}

export interface DiffLine {
  kind: "condition" | "disqualifier" | "field" | "time";
  what: string;
  before: string | null;
  after: string | null;
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
  status: "locked" | "paid" | "refundable" | "unfunded";
  detail: string;
  locktime: number | null;
}

export interface ReportView {
  id: string;
  payload: any;
  contract: string | null;
  note: string | null;
  artifacts: string[];
  /** Hash of the derivation record (Core §9), if the performer stored one. */
  derivation: string | null;
  version: number;
}

export interface Allowed {
  satisfy: boolean;
  reject: boolean;
  withdraw: boolean;
  accept_counter: boolean;
  /** Propose other terms with a COUNTER (profile N, while negotiating). */
  counter: boolean;
  dispute: boolean;
  /** Contingent: pay against the reported hashlock (CR-002). */
  fund: boolean;
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
  disqualifiers: DisqualifierView[];
  versions: TermsVersionView[];
  terms_card: TermsCard | null;
  pending_card: TermsCard | null;
  /** The terms under negotiation as a document: what a counter-offer edits. */
  negotiable_terms: any | null;
  terms_diff: DiffLine[];
  counter: CounterView | null;
  settlement: SettlementView | null;
  anomalies: AnomalyView[];
  allowed: Allowed;
  /** Overall assurance (weakest condition); null while the terms are unavailable. */
  assurance: Assurance | null;
  /** Priced terms: whose act releases the payment (spec §12). */
  payment_depends_on: PaymentDependency | null;
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
  | { type: "accept_counter" } | { type: "satisfy" } | { type: "withdraw" } | { type: "wait" } | { type: "done" } | { type: "fund" }
  | { type: "reject"; condition: string; note: string }
  | { type: "dispute"; reason: string }
  | { type: "advance_clock"; secs: number }
  | { type: "counter"; terms: any; reason: string };

/** The requester drafter's input and output (acn-author, CR-003 §3). */
export interface Intent {
  goal: string;
  requirements: string[];
  price_sat: number;
  deadline: number;
  review_window: number;
}

export interface RequestDraft {
  terms: any;
  template: string | null;
  notes: string[];
  findings: LintFinding[];
}

export interface Hint {
  commitment: string;
  action: HintAction;
  label: string;
  narration: string;
}

export interface BuildInfo {
  version: string;
  commit: string;
  built_at: number;
  acn: string;
  /** Debug build from a source checkout (updates = rebuild from the repo). */
  dev: boolean;
}

export interface UpdateStatus {
  current: string;
  latest: string;
  available: boolean;
  changes: string[];
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

export type PaymentDependency =
  | { kind: "requester"; arbiter: string | null }
  | { kind: "verifier"; verifier: string }
  | { kind: "contingent"; verifier: string };
