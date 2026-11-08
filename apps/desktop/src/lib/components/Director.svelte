<script lang="ts">
  import { store } from "../store.svelte";
  import type { Contact } from "../types";
  import { displayName } from "../format";

  let { contact }: { contact: Contact } = $props();

  let brief = $state("");
  const tools = $derived(contact.acn?.tools ?? []);
  const focused = $derived(store.focus ? store.commitments[store.focus] : null);
  const action = $derived(store.hint?.action.type);
  const canNext = $derived(!!action && action !== "wait" && action !== "done");

  // Two-step confirm (no native dialogs: they're unreliable in the Tauri webview).
  let armed = $state(false);
  let armTimer: number | undefined;
  async function reset() {
    if (!armed) {
      armed = true;
      armTimer = window.setTimeout(() => (armed = false), 4000);
      return;
    }
    clearTimeout(armTimer);
    armed = false;
    await store.resetDemo();
  }

  function fmtOffset(s: number) {
    if (!s) return "live";
    if (s < 3600) return `+${Math.round(s / 60)} min`;
    if (s < 86400) return `+${(s / 3600).toFixed(1)} h`;
    return `+${(s / 86400).toFixed(1)} d`;
  }
</script>

<aside class="director" aria-label="Demo director">
  <header>
    <span class="clap">🎬</span>
    <div>
      <div class="title">Director</div>
      <div class="sub muted">{store.backend?.kind === "presenter" ? "Presenter mode · in-browser, real validator (WASM)" : "Live · real relay & signed events"}</div>
    </div>
    <button class="x" onclick={() => (store.directorOpen = false)} aria-label="Close director">×</button>
  </header>

  <section>
    <div class="h">Scenarios · {displayName(contact)}</div>
    <input class="field" placeholder="Brief (optional), e.g. a landing page" bind:value={brief} />
    <ul class="scenarios">
      {#each tools as t}
        <li>
          <button class="scn" onclick={() => store.startScenario(t.name, brief.trim())}>
            <span class="row1">
              <span class="name">{t.title}</span>
              <span class="pill">{t.profile}</span>
              {#if t.price_sat}<span class="pill sat">{t.price_sat} sat</span>{/if}
            </span>
            <span class="desc muted">{t.description}</span>
            {#if t.shows.length}<span class="shows">{#each t.shows as s}<span>{s}</span>{/each}</span>{/if}
          </button>
        </li>
      {:else}
        <li class="empty">
          <p class="muted">{displayName(contact)} doesn't offer ACN scenarios. The demo cast (Atlas, Rogue) does.</p>
          {#if store.backend?.kind === "tauri"}
            <p class="muted">Start it in the repo root, against the same relay:</p>
            <code>./target/debug/nmesh-acn-sim</code>
          {/if}
          <button class="btn primary" onclick={() => store.openDemo()} disabled={store.demoSearching}>
            {store.demoSearching ? "Looking…" : "Find the demo cast"}
          </button>
        </li>
      {/each}
    </ul>
  </section>

  <section class="now">
    <div class="h">Now</div>
    {#if focused}
      <div class="focus">
        <span class="mono">/{focused.tool}</span>
        <span class="muted">· {focused.phase ?? "pending"}</span>
      </div>
      <p class="narration">{store.hint?.narration ?? "…"}</p>
      <div class="controls">
        <button class="btn primary" disabled={!canNext} onclick={() => store.next()}>
          {canNext ? `▶ ${store.hint?.label}` : action === "done" ? "✓ Done" : "Waiting…"}
        </button>
        <label class="auto"><input type="checkbox" bind:checked={store.autopilot} /> Autopilot</label>
      </div>
    {:else}
      <p class="muted narration">Pick a scenario. You play the requester; the cast reacts with real signed acts.</p>
    {/if}
  </section>

  <section>
    <div class="h">Balances <button class="link" onclick={() => (store.walletOpen = true)}>open wallet</button></div>
    {#if store.wallet}
      <div class="bal"><span>You</span><span class="mono">{store.wallet.available} sat</span>
        {#if store.wallet.locked_out}<span class="muted">· {store.wallet.locked_out} locked</span>{/if}</div>
      {#each store.castWallets as cw}
        <div class="bal"><span>{cw.name}</span><span class="mono">{cw.wallet.available} sat</span>
          {#if cw.wallet.incoming}<span class="muted">· {cw.wallet.incoming} incoming</span>{/if}</div>
      {/each}
      <p class="fine muted">{store.wallet.simulated ? "Simulated mint in the browser (NUT-14 rules)." : "Real ecash at the cast's CDK test mint. Atlas DMs you when it redeems."}</p>
    {:else}
      <p class="fine muted">No wallet configured: paid scenarios will be refused.</p>
    {/if}
  </section>

  <section>
    <div class="h">Presentation clock <span class="offset mono">{fmtOffset(store.clock.offset)}</span></div>
    <div class="clock">
      <button class="btn" onclick={() => store.advanceClock(60)}>+1 min</button>
      <button class="btn" onclick={() => store.advanceClock(600)}>+10 min</button>
      <button class="btn" onclick={() => store.advanceClock(3600)}>+1 h</button>
      <button class="btn" onclick={() => store.resetClock()} disabled={!store.clock.offset}>Reset</button>
    </div>
    <p class="fine muted">Moves the clock used for deadlines and lapses (<span class="mono">status_at</span>). Signing always uses real time.</p>
  </section>

  <section>
    <div class="h">Reset</div>
    <button class="btn reset" class:armed onclick={reset}>{armed ? "Click again to clear everything" : "↺ Reset demo"}</button>
    <p class="fine muted">
      Clears all commitments, stops autopilot and resets the clock.
      {#if store.backend?.kind === "presenter"}The simulated mint goes back to 100 sat.{:else}Signed acts stay on the relay but are ignored from now on. Your wallet (real test ecash) is kept.{/if}
    </p>
  </section>

  <section class="legend">
    <div class="h">Legend</div>
    <div><span class="dot you"></span> your acts <span class="dot perf"></span> performer <span class="dot third"></span> verifier / judge / arbiter / gate</div>
  </section>
</aside>

<style>
  .director {
    width: 320px; flex-shrink: 0; border-left: 1px solid var(--border); background: var(--panel);
    display: flex; flex-direction: column; overflow-y: auto;
  }
  header { display: flex; gap: 10px; align-items: center; padding: 12px 14px; border-bottom: 1px solid var(--border); }
  .clap { font-size: 20px; }
  .title { font-weight: 700; }
  .sub { font-size: 11px; }
  .x { margin-left: auto; border: none; background: none; font-size: 20px; color: var(--faint); }
  section { padding: 12px 14px; border-bottom: 1px solid var(--border); display: flex; flex-direction: column; gap: 8px; }
  .h { font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.05em; color: var(--muted); display: flex; justify-content: space-between; }
  .scenarios { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .scn {
    width: 100%; text-align: left; border: 1px solid var(--border); background: var(--panel); border-radius: var(--radius-sm);
    padding: 8px 10px; display: flex; flex-direction: column; gap: 3px;
  }
  .scn:hover { border-color: var(--accent); background: var(--accent-soft); }
  .row1 { display: flex; gap: 6px; align-items: center; }
  .name { font-weight: 650; flex: 1; }
  .sat { color: #c2410c; }
  .desc { font-size: 12px; line-height: 1.35; }
  .shows { display: flex; flex-wrap: wrap; gap: 4px; }
  .shows span { font-size: 10px; padding: 0 6px; border-radius: 999px; background: var(--sunken); color: var(--faint); font-family: var(--mono); }
  .empty { font-size: 12px; display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .empty p { margin: 0; }
  .empty code { font-family: var(--mono); font-size: 11.5px; background: var(--sunken); padding: 4px 7px; border-radius: 6px; }
  .now { background: var(--agent-soft); }
  .focus { font-size: 13px; }
  .narration { margin: 0; font-size: 13.5px; line-height: 1.45; }
  .controls { display: flex; gap: 10px; align-items: center; }
  .auto { font-size: 12px; display: flex; gap: 5px; align-items: center; color: var(--muted); }
  .auto input { accent-color: var(--agent); }
  .clock { display: flex; gap: 6px; flex-wrap: wrap; }
  .clock .btn { padding: 5px 9px; font-size: 12px; }
  .offset { color: var(--agent); text-transform: none; letter-spacing: 0; }
  .fine { font-size: 11px; margin: 0; }
  .reset { align-self: flex-start; }
  .reset.armed { border-color: var(--danger); color: var(--danger); font-weight: 600; }
  .bal { display: flex; gap: 8px; font-size: 13px; align-items: baseline; }
  .bal span:first-child { font-weight: 600; min-width: 48px; }
  .link { border: none; background: none; padding: 0; color: var(--accent); font-size: 11px; text-transform: none; letter-spacing: 0; font-weight: 600; }
  .legend div:last-child { font-size: 12px; display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  .dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
  .dot.you { background: var(--accent); }
  .dot.perf { background: var(--agent); }
  .dot.third { background: #8b5cf6; }
</style>
