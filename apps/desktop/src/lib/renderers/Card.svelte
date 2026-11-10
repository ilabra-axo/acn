<script lang="ts">
  import type { RendererHost } from "../types";

  let { payload, host, mine }: { payload: any; host: RendererHost; mine: boolean } = $props();
  let pressed = $state<string | null>(null);

  async function press(a: { id: string; label: string }) {
    pressed = a.id;
    if (!(await host.action(a.id, a.label))) pressed = null;
  }

  // Remote images never load on their own (A16): the sender would learn when you read the
  // message. On click, the host fetches a hash-pinned image natively and checks it; an unpinned
  // one can only be opened in the browser.
  let img = $state<string | null>(null);
  let imgError = $state<string | null>(null);
  const imgHost = $derived.by(() => {
    try {
      return new URL(payload.image?.url ?? "").host;
    } catch {
      return "an unknown host";
    }
  });
  async function loadImage() {
    imgError = null;
    try {
      img = await host.loadImage(payload.image.url, payload.image.sha256);
    } catch (e) {
      imgError = String(e);
    }
  }
</script>

<div class="card">
  {#if payload.image?.url}
    {#if img}
      <img src={img} alt={payload.image.alt ?? ""} />
    {:else if payload.image.sha256}
      <button class="img-gate" onclick={loadImage} title={payload.image.url}>
        🖼 Load image from {imgHost}{payload.image.alt ? ` · ${payload.image.alt}` : ""}
      </button>
      {#if imgError}<div class="img-err">{imgError}</div>{/if}
    {:else}
      <button class="img-gate" onclick={() => host.openUrl(payload.image.url)} title={payload.image.url}>
        🖼 Image on {imgHost} (not hash-pinned) · open in browser
      </button>
    {/if}
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
  .img-gate { margin: 0 0 8px; padding: 10px; width: 100%; text-align: left; border: 1px dashed var(--border); border-radius: var(--radius-sm); background: var(--sunken); color: var(--muted); cursor: pointer; font-size: 12px; }
  .img-err { color: var(--warn); font-size: 11px; margin-bottom: 6px; }
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
