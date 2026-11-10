<script lang="ts">
  // Edit a terms document before it is sent: a counter-offer (profile N) or a drafted request.
  // The linter (acn-author, CR-003 §3) runs on every change: errors block sending, warnings are
  // advice. Nothing here has semantics: the validator judges what is sent (INV-12).
  import { untrack } from "svelte";
  import type { LintFinding } from "../types";
  import { store } from "../store.svelte";

  let {
    terms,
    submitLabel,
    reasonLabel = null,
    binding = false,
    lockedPrice = false,
    onsubmit,
    oncancel,
  }: {
    terms: any;
    submitLabel: string;
    /** Ask for a reason (a COUNTER carries one). */
    reasonLabel?: string | null;
    /** Lint as terms about to bind (L3: nothing may stay open). */
    binding?: boolean;
    /** The price can't change (a token is already locked for it). */
    lockedPrice?: boolean;
    onsubmit: (terms: any, reason: string) => Promise<boolean | void>;
    oncancel: () => void;
  } = $props();

  // A working copy of the terms as they were when the editor opened; the caller's document is
  // never mutated.
  const initial = untrack(() => structuredClone($state.snapshot(terms)));
  let t = $state(initial);
  let reason = $state("");
  let busy = $state(false);
  let findings = $state<LintFinding[]>([]);
  let invalid = $state<string | null>(null);

  const nowSecs = () => Math.floor(Date.now() / 1000);
  let deadlineHours = $state(Math.max(1, Math.round(((initial.deadline ?? nowSecs() + 3600) - nowSecs()) / 3600)));
  let reviewHours = $state(Math.max(1, Math.round((initial.review_window ?? 3600) / 3600)));

  const EDITABLE = ["open", "mechanical", "human", "interpretive"];
  const LABEL: Record<string, string> = {
    open: "open (to be closed)", mechanical: "mechanical (a check)", human: "human (your call)",
    interpretive: "interpretive (advisory opinion)", judged: "judged", grounding: "grounding",
  };

  function setClass(i: number, cls: string) {
    const c = t.conditions[i];
    if (cls === "open") {
      c.suggested_class = c.class === "open" ? (c.suggested_class ?? "mechanical") : c.class;
    } else {
      delete c.suggested_class;
    }
    c.class = cls;
  }
  function addCondition() {
    const ids = new Set([...(t.conditions ?? []).map((c: any) => c.id), ...(t.disqualifiers ?? []).map((d: any) => d.id)]);
    let n = 1;
    while (ids.has(`c${n}`)) n++;
    t.conditions = [...(t.conditions ?? []), { id: `c${n}`, class: "open", description: "", suggested_class: "human" }];
  }
  function removeCondition(i: number) {
    t.conditions = t.conditions.filter((_: any, j: number) => j !== i);
  }
  function removeDisqualifier(i: number) {
    t.disqualifiers = t.disqualifiers.filter((_: any, j: number) => j !== i);
    if (!t.disqualifiers.length) delete t.disqualifiers;
  }

  // What would be sent.
  const out = $derived.by(() => {
    const o = $state.snapshot(t);
    o.deadline = nowSecs() + Math.max(1, Number(deadlineHours) || 1) * 3600;
    o.review_window = Math.max(1, Number(reviewHours) || 1) * 3600;
    o.rework_limit = Math.max(0, Number(o.rework_limit) || 0);
    o.price = { ...(o.price ?? {}), amount: Math.max(0, Number(o.price?.amount) || 0) };
    return o;
  });

  // Lint on every change (debounced).
  let timer: number | undefined;
  $effect(() => {
    const doc = out;
    clearTimeout(timer);
    timer = window.setTimeout(async () => {
      try {
        findings = (await store.backend?.acnLint(doc, binding)) ?? [];
        invalid = null;
      } catch (e) {
        findings = [];
        invalid = String(e);
      }
    }, 250);
  });
  const errors = $derived(findings.filter((f) => f.level === "error"));

  async function submit(e: Event) {
    e.preventDefault();
    busy = true;
    const ok = await onsubmit(out, reason.trim() || "Revised terms");
    busy = false;
    if (ok === false) return;
  }
</script>

<form class="editor" onsubmit={submit}>
  <div class="grid">
    <label>Price <span class="unit">sat</span>
      <input class="field num" type="number" min="0" bind:value={t.price.amount} disabled={lockedPrice || busy}
        title={lockedPrice ? "Your token is locked for this price: send a new request to change it" : ""} />
    </label>
    <label>Deliver within <span class="unit">h</span>
      <input class="field num" type="number" min="1" bind:value={deadlineHours} disabled={busy} />
    </label>
    <label>Review window <span class="unit">h</span>
      <input class="field num" type="number" min="1" bind:value={reviewHours} disabled={busy} />
    </label>
    <label>Revisions
      <input class="field num" type="number" min="0" max="9" bind:value={t.rework_limit} disabled={busy} />
    </label>
  </div>

  <div class="sh">Done means</div>
  {#each t.conditions ?? [] as c, i (c.id)}
    <div class="row">
      <span class="mono id">{c.id}</span>
      {#if EDITABLE.includes(c.class)}
        <select class="field cls" value={c.class} onchange={(e) => setClass(i, (e.target as HTMLSelectElement).value)} disabled={busy}>
          {#each EDITABLE as k}<option value={k}>{LABEL[k]}</option>{/each}
        </select>
      {:else}
        <span class="pill" title="Edit its fields in the terms document">{LABEL[c.class] ?? c.class}</span>
      {/if}
      <input class="field desc" placeholder="What has to be true" bind:value={c.description} disabled={busy} />
      <button type="button" class="x" title="Remove" onclick={() => removeCondition(i)} disabled={busy}>×</button>
    </div>
  {/each}
  <button type="button" class="link" onclick={addCondition} disabled={busy}>+ condition</button>

  {#if t.disqualifiers?.length}
    <div class="sh">Disqualified if</div>
    {#each t.disqualifiers as d, i (d.id)}
      <div class="row">
        <span class="mono id">{d.id}</span>
        <span class="pill">{d.class}</span>
        <span class="desc ro">{d.description}</span>
        <button type="button" class="x" title="Remove" onclick={() => removeDisqualifier(i)} disabled={busy}>×</button>
      </div>
    {/each}
  {/if}

  {#if invalid}
    <div class="lf" data-l="error">⚠ Not valid terms: {invalid}</div>
  {/if}
  {#each findings as f (f.rule + (f.item ?? ""))}
    <div class="lf" data-l={f.level}><span class="mono">{f.rule}</span>{#if f.item} <span class="mono">{f.item}</span>{/if} {f.message}</div>
  {/each}

  {#if reasonLabel}
    <input class="field" placeholder={reasonLabel} bind:value={reason} disabled={busy} />
  {/if}
  <div class="actions">
    <button class="btn primary" type="submit" disabled={busy || !!invalid || errors.length > 0}
      title={errors.length ? "Fix the errors first" : ""}>{submitLabel}</button>
    <button class="btn" type="button" onclick={oncancel} disabled={busy}>Cancel</button>
  </div>
</form>

<style>
  .editor { display: flex; flex-direction: column; gap: 6px; padding: 10px; border: 1px solid var(--border); border-radius: 10px; background: var(--sunken); }
  .grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
  label { display: flex; flex-direction: column; gap: 2px; font-size: 11px; color: var(--muted); }
  .unit { color: var(--faint); }
  .num { width: 100%; }
  .sh { font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.05em; color: var(--faint); font-weight: 650; margin-top: 4px; }
  .row { display: flex; gap: 6px; align-items: center; }
  .id { font-size: 11px; min-width: 22px; }
  .cls { width: 180px; flex: none; }
  .desc { flex: 1; min-width: 0; }
  .ro { font-size: 12px; color: var(--muted); }
  .pill { font-size: 10.5px; padding: 1px 7px; border-radius: 999px; background: var(--bg); color: var(--muted); }
  .x { background: none; border: 0; color: var(--muted); cursor: pointer; font-size: 15px; padding: 0 4px; }
  .link { align-self: flex-start; background: none; border: 0; color: var(--accent); font-size: 12px; cursor: pointer; padding: 0; }
  .lf { font-size: 11.5px; color: var(--muted); }
  .lf[data-l="error"] { color: var(--warn); }
  .lf .mono { font-size: 10.5px; }
  .actions { display: flex; gap: 6px; }
</style>
