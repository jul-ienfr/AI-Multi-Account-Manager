<script lang="ts">
  import { config } from "../../lib/stores/config";
  import Toggle from "../ui/Toggle.svelte";
  import Card from "../ui/Card.svelte";
  import { onMount } from "svelte";
  import type { AppConfig } from "../../lib/types";

  let cfg: AppConfig | null = $state(null);

  onMount(() => {
    config.load();
    const unsub = config.subscribe(c => { cfg = c; });
    return unsub;
  });

  async function updateRefreshEnabled(checked: boolean) {
    await config.save({ adaptiveRefresh: checked } as Partial<AppConfig>);
  }

  async function updateInterval(e: Event) {
    const val = parseInt((e.target as HTMLInputElement).value);
    if (val >= 10 && val <= 600) {
      await config.save({ refreshIntervalSecs: val } as Partial<AppConfig>);
    }
  }

  async function updateAutoSwitchEnabled(checked: boolean) {
    if (!cfg?.proxy) return;
    const proxy = { ...cfg.proxy };
    if (checked) {
      proxy.autoSwitchThreshold5h = 0.85;
      proxy.autoSwitchThreshold7d = 0.90;
    } else {
      proxy.autoSwitchThreshold5h = 0;
      proxy.autoSwitchThreshold7d = 0;
    }
    await config.save({ proxy } as Partial<AppConfig>);
  }

  async function updateRotationEnabled(checked: boolean) {
    if (!cfg?.proxy) return;
    const proxy = { ...cfg.proxy, rotationEnabled: checked };
    await config.save({ proxy } as Partial<AppConfig>);
  }

  async function updateRotationInterval(e: Event) {
    if (!cfg?.proxy) return;
    const val = parseInt((e.target as HTMLInputElement).value);
    if (val >= 1 && val <= 120) {
      const proxy = { ...cfg.proxy, rotationIntervalSecs: val * 60 };
      await config.save({ proxy } as Partial<AppConfig>);
    }
  }
</script>

<div class="general-settings">
  <h3 class="section-title">General</h3>

  {#if cfg}
    <div class="settings-group">
      <Card hoverable={false}>
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Auto-refresh</span>
            <span class="setting-desc">Rafraichir automatiquement les quotas</span>
          </div>
          <Toggle
            checked={cfg?.adaptiveRefresh ?? false}
            onchange={updateRefreshEnabled}
          />
        </div>
      </Card>

      {#if cfg?.adaptiveRefresh}
        <Card hoverable={false}>
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-label">Intervalle (secondes)</span>
              <span class="setting-desc">Frequence de rafraichissement</span>
            </div>
            <input
              type="number"
              class="setting-input"
              value={cfg?.refreshIntervalSecs ?? 60}
              min="10"
              max="600"
              onchange={updateInterval}
            />
          </div>
        </Card>
      {/if}

      <Card hoverable={false}>
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Auto-switch</span>
            <span class="setting-desc">Changer de compte automatiquement quand quota atteint</span>
          </div>
          <Toggle
            checked={(cfg?.proxy?.autoSwitchThreshold5h ?? 0) > 0}
            onchange={updateAutoSwitchEnabled}
          />
        </div>
      </Card>

      <Card hoverable={false}>
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Rotation</span>
            <span class="setting-desc">Rotation automatique entre comptes</span>
          </div>
          <Toggle
            checked={cfg?.proxy?.rotationEnabled ?? false}
            onchange={updateRotationEnabled}
          />
        </div>
      </Card>

      {#if cfg?.proxy?.rotationEnabled}
        <Card hoverable={false}>
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-label">Intervalle rotation (min)</span>
              <span class="setting-desc">Duree avant de changer de compte</span>
            </div>
            <input
              type="number"
              class="setting-input"
              value={Math.round((cfg?.proxy?.rotationIntervalSecs ?? 3600) / 60)}
              min="1"
              max="120"
              onchange={updateRotationInterval}
            />
          </div>
        </Card>
      {/if}
    </div>
  {:else}
    <p class="loading-text">Chargement de la configuration...</p>
  {/if}
</div>

<style>
  .general-settings {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .section-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--fg-primary);
    margin-bottom: 4px;
  }

  .settings-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .setting-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .setting-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
  }

  .setting-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--fg-primary);
  }

  .setting-desc {
    font-size: 11px;
    color: var(--fg-dim);
  }

  .setting-input {
    width: 80px;
    padding: 4px 8px;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-primary);
    font-size: 13px;
    text-align: center;
  }

  .setting-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .loading-text {
    color: var(--fg-dim);
    font-size: 13px;
  }
</style>
