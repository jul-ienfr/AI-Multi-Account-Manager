<script lang="ts">
  import { toasts } from "../lib/stores/toast";
  import { toast } from "../lib/stores/toast";
  import { X, Info, CheckCircle, AlertTriangle, AlertCircle, Repeat } from "lucide-svelte";
  import { onMount } from "svelte";
  import type { Toast as ToastType } from "../lib/types";

  let items: ToastType[] = $state([]);

  onMount(() => {
    const unsub = toasts.subscribe(t => { items = t; });
    return unsub;
  });

  const icons: Record<string, any> = {
    info: Info,
    success: CheckCircle,
    warning: AlertTriangle,
    error: AlertCircle,
    switch: Repeat,
  } as const;

  const colors = {
    info: "var(--accent)",
    success: "var(--phase-cruise)",
    warning: "var(--status-warning)",
    error: "var(--status-error)",
    switch: "var(--provider-xai)",
  } as const;
</script>

{#if items.length > 0}
  <div class="toast-container">
    {#each items as item (item.id)}
      {@const Icon = icons[item.type]}
      <div
        class="toast-item toast-{item.type}"
        style="--toast-color: {colors[item.type]}"
      >
        <span class="toast-icon">
          <Icon size={16} />
        </span>
        <div class="toast-content">
          <span class="toast-title">{item.title}</span>
          {#if item.message}
            <span class="toast-message">{item.message}</span>
          {/if}
        </div>
        <button class="toast-close" onclick={() => toast.remove(item.id)} aria-label="Fermer">
          <X size={14} />
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-container {
    position: fixed;
    bottom: 48px;
    right: 16px;
    z-index: 200;
    display: flex;
    flex-direction: column-reverse;
    gap: 8px;
    max-width: 380px;
    pointer-events: none;
  }

  .toast-item {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px 14px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-left: 3px solid var(--toast-color);
    border-radius: var(--radius-md);
    backdrop-filter: blur(12px);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    animation: slide-in-right 0.25s ease;
    pointer-events: all;
  }

  .toast-icon {
    display: flex;
    align-items: center;
    color: var(--toast-color);
    flex-shrink: 0;
    margin-top: 1px;
  }

  .toast-content {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }

  .toast-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .toast-message {
    font-size: 12px;
    color: var(--fg-secondary);
    line-height: 1.4;
  }

  .toast-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-sm);
    color: var(--fg-dim);
    cursor: pointer;
    background: none;
    border: none;
    flex-shrink: 0;
  }

  .toast-close:hover {
    background: var(--bg-card-hover);
    color: var(--fg-primary);
  }
</style>
