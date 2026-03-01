<script lang="ts">
  import type { Snippet } from "svelte";
  import { X } from "lucide-svelte";

  interface Props {
    open?: boolean;
    title?: string;
    onclose?: () => void;
    children: Snippet;
    actions?: Snippet;
  }

  let { open = $bindable(false), title = "", onclose, children, actions }: Props = $props();

  function handleBackdrop() {
    open = false;
    onclose?.();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") handleBackdrop();
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog-backdrop" onclick={handleBackdrop} onkeydown={handleKeydown}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dialog-content" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
      <header class="dialog-header">
        <h2 class="dialog-title">{title}</h2>
        <button class="dialog-close" onclick={handleBackdrop} aria-label="Fermer">
          <X size={18} />
        </button>
      </header>
      <div class="dialog-body">
        {@render children()}
      </div>
      {#if actions}
        <footer class="dialog-actions">
          {@render actions()}
        </footer>
      {/if}
    </div>
  </div>
{/if}

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(4px);
    animation: fade-in 0.15s ease;
  }

  .dialog-content {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-xl);
    width: 90%;
    max-width: 520px;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: fade-in 0.2s ease;
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border);
  }

  .dialog-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .dialog-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    color: var(--fg-dim);
    cursor: pointer;
    background: none;
    border: none;
  }

  .dialog-close:hover {
    background: var(--bg-card-hover);
    color: var(--fg-primary);
  }

  .dialog-body {
    padding: 20px;
    overflow-y: auto;
    flex: 1;
  }

  .dialog-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px;
    border-top: 1px solid var(--border);
  }
</style>
