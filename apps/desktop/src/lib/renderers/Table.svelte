<script lang="ts">
  let { payload }: { payload: any } = $props();

  const cell = (v: unknown) =>
    v === undefined || v === null ? "" : typeof v === "object" ? JSON.stringify(v) : String(v);
</script>

{#if payload.title}<div class="title">{payload.title}</div>{/if}
<div class="wrap">
  <table>
    <thead>
      <tr>
        {#each payload.columns as c}
          <th class:num={c.type === "number"}>{c.label}</th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each payload.rows as r}
        <tr>
          {#each payload.columns as c}
            <td class:num={c.type === "number"}>{cell(r[c.key])}</td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .title {
    font-weight: 650;
    margin-bottom: 8px;
  }
  .wrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 13px;
  }
  th,
  td {
    text-align: left;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  th {
    background: var(--sunken);
    font-weight: 600;
    color: var(--muted);
    font-size: 12px;
  }
  tr:last-child td {
    border-bottom: none;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
