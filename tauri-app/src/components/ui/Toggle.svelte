<script lang="ts">
  interface Props {
    checked?: boolean;
    onchange?: (checked: boolean) => void;
    disabled?: boolean;
    label?: string;
  }

  let { checked = $bindable(false), onchange, disabled = false, label }: Props = $props();

  function toggle() {
    if (disabled) return;
    checked = !checked;
    onchange?.(checked);
  }
</script>

<button
  class="toggle-wrapper"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  onclick={toggle}
>
  <span class="toggle-track" class:active={checked}>
    <span class="toggle-thumb" class:active={checked}></span>
  </span>
  {#if label}
    <span class="toggle-label">{label}</span>
  {/if}
</button>

<style>
  .toggle-wrapper {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    background: none;
    border: none;
    padding: 0;
  }

  .toggle-wrapper:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .toggle-track {
    position: relative;
    width: 36px;
    height: 20px;
    background: var(--border);
    border-radius: 10px;
    transition: background 0.2s ease;
  }

  .toggle-track.active {
    background: var(--accent);
  }

  .toggle-thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    background: var(--fg-primary);
    border-radius: 50%;
    transition: transform 0.2s ease;
  }

  .toggle-thumb.active {
    transform: translateX(16px);
  }

  .toggle-label {
    font-size: 13px;
    color: var(--fg-secondary);
    user-select: none;
  }
</style>
