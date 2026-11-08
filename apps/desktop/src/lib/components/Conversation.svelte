<script lang="ts">
  import { tick } from "svelte";
  import { store } from "../store.svelte";
  import type { Contact, RendererHost } from "../types";
  import { displayName, HOSTING_LABEL } from "../format";
  import Avatar from "./Avatar.svelte";
  import AgentConsent from "./AgentConsent.svelte";
  import Composer from "./Composer.svelte";
  import MessageItem from "./MessageItem.svelte";
  import Director from "./Director.svelte";

  let { contact }: { contact: Contact } = $props();
  let list: HTMLDivElement | undefined = $state();
  let details = $state(false);
  // ACN-capable agents default to commitments; plain chat stays one click away.
  let commit = $state(true);

  const a = $derived(contact.agent);
  const needsConsent = $derived(contact.is_agent && !contact.consented);

  // Each message gets a host bound to it, so actions always reference the right target.
  function hostFor(messageId: string): RendererHost {
    return {
      action: (actionId, label) => store.sendAction(messageId, actionId, null, label),
      submit: (actionId, values, label) => store.sendAction(messageId, actionId ?? null, values, label ?? null),
      openUrl: (url) => store.openUrl(url),
    };
  }

  $effect(() => {
    // Scroll to bottom when messages change.
    store.messages.length;
    Object.keys(store.commitments).length;
    tick().then(() => list?.scrollTo({ top: list.scrollHeight, behavior: "smooth" }));
  });
</script>

<div class="layout">
<section>
  <header>
    <Avatar pubkey={contact.pubkey} name={displayName(contact)} agent={contact.is_agent} />
    <div class="who">
      <div class="name">
        {displayName(contact)}
        {#if contact.is_agent}<span class="pill agent">AGENT</span>{/if}
        {#if contact.acn?.simulated}<span class="pill">ACN demo</span>{/if}
      </div>
      <div class="npub mono">{contact.npub}</div>
    </div>
    {#if !needsConsent}
      <button class="btn director-btn" class:on={store.directorOpen} onclick={() => (store.directorOpen = !store.directorOpen)}>
        🎬 Director
      </button>
    {/if}
  </header>

  {#if contact.is_agent && !needsConsent}
    <button class="banner" onclick={() => (details = !details)} aria-expanded={details}>
      <svg viewBox="0 0 16 16" width="14" height="14"><path d="M8 1.5l5.5 2v4c0 3.3-2.3 6.1-5.5 7-3.2-.9-5.5-3.7-5.5-7v-4z" fill="none" stroke="currentColor" stroke-width="1.4" /><circle cx="8" cy="7.2" r="1.3" fill="currentColor" /></svg>
      <span>
        <strong>{displayName(contact)} reads everything in this conversation.</strong>
        {#if a}{HOSTING_LABEL[a.hosting] ?? a.hosting} by {a.operator}.{:else}Data handling unknown.{/if}
      </span>
      <span class="more">{details ? "Hide" : "Details"}</span>
    </button>
    {#if details && a}
      <div class="details">
        <div><span>Keeps data</span>{a.data_handling.retention}</div>
        <div><span>Shares with</span>{a.data_handling.forwards_to.join(", ") || "nobody declared"}</div>
        <div><span>Manifest</span><span class="mono">{a.id}</span></div>
      </div>
    {/if}
  {/if}

  {#if needsConsent}
    <div class="center">
      <AgentConsent {contact} onaccept={() => store.acceptAgent(contact.pubkey)} />
    </div>
  {:else}
    <div class="messages" bind:this={list}>
      {#each store.messages as m (m.id)}
        <MessageItem message={m} host={hostFor(m.id)} />
      {:else}
        <div class="empty muted">
          {#if contact.is_agent && contact.commands.length}
            Type <span class="mono">/</span> to see what {displayName(contact)} can do.
          {:else}
            No messages yet. Say hi 👋
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <Composer
    commands={contact.commands}
    disabled={needsConsent}
    acn={!!contact.acn}
    bind:commit
    placeholder={contact.is_agent ? `Message ${displayName(contact)} — type / for commands` : `Message ${displayName(contact)}`}
    onsend={async (t) => {
      if (contact.acn && commit && t.startsWith("/")) {
        const id = await store.request(t);
        if (id) store.focus = id;
      } else await store.sendText(t);
    }}
  />
</section>
{#if store.directorOpen && !needsConsent}
  <Director {contact} />
{/if}
</div>

<style>
  .layout {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .director-btn {
    margin-left: auto;
    padding: 5px 11px;
    font-size: 12.5px;
  }
  .director-btn.on {
    background: var(--agent-soft);
    border-color: var(--agent-border);
    color: var(--agent);
  }
  section {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 18px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
  }
  .who {
    min-width: 0;
  }
  .name {
    font-weight: 650;
    font-size: 15px;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .npub {
    font-size: 11px;
    color: var(--faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .banner {
    display: flex;
    gap: 10px;
    align-items: center;
    text-align: left;
    width: 100%;
    padding: 8px 18px;
    border: none;
    border-bottom: 1px solid var(--agent-border);
    background: var(--agent-soft);
    color: var(--agent);
    font-size: 12.5px;
  }
  .banner strong {
    font-weight: 650;
  }
  .more {
    margin-left: auto;
    font-weight: 600;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .details {
    display: flex;
    gap: 24px;
    flex-wrap: wrap;
    padding: 8px 18px 10px 42px;
    background: var(--agent-soft);
    border-bottom: 1px solid var(--agent-border);
    font-size: 12.5px;
  }
  .details span:first-child {
    color: var(--agent);
    font-weight: 600;
    margin-right: 6px;
  }
  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .center {
    flex: 1;
    display: flex;
    overflow-y: auto;
    padding: 24px;
  }
  .empty {
    margin: auto;
    text-align: center;
    font-size: 13px;
  }
</style>
