<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    color?: string;
    small?: boolean;
    children: Snippet;
  }

  let { color = "var(--accent)", small = false, children }: Props = $props();
</script>

<span class="badge" class:small style="--badge-color: {color}">
  {@render children()}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    font-size: 11px;
    font-weight: 600;
    line-height: 1.4;
    border-radius: 9999px;
    background: color-mix(in srgb, var(--badge-color) 15%, transparent);
    color: var(--badge-color);
    white-space: nowrap;
    user-select: none;
  }

  .badge.small {
    padding: 1px 6px;
    font-size: 10px;
  }
</style>
