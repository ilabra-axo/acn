<script lang="ts">
  import type { RendererHost } from "../types";

  let { payload, host }: { payload: any; host: RendererHost } = $props();

  const lat = $derived(Number(payload.lat).toFixed(5));
  const lng = $derived(Number(payload.lng).toFixed(5));
  const osm = $derived(
    `https://www.openstreetmap.org/?mlat=${payload.lat}&mlon=${payload.lng}#map=17/${payload.lat}/${payload.lng}`,
  );
  const anchor = $derived(payload.spatial?.anchor);
</script>

<div class="loc">
  <div class="map" aria-hidden="true">
    <svg viewBox="0 0 120 80" preserveAspectRatio="xMidYMid slice">
      <defs>
        <pattern id="grid" width="12" height="12" patternUnits="userSpaceOnUse">
          <path d="M12 0H0V12" fill="none" stroke="currentColor" stroke-opacity="0.12" />
        </pattern>
      </defs>
      <rect width="120" height="80" fill="url(#grid)" />
      <path d="M0 55 Q40 40 60 48 T120 30" stroke="currentColor" stroke-opacity="0.25" stroke-width="5" fill="none" />
      <path d="M60 18c-7 0-12 5-12 11 0 9 12 21 12 21s12-12 12-21c0-6-5-11-12-11z" fill="var(--accent)" />
      <circle cx="60" cy="29" r="4.5" fill="white" />
    </svg>
  </div>
  <div class="info">
    <div class="label">{payload.label ?? "Location"}</div>
    <div class="coords mono">{lat}, {lng}{#if payload.accuracy_m} · ±{payload.accuracy_m} m{/if}</div>
    <div class="row">
      <button class="btn" onclick={() => host.openUrl(osm)}>Open map</button>
      {#if anchor}
        <span class="pill" title="Includes an AR anchor (spec 0005). Renders as a placed object in the AR client.">
          AR · {anchor.type === "geo" ? "geospatial anchor" : "cloud anchor"}
        </span>
      {/if}
    </div>
  </div>
</div>

<style>
  .loc {
    display: flex;
    gap: 12px;
    align-items: stretch;
    min-width: 300px;
  }
  .map {
    width: 120px;
    min-height: 80px;
    border-radius: var(--radius-sm);
    background: var(--sunken);
    color: var(--text);
    overflow: hidden;
    flex-shrink: 0;
  }
  .map svg {
    width: 100%;
    height: 100%;
    display: block;
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    justify-content: center;
  }
  .label {
    font-weight: 650;
  }
  .coords {
    color: var(--muted);
    font-size: 12px;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 4px;
  }
</style>
