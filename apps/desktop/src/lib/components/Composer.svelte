<script lang="ts">
  import type { CommandView } from "../types";

  let {
    commands,
    disabled = false,
    placeholder = "Message",
    acn = false,
    commit = $bindable(false),
    onsend,
  }: {
    commands: CommandView[];
    disabled?: boolean;
    placeholder?: string;
    /** The peer offers ACN commitments: show the commitment toggle. */
    acn?: boolean;
    /** Send slash commands as ACN commitments instead of chat messages. */
    commit?: boolean;
    onsend: (text: string) => Promise<void>;
  } = $props();

  let text = $state("");
  const committing = $derived(acn && commit && text.trim().startsWith("/"));
  let selected = $state(0);
  let el: HTMLTextAreaElement | undefined = $state();

  // "/ca" → suggest commands; "/card " → show argument hint for /card.
  const token = $derived(text.match(/^\/(\S*)$/)?.[1] ?? null);
  const suggestions = $derived(
    token === null ? [] : commands.filter((c) => c.command.startsWith(token)).slice(0, 8),
  );
  const current = $derived.by(() => {
    const m = text.match(/^\/(\S+)\s/);
    return m ? (commands.find((c) => c.command === m[1]) ?? null) : null;
  });

  $effect(() => {
    // Keep selection in range as the list changes.
    if (selected >= suggestions.length) selected = 0;
  });

  function usage(c: CommandView) {
    return c.args.map((a) => (a.required ? `<${a.name}>` : `[${a.name}]`)).join(" ");
  }

  function pick(c: CommandView) {
    text = `/${c.command}${c.args.length ? " " : ""}`;
    el?.focus();
  }

  async function send() {
    const t = text.trim();
    if (!t || disabled) return;
    text = "";
    await onsend(t);
    el?.focus();
  }

  function onkeydown(e: KeyboardEvent) {
    if (suggestions.length) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        selected = (selected + 1) % suggestions.length;
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        selected = (selected - 1 + suggestions.length) % suggestions.length;
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey && suggestions[selected].command !== token)) {
        e.preventDefault();
        pick(suggestions[selected]);
        return;
      }
      if (e.key === "Escape") {
        text = "";
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  }

  $effect(() => {
    // Auto-grow up to ~6 lines.
    text;
    if (el) {
      el.style.height = "auto";
      el.style.height = `${Math.min(el.scrollHeight, 140)}px`;
    }
  });
</script>

<div class="composer">
  {#if suggestions.length}
    <ul class="suggest" role="listbox">
      {#each suggestions as c, i}
        <li>
          <button
            class:sel={i === selected}
            onmousedown={(e) => {
              e.preventDefault();
              pick(c);
            }}
          >
            <span class="mono cmd">/{c.command}</span>
            <span class="mono args">{usage(c)}</span>
            <span class="desc">{c.description}</span>
          </button>
        </li>
      {/each}
    </ul>
  {:else if current}
    <div class="hint mono">/{current.command} {usage(current)} <span class="desc">— {current.description}</span></div>
  {/if}

  <div class="box" class:committing>
    <textarea
      bind:this={el}
      bind:value={text}
      {onkeydown}
      rows="1"
      {placeholder}
      {disabled}
    ></textarea>
    <button
      class="btn primary send"
      onclick={send}
      disabled={disabled || !text.trim()}
      aria-label={committing ? "Request as commitment" : "Send"}
      title={committing ? "Request as a signed ACN commitment" : "Send"}
    >
      {#if committing}
        <span class="seal">⚖</span>
      {:else}
        <svg viewBox="0 0 20 20" width="16" height="16"><path d="M3 10l14-7-5 14-2-6-7-1z" fill="currentColor" /></svg>
      {/if}
    </button>
  </div>

  {#if acn}
    <label class="commit" title="Slash commands become signed ACN commitments: the agent must REPORT, and you SATISFY or REJECT.">
      <input type="checkbox" bind:checked={commit} />
      <span>⚖ Request commands as commitments</span>
    </label>
  {/if}

  {#if commands.length && !text}
    <div class="chips">
      {#each commands.slice(0, 6) as c}
        <button class="pill chip mono" onclick={() => pick(c)} title={c.description}>/{c.command}</button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .composer {
    position: relative;
    padding: 10px 16px 14px;
    border-top: 1px solid var(--border);
    background: var(--panel);
  }
  .box {
    display: flex;
    gap: 8px;
    align-items: flex-end;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 6px 6px 6px 12px;
    background: var(--panel);
    transition: border-color 0.12s, box-shadow 0.12s;
  }
  .box:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  textarea {
    flex: 1;
    border: none;
    outline: none;
    resize: none;
    background: transparent;
    padding: 6px 0;
    max-height: 140px;
  }
  .box.committing {
    border-color: var(--agent);
    box-shadow: 0 0 0 3px var(--agent-soft);
  }
  .box.committing .send {
    background: var(--agent);
    border-color: var(--agent);
  }
  .seal {
    font-size: 15px;
    line-height: 1;
  }
  .commit {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 8px;
    font-size: 12px;
    color: var(--muted);
    cursor: pointer;
    user-select: none;
  }
  .commit input {
    accent-color: var(--agent);
  }
  .send {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    padding: 0;
    border-radius: 10px;
  }
  .suggest {
    position: absolute;
    left: 16px;
    right: 16px;
    bottom: calc(100% - 6px);
    margin: 0;
    padding: 4px;
    list-style: none;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    box-shadow: 0 8px 24px rgba(20, 20, 40, 0.12);
    z-index: 5;
  }
  .suggest button {
    display: flex;
    gap: 10px;
    align-items: baseline;
    width: 100%;
    text-align: left;
    border: none;
    background: none;
    padding: 7px 9px;
    border-radius: 7px;
  }
  .suggest button.sel,
  .suggest button:hover {
    background: var(--accent-soft);
  }
  .cmd {
    font-weight: 600;
    color: var(--accent);
  }
  .args {
    font-size: 12px;
    color: var(--muted);
  }
  .desc {
    font-size: 12px;
    color: var(--faint);
    margin-left: auto;
    font-family: inherit;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    font-size: 12px;
    color: var(--muted);
    padding: 0 4px 6px;
    display: flex;
    gap: 6px;
  }
  .hint .desc {
    margin-left: 0;
  }
  .chips {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    padding-top: 8px;
  }
  .chip {
    border: 1px solid var(--border);
    cursor: pointer;
    font-weight: 500;
  }
  .chip:hover {
    color: var(--accent);
    border-color: var(--accent);
  }
</style>
