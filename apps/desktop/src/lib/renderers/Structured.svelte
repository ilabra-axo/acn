<script lang="ts">
  import type { Body, RendererHost } from "../types";
  import { BUILTIN } from "./index";
  import Generic from "./Generic.svelte";

  type StructuredBody = Extract<Body, { kind: "structured" }>;
  let { body, host, mine }: { body: StructuredBody; host: RendererHost; mine: boolean } = $props();

  const Renderer = $derived(body.renderer.tier === "installed" ? BUILTIN[body.renderer.id] : undefined);
  let showRaw = $state(false);
</script>

<div class="structured">
  {#if showRaw}
    <Generic payload={body.payload} />
  {:else if Renderer}
    <Renderer payload={body.payload} {host} {mine} />
  {:else if body.renderer.tier === "alt" || !body.valid}
    <div class="alt">
      <div>{body.alt || "(no description)"}</div>
      {#if body.error}<div class="err">Not rendered: {body.error}</div>{/if}
    </div>
  {:else}
    <Generic payload={body.payload} />
  {/if}

  <footer>
    <span
      class="contract mono"
      class:invalid={!body.valid}
      title={body.schema_hash ? `schema sha256 ${body.schema_hash}` : "unpinned contract"}
    >
      {body.valid ? "✓" : "⚠"} {body.contract}
    </span>
    <button class="raw" onclick={() => (showRaw = !showRaw)}>{showRaw ? "rendered" : "data"}</button>
  </footer>
</div>

<style>
  .structured {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .alt {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .err {
    font-size: 12px;
    color: var(--warn);
  }
  footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    border-top: 1px dashed var(--border);
    padding-top: 6px;
    margin-top: 2px;
  }
  .contract {
    font-size: 11px;
    color: var(--faint);
  }
  .contract.invalid {
    color: var(--warn);
  }
  .raw {
    border: none;
    background: none;
    padding: 0;
    font-size: 11px;
    color: var(--faint);
  }
  .raw:hover {
    color: var(--accent);
  }
</style>
