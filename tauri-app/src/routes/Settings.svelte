<script lang="ts">
  import SettingsLayout from "../components/settings/SettingsLayout.svelte";
  import GeneralSettings from "../components/settings/GeneralSettings.svelte";
  import AlertSettings from "../components/settings/AlertSettings.svelte";
  import NetworkSettings from "../components/settings/NetworkSettings.svelte";
  import ProviderSettings from "../components/settings/ProviderSettings.svelte";
  import { config } from "../lib/stores/config";
  import { theme, type Theme } from "../lib/stores/theme";
  import { t, setLocale, i18nStore, type Locale } from "../lib/i18n";
  import { onMount } from "svelte";

  let currentTheme: Theme = $state("dark");
  theme.subscribe((t) => { currentTheme = t; });
  import {
    Settings, Bell, Wifi, Server, Palette, Database, Keyboard, Calendar,
  } from "lucide-svelte";

  type Section = "general" | "alerts" | "schedule" | "network" | "providers" | "theme" | "data" | "hotkeys";
  let activeSection: Section = $state("general");

  const sections: Array<{ id: Section; label: string; icon: any }> = [
    { id: "general", label: "Général", icon: Settings },
    { id: "alerts", label: "Alertes", icon: Bell },
    { id: "schedule", label: "Schedule", icon: Calendar },
    { id: "network", label: "Réseau", icon: Wifi },
    { id: "providers", label: "Providers", icon: Server },
    { id: "theme", label: "Thème", icon: Palette },
    { id: "data", label: "Données", icon: Database },
    { id: "hotkeys", label: "Hotkeys", icon: Keyboard },
  ];

  function onLangChange(e: Event) {
    const select = e.currentTarget as HTMLSelectElement;
    setLocale(select.value as Locale);
  }

  onMount(async () => {
    try { await config.load(); } catch (e) { console.error("Failed to load config:", e); }
  });
</script>

<div class="settings-page">
  <header class="page-header">
    <h1>{t('settings.title')}</h1>
  </header>

  <div class="settings-body">
    <nav class="settings-nav">
      {#each sections as section}
        {@const Icon = section.icon}
        <button class="nav-item" class:active={activeSection === section.id} onclick={() => (activeSection = section.id)}>
          <Icon size={16} />
          <span>{section.label}</span>
        </button>
      {/each}
    </nav>

    <div class="settings-content">
      {#if activeSection === "general"}
        <GeneralSettings />
      {:else if activeSection === "alerts"}
        <AlertSettings />
      {:else if activeSection === "schedule"}
        <SettingsLayout title="Schedule">
          <div class="setting-row">
            <label for="schedule-start">Plage horaire active</label>
            <div class="time-range">
              <input id="schedule-start" type="number" min="0" max="23" value="9" class="time-input" />
              <span class="time-sep">→</span>
              <input id="schedule-end" type="number" min="0" max="23" value="18" class="time-input" />
            </div>
          </div>
          <p class="hint">Les rafraîchissements automatiques ne fonctionnent que dans cette plage.</p>
        </SettingsLayout>
      {:else if activeSection === "network"}
        <NetworkSettings />
      {:else if activeSection === "providers"}
        <ProviderSettings />
      {:else if activeSection === "theme"}
        <SettingsLayout title={t('settings.theme')}>
          <div class="setting-row">
            <span class="setting-label">Mode</span>
            <div class="theme-options">
              <button class="theme-btn" class:active={currentTheme === "dark"} onclick={() => theme.set("dark")}>Sombre</button>
              <button class="theme-btn" class:active={currentTheme === "light"} onclick={() => theme.set("light")}>Clair</button>
              <button class="theme-btn" class:active={currentTheme === "system"} onclick={() => theme.set("system")}>Système</button>
            </div>
          </div>
          <div class="setting-row">
            <label for="font-select">Police</label>
            <select id="font-select" class="select-input"><option>Inter</option><option>Geist Sans</option><option>System</option></select>
          </div>
          <div class="setting-row">
            <label for="lang-select">{t('settings.language')}</label>
            <select id="lang-select" class="select-input" value={$i18nStore} onchange={onLangChange}>
              <option value="fr">Français</option>
              <option value="en">English</option>
            </select>
          </div>
        </SettingsLayout>
      {:else if activeSection === "data"}
        <SettingsLayout title="Données">
          <div class="action-buttons">
            <button class="btn-secondary">Exporter la config</button>
            <button class="btn-secondary">Créer un backup</button>
            <button class="btn-danger">Réinitialiser</button>
          </div>
          <p class="hint">L'export crée un JSON contenant vos paramètres (sans les tokens).</p>
        </SettingsLayout>
      {:else if activeSection === "hotkeys"}
        <SettingsLayout title="Raccourcis clavier">
          <div class="setting-row"><span class="setting-label">Compte suivant</span><kbd class="hotkey">Ctrl+Alt+N</kbd></div>
          <div class="setting-row"><span class="setting-label">Compte précédent</span><kbd class="hotkey">Ctrl+Alt+P</kbd></div>
          <div class="setting-row"><span class="setting-label">Rafraîchir</span><kbd class="hotkey">Ctrl+Alt+R</kbd></div>
        </SettingsLayout>
      {/if}
    </div>
  </div>
</div>

<style>
  .settings-page { display: flex; flex-direction: column; gap: 20px; height: 100%; }
  .page-header h1 { font-size: 20px; font-weight: 700; color: var(--fg-primary); }
  .settings-body { display: grid; grid-template-columns: 180px 1fr; gap: 24px; flex: 1; min-height: 0; }
  .settings-nav { display: flex; flex-direction: column; gap: 2px; }
  .nav-item { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-radius: 6px; border: none; background: none; color: var(--fg-secondary); cursor: pointer; font-size: 13px; text-align: left; transition: all 0.15s; }
  .nav-item:hover { background: var(--bg-card); color: var(--fg-primary); }
  .nav-item.active { background: var(--accent-glow); color: var(--fg-accent); }
  .settings-content { overflow-y: auto; }
  .setting-row { display: flex; align-items: center; justify-content: space-between; padding: 12px 0; border-bottom: 1px solid var(--border); }
  .setting-row label, .setting-label { color: var(--fg-primary); font-size: 13px; }
  .time-range { display: flex; align-items: center; gap: 8px; }
  .time-input { width: 60px; padding: 6px 8px; border-radius: 6px; border: 1px solid var(--border); background: var(--bg-card); color: var(--fg-primary); font-size: 13px; text-align: center; }
  .time-sep { color: var(--fg-dim); }
  .hint { color: var(--fg-dim); font-size: 12px; margin-top: 8px; }
  .theme-options { display: flex; gap: 4px; }
  .theme-btn { padding: 6px 14px; border-radius: 6px; border: 1px solid var(--border); background: var(--bg-card); color: var(--fg-secondary); cursor: pointer; font-size: 12px; transition: all 0.15s; }
  .theme-btn.active { background: var(--accent-glow); color: var(--fg-accent); border-color: var(--accent); }
  .select-input { padding: 6px 12px; border-radius: 6px; border: 1px solid var(--border); background: var(--bg-card); color: var(--fg-primary); font-size: 13px; }
  .action-buttons { display: flex; gap: 8px; padding: 12px 0; }
  .btn-secondary { padding: 8px 16px; border-radius: 6px; border: 1px solid var(--border); background: var(--bg-card); color: var(--fg-primary); cursor: pointer; font-size: 13px; transition: all 0.15s; }
  .btn-secondary:hover { background: var(--bg-card-hover); }
  .btn-danger { padding: 8px 16px; border-radius: 6px; border: 1px solid var(--phase-critical); background: transparent; color: var(--phase-critical); cursor: pointer; font-size: 13px; }
  .btn-danger:hover { background: rgba(239, 68, 68, 0.1); }
  .hotkey { padding: 4px 10px; border-radius: 4px; border: 1px solid var(--border); background: var(--bg-card); color: var(--fg-secondary); font-family: monospace; font-size: 12px; }
</style>
