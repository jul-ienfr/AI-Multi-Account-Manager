<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    hoverable?: boolean;
    active?: boolean;
    padding?: string;
    onclick?: (e: MouseEvent) => void;
    children: Snippet;
  }

  let { hoverable = true, active = false, padding = "16px", onclick, children }: Props = $props();
</script>

{#if onclick}
  <button
    class="card"
    class:hoverable
    class:active
    style="padding: {padding}"
    {onclick}
  >
    {@render children()}
  </button>
{:else}
  <div
    class="card"
    class:hoverable
    class:active
    style="padding: {padding}"
  >
    {@render children()}
  </div>
{/if}

<style>
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    backdrop-filter: blur(8px);
    transition: all 0.2s ease;
    width: 100%;
    text-align: left;
  }

  .card.hoverable:hover {
    background: var(--bg-card-hover);
    border-color: var(--border-hover);
    transform: translateY(-1px);
  }

  .card.active {
    background: var(--bg-card-active);
    border-color: var(--accent);
    box-shadow: 0 0 20px var(--accent-glow);
  }
</style>
