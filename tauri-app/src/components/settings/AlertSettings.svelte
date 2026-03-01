<script lang="ts">
  import { config } from "../../lib/stores/config";
  import Toggle from "../ui/Toggle.svelte";
  import Card from "../ui/Card.svelte";
  import { onMount } from "svelte";
  import type { AppConfig } from "../../lib/types";

  let cfg: AppConfig | null = $state(null);

  onMount(() => {
    const unsub = config.subscribe(c => { cfg = c; });
    return unsub;
  });

  async function updateSound(checked: boolean) {
    if (!cfg?.alerts) return;
    await config.save({ alerts: { ...cfg.alerts, soundEnabled: checked } } as Partial<AppConfig>);
  }

  async function updateToasts(checked: boolean) {
    if (!cfg?.alerts) return;
    await config.save({ alerts: { ...cfg.alerts, toastsEnabled: checked } } as Partial<AppConfig>);
  }

  async function updateQuotaThreshold(e: Event) {
    if (!cfg?.alerts) return;
    const val = parseInt((e.target as HTMLInputElement).value);
    if (val >= 50 && val <= 99) {
      await config.save({ alerts: { ...cfg.alerts, quotaAlertThreshold: val / 100 } } as Partial<AppConfig>);
    }
  }
</script>

<div class="alert-settings">
  <h3 class="section-title">Alertes & Notifications</h3>

  {#if cfg}
    <div class="settings-group">
      <Card hoverable={false}>
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Son</span>
            <span class="setting-desc">Jouer un son lors des notifications</span>
          </div>
          <Toggle
            checked={cfg?.alerts?.soundEnabled ?? false}
            onchange={updateSound}
          />
        </div>
      </Card>

      <Card hoverable={false}>
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Toasts</span>
            <span class="setting-desc">Afficher les notifications toast</span>
          </div>
          <Toggle
            checked={cfg?.alerts?.toastsEnabled ?? true}
            onchange={updateToasts}
          />
        </div>
      </Card>

      <Card hoverable={false}>
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Seuil d'alerte (%)</span>
            <span class="setting-desc">Pourcentage de quota avant notification</span>
          </div>
          <div class="threshold-input">
            <input
              type="range"
              class="range-input"
              value={Math.round((cfg?.alerts?.quotaAlertThreshold ?? 0.80) * 100)}
              min="50"
              max="99"
              oninput={updateQuotaThreshold}
            />
            <span class="threshold-value">{Math.round((cfg?.alerts?.quotaAlertThreshold ?? 0.80) * 100)}%</span>
          </div>
        </div>
      </Card>
    </div>
  {:else}
    <p class="loading-text">Chargement...</p>
  {/if}
</div>

<style>
  .alert-settings {
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

  .threshold-input {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .range-input {
    width: 120px;
    accent-color: var(--accent);
  }

  .threshold-value {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-primary);
    width: 36px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .loading-text {
    color: var(--fg-dim);
    font-size: 13px;
  }
</style>
