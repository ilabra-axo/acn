<script lang="ts">
  import { untrack } from "svelte";
  import type { RendererHost } from "../types";

  let { payload, host, mine }: { payload: any; host: RendererHost; mine: boolean } = $props();

  type Field = {
    name: string;
    label?: string;
    type: string;
    required?: boolean;
    options?: { value: string; label?: string }[];
    default?: unknown;
  };

  const fields: Field[] = $derived(payload.fields ?? []);
  // Message payloads are immutable, so initial form values are taken once.
  let values = $state<Record<string, any>>(
    untrack(() =>
      Object.fromEntries(
        (payload.fields as Field[] ?? []).map((f) => [
          f.name,
          f.default ?? (f.type === "bool" ? false : f.type === "multiselect" ? [] : ""),
        ]),
      ),
    ),
  );
  let submitted = $state(false);

  const missing = $derived(fields.filter((f) => f.required && (values[f.name] === "" || values[f.name] == null)));

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (missing.length || submitted) return;
    const out: Record<string, unknown> = {};
    for (const f of fields) {
      const v = values[f.name];
      if (v === "" || v == null) continue;
      out[f.name] = f.type === "number" ? Number(v) : v;
    }
    submitted = true;
    await host.submit(payload.submit_action, out, payload.submit_label);
  }
</script>

<form onsubmit={submit}>
  {#if payload.title}<div class="title">{payload.title}</div>{/if}
  {#if payload.instructions}<div class="muted">{payload.instructions}</div>{/if}

  {#each fields as f}
    <label>
      <span>{f.label ?? f.name}{#if f.required}<em>*</em>{/if}</span>
      {#if f.type === "multiline"}
        <textarea class="field" rows="3" bind:value={values[f.name]} disabled={submitted || mine}></textarea>
      {:else if f.type === "bool"}
        <input type="checkbox" bind:checked={values[f.name]} disabled={submitted || mine} />
      {:else if f.type === "select"}
        <select class="field" bind:value={values[f.name]} disabled={submitted || mine}>
          <option value="" disabled>Choose…</option>
          {#each f.options ?? [] as o}<option value={o.value}>{o.label ?? o.value}</option>{/each}
        </select>
      {:else if f.type === "multiselect"}
        <select class="field" multiple bind:value={values[f.name]} disabled={submitted || mine}>
          {#each f.options ?? [] as o}<option value={o.value}>{o.label ?? o.value}</option>{/each}
        </select>
      {:else}
        <input
          class="field"
          type={f.type === "number" ? "number" : f.type === "date" ? "date" : "text"}
          placeholder={f.type === "pubkey" ? "npub1…" : f.type === "location" ? "lat, lng" : ""}
          bind:value={values[f.name]}
          disabled={submitted || mine}
        />
      {/if}
    </label>
  {/each}

  {#if !mine}
    <div class="row">
      <button class="btn primary" type="submit" disabled={submitted || missing.length > 0}>
        {submitted ? "Submitted ✓" : (payload.submit_label ?? "Submit")}
      </button>
    </div>
  {/if}
</form>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 300px;
  }
  .title {
    font-weight: 650;
    font-size: 15px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }
  label > span {
    color: var(--muted);
    font-weight: 500;
  }
  em {
    color: var(--danger);
    font-style: normal;
    margin-left: 2px;
  }
  textarea {
    resize: vertical;
  }
  input[type="checkbox"] {
    width: 16px;
    height: 16px;
    accent-color: var(--accent);
  }
  .row {
    display: flex;
    justify-content: flex-end;
  }
</style>
