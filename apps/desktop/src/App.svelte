<script lang="ts">
  import { onMount } from "svelte";
  import { store } from "./lib/store.svelte";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import Conversation from "./lib/components/Conversation.svelte";
  import WalletPanel from "./lib/components/WalletPanel.svelte";
  import Board from "./lib/components/Board.svelte";
  import ActionConfirm from "./lib/components/ActionConfirm.svelte";

  onMount(() => {
    store.init().then(() => store.startTicker());
  });
</script>

<div class="shell">
  <Sidebar />
  <main>
    {#if !store.ready}
      <div class="placeholder muted">{store.error ? "" : "Connecting…"}</div>
    {:else if store.view === "board"}
      <Board />
    {:else if store.activeContact}
      {#key store.activeContact.pubkey}
        <Conversation contact={store.activeContact} />
      {/key}
    {:else}
      <div class="placeholder">
        <img src="/icon.png" alt="" />
        <h1>Welcome to nmesh</h1>
        <p class="muted">
          Chat with people and agents over Nostr. Everything is end-to-end encrypted.
          <br />Paste an <span class="mono">npub</span> on the left to start.
        </p>
      </div>
    {/if}
  </main>

  {#if store.walletOpen}
    <WalletPanel />
  {/if}

  <ActionConfirm />
  {#if store.error}
    <div class="toast" role="alert">{store.error}</div>
  {/if}
</div>

<style>
  .placeholder {
    margin: auto;
    text-align: center;
    max-width: 380px;
    padding: 24px;
  }
  .placeholder img {
    width: 64px;
    height: 64px;
    border-radius: 16px;
  }
  h1 {
    font-size: 20px;
    margin: 14px 0 6px;
  }
  .toast {
    position: fixed;
    bottom: 18px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--text);
    color: var(--panel);
    padding: 9px 14px;
    border-radius: 10px;
    font-size: 13px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.18);
    max-width: 80vw;
    z-index: 30;
  }
</style>
