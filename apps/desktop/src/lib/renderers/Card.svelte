<script lang="ts">
  import type { RendererHost } from "../types";

  let { payload, host, mine }: { payload: any; host: RendererHost; mine: boolean } = $props();
  let pressed = $state<string | null>(null);

  async function press(a: { id: string; label: string }) {
    pressed = a.id;
    await host.action(a.id, a.label);
  }
</script>

<div class="card">
  {#if payload.image?.url}
    <img src={payload.image.url} alt={payload.image.alt ?? ""} />
  {/if}
  <div class="head">
    <div class="title">{payload.title}</div>
    {#if payload.subtitle}<div class="subtitle">{payload.subtitle}</div>{/if}
  </div>
  {#if payload.body}<p class="body">{payload.body}</p>{/if}
  {#if payload.fields?.length}
    <dl>
      {#each payload.fields as f}
        <dt>{f.label}</dt>
        <dd>{f.value}</dd>
      {/each}
    </dl>
  {/if}
  {#if payload.actions?.length && !mine}
    <div class="actions">
      {#each payload.actions as a}
        <button
          class="btn"
          class:primary={a.style === "primary"}
          class:danger={a.style === "danger"}
          disabled={pressed !== null}
          onclick={() => press(a)}
        >
          {pressed === a.id ? "Sent ✓" : a.label}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  img {
    width: 100%;
    max-height: 220px;
    object-fit: cover;
    border-radius: var(--radius-sm);
  }
  .title {
    font-weight: 650;
    font-size: 15px;
  }
  .subtitle {
    color: var(--muted);
    font-size: 13px;
  }
  .body {
    margin: 0;
    white-space: pre-wrap;
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 14px;
    margin: 0;
    font-size: 13px;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 2px;
  }
</style>
