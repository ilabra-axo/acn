<script lang="ts">
  import type { Contact } from "../types";
  import { displayName, HOSTING_LABEL } from "../format";
  import Avatar from "./Avatar.svelte";

  let { contact, onaccept }: { contact: Contact; onaccept: () => void } = $props();
  const a = $derived(contact.agent);
</script>

<div class="consent" role="dialog" aria-labelledby="consent-title">
  <Avatar pubkey={contact.pubkey} name={displayName(contact)} agent size={56} />
  <h2 id="consent-title">Talk to {displayName(contact)}?</h2>
  <p class="lede">
    This is an <strong>agent</strong>. Messages are end-to-end encrypted to it — which means
    <strong>the agent itself reads everything you send it</strong>, and so does whatever it uses.
  </p>

  <dl>
    <dt>Who runs it</dt>
    <dd>{a ? `${HOSTING_LABEL[a.hosting] ?? a.hosting} · ${a.operator}` : "Unknown — no manifest published"}</dd>
    <dt>Keeps your data</dt>
    <dd>{a?.data_handling.retention ?? "Unknown"}</dd>
    <dt>Shares with</dt>
    <dd>
      {#if a?.data_handling.forwards_to.length}
        {#each a.data_handling.forwards_to as f}<span class="pill mono">{f}</span> {/each}
      {:else if a}
        Nobody declared
      {:else}
        Unknown
      {/if}
    </dd>
    {#if contact.acn}
      <dt>Accountability</dt>
      <dd>
        {#if contact.acn.binding.bound}
          <span class="ok">Bound to operator</span>
          <span class="pill mono" title={contact.acn.binding.operator ?? ""}>{(contact.acn.binding.operator ?? "").slice(0, 14)}…</span>
        {:else}
          <span class="bad">{contact.acn.binding.detail}</span>
        {/if}
      </dd>
      <dt>Commitments</dt>
      <dd>ACN {contact.acn.profiles.join(", ")} · {contact.acn.min_price_sat ? `from ${contact.acn.min_price_sat} sat` : "free"}{#if contact.acn.binding_level} · <span class="pill mono" title="Binding API conformance level the agent claims (K_SVC binding_level). The level's conformance suite is what proves it.">claims {contact.acn.binding_level}</span>{/if}{#if contact.acn.blossom.length} · evidence on {contact.acn.blossom.length} Blossom server{contact.acn.blossom.length === 1 ? "" : "s"}{/if}</dd>
    {/if}
    {#if contact.commands.length}
      <dt>Can do</dt>
      <dd>
        {#each contact.commands as c}<span class="pill mono">/{c.command}</span> {/each}
      </dd>
    {/if}
  </dl>

  {#if !a}
    <p class="warn">This agent didn't publish a manifest, so its data handling is unknown.</p>
  {/if}

  <button class="btn primary" onclick={onaccept}>I understand — start conversation</button>
  <p class="fine muted">You can see this again any time from the banner at the top of the chat.</p>
</div>

<style>
  .consent {
    margin: auto;
    max-width: 460px;
    padding: 28px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    text-align: center;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 18px;
    box-shadow: var(--shadow);
  }
  h2 {
    margin: 4px 0 0;
    font-size: 18px;
  }
  .lede {
    margin: 0;
    color: var(--muted);
  }
  dl {
    width: 100%;
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 8px 16px;
    text-align: left;
    margin: 6px 0;
    padding: 14px;
    background: var(--agent-soft);
    border: 1px solid var(--agent-border);
    border-radius: var(--radius-sm);
    font-size: 13px;
  }
  dt {
    color: var(--agent);
    font-weight: 600;
  }
  dd {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .ok {
    color: var(--ok);
    font-weight: 600;
  }
  .bad {
    color: var(--warn);
  }
  .warn {
    color: var(--warn);
    font-size: 13px;
    margin: 0;
  }
  .fine {
    font-size: 12px;
    margin: 0;
  }
</style>
