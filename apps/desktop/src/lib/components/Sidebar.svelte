<script lang="ts">
  import { store } from "../store.svelte";
  import { displayName, shortNpub, time } from "../format";
  import Avatar from "./Avatar.svelte";

  let input = $state("");
  let adding = $state(false);
  let copied = $state(false);

  async function add(e: SubmitEvent) {
    e.preventDefault();
    adding = true;
    if (await store.addContact(input)) input = "";
    adding = false;
  }

  async function copyNpub() {
    if (!store.me) return;
    await navigator.clipboard?.writeText(store.me.npub);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<aside>
  <header>
    <div class="brand">
      <img src="/icon.png" alt="" />
      <span>nmesh</span>
      {#if store.backend?.kind === "presenter"}<span class="pill" title="Presenter mode: in-browser, ACN via the real validator compiled to WASM">presenter</span>{/if}
    </div>
    {#if store.me}
      <button class="me" onclick={copyNpub} title="Copy your npub">
        <span class="mono">{copied ? "copied ✓" : shortNpub(store.me.npub)}</span>
      </button>
    {/if}
  </header>

  <div class="views" role="tablist">
    <button role="tab" aria-selected={store.view === "chats"} class:on={store.view === "chats"} onclick={() => store.showView("chats")}>Chats</button>
    <button role="tab" aria-selected={store.view === "board"} class:on={store.view === "board"} onclick={() => store.showView("board")}>
      Board{#if store.board?.items.some((c) => c.needs_me)}<span class="dot-me" title="Something needs you"></span>{/if}
    </button>
  </div>

  <form class="add" onsubmit={add}>
    <input class="field" placeholder="Start a chat: paste an npub" bind:value={input} disabled={adding} />
  </form>
  <div class="demo">
    <button class="btn demo-btn" onclick={() => store.openDemo()} disabled={store.demoSearching}>
      {store.demoSearching ? "Looking for the demo cast…" : "🎬 ACN demo"}
    </button>
    <button class="btn wallet-btn" onclick={() => (store.walletOpen = true)} title="Wallet">
      <span class="coin">₿</span>
      {#if store.wallet}
        <span class="bal">{store.wallet.available} sat</span>
        {#if store.wallet.locked_out}<span class="muted">· {store.wallet.locked_out} locked</span>{/if}
        <span class="pill">{store.wallet.simulated ? "simulated" : "test sats"}</span>
      {:else}
        <span class="muted">No wallet</span>
      {/if}
    </button>
  </div>

  <nav>
    {#each store.contacts as c (c.pubkey)}
      <button class="item" class:active={store.view === "chats" && store.active === c.pubkey} onclick={() => { store.showView("chats"); store.open(c.pubkey); }}>
        <Avatar pubkey={c.pubkey} name={displayName(c)} agent={c.is_agent} />
        <div class="meta">
          <div class="top">
            <span class="name">{displayName(c)}</span>
            <span class="when">{time(c.last_at)}</span>
          </div>
          <div class="bottom">
            <span class="last">{c.last_message ?? (c.is_agent ? c.about ?? "Agent" : "No messages yet")}</span>
            {#if c.unread > 0}<span class="unread">{c.unread}</span>{/if}
          </div>
        </div>
      </button>
    {:else}
      <div class="empty muted">
        No conversations yet. Paste someone's <span class="mono">npub</span> above — a friend or an agent.
      </div>
    {/each}
  </nav>

  {#if store.me}
    <footer class="muted">
      {store.me.relays.length} relay{store.me.relays.length === 1 ? "" : "s"} ·
      <span class="mono" title={store.me.relays.join("\n")}>{store.me.relays[0]}</span>
    </footer>
  {/if}
</aside>

<style>
  aside {
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border-right: 1px solid var(--border);
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 14px 10px;
    gap: 8px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 700;
    font-size: 16px;
    letter-spacing: -0.01em;
  }
  .brand img {
    width: 22px;
    height: 22px;
    border-radius: 6px;
  }
  .me {
    border: 1px solid var(--border);
    background: var(--sunken);
    border-radius: 999px;
    padding: 3px 9px;
    font-size: 11px;
    color: var(--muted);
  }
  .add {
    padding: 0 12px 8px;
  }
  .views {
    display: grid;
    grid-template-columns: 1fr 1fr;
    margin: 0 12px 10px;
    padding: 3px;
    gap: 3px;
    background: var(--sunken);
    border-radius: 10px;
  }
  .views button {
    border: none;
    background: none;
    border-radius: 8px;
    padding: 5px 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--muted);
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
  }
  .views button.on {
    background: var(--panel);
    color: var(--text);
    box-shadow: var(--shadow);
  }
  .dot-me {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
  }
  .demo {
    padding: 0 12px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .wallet-btn {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  .wallet-btn .coin {
    color: #c2410c;
    font-weight: 700;
  }
  .wallet-btn .bal {
    font-weight: 650;
    font-variant-numeric: tabular-nums;
  }
  .wallet-btn .pill {
    margin-left: auto;
  }
  .demo-btn {
    width: 100%;
    background: var(--agent-soft);
    border-color: var(--agent-border);
    color: var(--agent);
    font-weight: 600;
  }
  nav {
    flex: 1;
    overflow-y: auto;
    padding: 0 6px 8px;
  }
  .item {
    display: flex;
    gap: 10px;
    align-items: center;
    width: 100%;
    text-align: left;
    border: none;
    background: none;
    padding: 9px 8px;
    border-radius: 10px;
  }
  .item:hover {
    background: var(--sunken);
  }
  .item.active {
    background: var(--accent-soft);
  }
  .meta {
    min-width: 0;
    flex: 1;
  }
  .top,
  .bottom {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    align-items: center;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    font-size: 11px;
    color: var(--faint);
    flex-shrink: 0;
  }
  .last {
    font-size: 13px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .unread {
    background: var(--accent);
    color: var(--accent-text);
    border-radius: 999px;
    font-size: 11px;
    font-weight: 700;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    display: grid;
    place-items: center;
  }
  .empty {
    padding: 24px 12px;
    font-size: 13px;
    text-align: center;
  }
  footer {
    font-size: 11px;
    padding: 8px 14px;
    border-top: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
