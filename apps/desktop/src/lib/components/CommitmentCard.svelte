<script lang="ts">
  import type { Commitment, Entry, RendererHost } from "../types";
  import { store } from "../store.svelte";
  import Structured from "../renderers/Structured.svelte";

  let { commitment: c, host }: { commitment: Commitment; host: RendererHost } = $props();

  // Wallet facts (not protocol state): what happened to the token I locked for this commitment.
  const lock = $derived(store.lockFor(c.id));
  const performerName = $derived(store.contacts.find((x) => x.pubkey === c.performer)?.name ?? "the performer");
  const nowSecs = $derived(store.backend?.kind === "presenter" ? store.clock.now : Math.floor(Date.now() / 1000));

  let mode = $state<"idle" | "reject" | "dispute">("idle");
  let note = $state("");
  let condition = $state<string | null>(null);
  let busy = $state(false);
  let showTimeline = $state(false);

  const PHASE: Record<string, string> = {
    requested: "Requested", countered: "Counter-offer", promised: "Promised", reported: "Reported",
    reworking: "Reworking", satisfied: "Satisfied", rejected: "Rejected", disputed: "Disputed",
    declined: "Declined", withdrawn: "Withdrawn", revoked: "Revoked",
  };
  const ROLE: Record<string, string> = {
    requester: "you", performer: "performer", arbiter: "arbiter", verifier: "verifier", governance: "gate", other: "other",
  };

  const tone = $derived(
    c.phase === "satisfied" ? "ok"
    : c.phase === "rejected" || c.phase === "declined" || c.phase === "revoked" ? (c.terminal ? "bad" : "you")
    : c.phase === "withdrawn" ? "muted"
    : c.lapsed ? "bad"
    : c.awaiting_me ? "you"
    : "wait",
  );
  const status = $derived.by(() => {
    if (!c.phase) return "Waiting for terms";
    if (c.lapsed) return c.lapsed === "requester" ? "You missed the review window" : "Lapsed: deadline missed";
    if (c.terminal) return PHASE[c.phase];
    if (c.awaiting_me) return c.phase === "countered" ? "Review counter-offer" : c.phase === "rejected" ? "Disputable" : "Your review";
    if (c.awaiting === "arbiter") return "With the arbiter";
    if (c.phase === "rejected") return "Rejected · disputable";
    return c.phase === "countered" ? "Counter sent" : `${PHASE[c.phase]} · agent working`;
  });
  const brief = $derived(c.input?.brief ?? (c.input && Object.values(c.input).find((v) => typeof v === "string")) ?? "");
  const pending = $derived(c.timeline.filter((e) => e.status === "pending"));
  const rejectedActs = $derived(c.timeline.filter((e) => e.status === "rejected"));
  // Flags on applied acts; notes on unapplied acts are listed with those acts.
  const flags = $derived(
    c.anomalies.filter((a) => a.severity === "flag" && !rejectedActs.some((e) => e.id === a.event)),
  );
  const rejectable = $derived(c.conditions.length ? c.conditions : [{ id: "result", class: "human", description: "The result", status: null, evidence: [] }]);

  function when(ts: number) {
    return new Date(ts * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }
  function date(ts: number) {
    return new Date(ts * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  }
  function countdown(s: number | null) {
    if (s === null) return "";
    if (s < 0) return "overdue";
    if (s < 120) return `${s}s left`;
    if (s < 7200) return `${Math.round(s / 60)} min left`;
    return `${Math.round(s / 3600)} h left`;
  }
  function dotClass(e: Entry) {
    return e.role === "requester" ? "you" : e.role === "performer" ? "perf" : "third";
  }

  async function act(kind: "satisfy" | "reject" | "withdraw" | "accept_counter" | "dispute") {
    busy = true;
    await store.commitmentAct(kind, c.id, note, condition, c.conversation);
    busy = false;
    mode = "idle";
    note = "";
  }

  function startReject() {
    condition = rejectable[0]?.id ?? null;
    mode = "reject";
  }
</script>

<div class="commitment" data-tone={tone} class:focused={store.focus === c.id}>
  <header>
    <span class="seal" aria-hidden="true">⚖</span>
    <div class="what">
      <div class="title">
        <span class="mono cmd">/{c.tool ?? "?"}</span>
        {#if brief}<span class="args">{brief}</span>{/if}
      </div>
      <div class="meta">
        {c.description} · {c.profile} · {c.price_sat ? `${c.price_sat} sat` : "free"}
        ·
        {#if c.performer_binding.bound}
          <span class="bound" title={c.performer_binding.operator ?? ""}>operator-bound ✓</span>
        {:else}
          <span class="unbound" title={c.performer_binding.detail}>unaccountable ⚠</span>
        {/if}
      </div>
    </div>
    <span class="status" data-tone={tone}>{status}</span>
  </header>

  <ol class="chain" aria-label="Applied acts">
    {#each c.chain as e, i}
      <li title={`${e.act} by ${ROLE[e.role]} · ${when(e.created_at)}\n${e.summary}`}>
        {#if i > 0}<span class="line"></span>{/if}
        <span class="dot {dotClass(e)}"></span>
        <span class="act">{e.act.toLowerCase()}</span>
      </li>
    {/each}
    {#each pending as e}
      <li class="pend" title={e.notes[0]?.detail ?? "pending"}>
        <span class="line"></span><span class="dot pending"></span>
        <span class="act">{e.act.toLowerCase()} · pending</span>
      </li>
    {/each}
    {#if !c.terminal && c.deadline_in !== null && !pending.length}
      <li class="pend"><span class="line"></span><span class="dot pending"></span><span class="act">{countdown(c.deadline_in)}</span></li>
    {/if}
  </ol>

  {#if c.counter}
    <div class="panel counter">
      <div class="ph">↔ Counter-offer from {c.counter.mine ? "you" : ROLE[c.counter.by]}</div>
      <p class="reason">"{c.counter.reason}"</p>
      <table>
        <tbody>
          {#if c.counter.deadline !== c.counter.current_deadline}
            <tr><td>Deadline</td><td class="old">{date(c.counter.current_deadline)}</td><td>→</td><td class="new">{date(c.counter.deadline)}</td></tr>
          {/if}
          {#if c.counter.price_sat !== c.counter.current_price_sat}
            <tr><td>Price</td><td class="old">{c.counter.current_price_sat} sat</td><td>→</td><td class="new">{c.counter.price_sat} sat</td></tr>
          {/if}
          {#if c.counter.rework_limit !== c.counter.current_rework_limit}
            <tr><td>Revisions</td><td class="old">{c.counter.current_rework_limit}</td><td>→</td><td class="new">{c.counter.rework_limit}</td></tr>
          {/if}
        </tbody>
      </table>
      <div class="hash mono" title="Accepting binds you both to this hash (INV-4)">terms {c.counter.terms_hash.slice(0, 16)}…</div>
    </div>
  {/if}

  {#if c.result}
    <div class="result">
      {#if c.report && (c.report.version > 1 || c.report.note)}
        <div class="rh">
          {#if c.report.version > 1}<span class="pill">v{c.report.version}</span>{/if}
          {#if c.report.note}<span class="muted">{c.report.note}</span>{/if}
        </div>
      {/if}
      {#if c.result.kind === "text"}
        <p class="text">{c.result.text}</p>
      {:else if c.result.kind === "structured"}
        <Structured body={c.result} {host} mine={false} />
      {/if}
    </div>
  {/if}

  {#if c.conditions.length && c.report}
    <div class="panel conditions">
      <div class="ph">Conditions of satisfaction</div>
      {#each c.conditions as cond}
        {@const s = cond.status?.status}
        <div class="cond">
          <span class="badge" data-s={s}>
            {s === "pass" ? "✓ pass" : s === "fail" ? "✗ fail" : s === "advisory" ? "advisory" : s === "human" ? "your call" : "unverified"}
          </span>
          <span class="cid mono">{cond.id}</span>
          <span class="class">{cond.class}</span>
          <span class="desc muted">{cond.description}</span>
        </div>
        {#each cond.evidence as ev}
          <div class="ev" class:not-counted={!ev.counted}>
            <span class="who">{ROLE[ev.role]}</span>
            <span class="mono">{ev.act.toLowerCase()}</span>
            <span>“{ev.verdict}”</span>
            {#if ev.counted}<span class="counted">counted</span>{:else}<span class="nc" title={ev.why ?? ""}>not counted · {ev.why}</span>{/if}
          </div>
        {/each}
      {/each}
    </div>
  {/if}

  {#if c.settlement}
    <div class="settle" data-s={c.settlement.status}>
      <span class="coin">₿</span>
      <span class="amt">{c.settlement.amount_sat} sat</span>
      <span class="st">{c.settlement.status === "locked" ? "🔒 locked" : c.settlement.status === "paid" ? "✓ paid" : "↩ refundable"}</span>
      <span class="muted d">{c.settlement.detail}</span>
      {#if c.settlement.simulated}<span class="pill sim" title={c.settlement.mint}>simulated mint</span>
      {:else}<span class="pill test" title={c.settlement.mint}>test sats</span>{/if}
      {#if lock}
        <span class="token" data-t={lock.state}>
          your token: {lock.state === "locked" ? "locked at the mint" : lock.state === "redeemed" ? `redeemed by ${performerName}` : "refunded to you"}
        </span>
        {#if lock.state === "locked" && c.settlement.status === "refundable"}
          <button class="btn refund" disabled={store.walletBusy || nowSecs <= lock.locktime}
            title={nowSecs <= lock.locktime ? `Refund path opens ${new Date(lock.locktime * 1000).toLocaleString()}` : "Reclaim with your refund key (NUT-14)"}
            onclick={() => store.refund(c.conversation, c.id)}>
            {nowSecs <= lock.locktime ? "Refund after locktime" : "↩ Refund"}
          </button>
        {/if}
      {/if}
    </div>
  {/if}

  {#if flags.length || rejectedActs.length}
    <div class="panel notes">
      {#each flags as a}
        <div class="note" data-sev="flag"><span class="mono code">{a.code}</span> <span class="muted">on {a.act.toLowerCase()}</span> — {a.detail}</div>
      {/each}
      {#each rejectedActs as e}
        {#each e.notes as n}
          <div class="note" data-sev={n.severity}><span class="mono code">{n.code}</span> <span class="muted">{e.act.toLowerCase()} by {ROLE[e.role]} not applied</span> — {n.detail}</div>
        {/each}
      {/each}
    </div>
  {/if}

  {#if mode === "reject"}
    <form class="form" onsubmit={(e) => { e.preventDefault(); act("reject"); }}>
      {#if rejectable.length > 1}
        <select class="field cond-select" bind:value={condition}>
          {#each rejectable as r}<option value={r.id}>{r.id}</option>{/each}
        </select>
      {/if}
      <input class="field" placeholder="What's wrong?" bind:value={note} disabled={busy} />
      <button class="btn danger" type="submit" disabled={busy}>Reject</button>
      <button class="btn" type="button" onclick={() => (mode = "idle")} disabled={busy}>Cancel</button>
    </form>
  {:else if mode === "dispute"}
    <form class="form" onsubmit={(e) => { e.preventDefault(); act("dispute"); }}>
      <input class="field" placeholder="Why should the arbiter look at this?" bind:value={note} disabled={busy} />
      <button class="btn" type="submit" disabled={busy}>Dispute</button>
      <button class="btn" type="button" onclick={() => (mode = "idle")} disabled={busy}>Cancel</button>
    </form>
  {:else if Object.values(c.allowed).some(Boolean)}
    <div class="actions">
      {#if c.allowed.accept_counter}<button class="btn primary" onclick={() => act("accept_counter")} disabled={busy}>Accept counter-offer</button>{/if}
      {#if c.allowed.satisfy}
        <button class="btn primary" onclick={() => act("satisfy")} disabled={busy}>{c.settlement ? "Satisfy & pay" : "Accept result"}</button>
      {/if}
      {#if c.allowed.reject}<button class="btn" onclick={startReject} disabled={busy}>Reject…</button>{/if}
      {#if c.allowed.dispute}<button class="btn" onclick={() => (mode = "dispute")} disabled={busy}>Dispute…</button>{/if}
      {#if c.allowed.withdraw}<button class="btn" onclick={() => act("withdraw")} disabled={busy}>Withdraw</button>{/if}
      <span class="hint muted">Signed acts · only you can close this</span>
    </div>
  {/if}

  <button class="tl-toggle" onclick={() => (showTimeline = !showTimeline)}>
    {showTimeline ? "Hide" : "Show"} signed events ({c.timeline.length}) {showTimeline ? "▴" : "▾"}
  </button>
  {#if showTimeline}
    <ol class="timeline">
      {#each c.timeline as e}
        <li data-status={e.status}>
          <span class="t mono">{when(e.created_at)}</span>
          <span class="dot {dotClass(e)}"></span>
          <span class="act mono">{e.act}</span>
          <span class="who">{ROLE[e.role]}</span>
          <span class="sum">{e.summary}</span>
          {#if e.status !== "applied"}<span class="st">{e.status}</span>{/if}
          {#each e.notes as n}<span class="mono code" data-sev={n.severity} title={n.detail}>{n.code}</span>{/each}
          <span class="id mono" title={e.id}>{e.id.slice(0, 8)}</span>
        </li>
      {/each}
    </ol>
  {/if}
</div>

<style>
  .commitment {
    --tone: var(--accent);
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 380px;
  }
  .commitment.focused {
    outline: 2px solid color-mix(in srgb, var(--agent) 40%, transparent);
    outline-offset: 6px;
    border-radius: 6px;
  }
  [data-tone="ok"] { --tone: var(--ok); }
  [data-tone="bad"] { --tone: var(--danger); }
  [data-tone="muted"] { --tone: var(--faint); }
  [data-tone="you"] { --tone: var(--agent); }
  header { display: flex; gap: 10px; align-items: flex-start; }
  .seal {
    width: 30px; height: 30px; flex-shrink: 0; display: grid; place-items: center;
    border-radius: 9px; background: var(--sunken); color: var(--tone); font-size: 16px;
  }
  .what { flex: 1; min-width: 0; }
  .title { display: flex; gap: 6px; align-items: baseline; flex-wrap: wrap; }
  .cmd { font-weight: 650; color: var(--accent); }
  .args { font-size: 13px; color: var(--muted); }
  .meta { font-size: 11.5px; color: var(--faint); margin-top: 1px; }
  .bound { color: var(--ok); }
  .unbound { color: var(--warn); }
  .status {
    font-size: 11px; font-weight: 650; padding: 2px 8px; border-radius: 999px; color: var(--tone);
    border: 1px solid color-mix(in srgb, var(--tone) 35%, transparent);
    background: color-mix(in srgb, var(--tone) 9%, transparent); white-space: nowrap;
  }
  .chain { display: flex; flex-wrap: wrap; align-items: center; row-gap: 6px; list-style: none; margin: 0; padding: 0 2px; font-size: 11px; color: var(--muted); }
  .chain li { display: flex; align-items: center; gap: 5px; }
  .line { width: 14px; height: 1.5px; background: var(--border); margin: 0 4px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--faint); flex-shrink: 0; }
  .dot.you { background: var(--accent); }
  .dot.perf { background: var(--agent); }
  .dot.third { background: #8b5cf6; }
  .dot.pending { background: transparent; border: 1.5px dashed var(--faint); }
  .pend .act { font-style: italic; }
  .panel {
    border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 9px 11px;
    background: var(--sunken); display: flex; flex-direction: column; gap: 6px; font-size: 12.5px;
  }
  .ph { font-weight: 650; font-size: 12px; color: var(--muted); }
  .counter { border-color: var(--agent-border); background: var(--agent-soft); }
  .counter .ph { color: var(--agent); }
  .reason { margin: 0; font-style: italic; }
  .counter table { border-collapse: collapse; font-size: 12.5px; }
  .counter td { padding: 1px 8px 1px 0; }
  .old { color: var(--faint); text-decoration: line-through; }
  .new { font-weight: 650; }
  .hash { font-size: 10.5px; color: var(--faint); }
  .result { border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 10px 12px; background: var(--panel); display: flex; flex-direction: column; gap: 6px; }
  .rh { display: flex; gap: 8px; align-items: center; font-size: 12px; }
  .text { margin: 0; white-space: pre-wrap; }
  .cond { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
  .badge { font-size: 10.5px; font-weight: 700; padding: 1px 7px; border-radius: 999px; background: var(--panel); border: 1px solid var(--border); color: var(--muted); }
  .badge[data-s="pass"] { color: var(--ok); border-color: color-mix(in srgb, var(--ok) 40%, transparent); }
  .badge[data-s="fail"] { color: var(--danger); }
  .badge[data-s="human"] { color: var(--agent); }
  .cid { font-weight: 600; }
  .class { font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--faint); }
  .desc { font-size: 12px; }
  .ev { display: flex; gap: 6px; align-items: baseline; padding-left: 18px; font-size: 12px; flex-wrap: wrap; }
  .ev .who { color: #8b5cf6; font-weight: 600; }
  .counted { color: var(--ok); font-size: 11px; }
  .nc { color: var(--warn); font-size: 11px; }
  .not-counted { opacity: 0.85; }
  .settle {
    display: flex; gap: 8px; align-items: center; flex-wrap: wrap; font-size: 12.5px;
    border: 1px dashed var(--border); border-radius: var(--radius-sm); padding: 7px 10px;
  }
  .settle[data-s="paid"] { border-color: color-mix(in srgb, var(--ok) 50%, transparent); background: color-mix(in srgb, var(--ok) 6%, transparent); }
  .settle[data-s="locked"] { border-color: var(--agent-border); }
  .pill.test { background: #dcfce7; color: #166534; }
  .token { font-size: 11.5px; padding: 1px 7px; border-radius: 999px; background: var(--sunken); }
  .token[data-t="redeemed"] { color: var(--ok); }
  .token[data-t="refunded"] { color: var(--agent); }
  .settle .refund { padding: 3px 9px; font-size: 12px; }
  .coin { color: #f7931a; font-weight: 700; }
  .amt { font-weight: 700; }
  .st { font-weight: 600; }
  .d { font-size: 11.5px; flex: 1; min-width: 160px; }
  .sim { font-size: 10px; }
  .notes { background: color-mix(in srgb, var(--warn) 6%, var(--sunken)); border-color: color-mix(in srgb, var(--warn) 25%, var(--border)); }
  .note { font-size: 12px; }
  .code { font-size: 11px; font-weight: 650; }
  [data-sev="reject"] .code, .code[data-sev="reject"] { color: var(--danger); }
  [data-sev="flag"] .code, .code[data-sev="flag"] { color: var(--warn); }
  [data-sev="pending"] .code, .code[data-sev="pending"] { color: var(--faint); }
  .actions, .form { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  .form .field { flex: 1; min-width: 160px; }
  .cond-select { flex: 0 0 auto !important; min-width: 110px !important; }
  .hint { font-size: 11px; margin-left: auto; }
  .tl-toggle { align-self: flex-start; border: none; background: none; padding: 0; font-size: 11px; color: var(--faint); }
  .tl-toggle:hover { color: var(--accent); }
  .timeline { list-style: none; margin: 0; padding: 8px 10px; background: var(--sunken); border-radius: var(--radius-sm); display: flex; flex-direction: column; gap: 4px; font-size: 11.5px; }
  .timeline li { display: flex; gap: 6px; align-items: baseline; flex-wrap: wrap; }
  .timeline li[data-status="rejected"] { opacity: 0.75; }
  .timeline li[data-status="rejected"] .act { text-decoration: line-through; }
  .timeline .t { color: var(--faint); font-size: 10.5px; }
  .timeline .act { font-weight: 650; font-size: 11px; }
  .timeline .who { color: var(--muted); }
  .timeline .sum { flex: 1; min-width: 120px; }
  .timeline .st { font-size: 10px; text-transform: uppercase; color: var(--warn); }
  .timeline .id { color: var(--faint); font-size: 10px; }
</style>
