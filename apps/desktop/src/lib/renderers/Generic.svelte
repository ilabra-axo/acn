<script lang="ts">
  import Generic from "./Generic.svelte";

  let { payload, depth = 0 }: { payload: any; depth?: number } = $props();

  const isObj = (v: unknown) => v !== null && typeof v === "object";
  const entries = $derived(
    Array.isArray(payload) ? payload.map((v: unknown, i: number) => [String(i), v]) : Object.entries(payload ?? {}),
  );
</script>

{#if !isObj(payload)}
  <span class="scalar mono">{JSON.stringify(payload)}</span>
{:else}
  <div class="tree" class:root={depth === 0}>
    {#each entries as [k, v]}
      <div class="row">
        <span class="key">{k}</span>
        {#if isObj(v) && depth < 4}
          <Generic payload={v} depth={depth + 1} />
        {:else}
          <span class="scalar mono">{typeof v === "string" ? v : JSON.stringify(v)}</span>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .tree {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 13px;
  }
  .tree:not(.root) {
    padding-left: 10px;
    border-left: 2px solid var(--border);
    margin-top: 2px;
  }
  .row {
    display: flex;
    gap: 10px;
    align-items: baseline;
    flex-wrap: wrap;
  }
  .key {
    color: var(--muted);
    font-weight: 500;
    min-width: 60px;
  }
  .scalar {
    font-size: 12px;
    word-break: break-word;
  }
</style>
