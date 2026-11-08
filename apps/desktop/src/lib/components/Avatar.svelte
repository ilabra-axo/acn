<script lang="ts">
  import { hue, initials } from "../format";

  let {
    pubkey,
    name,
    agent = false,
    size = 36,
  }: { pubkey: string; name: string; agent?: boolean; size?: number } = $props();

  const h = $derived(hue(pubkey));
</script>

<div
  class="avatar"
  class:agent
  style="--size:{size}px; --h:{h}"
  title={agent ? `${name} (agent)` : name}
>
  <span>{initials(name)}</span>
  {#if agent}
    <svg class="badge" viewBox="0 0 16 16" aria-label="agent">
      <rect x="3" y="5" width="10" height="8" rx="2.5" fill="currentColor" />
      <circle cx="6.3" cy="9" r="1.1" fill="var(--agent-soft)" />
      <circle cx="9.7" cy="9" r="1.1" fill="var(--agent-soft)" />
      <path d="M8 2v3" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
    </svg>
  {/if}
</div>

<style>
  .avatar {
    position: relative;
    width: var(--size);
    height: var(--size);
    border-radius: 50%;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    background: hsl(var(--h) 70% 92%);
    color: hsl(var(--h) 45% 32%);
    font-weight: 650;
    font-size: calc(var(--size) * 0.36);
    letter-spacing: 0.02em;
    user-select: none;
  }
  .avatar.agent {
    border-radius: 30%;
  }
  .badge {
    position: absolute;
    right: -3px;
    bottom: -3px;
    width: calc(var(--size) * 0.5);
    height: calc(var(--size) * 0.5);
    padding: 2px;
    border-radius: 50%;
    background: var(--agent-soft);
    color: var(--agent);
    border: 1.5px solid var(--panel);
  }
  @media (prefers-color-scheme: dark) {
    .avatar {
      background: hsl(var(--h) 35% 24%);
      color: hsl(var(--h) 70% 82%);
    }
  }
</style>
