<script lang="ts">
  import { Users, ArrowLeftRight, Activity, Settings } from "lucide-svelte";
  import { t, i18nStore } from "../lib/i18n";

  type Screen = "accounts" | "proxy" | "monitoring" | "settings";

  interface Props {
    currentScreen: Screen;
  }

  let { currentScreen = $bindable() }: Props = $props();

  // navItems are derived so they re-evaluate when the locale changes.
  // We reference $i18nStore so Svelte tracks the dependency.
  const navItems: Array<{ id: Screen; labelKey: string; icon: typeof Users }> = [
    { id: "accounts", labelKey: "nav.accounts", icon: Users },
    { id: "proxy",    labelKey: "nav.proxy",    icon: ArrowLeftRight },
    { id: "monitoring", labelKey: "nav.monitoring", icon: Activity },
    { id: "settings", labelKey: "nav.settings", icon: Settings },
  ];
</script>

<aside class="sidebar">
  <div class="sidebar-logo">
    <div class="logo-icon">AI</div>
    <div class="logo-text">
      <span class="logo-title">AI Manager</span>
      <span class="logo-version">v3</span>
    </div>
  </div>

  <nav class="sidebar-nav">
    {#each navItems as item}
      <button
        class="nav-item"
        class:active={currentScreen === item.id}
        onclick={() => (currentScreen = item.id)}
      >
        <span class="nav-icon">
          <item.icon size={18} />
        </span>
        <span class="nav-label">{$i18nStore, t(item.labelKey)}</span>
        {#if currentScreen === item.id}
          <span class="nav-indicator"></span>
        {/if}
      </button>
    {/each}
  </nav>

  <div class="sidebar-footer">
    <span class="footer-text">Multi-Account Manager</span>
  </div>
</aside>

<style>
  .sidebar {
    width: var(--sidebar-width);
    height: 100vh;
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    grid-row: 1 / -1;
    grid-column: 1;
    user-select: none;
    overflow: hidden;
  }

  .sidebar-logo {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 20px 16px 24px;
  }

  .logo-icon {
    width: 32px;
    height: 32px;
    background: var(--accent);
    border-radius: var(--radius-md);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    font-weight: 700;
    color: #fff;
    flex-shrink: 0;
  }

  .logo-text {
    display: flex;
    flex-direction: column;
    line-height: 1.2;
  }

  .logo-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .logo-version {
    font-size: 11px;
    color: var(--fg-dim);
  }

  .sidebar-nav {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 8px;
  }

  .nav-item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-radius: var(--radius-md);
    color: var(--fg-secondary);
    cursor: pointer;
    background: none;
    border: none;
    font-size: 13px;
    font-weight: 500;
    width: 100%;
    text-align: left;
    transition: all 0.15s ease;
  }

  .nav-item:hover {
    color: var(--fg-primary);
    background: var(--bg-card-hover);
  }

  .nav-item.active {
    color: var(--fg-accent);
    background: var(--accent-glow);
  }

  .nav-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    flex-shrink: 0;
  }

  .nav-label {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .nav-indicator {
    position: absolute;
    left: 0;
    top: 50%;
    transform: translateY(-50%);
    width: 3px;
    height: 16px;
    background: var(--accent);
    border-radius: 0 3px 3px 0;
  }

  .sidebar-footer {
    padding: 12px 16px;
    border-top: 1px solid var(--border);
  }

  .footer-text {
    font-size: 11px;
    color: var(--fg-dim);
  }

  /* Collapsed sidebar at narrow widths */
  @media (max-width: 800px) {
    .nav-label,
    .logo-text,
    .footer-text { display: none; }
    .sidebar-logo { justify-content: center; padding: 20px 8px 24px; }
    .nav-item { justify-content: center; padding: 8px; }
    .sidebar-nav { padding: 0 4px; }
  }
</style>
