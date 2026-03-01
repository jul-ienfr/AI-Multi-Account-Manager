<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title?: string;
    children: Snippet;
  }

  let { title = "", children }: Props = $props();
</script>

<div class="settings-section">
  {#if title}
    <h3 class="section-title">{title}</h3>
  {/if}
  <div class="section-content">
    {@render children()}
  </div>
</div>

<style>
  .settings-section {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .section-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .section-content {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
</style>
