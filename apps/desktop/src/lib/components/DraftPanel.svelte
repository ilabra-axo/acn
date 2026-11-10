<script lang="ts">
  // Draft a request with the requester drafter (acn-author, CR-003 §3), review it in the terms
  // editor (with the linter), and send it. Requirements the drafter can't close stay `open`;
  // the performer's counter closes them, and nobody can promise until it does (V-26).
  import type { Contact, RequestDraft } from "../types";
  import { store } from "../store.svelte";
  import TermsEditor from "./TermsEditor.svelte";

  let { contact, onclose }: { contact: Contact; onclose: () => void } = $props();

  const tools = $derived(contact.acn?.tools ?? []);
  let tool = $state("");
  let goal = $state("");
  let requirements = $state("");
  let price = $state(0);
  let hours = $state(2);
  let draft = $state<RequestDraft | null>(null);
  let busy = $state(false);

  $effect(() => {
    if (!tool && tools.length) tool = tools[0].name;
  });

  async function run(e: Event) {
    e.preventDefault();
    if (!store.backend || !goal.trim()) return;
    busy = true;
    try {
      const now = Math.floor(Date.now() / 1000);
      const d = await store.backend.acnDraft({
        goal: goal.trim(),
        requirements: requirements.split("\n").map((r) => r.trim()).filter(Boolean),
        price_sat: Math.max(0, Number(price) || 0),
        deadline: now + Math.max(1, Number(hours) || 1) * 3600,
        review_window: 3600,
      });
      // The agent works by tool; the goal is its brief.
      d.terms.deliverable = { ...d.terms.deliverable, tool, input: { brief: goal.trim() } };
      if (d.terms.price?.amount > 0 && store.wallet) d.terms.payment = { mint: store.wallet.mint, keysets: [] };
      draft = d;
    } catch (err) {
      store.fail(err);
    }
    busy = false;
  }

  async function send(terms: any) {
    const ok = await store.requestTerms("N", terms);
    if (ok) onclose();
    return ok;
  }
</script>

<div class="draft">
  <div class="dh">
    <span class="ttl">Draft terms for {contact.name ?? "this agent"}</span>
    <button class="x" onclick={onclose} title="Close">×</button>
  </div>
  {#if !draft}
    <form class="form" onsubmit={run}>
      <label>What it should do <input class="field" placeholder="e.g. Retry with backoff in the relay client" bind:value={goal} disabled={busy} /></label>
      <label>Requirements, one per line
        <textarea class="field" rows="3" placeholder={"The existing test suite passes\nDon't make it slower"} bind:value={requirements} disabled={busy}></textarea>
      </label>
      <div class="row">
        <label>Tool
          <select class="field" bind:value={tool} disabled={busy}>
            {#each tools as t}<option value={t.name}>{t.title || t.name}</option>{/each}
          </select>
        </label>
        <label>Price <span class="unit">sat</span><input class="field" type="number" min="0" bind:value={price} disabled={busy} /></label>
        <label>Deliver within <span class="unit">h</span><input class="field" type="number" min="1" bind:value={hours} disabled={busy} /></label>
      </div>
      <div class="actions">
        <button class="btn primary" type="submit" disabled={busy || !goal.trim()}>Draft</button>
        <span class="muted hint">Matches the agent's published templates; what it can't close stays open for the agent's counter-offer.</span>
      </div>
    </form>
  {:else}
    <div class="notes">
      {#if draft.template}<div>✓ from template <span class="mono">{draft.template}</span></div>{/if}
      {#each draft.notes as n}<div class="muted">{n}</div>{/each}
    </div>
    <TermsEditor terms={draft.terms} submitLabel="Send request" onsubmit={send} oncancel={() => (draft = null)} />
  {/if}
</div>

<style>
  .draft { display: flex; flex-direction: column; gap: 8px; padding: 10px 14px; border-top: 1px solid var(--border); background: var(--panel); }
  .dh { display: flex; align-items: center; }
  .ttl { font-weight: 650; font-size: 13px; }
  .x { margin-left: auto; background: none; border: 0; color: var(--muted); font-size: 16px; cursor: pointer; }
  .form { display: flex; flex-direction: column; gap: 6px; }
  label { display: flex; flex-direction: column; gap: 2px; font-size: 11px; color: var(--muted); flex: 1; }
  .unit { color: var(--faint); }
  .row { display: flex; gap: 8px; }
  textarea { resize: vertical; font: inherit; }
  .actions { display: flex; gap: 8px; align-items: center; }
  .hint { font-size: 11.5px; }
  .notes { display: flex; flex-direction: column; gap: 2px; font-size: 12px; }
</style>
