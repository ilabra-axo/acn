<script lang="ts">
  import type { Message, RendererHost } from "../types";
  import { time } from "../format";
  import Structured from "../renderers/Structured.svelte";
  import CommitmentCard from "./CommitmentCard.svelte";
  import { store } from "../store.svelte";

  let { message, host }: { message: Message; host: RendererHost } = $props();

  const body = $derived(message.body);
  // Highlight a leading slash command in text messages.
  const command = $derived(
    body.kind === "text" && /^\/\S+/.test(body.text) ? body.text.match(/^\/\S+/)![0] : null,
  );
</script>

{#if body.kind === "commitment"}
  {@const c = store.commitments[body.commitment]}
  <div class="row" class:mine={message.mine}>
    <div class="bubble structured commitment">
      {#if c}
        <CommitmentCard commitment={c} {host} />
      {:else}
        <span class="muted">Commitment {body.commitment.slice(0, 8)}…</span>
      {/if}
      <span class="t">{time(message.created_at)}</span>
    </div>
  </div>
{:else if body.kind === "action"}
  <div class="action" class:mine={message.mine}>
    <span>↳ {message.mine ? "You" : "They"}: {body.alt}</span>
    <span class="t">{time(message.created_at)}</span>
  </div>
{:else}
  <div class="row" class:mine={message.mine}>
    <div class="bubble" class:structured={body.kind === "structured"}>
      {#if body.kind === "text"}
        {#if command}
          <span class="cmd mono">{command}</span>{body.text.slice(command.length)}
        {:else}
          {body.text}
        {/if}
      {:else if body.kind === "structured"}
        <Structured {body} {host} mine={message.mine} />
      {:else}
        <span class="muted">{body.alt ?? `Unsupported event kind ${body.event_kind}`}</span>
      {/if}
      <span class="t">{time(message.created_at)}</span>
    </div>
  </div>
{/if}

<style>
  .row {
    display: flex;
    justify-content: flex-start;
  }
  .row.mine {
    justify-content: flex-end;
  }
  .bubble {
    position: relative;
    max-width: min(560px, 78%);
    padding: 8px 12px 18px;
    border-radius: var(--radius);
    background: var(--theirs);
    border: 1px solid var(--border);
    white-space: pre-wrap;
    word-wrap: break-word;
    box-shadow: var(--shadow);
  }
  .mine .bubble {
    background: var(--mine);
    color: var(--mine-text);
    border-color: transparent;
  }
  .bubble.structured {
    white-space: normal;
    padding: 12px 14px 20px;
    min-width: 260px;
  }
  .mine .bubble.structured {
    background: var(--panel);
    color: var(--text);
    border: 1px solid var(--accent);
  }
  .bubble.commitment {
    max-width: min(600px, 86%);
    border-style: solid;
    border-width: 1px 1px 1px 4px;
    border-color: var(--border);
    border-left-color: var(--accent);
  }
  .mine .bubble.commitment {
    border-color: var(--border);
    border-left-color: var(--accent);
  }
  .t {
    position: absolute;
    right: 10px;
    bottom: 3px;
    font-size: 10px;
    opacity: 0.55;
  }
  .cmd {
    font-size: 12.5px;
    padding: 1px 5px;
    border-radius: 5px;
    background: rgba(127, 127, 160, 0.16);
    margin-right: 1px;
  }
  .mine .cmd {
    background: rgba(255, 255, 255, 0.2);
  }
  .action {
    align-self: flex-start;
    display: flex;
    gap: 8px;
    font-size: 12px;
    color: var(--muted);
    padding: 0 6px;
  }
  .action.mine {
    align-self: flex-end;
  }
  .action .t {
    position: static;
  }
</style>
