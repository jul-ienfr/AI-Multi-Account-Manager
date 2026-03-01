<script lang="ts">
  import { AlertTriangle, RefreshCw } from "lucide-svelte";

  interface Props {
    title?: string;
    message?: string;
    onretry?: () => void;
    retrying?: boolean;
  }

  let { title = "Erreur", message = "Une erreur inattendue s'est produite.", onretry, retrying = false }: Props = $props();
</script>

<div class="error-state">
  <div class="error-icon">
    <AlertTriangle size={36} />
  </div>
  <h3 class="error-title">{title}</h3>
  <p class="error-message">{message}</p>
  {#if onretry}
    <button class="retry-btn" onclick={onretry} disabled={retrying}>
      <RefreshCw size={14} class={retrying ? "spin" : ""} />
      {retrying ? "Nouvelle tentative..." : "Reessayer"}
    </button>
  {/if}
</div>

<style>
  .error-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 40px 24px;
    text-align: center;
  }

  .error-icon {
    display: flex;
    color: var(--status-error);
    opacity: 0.8;
  }

  .error-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .error-message {
    font-size: 13px;
    color: var(--fg-secondary);
    max-width: 360px;
    line-height: 1.5;
  }

  .retry-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    margin-top: 4px;
    font-size: 13px;
    font-weight: 500;
    color: var(--fg-primary);
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .retry-btn:hover:not(:disabled) {
    background: var(--bg-card-hover);
    border-color: var(--accent);
    color: var(--fg-accent);
  }

  .retry-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  :global(.spin) {
    animation: spin 1s linear infinite;
  }
</style>
