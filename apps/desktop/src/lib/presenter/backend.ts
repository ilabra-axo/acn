// Presenter-mode backend: runs entirely in the browser (`pnpm dev`, no Tauri, no relay).
//
// - Chat (Echo agent, Alice) is a small stand-in for nmesh-app, for showing renderers.
// - ACN (Atlas, Rogue) is real: signed events, judged by the Rust validator via WASM, with the
//   same scenario cast as the live simulator (see engine.ts).

import type { Backend } from "../api";
import type { AppEvent, Body, Commitment, Contact, Message } from "../types";
import { AcnWorld } from "./engine";
import { loadAcn } from "./wasm";

const ME_LABEL = "You";
const ECHO = "e".repeat(64);
const ALICE = "b".repeat(64);

const RENDERERS: Record<string, string> = {
  "nmesh.card/v1": "nmesh.card",
  "nmesh.table/v1": "nmesh.table",
  "nmesh.form/v1": "nmesh.form",
  "nmesh.location/v1": "nmesh.location",
};

const now = () => Math.floor(Date.now() / 1000);
let seq = 0;
const id = () => (++seq).toString(16).padStart(64, "0");

export function structured(contract: string, payload: any, alt: string): Body {
  const r = RENDERERS[contract];
  return {
    kind: "structured",
    contract,
    schema_hash: null,
    payload,
    alt,
    valid: true,
    error: null,
    renderer: r ? { tier: "installed", id: r } : { tier: "generic" },
    reply_to: null,
  };
}

const fakeNpub = (label: string) => `npub1${label}`.padEnd(63, "x");

function agentContact(pubkey: string, name: string, about: string, extra: Partial<Contact>): Contact {
  return {
    pubkey,
    npub: fakeNpub(name.toLowerCase()),
    name,
    about,
    picture: null,
    is_agent: true,
    agent: {
      id: name.toLowerCase(),
      name,
      description: about,
      hosting: "self_hosted",
      operator: "ACN demo",
      data_handling: { sees_plaintext: true, retention: "none (demo)", forwards_to: [] },
      tools: [],
    },
    commands: [],
    consented: false,
    last_message: null,
    last_at: null,
    unread: 0,
    acn: null,
    ...extra,
  };
}

export async function createPresenterBackend(): Promise<Backend> {
  await loadAcn();
  const listeners = new Set<(e: AppEvent) => void>();
  const emit = (e: AppEvent) => listeners.forEach((l) => l(structuredClone(e)));

  const world: AcnWorld = new AcnWorld((c) => emitCommitment(c));
  world.onWallet = () => emit({ type: "wallet", wallet: world.wallet() });
  const scenarios = world.scenarios();

  const acnFor = (actor: "performer" | "rogue") => {
    const tools = scenarios.filter((s) => s.actor === actor);
    return {
      profiles: [...new Set(tools.map((t) => t.profile))],
      min_price_sat: 0,
      deadline_secs: 600,
      review_window_secs: 3600,
      simulated: true,
      binding:
        actor === "performer"
          ? { bound: true, operator: fakeNpub("operator"), detail: "bound to an accountable operator key" }
          : { bound: false, operator: null, detail: "unaccountable: no operator binding" },
      tools: tools.map((t) => {
        const tpl = world.template(t.id);
        return {
          name: t.id,
          title: t.title,
          description: t.summary,
          profile: t.profile,
          shows: t.shows,
          price_sat: tpl.terms?.price?.amount ?? 0,
        };
      }),
    };
  };
  const commandsFor = (actor: "performer" | "rogue") =>
    scenarios
      .filter((s) => s.actor === actor)
      .map((s) => ({
        command: s.id,
        title: s.title,
        description: `[${s.profile}] ${s.summary}`,
        args: [{ name: "brief", type: "text", required: false, label: "What to work on", choices: [] }],
      }));

  const contacts = new Map<string, Contact>([
    [world.cast.performer, agentContact(world.cast.performer, "Atlas", "Simulated performer agent (ACN demo). Bound to an operator key.", {
      acn: acnFor("performer"), commands: commandsFor("performer"), last_at: now(), last_message: "ACN demo performer",
    })],
    [world.cast.rogue, agentContact(world.cast.rogue, "Rogue", "Simulated misbehaving agent (ACN demo). Unbound, sends conflicting acts.", {
      acn: acnFor("rogue"), commands: commandsFor("rogue"),
    })],
    [ECHO, agentContact(ECHO, "Echo agent", "Demo agent backed by the echo MCP server.", {
      commands: [
        { command: "card", title: "card", description: "Return a card with the given title and body", args: [
          { name: "title", type: "word", required: true, label: "title", choices: [] },
          { name: "body", type: "text", required: false, label: "body", choices: [] },
        ] },
        { command: "echo", title: "echo", description: "Echo the given text back", args: [
          { name: "text", type: "text", required: true, label: "Text to echo", choices: [] },
        ] },
        { command: "form", title: "form", description: "Ask for a card's title and body using a form", args: [] },
        { command: "status", title: "status", description: "Show a status table", args: [] },
      ],
    })],
    [ALICE, {
      pubkey: ALICE, npub: fakeNpub("alice"), name: "Alice", about: null, picture: null, is_agent: false, agent: null,
      commands: [], consented: true, last_message: "Shall we meet at the usual place?", last_at: now() - 3600, unread: 1, acn: null,
    }],
  ]);
  contacts.get(world.cast.performer)!.agent!.operator = "ACN demo operator";

  const history = new Map<string, Message[]>([
    [ALICE, [
      { id: id(), conversation: ALICE, sender: ALICE, mine: false, created_at: now() - 3700, body: { kind: "text", text: "Hey! Are you around this week?" } },
      { id: id(), conversation: ALICE, sender: ALICE, mine: false, created_at: now() - 3600, body: structured("nmesh.location/v1", { label: "The usual place", lat: 59.9139, lng: 10.7522, spatial: { anchor: { type: "geo", lat: 59.9139, lng: 10.7522 } } }, "Location: The usual place") },
      { id: id(), conversation: ALICE, sender: ALICE, mine: false, created_at: now() - 3590, body: { kind: "text", text: "Shall we meet at the usual place?" } },
    ]],
  ]);
  let active: string | null = null;

  function push(conv: string, m: Message) {
    const list = history.get(conv) ?? [];
    list.push(m);
    history.set(conv, list);
    const c = contacts.get(conv)!;
    c.last_message = summary(m.body);
    c.last_at = m.created_at;
    if (!m.mine && active !== conv) c.unread += 1;
    emit({ type: "contact", contact: c });
  }

  function summary(b: Body): string {
    switch (b.kind) {
      case "text": return b.text;
      case "structured": return b.alt;
      case "action": return `↳ ${b.alt}`;
      case "commitment": return "⚖ commitment";
      default: return b.alt ?? "(unsupported)";
    }
  }

  // --- ACN projections → UI view (adds conversation, rendered result, binding view) ---
  function view(p: any): Commitment {
    const b = p.binding ?? { status: "unbound" };
    return {
      ...p,
      conversation: p.performer,
      result: p.report
        ? p.report.contract
          ? structured(p.report.contract, p.report.payload, p.report.payload?.title ?? "Result")
          : { kind: "text", text: p.report.payload?.text ?? JSON.stringify(p.report.payload) }
        : null,
      performer_binding:
        b.status === "bound"
          ? { bound: true, operator: fakeNpub("operator"), detail: "bound to an accountable operator key" }
          : { bound: false, operator: null, detail: b.status === "unbound" ? "unaccountable: no operator binding" : `binding ${b.status}` },
    };
  }

  function commitment(cid: string): Commitment | null {
    const p = world.project().find((x) => x.id === cid);
    return p ? view(p) : null;
  }

  function emitCommitment(cid: string) {
    const c = commitment(cid);
    if (!c) return;
    const contact = contacts.get(c.conversation);
    const last = c.timeline[c.timeline.length - 1];
    if (contact && last && !last.mine) {
      contact.last_message = `⚖ ${last.act.toLowerCase()}`;
      contact.last_at = last.created_at;
      if (active !== c.conversation) contact.unread += 1;
      emit({ type: "contact", contact });
    }
    emit({ type: "commitment", commitment: c });
  }

  const requireConsent = (pubkey: string) => {
    const c = contacts.get(pubkey);
    if (c?.is_agent && !c.consented) throw "review and accept this agent's data handling first";
  };

  function echoReply(text: string, named?: Record<string, unknown>) {
    setTimeout(() => {
      const m: Message = { id: id(), conversation: ECHO, sender: ECHO, mine: false, created_at: now(), body: echoBody(text, named) };
      push(ECHO, m);
      emit({ type: "message", message: m });
    }, 450);
  }

  function actorChat(pubkey: string) {
    const name = contacts.get(pubkey)?.name ?? "Agent";
    setTimeout(() => {
      const lines = [`${name} is an ACN demo agent. Turn on "Request commands as commitments" (or use the 🎬 Director) and try:`];
      for (const s of scenarios.filter((x) => world.cast[x.actor as "performer" | "rogue"] === pubkey)) lines.push(`/${s.id} — ${s.title} (${s.profile})`);
      const m: Message = { id: id(), conversation: pubkey, sender: pubkey, mine: false, created_at: now(), body: { kind: "text", text: lines.join("\n") } };
      push(pubkey, m);
      emit({ type: "message", message: m });
    }, 400);
  }

  const backend: Backend = {
    kind: "presenter",
    identity: async () => ({ pubkey: world.me, npub: fakeNpub("you"), name: ME_LABEL, relays: ["(presenter: in-browser)"] }),
    contacts: async () => [...contacts.values()].sort((a, b) => (b.last_at ?? 0) - (a.last_at ?? 0)),
    openConversation: async (pubkey) => {
      active = pubkey;
      const c = contacts.get(pubkey);
      if (c) c.unread = 0;
      return structuredClone(history.get(pubkey) ?? []);
    },
    addContact: async (input) => {
      const q = input.trim().toLowerCase();
      const found = [...contacts.values()].find((c) => c.npub === q || c.pubkey === q || c.name?.toLowerCase() === q);
      if (found) return found;
      throw "presenter mode: try Atlas, Rogue, Echo agent or Alice";
    },
    acceptAgent: async (pubkey) => {
      const c = contacts.get(pubkey)!;
      c.consented = true;
      return c;
    },
    sendText: async (pubkey, text) => {
      requireConsent(pubkey);
      const m: Message = { id: id(), conversation: pubkey, sender: world.me, mine: true, created_at: now(), body: { kind: "text", text } };
      push(pubkey, m);
      if (pubkey === ECHO && text.startsWith("/")) echoReply(text);
      else if (contacts.get(pubkey)?.acn) actorChat(pubkey);
      return m;
    },
    sendAction: async (pubkey, target, actionId, values, label) => {
      requireConsent(pubkey);
      const payload: any = {};
      if (actionId) payload.action_id = actionId;
      if (values) payload.values = values;
      const m: Message = { id: id(), conversation: pubkey, sender: world.me, mine: true, created_at: now(),
        body: { kind: "action", target, contract: "nmesh.form.submit/v1", payload, alt: label ?? actionId ?? "Submitted form" } };
      push(pubkey, m);
      if (pubkey === ECHO && actionId?.startsWith("/")) echoReply(actionId, values ?? undefined);
      return m;
    },
    onEvent: async (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    openUrl: async (url) => {
      window.open(url, "_blank", "noopener");
    },

    commitments: async (pubkey) => world.project().filter((p) => p.performer === pubkey).map(view),
    acnRequest: async (pubkey, text) => {
      requireConsent(pubkey);
      if (!contacts.get(pubkey)?.acn) throw "this contact doesn't offer ACN commitments";
      const [cmd] = text.slice(1).split(/\s+/);
      const brief = text.slice(1 + cmd.length).trim() || "the demo";
      if (!scenarios.some((s) => s.id === cmd)) throw `unknown command /${cmd}`;
      const cid = world.request(pubkey, cmd, brief);
      const c = commitment(cid)!;
      push(pubkey, { id: cid, conversation: pubkey, sender: world.me, mine: true, created_at: c.created_at, body: { kind: "commitment", commitment: cid } });
      return c;
    },
    acnSatisfy: async (_p, c) => (world.act(c, { type: "satisfy" }), commitment(c)!),
    acnReject: async (_p, c, condition, note) => {
      const cond = condition ?? commitment(c)?.conditions[0]?.id ?? "result";
      world.act(c, { type: "reject", condition: cond, note: note || "Not what I asked for" });
      return commitment(c)!;
    },
    acnWithdraw: async (_p, c) => (world.act(c, { type: "withdraw" }), commitment(c)!),
    acnAcceptCounter: async (_p, c) => (world.act(c, { type: "accept_counter" }), commitment(c)!),
    acnFund: async () => {
      throw "contingent settlement is live-mode only for now";
    },
    acnDispute: async (_p, c, reason) => (world.act(c, { type: "dispute", reason: reason || "I dispute this outcome" }), commitment(c)!),
    acnHint: async (_p, c) => {
      const h = world.hint(c);
      if (h && world.narration[c] && h.action.type === "wait") h.narration = world.narration[c];
      return h;
    },
    acnRunHint: async (_p, c) => {
      const h = world.hint(c);
      if (!h) return null;
      if (h.action.type === "advance_clock") world.setOffset(world.offset + h.action.secs);
      else if (h.action.type !== "wait" && h.action.type !== "done") world.act(c, h.action);
      return commitment(c);
    },
    discoverDemo: async () => [world.cast.performer, world.cast.rogue].map((pk) => contacts.get(pk)!),
    clock: async () => ({ now: world.now(), offset: world.offset }),
    setClockOffset: async (offset) => {
      world.setOffset(offset);
      return { now: world.now(), offset: world.offset };
    },
    wallet: async () => world.wallet(),
    walletTopUp: async (amount) => (world.topUp(amount), world.wallet()),
    walletRefund: async (_p, c) => (world.refund(c), world.wallet()),
    castWallets: async () => [{ name: "Atlas", wallet: world.wallet(world.cast.performer) }],
    board: async (closedLimit) => {
      const all = world.project().map(view).sort((a, b) => b.updated_at - a.updated_at);
      const closed = all.filter((c) => c.lane === "closed");
      return { items: [...all.filter((c) => c.lane !== "closed"), ...closed.slice(0, closedLimit)], closed_total: closed.length, now: world.now() };
    },
    resetDemo: async () => {
      world.reset();
      for (const pk of [world.cast.performer, world.cast.rogue]) {
        history.set(pk, (history.get(pk) ?? []).filter((m) => m.body.kind !== "commitment"));
        const c = contacts.get(pk)!;
        c.last_message = pk === world.cast.performer ? "ACN demo performer" : null;
        c.unread = 0;
        emit({ type: "contact", contact: c });
      }
      emit({ type: "wallet", wallet: world.wallet() });
    },
  };
  return backend;
}

function echoBody(text: string, named?: Record<string, unknown>): Body {
  const [cmd, ...rest] = text.slice(1).split(/\s+/);
  const restText = text.slice(1 + cmd.length).trim();
  switch (cmd) {
    case "help":
      return { kind: "text", text: "Echo agent — operated by me (SelfHosted). I can read what you send me.\n/card <title> [body]\n/echo <text>\n/form\n/status" };
    case "echo":
      return { kind: "text", text: (named?.text as string) ?? (restText || "(nothing)") };
    case "card": {
      const title = (named?.title as string) ?? rest[0] ?? "Untitled";
      const body = (named?.body as string) ?? rest.slice(1).join(" ");
      return structured("nmesh.card/v1", { title, ...(body ? { body } : {}), actions: [{ id: "/echo You pressed the button", label: "Press me", style: "primary" }] }, body ? `${title}: ${body}` : title);
    }
    case "form":
      return structured("nmesh.form/v1", {
        title: "Make a card", instructions: "I'll turn this into a card.",
        fields: [
          { name: "title", label: "Title", type: "text", required: true },
          { name: "body", label: "Body", type: "multiline" },
        ],
        submit_label: "Create card", submit_action: "/card",
      }, "Make a card");
    case "status":
      return structured("nmesh.table/v1", {
        title: "Services",
        columns: [{ key: "name", label: "Service" }, { key: "state", label: "State" }, { key: "latency", label: "p50 ms", type: "number" }],
        rows: [
          { name: "relay", state: "healthy", latency: 12 },
          { name: "bridge", state: "healthy", latency: 4 },
          { name: "blossom", state: "degraded", latency: 230 },
        ],
      }, "Services");
    default:
      return { kind: "text", text: `Unknown command /${cmd}. Try /help.` };
  }
}
