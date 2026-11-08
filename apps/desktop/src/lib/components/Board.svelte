<script lang="ts">
  import { store } from "../store.svelte";
  import type { Commitment, Lane, RendererHost } from "../types";
  import BoardTile from "./BoardTile.svelte";
  import CommitmentCard from "./CommitmentCard.svelte";

  // Columns follow the ACN conversation for action (Core.md §6). Tiles move only when signed
  // acts arrive; there is deliberately no drag-and-drop.
  const LANES: { id: Lane; title: string; acts: string }[] = [
    { id: "requested", title: "Requested", acts: "REQUEST" },
    { id: "negotiating", title: "Negotiating", acts: "COUNTER · ACCEPT" },
    { id: "in_progress", title: "In progress", acts: "PROMISE · PROGRESS · rework" },
    { id: "review", title: "Review", acts: "REPORT → SATISFY / REJECT" },
    { id: "dispute", title: "Dispute", acts: "DISPUTE → RULING" },
    { id: "closed", title: "Closed", acts: "satisfied · declined · withdrawn · rejected" },
  ];

  const items = $derived(store.board?.items ?? []);
  const shown = $derived(store.needsMeOnly ? items.filter((c) => c.needs_me) : items);
  const needsMe = $derived(items.filter((c) => c.needs_me).length);
  const byLane = $derived(
    Object.fromEntries(LANES.map((l) => [l.id, shown.filter((c) => c.lane === l.id)])) as Record<Lane, Commitment[]>,
  );
  const closedMore = $derived(Math.max(0, (store.board?.closed_total ?? 0) - byLane.closed.length));
  const open = $derived(store.boardOpen ? items.find((c) => c.id === store.boardOpen) ?? null : null);

  // Result renderers in the drawer: buttons and forms belong to the conversation.
  const host: RendererHost = {
    action: async () => { if (open) await store.openInChat(open.id); },
    submit: async () => { if (open) await store.openInChat(open.id); },
    openUrl: (url) => store.openUrl(url),
  };
</script>

<section class="board">
  <header>
    <div class="title">
      <h2>Board</h2>
      <span class="muted">Every commitment, across agents and roles. Cards move when signed acts arrive.</span>
    </div>
    <div class="tools">
      <button class="btn filter" class:on={store.needsMeOnly} onclick={() => (store.needsMeOnly = !store.needsMeOnly)}>
        Needs you{#if needsMe}<span class="count">{needsMe}</span>{/if}
      </button>
    </div>
  </header>

  {#if !items.length}
    <div class="empty">
      <p>No commitments yet.</p>
      <p class="muted">Start one from a chat, or open <strong>🎬 ACN demo</strong> and run a scenario.</p>
    </div>
  {:else}
    <div class="lanes">
      {#each LANES as lane (lane.id)}
        <div class="lane" data-lane={lane.id}>
          <div class="lane-h">
            <span class="name">{lane.title}</span>
            <span class="n">{lane.id === "closed" ? store.board?.closed_total ?? 0 : byLane[lane.id].length}</span>
          </div>
          <div class="acts mono">{lane.acts}</div>
          <div class="tiles">
            {#each byLane[lane.id] as c (c.id)}
              <BoardTile {c} />
            {:else}
              <div class="none">—</div>
            {/each}
            {#if lane.id === "closed" && closedMore > 0 && !store.needsMeOnly}
              <button class="btn older" onclick={() => store.showOlder()}>Show older ({closedMore} more)</button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</section>

{#if open}
  <div class="backdrop" role="presentation" onclick={() => (store.boardOpen = null)}></div>
  <aside class="drawer" aria-label="Commitment">
    <header>
      <span class="mono">/{open.tool ?? "request"}</span>
      <button class="btn link" onclick={() => store.openInChat(open.id)}>Open in chat →</button>
      <button class="x" onclick={() => (store.boardOpen = null)} aria-label="Close">×</button>
    </header>
    <div class="card">
      {#key open.id}
        <CommitmentCard commitment={open} {host} />
      {/key}
    </div>
  </aside>
{/if}

<style>
  .board { display: flex; flex-direction: column; min-height: 0; height: 100%; background: var(--bg); }
  header { display: flex; align-items: center; gap: 12px; padding: 14px 18px 10px; }
  .title { display: flex; align-items: baseline; gap: 12px; flex-wrap: wrap; }
  h2 { margin: 0; font-size: 17px; }
  .title .muted { font-size: 12px; }
  .tools { margin-left: auto; display: flex; gap: 8px; }
  .filter { display: flex; align-items: center; gap: 6px; font-size: 12.5px; }
  .filter.on { background: var(--accent-soft); border-color: var(--accent); color: var(--accent); font-weight: 600; }
  .count { background: var(--accent); color: var(--accent-text); border-radius: 999px; font-size: 10.5px; font-weight: 700; padding: 0 6px; }
  .lanes {
    flex: 1; min-height: 0; display: grid; grid-template-columns: repeat(6, minmax(200px, 1fr));
    gap: 10px; padding: 4px 18px 18px; overflow-x: auto;
  }
  .lane { display: flex; flex-direction: column; min-height: 0; background: var(--sunken); border-radius: var(--radius); padding: 10px 8px 8px; }
  .lane-h { display: flex; align-items: center; gap: 6px; padding: 0 4px; }
  .lane-h .name { font-weight: 700; font-size: 13px; }
  .lane-h .n { font-size: 11px; color: var(--faint); background: var(--panel); border-radius: 999px; padding: 0 7px; }
  .lane[data-lane="review"] .lane-h .name { color: var(--accent); }
  .lane[data-lane="dispute"] .lane-h .name { color: var(--agent); }
  .acts { font-size: 9.5px; color: var(--faint); padding: 2px 4px 8px; letter-spacing: 0.02em; }
  .tiles { display: flex; flex-direction: column; gap: 8px; overflow-y: auto; min-height: 0; padding: 2px; }
  .none { text-align: center; color: var(--faint); font-size: 12px; padding: 10px 0; }
  .older { font-size: 12px; align-self: center; }
  .empty { margin: auto; text-align: center; }
  .empty p { margin: 4px 0; }
  .backdrop { position: fixed; inset: 0; background: rgba(10, 10, 20, 0.18); z-index: 20; }
  .drawer {
    position: fixed; top: 0; right: 0; bottom: 0; width: 560px; max-width: 94vw; z-index: 21;
    background: var(--bg); border-left: 1px solid var(--border); box-shadow: -8px 0 30px rgba(0, 0, 0, 0.12);
    display: flex; flex-direction: column;
  }
  .drawer header { background: var(--panel); border-bottom: 1px solid var(--border); padding: 10px 14px; }
  .drawer .mono { font-weight: 700; }
  .link { margin-left: auto; font-size: 12px; }
  .x { border: none; background: none; font-size: 20px; color: var(--faint); }
  .card { overflow-y: auto; padding: 14px; }
</style>
