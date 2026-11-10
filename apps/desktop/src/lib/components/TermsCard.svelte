<script lang="ts">
  // The terms card (spec §12, CR-003 §2.7): shown before PROMISE, ACCEPT or SATISFY. Done means ·
  // Checked by · Pay if · Disqualified if, with who drafted the version and the diff against the
  // previous one. Pure display of acn-view's projection (INV-12).
  import type { DiffLine, TermsCard } from "../types";
  import { store } from "../store.svelte";

  let {
    card,
    diff = [],
    title,
    performer,
  }: { card: TermsCard; diff?: DiffLine[]; title: string; performer: string } = $props();

  let open = $state(false);
  const name = (pk: string) => store.contacts.find((x) => x.pubkey === pk)?.name ?? `${pk.slice(0, 8)}…`;
  const drafter = $derived(card.drafted_by_me ? "you" : name(card.drafted_by));
  const when = (ts: number) => new Date(ts * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  const dur = (s: number) => (s >= 3600 ? `${Math.round(s / 360) / 10} h` : `${Math.round(s / 60)} min`);
  const TRIGGER: Record<string, string> = {
    requester: "the requester's SATISFY",
    verifier: "the settlement verifier's RELEASE",
    contingent: "the requester's FUND, against a sealed offer",
  };
  const checkers = (i: TermsCard["checked_by"][number]) =>
    i.requester ? "the requester" : i.keys.length ? i.keys.map(name).join(", ") : "nobody named";
  // Template slot values (template laundering is about these, Core §13): keys and hashes shortened.
  const slotValue = (v: unknown) =>
    typeof v === "string" && /^[0-9a-f]{64}$/.test(v) ? name(v) : typeof v === "string" ? `"${v}"` : JSON.stringify(v);
  const shown = (l: DiffLine, v: string | null) => (v && l.kind === "time" ? when(Number(v)) : v);
</script>

<div class="terms-card" class:unclosed={!!card.unclosed}>
  <div class="th">
    <span class="ttl">{title}</span>
    <span class="pill v" title={`terms ${card.hash}`}>v{card.version} · drafted by {drafter}</span>
    {#if card.assurance}<span class="pill a" data-a={card.assurance}>assurance: {card.assurance}</span>{/if}
    <button class="link" onclick={() => (open = !open)}>{open ? "hide" : "details"}</button>
  </div>
  {#if card.template}
    {@const tv = card.template}
    <div class="tpl" data-s={tv.status.status} title={`Template event ${tv.event}`}>
      {#if tv.status.status === "counted"}✓ from template{:else if tv.status.status === "mismatch"}⚠ claims template{:else}from template{/if}
      <span class="mono">{tv.name ?? `${tv.event.slice(0, 10)}…`}</span>
      {#if tv.author}by {name(tv.author)}{/if}
      {#if tv.status.status === "mismatch"}<span class="bad">· doesn't match it: {tv.status.reason}</span>
      {:else if tv.status.status === "unavailable"}<span class="muted">· template not fetched, can't check</span>
      {:else}
        <span class="muted">· record here: {tv.record.commitments} commitment{tv.record.commitments === 1 ? "" : "s"}, {tv.record.paid} paid, {tv.record.rejected} rejected{tv.record.disqualified ? ` (${tv.record.disqualified} disqualified)` : ""}{tv.record.disputed ? `, ${tv.record.disputed} disputed` : ""}{tv.record.open ? `, ${tv.record.open} open` : ""}</span>
      {/if}
    </div>
    {#if Object.keys(tv.slots).length}
      <div class="tpl-slots">
        filled in:
        {#each Object.entries(tv.slots) as [k, v], i}{i ? " · " : " "}<span class="mono">{k}</span> = {slotValue(v)}{/each}
      </div>
    {/if}
  {/if}
  {#if card.unclosed}
    <div class="warn">⚠ Can't be promised or accepted yet: {card.unclosed}.</div>
  {/if}
  {#if card.lint.length}
    <div class="lint" title="The terms linter (CR-003 §3): advice on how the terms are written. The validator doesn't use it.">
      {#each card.lint as f (f.rule + (f.item ?? ""))}
        <div class="lf" data-l={f.level}><span class="mono">{f.rule}</span>{#if f.item} <span class="mono">{f.item}</span>{/if} {f.message}</div>
      {/each}
    </div>
  {/if}

  {#if diff.length}
    <div class="sec">
      <div class="sh">Changes from the previous version</div>
      {#each diff as l}
        <div class="diff" data-k={l.kind}>
          <span class="mono w">{l.what}</span>
          {#if l.before && l.after}
            <span class="old">{shown(l, l.before)}</span><span>→</span><span class="new">{shown(l, l.after)}</span>
          {:else if l.after}
            <span class="add">+ {shown(l, l.after)}</span>
          {:else}
            <span class="old">− {shown(l, l.before)}</span>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <div class="sec">
    <div class="sh">Done means</div>
    {#each card.done_means as d}
      <div class="row">
        <span class="mono w">{d.id}</span>
        <span class="cls" data-c={d.class}>{d.class === "open" ? `open${d.suggested_class ? ` (suggested: ${d.suggested_class})` : ""}` : d.class}</span>
        {#if d.assurance}<span class="lbl" data-a={d.assurance}>{d.assurance}</span>{/if}
        <span>{d.description}</span>
        {#if open && (d.check_ref || d.rubric_ref)}<span class="mono ref">{d.check_ref ?? d.rubric_ref}</span>{/if}
      </div>
      {#if open && d.rubric_text}<div class="rubric">Rubric: {d.rubric_text}</div>{/if}
      {#if d.instances}
        <div class="pool" title={`Pool root ${d.instances.pool_root}`}>
          Held-out: {d.instances.sample_size} of {d.instances.pool_size} cases, drawn after delivery from both parties' seeds. Pool: {d.instances.pool_description}. Rule: {d.instances.pool_rule}.
        </div>
      {/if}
    {/each}
    {#if !card.done_means.length}<div class="row muted">No conditions: the requester's own judgement.</div>{/if}
  </div>

  {#if card.disqualified_if.length}
    <div class="sec dq">
      <div class="sh">Disqualified if</div>
      {#each card.disqualified_if as d}
        <div class="row">
          <span class="mono w">{d.id}</span>
          <span class="cls">{d.class}</span>
          {#if d.assurance}<span class="lbl" data-a={d.assurance}>{d.assurance}</span>{/if}
          <span>{d.description}</span>
        </div>
      {/each}
      <div class="note">A hit fails the commitment for breach, however the conditions scored.</div>
    </div>
  {/if}

  {#if open}
    <div class="sec">
      <div class="sh">Checked by</div>
      {#each card.checked_by as i}
        <div class="row">
          <span class="mono w">{i.item}</span>
          <span>{checkers(i)}</span>
          {#if i.rule}<span class="muted">{i.rule}</span>{/if}
        </div>
        {#each i.judges as j}<div class="judge mono">{j}</div>{/each}
      {/each}
    </div>
  {/if}

  <div class="sec">
    <div class="sh">Pay if</div>
    <div class="row">
      {#if card.pay_if.price_sat}
        <span><b>{card.pay_if.price_sat} sat</b>, released by {TRIGGER[card.pay_if.trigger]}{card.pay_if.arbiter ? ", or the arbiter's ruling" : ""}.</span>
      {:else}
        <span>Free.</span>
      {/if}
      <span class="muted">Deliver by {when(card.pay_if.deadline)} · review {dur(card.pay_if.review_window)} · {card.pay_if.rework_limit} revision{card.pay_if.rework_limit === 1 ? "" : "s"}</span>
    </div>
  </div>
  <div class="hash mono">{performer} binds to terms {card.hash.slice(0, 16)}…</div>
</div>

<style>
  .terms-card {
    border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 9px 11px;
    background: var(--panel); display: flex; flex-direction: column; gap: 7px; font-size: 12.5px;
  }
  .terms-card.unclosed { border-color: color-mix(in srgb, var(--warn) 45%, transparent); }
  .th { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  .ttl { font-weight: 650; font-size: 12px; color: var(--muted); }
  .pill { font-size: 10.5px; padding: 1px 7px; border-radius: 999px; background: var(--sunken); color: var(--muted); }
  .link { margin-left: auto; background: none; border: 0; color: var(--accent); font-size: 11.5px; cursor: pointer; padding: 0; }
  .warn { color: var(--warn); font-size: 12px; }
  .tpl { font-size: 11.5px; color: var(--ok); }
  .tpl[data-s="mismatch"] { color: var(--warn); }
  .tpl[data-s="unavailable"] { color: var(--muted); }
  .tpl .bad { color: var(--warn); }
  .tpl-slots { font-size: 11px; color: var(--muted); }
  .lint { display: flex; flex-direction: column; gap: 2px; font-size: 11.5px; color: var(--muted); }
  .lf[data-l="error"] { color: var(--warn); }
  .lf .mono { font-size: 10.5px; }
  .sec { display: flex; flex-direction: column; gap: 3px; }
  .sh { font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.05em; color: var(--faint); font-weight: 650; }
  .row { display: flex; gap: 7px; align-items: baseline; flex-wrap: wrap; }
  .w { font-weight: 600; min-width: 22px; }
  .cls { font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--faint); }
  .cls[data-c="open"] { color: var(--warn); }
  .lbl { font-size: 10.5px; padding: 0 6px; border-radius: 999px; border: 1px solid var(--border); color: var(--muted); }
  .lbl[data-a="opinion"] { color: var(--warn); }
  .lbl[data-a="check"], .lbl[data-a="grounded"] { color: var(--ok); }
  .ref { font-size: 10.5px; color: var(--faint); }
  .rubric { font-size: 11.5px; color: var(--muted); padding-left: 29px; font-style: italic; }
  .pool { font-size: 11.5px; color: var(--muted); padding-left: 29px; }
  .judge { font-size: 10.5px; color: var(--faint); padding-left: 29px; }
  .dq .w { color: var(--danger); }
  .note { font-size: 11px; color: var(--faint); }
  .diff { display: flex; gap: 6px; align-items: baseline; flex-wrap: wrap; font-size: 12px; }
  .old { color: var(--faint); text-decoration: line-through; }
  .new, .add { font-weight: 600; }
  .add { color: var(--ok); }
  .muted { color: var(--muted); }
  .hash { font-size: 10.5px; color: var(--faint); }
</style>
