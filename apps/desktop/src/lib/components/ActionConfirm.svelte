<script lang="ts">
  // Host-drawn confirmation for renderer actions (A17). Shows the invocation the agent will
  // actually receive (command and values) and who receives it; the button's label is shown as
  // the sender's description only.
  import { store } from "../store.svelte";
  import { displayName } from "../format";

  const p = $derived(store.pendingAction);
  const contact = $derived(p ? store.contacts.find((c) => c.pubkey === p.conversation) : undefined);
  let always = $state(false);
  const values = $derived(p?.values ? Object.entries(p.values) : []);

  function answer(ok: boolean) {
    store.answerAction(ok, always);
    always = false;
  }
</script>

{#if p}
  <div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && answer(false)}>
  <div class="confirm" role="dialog" aria-modal="true" aria-labelledby="ac-title">
    <h3 id="ac-title">Send this to {contact ? displayName(contact) : "this agent"}?</h3>
    {#if p.label}<p class="muted">The button said: “{p.label}”</p>{/if}
    <dl>
      <dt>Invokes</dt>
      <dd class="mono">{p.actionId ?? "(form submission, no command)"}</dd>
      {#if values.length}
        <dt>With</dt>
        <dd>
          <table><tbody>
            {#each values as [k, v]}
              <tr><td class="mono">{k}</td><td class="mono">{typeof v === "string" ? v : JSON.stringify(v)}</td></tr>
            {/each}
          </tbody></table>
        </dd>
      {/if}
    </dl>
    <p class="muted small">The agent decides what this command does; nmesh can't check it. Only send it if you'd type it yourself.</p>
    <label class="small"><input type="checkbox" bind:checked={always} /> Don't ask again for <span class="mono">{p.actionId ?? "this form"}</span> in this conversation (until restart)</label>
    <div class="actions">
      <button class="btn primary" onclick={() => answer(true)}>Send</button>
      <button class="btn" onclick={() => answer(false)}>Cancel</button>
    </div>
  </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.35); z-index: 1000; display: flex; align-items: flex-start; justify-content: center; padding-top: 18vh; }
  .confirm { width: min(460px, 92vw); background: var(--panel); border: 1px solid var(--border); border-radius: var(--radius); padding: 16px 18px; display: flex; flex-direction: column; gap: 8px; box-shadow: var(--shadow); }
  h3 { margin: 0; font-size: 15px; }
  dl { display: grid; grid-template-columns: auto 1fr; gap: 4px 10px; margin: 0; font-size: 13px; }
  dt { color: var(--muted); }
  dd { margin: 0; word-break: break-all; }
  table { border-collapse: collapse; }
  td { padding: 1px 8px 1px 0; vertical-align: top; }
  .small { font-size: 12px; }
  .actions { display: flex; gap: 8px; margin-top: 4px; }
</style>
