<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    variant?: "primary" | "secondary" | "ghost";
    size?: "sm" | "md" | "lg";
    disabled?: boolean;
    onclick?: (e: MouseEvent) => void;
    children: Snippet;
  }

  let { variant = "primary", size = "md", disabled = false, onclick, children }: Props = $props();
</script>

<button
  class="btn btn-{variant} btn-{size}"
  {disabled}
  {onclick}
>
  {@render children()}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font-weight: 500;
    border-radius: var(--radius-md);
    white-space: nowrap;
    user-select: none;
    transition: all 0.15s ease;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Sizes */
  .btn-sm { padding: 4px 10px; font-size: 12px; height: 28px; }
  .btn-md { padding: 6px 14px; font-size: 13px; height: 34px; }
  .btn-lg { padding: 8px 18px; font-size: 14px; height: 40px; }

  /* Variants */
  .btn-primary {
    background: var(--accent);
    color: #fff;
  }
  .btn-primary:hover:not(:disabled) {
    background: var(--accent-dim);
    box-shadow: 0 0 16px var(--accent-glow);
  }

  .btn-secondary {
    background: var(--bg-card);
    color: var(--fg-primary);
    border: 1px solid var(--border);
  }
  .btn-secondary:hover:not(:disabled) {
    background: var(--bg-card-hover);
    border-color: var(--border-hover);
  }

  .btn-ghost {
    background: transparent;
    color: var(--fg-secondary);
  }
  .btn-ghost:hover:not(:disabled) {
    background: var(--bg-card-hover);
    color: var(--fg-primary);
  }
</style>
