<script lang="ts">
  import Sidebar from "./components/Sidebar.svelte";
  import StatusBar from "./components/StatusBar.svelte";
  import Toast from "./components/Toast.svelte";
  import Accounts from "./routes/Accounts.svelte";
  import Proxy from "./routes/Proxy.svelte";
  import Monitoring from "./routes/Monitoring.svelte";
  import Settings from "./routes/Settings.svelte";
  import { accounts } from "./lib/stores/accounts";
  import { onQuotaUpdate, onToast, onAccountSwitch } from "./lib/tauri";
  import { toast } from "./lib/stores/toast";
  import { onMount } from "svelte";
  import { fade } from "svelte/transition";
  // i18n — initialise the store so the locale is loaded from localStorage
  import { i18nStore } from "./lib/i18n";

  type Screen = "accounts" | "proxy" | "monitoring" | "settings";
  let currentScreen: Screen = $state("accounts");
  let settingsSection: string = $state("general");

  function handleNav(raw: string) {
    const [screen, section] = raw.split(":");
    currentScreen = screen as Screen;
    if (section) settingsSection = section;
  }

  onMount(async () => {
    try {
      await accounts.load();
    } catch (e) {
      console.error("Failed to load accounts:", e);
    }
    onQuotaUpdate(({ key, quota }) => accounts.updateQuota(key, quota));
    onToast((t: any) => (toast as any)[t.type]?.(t.title, t.message));
    onAccountSwitch((key) => accounts.switch(key).catch(e => console.error("Account switch failed:", e)));
  });

  // Keyboard navigation (Phase 7.7)
  function handleKeyboard(e: KeyboardEvent) {
    if (e.ctrlKey && e.altKey) {
      const screens: Screen[] = ["accounts", "proxy", "monitoring", "settings"];
      const idx = screens.indexOf(currentScreen);
      if (e.key === "n" || e.key === "N") {
        currentScreen = screens[(idx + 1) % screens.length];
        e.preventDefault();
      } else if (e.key === "p" || e.key === "P") {
        currentScreen = screens[(idx - 1 + screens.length) % screens.length];
        e.preventDefault();
      } else if (e.key === "r" || e.key === "R") {
        accounts.load().catch(e => console.error("Reload accounts failed:", e));
        e.preventDefault();
      }
    }
  }
</script>

<svelte:window onkeydown={handleKeyboard} />

<div class="app-layout">
  <Sidebar bind:currentScreen />
  <main class="main-content">
    {#key currentScreen}
      <div class="screen-transition" in:fade={{ duration: 150 }}>
        {#if currentScreen === "accounts"}
          <Accounts />
        {:else if currentScreen === "proxy"}
          <Proxy />
        {:else if currentScreen === "monitoring"}
          <Monitoring />
        {:else}
          <Settings initialSection={settingsSection} />
        {/if}
      </div>
    {/key}
  </main>
  <StatusBar onnavigate={handleNav} />
  <Toast />
</div>

<style>
  .app-layout {
    display: grid;
    grid-template-columns: var(--sidebar-width, 220px) 1fr;
    grid-template-rows: 1fr 36px;
    height: 100vh;
    min-width: 800px;
    background: var(--bg-app);
    overflow: hidden;
  }
  .main-content {
    overflow-y: auto;
    padding: 24px 28px;
    grid-column: 2;
    grid-row: 1;
  }
  .screen-transition {
    animation: slide-up 0.15s ease-out;
  }
  @keyframes slide-up {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>
