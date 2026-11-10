<script lang="ts">
  import { store } from "../store.svelte";
  import type { WalletLock } from "../types";
  import { displayName, time } from "../format";

  const w = $derived(store.wallet);
  const me = $derived(store.me?.pubkey ?? "");
  const presenter = $derived(store.backend?.kind === "presenter");
  // The mint enforces locktimes with its own clock; presenter mode uses the presentation clock.
  const nowSecs = $derived(presenter ? store.clock.now : Math.floor(Date.now() / 1000));

  function who(pk: string) {
    if (pk === me) return "you";
    const c = store.contacts.find((x) => x.pubkey === pk);
    return c ? displayName(c) : `${pk.slice(0, 8)}…`;
  }

  function lockLabel(l: WalletLock) {
    const out = l.refund === me;
    switch (l.state) {
      case "locked":
        return out ? `🔒 locked to ${who(l.receiver)}` : `⏳ incoming from ${who(l.refund)}`;
      case "redeemed":
        return out ? `✓ redeemed by ${who(l.receiver)}` : "✓ redeemed";
      case "refunded":
        return out ? "↩ refunded to you" : `↩ refunded to ${who(l.refund)}`;
    }
  }

  /** Refund is possible only for my own lock on a commitment that closed unsatisfied, or on one
   *  whose request was never published (no commitment): the backend checks which. */
  function refundable(l: WalletLock) {
    if (l.state !== "locked" || l.refund !== me) return false;
    if (!l.commitment) return true;
    const c = store.commitments[l.commitment];
    // Not on screen: offer it once the locktime passed; the backend checks the commitment.
    return c ? c.settlement?.status === "refundable" : nowSecs > l.locktime;
  }

  const KIND: Record<string, string> = { top_up: "Top up", lock: "Lock", redeem: "Redeem", refund: "Refund" };
</script>

<div class="backdrop" role="presentation" onclick={() => (store.walletOpen = false)}></div>
<aside class="wallet" aria-label="Wallet">
  <header>
    <span class="coin">₿</span>
    <div>
      <div class="title">Wallet</div>
      {#if w}
        <div class="sub muted mono" title={w.mint}>{w.mint}</div>
      {/if}
    </div>
    <button class="x" onclick={() => (store.walletOpen = false)} aria-label="Close wallet">×</button>
  </header>

  {#if !w}
    <section>
      <p class="muted">No wallet is configured. Start the app with <span class="mono">NMESH_MINT</span> set to a Cashu mint URL
        (the demo cast hosts a test mint on <span class="mono">http://127.0.0.1:3338</span>).</p>
    </section>
  {:else}
    <section class="kind">
      {#if w.simulated}
        <span class="pill sim">simulated mint</span>
        <span class="muted">In-browser mint with the same rules as Cashu HTLCs (NUT-14). No real ecash.</span>
      {:else}
        <span class="pill test">test sats</span>
        <span class="muted">Real Cashu ecash at a CDK test mint whose Lightning backend is fake. Never use real funds.</span>
      {/if}
    </section>

    <section class="balances">
      <div><span class="n">{w.available}</span><span class="l">available</span></div>
      <div><span class="n">{w.locked_out}</span><span class="l">locked to performers</span></div>
      <div><span class="n">{w.incoming}</span><span class="l">incoming</span></div>
    </section>

    <section class="topup">
      <button class="btn primary" disabled={store.walletBusy} onclick={() => store.topUp(100)}>Top up 100 test sats</button>
      <button class="btn" disabled={store.walletBusy} onclick={() => store.topUp(21)}>+21</button>
      {#if store.walletBusy}<span class="muted">…</span>{/if}
    </section>

    {#if store.castWallets.length}
      <section>
        <div class="h">Cast</div>
        {#each store.castWallets as cw}
          <div class="cast"><span>{cw.name}</span><span class="mono">{cw.wallet.available} sat</span>
            {#if cw.wallet.incoming}<span class="muted">· {cw.wallet.incoming} incoming</span>{/if}</div>
        {/each}
      </section>
    {:else if !presenter}
      <section><p class="fine muted">Atlas runs in its own process with its own wallet; it tells you by DM when it redeems. Send it <span class="mono">wallet</span> to see its balance.</p></section>
    {/if}

    <section>
      <div class="h">Locks</div>
      {#each [...w.locks].reverse() as l (l.id)}
        <div class="lock" data-s={l.state}>
          <div class="row">
            <span class="amt mono">{l.amount} sat</span>
            <span>{lockLabel(l)}</span>
          </div>
          <div class="row fine muted">
            {#if l.commitment}<span class="mono" title={l.commitment}>commitment {l.commitment.slice(0, 8)}…</span>{/if}
            <span>{l.state === "locked" ? `refund path opens ${new Date(l.locktime * 1000).toLocaleString()}` : `settled ${time(l.settled_at)}`}</span>
          </div>
          {#if refundable(l)}
            <button class="btn" disabled={store.walletBusy || nowSecs <= l.locktime}
              title={nowSecs <= l.locktime ? (presenter ? "Fast-forward the presentation clock past the locktime" : "The mint only allows the refund after the locktime") : "Reclaim with your refund key (NUT-14)"}
              onclick={() => store.refund(l.receiver, l.commitment ?? l.id)}>
              {nowSecs <= l.locktime ? "Refund after locktime" : "↩ Refund"}
            </button>
          {/if}
        </div>
      {:else}
        <p class="fine muted">No payments yet. Paid scenarios (/paid, /dispute) lock sats to the performer.</p>
      {/each}
    </section>

    <section>
      <div class="h">History</div>
      {#each [...w.history].reverse() as e}
        <div class="entry">
          <span class="when muted">{time(e.at)}</span>
          <span class="k">{KIND[e.kind] ?? e.kind}</span>
          <span class="note muted">{e.note}</span>
          <span class="delta mono" class:neg={e.delta < 0}>{e.delta > 0 ? "+" : ""}{e.delta}</span>
        </div>
      {:else}
        <p class="fine muted">Nothing yet.</p>
      {/each}
    </section>
  {/if}
</aside>

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(10, 10, 20, 0.18); z-index: 20; }
  .wallet {
    position: fixed; top: 0; right: 0; bottom: 0; width: 380px; max-width: 92vw; z-index: 21;
    background: var(--panel); border-left: 1px solid var(--border); box-shadow: -8px 0 30px rgba(0, 0, 0, 0.12);
    display: flex; flex-direction: column; overflow-y: auto;
  }
  header { display: flex; gap: 10px; align-items: center; padding: 12px 14px; border-bottom: 1px solid var(--border); }
  .coin { font-size: 20px; color: #c2410c; }
  .title { font-weight: 700; }
  .sub { font-size: 11px; max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .x { margin-left: auto; border: none; background: none; font-size: 20px; color: var(--faint); }
  section { padding: 12px 14px; border-bottom: 1px solid var(--border); display: flex; flex-direction: column; gap: 8px; }
  section p { margin: 0; font-size: 13px; }
  .h { font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.05em; color: var(--muted); }
  .kind { flex-direction: row; align-items: baseline; gap: 8px; font-size: 12px; }
  .pill { white-space: nowrap; }
  .pill.test { background: #dcfce7; color: #166534; }
  .balances { flex-direction: row; justify-content: space-between; }
  .balances div { display: flex; flex-direction: column; align-items: flex-start; }
  .balances .n { font-size: 22px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .balances .l { font-size: 11px; color: var(--muted); }
  .topup { flex-direction: row; align-items: center; }
  .cast { display: flex; gap: 8px; font-size: 13px; }
  .cast span:first-child { font-weight: 600; min-width: 60px; }
  .lock { border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 8px 10px; display: flex; flex-direction: column; gap: 4px; }
  .lock[data-s="redeemed"] { border-color: #bbf7d0; }
  .lock[data-s="refunded"] { border-color: var(--agent-border); }
  .lock .btn { align-self: flex-start; padding: 4px 10px; font-size: 12px; }
  .row { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; font-size: 13px; }
  .amt { font-weight: 700; }
  .fine { font-size: 11px; }
  .entry { display: grid; grid-template-columns: 44px 58px 1fr auto; gap: 6px; font-size: 12.5px; align-items: baseline; }
  .note { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .delta { color: var(--ok); font-weight: 600; }
  .delta.neg { color: var(--danger); }
</style>
