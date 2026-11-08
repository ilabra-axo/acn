<script lang="ts">
  import { store } from "../store.svelte";
  import type { Commitment, Role } from "../types";
  import { displayName } from "../format";
  import Avatar from "./Avatar.svelte";

  let { c }: { c: Commitment } = $props();

  const ROLE_NAME: Record<Role, string> = {
    requester: "requester", performer: "performer", arbiter: "arbiter",
    verifier: "verifier", governance: "governance", other: "observer",
  };

  const counterparty = $derived(c.my_role === "requester" ? c.performer : c.requester);
  const contact = $derived(store.contacts.find((x) => x.pubkey === counterparty));
  const name = $derived(contact ? displayName(contact) : `${counterparty.slice(0, 8)}…`);
  const bound = $derived(c.performer_binding.bound);
  const lock = $derived(store.lockFor(c.id));

  function who(r: Role | null): string {
    if (!r) return "";
    if (r === c.my_role) return "you";
    if (r === (c.my_role === "requester" ? "performer" : "requester")) return name;
    const arb = r === "arbiter" && c.arbiter ? store.contacts.find((x) => x.pubkey === c.arbiter) : null;
    if (arb) return displayName(arb);
    return r === "governance" ? "the governance gate" : `the ${ROLE_NAME[r]}`;
  }

  /** One line: whose move, and what that move is. */
  const status = $derived.by(() => {
    if (c.lane === "closed") {
      const p = c.phase ?? "closed";
      const ruled = c.chain[c.chain.length - 1]?.act === "RULING";
      return { cls: p === "satisfied" || p === "settled" ? "ok" : "end", text: ruled ? `Ruled · ${p}` : p[0].toUpperCase() + p.slice(1) };
    }
    if (c.needs_me) {
      const what = c.lane === "review" ? "satisfy or reject" : c.lane === "negotiating" ? "accept or counter" : c.lane === "dispute" ? "respond" : "act";
      return { cls: "me", text: `Your move · ${what}` };
    }
    if (c.pending_endorsement) return { cls: "wait", text: "Reported · waiting for a governance sign-off" };
    const n = who(c.next);
    const doing: Record<string, string> = {
      requested: "to respond", negotiating: "to answer the counter", in_progress: c.phase === "reworking" ? "to rework" : "to deliver",
      review: "to review", dispute: c.phase === "disputed" ? "to rule" : "to dispute or accept",
    };
    return { cls: "wait", text: `Waiting for ${n} ${doing[c.lane] ?? ""}`.trim() };
  });

  function dur(s: number) {
    const a = Math.abs(s);
    if (a < 90) return `${Math.round(a)} s`;
    if (a < 5400) return `${Math.round(a / 60)} min`;
    if (a < 172800) return `${Math.round(a / 3600)} h`;
    return `${Math.round(a / 86400)} d`;
  }

  const timer = $derived.by(() => {
    if (c.lane === "closed" || c.deadline_in === null) return null;
    const label = c.lane === "review" || c.lane === "dispute" ? "review window" : "deadline";
    if (c.lapsed) return { late: true, text: `${label} passed ${dur(c.deadline_in)} ago · lapsed` };
    if (c.deadline_in < 0) return { late: true, text: `${label} passed ${dur(c.deadline_in)} ago` };
    return { late: false, text: `${dur(c.deadline_in)} left` };
  });

  const money = $derived.by(() => {
    if (!c.settlement) return null;
    const n = c.settlement.amount_sat;
    if (lock?.state === "redeemed") return { cls: "paid", text: `✓ ${n} sat paid` };
    if (lock?.state === "refunded") return { cls: "back", text: `↩ ${n} sat refunded` };
    return {
      locked: { cls: "locked", text: `🔒 ${n} sat` },
      paid: { cls: "paid", text: `✓ ${n} sat` },
      refundable: { cls: "back", text: `↩ ${n} sat refundable` },
    }[c.settlement.status];
  });

  // The most telling validator note: root causes (e.g. equivocation) over their effects
  // (stale), and nothing the tile already shows (unbound → the binding badge).
  const flag = $derived.by(() => {
    const notes = c.anomalies.filter((a) => a.severity !== "pending" && a.code !== "unbound");
    return notes.find((a) => a.code !== "stale") ?? notes[0];
  });

  function dot(st: Commitment["conditions"][number]["status"]) {
    return st?.status ?? "unverified";
  }

  const extra = $derived.by(() => {
    if (c.lane === "negotiating" && c.counter) {
      const parts = [];
      if (c.counter.deadline !== c.counter.current_deadline) parts.push(`deadline ${c.counter.deadline > c.counter.current_deadline ? "+" : "−"}${dur(c.counter.deadline - c.counter.current_deadline)}`);
      if (c.counter.price_sat !== c.counter.current_price_sat) parts.push(`${c.counter.price_sat} sat`);
      if (c.counter.rework_limit !== c.counter.current_rework_limit) parts.push(`${c.counter.rework_limit} revision${c.counter.rework_limit === 1 ? "" : "s"}`);
      return `Counter: ${parts.join(", ") || "new terms"}`;
    }
    if (c.report && c.report.version > 1 && c.lane !== "closed") return `Report v${c.report.version}`;
    if (c.phase === "reworking") return `Rework ${c.rework_count}/${c.rework_limit}`;
    return null;
  });
</script>

<button class="tile" class:me={c.needs_me} class:active={store.boardOpen === c.id} onclick={() => (store.boardOpen = c.id)}>
  <div class="top">
    <span class="tool mono">/{c.tool ?? "request"}</span>
    <span class="pill">{c.profile}</span>
    {#if c.assurance}<span class="assurance" data-a={c.assurance} title="Overall assurance (weakest condition), computed by the validator">{c.assurance}</span>{/if}
    {#if money}<span class="money {money.cls}">{money.text}</span>{/if}
  </div>

  {#if c.input?.brief || c.description}
    <div class="brief">{c.input?.brief || c.description}</div>
  {/if}

  <div class="who">
    <Avatar pubkey={counterparty} name={name} size={18} />
    <span class="name">{name}</span>
    {#if c.my_role === "requester"}
      <span class="bind" class:ok={bound} title={c.performer_binding.detail}>{bound ? "bound ✓" : "unbound ⚠"}</span>
    {:else}
      <span class="role">you: {ROLE_NAME[c.my_role]}</span>
    {/if}
  </div>

  <div class="status {status.cls}">{status.text}</div>

  {#if timer || extra}
    <div class="meta">
      {#if timer}<span class:late={timer.late}>⏱ {timer.text}</span>{/if}
      {#if extra}<span>{extra}</span>{/if}
    </div>
  {/if}

  {#if c.conditions.length || flag}
    <div class="foot">
      {#each c.conditions as cond}
        <span class="dot" data-s={dot(cond.status)} title="{cond.id} ({cond.class}): {dot(cond.status)}"></span>
      {/each}
      {#if flag}<span class="flag" title={flag.detail}>⚠ {flag.code}</span>{/if}
    </div>
  {/if}
</button>

<style>
  .tile {
    width: 100%; text-align: left; display: flex; flex-direction: column; gap: 6px;
    background: var(--panel); border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: 10px 11px; box-shadow: var(--shadow); font-size: 12.5px; color: var(--text);
    transition: border-color 0.12s, transform 0.12s;
  }
  .tile:hover { border-color: var(--accent); }
  .tile.active { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-soft); }
  .tile.me { border-left: 3px solid var(--accent); }
  .top { display: flex; align-items: center; gap: 6px; }
  .tool { font-weight: 700; font-size: 13px; }
  .assurance { font-size: 10px; font-weight: 650; padding: 0 6px; border-radius: 999px; background: var(--sunken); color: var(--muted); }
  .assurance[data-a="opinion"] { background: #f3e8ff; color: #6b21a8; }
  .assurance[data-a="check"] { background: #dcfce7; color: #166534; }
  .assurance[data-a="grounded"] { background: #dbeafe; color: #1e40af; }
  .money { margin-left: auto; font-size: 11.5px; font-weight: 650; white-space: nowrap; }
  .money.locked { color: #c2410c; }
  .money.paid { color: var(--ok); }
  .money.back { color: var(--agent); }
  .brief { color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .who { display: flex; align-items: center; gap: 6px; }
  .name { font-weight: 600; }
  .bind { font-size: 10.5px; color: var(--warn); }
  .bind.ok { color: var(--ok); }
  .role { font-size: 10.5px; padding: 0 6px; border-radius: 999px; background: var(--agent-soft); color: var(--agent); }
  .status { font-weight: 600; }
  .status.me { color: var(--accent); }
  .status.wait { color: var(--muted); font-weight: 500; }
  .status.ok { color: var(--ok); }
  .status.end { color: var(--muted); }
  .meta { display: flex; flex-wrap: wrap; gap: 4px 10px; font-size: 11.5px; color: var(--faint); }
  .late { color: var(--danger); font-weight: 600; }
  .foot { display: flex; align-items: center; gap: 4px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--border); }
  .dot[data-s="pass"] { background: var(--ok); }
  .dot[data-s="fail"] { background: var(--danger); }
  .dot[data-s="human"] { background: transparent; border: 1.5px solid var(--faint); width: 6px; height: 6px; }
  .dot[data-s="advisory"] { background: #8b5cf6; }
  .flag { margin-left: auto; font-size: 10.5px; color: var(--danger); font-family: var(--mono); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
</style>
