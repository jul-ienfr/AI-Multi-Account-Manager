<script lang="ts">
  import ProxyControl from "../components/proxy/ProxyControl.svelte";
  import StrategySelector from "../components/proxy/StrategySelector.svelte";
  import ModelMapping from "../components/proxy/ModelMapping.svelte";
  import ImpersonationProfiles from "../components/proxy/ImpersonationProfiles.svelte";
  import Toggle from "../components/ui/Toggle.svelte";
  import Card from "../components/ui/Card.svelte";
  import type { RoutingStrategy, AppConfig } from "../lib/types";
  import { config } from "../lib/stores/config";
  import { onMount } from "svelte";

  type ProxyTab = "control" | "strategy" | "models" | "profiles";
  let activeTab: ProxyTab = $state("control");

  let strategy: RoutingStrategy = $state("priority");
  let cfg: AppConfig | null = $state(null);

  onMount(() => {
    config.load();
    const unsub = config.subscribe(c => { cfg = c; });
    return unsub;
  });

  async function updateAutoSwitch(checked: boolean) {
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

  async function updateRotation(checked: boolean) {
    if (!cfg?.proxy) return;
    await config.save({ proxy: { ...cfg.proxy, rotationEnabled: checked } } as Partial<AppConfig>);
  }

  async function updateRotationInterval(e: Event) {
    if (!cfg?.proxy) return;
    const val = parseInt((e.target as HTMLInputElement).value);
    if (val >= 1 && val <= 120) {
      await config.save({ proxy: { ...cfg.proxy, rotationIntervalSecs: val * 60 } } as Partial<AppConfig>);
    }
  }

  const tabs: Array<{ id: ProxyTab; label: string }> = [
    { id: "control", label: "Instances" },
    { id: "strategy", label: "Strategie" },
    { id: "models", label: "Modeles" },
    { id: "profiles", label: "Profils" },
  ];
</script>

<div class="proxy-screen">
  <header class="screen-header">
    <h1 class="screen-title">Proxy</h1>
  </header>

  <div class="tab-bar">
    {#each tabs as tab}
      <button
        class="tab-item"
        class:active={activeTab === tab.id}
        onclick={() => (activeTab = tab.id)}
      >
        {tab.label}
      </button>
    {/each}
  </div>

  <div class="tab-content">
    {#if activeTab === "control"}
      <ProxyControl />
    {:else if activeTab === "strategy"}
      <div class="strategy-section">
        <StrategySelector bind:selected={strategy} />

        {#if cfg}
          <div class="strategy-options">
            <Card hoverable={false}>
              <div class="option-row">
                <div class="option-info">
                  <span class="option-label">Auto-switch</span>
                  <span class="option-desc">Changer de compte quand le quota atteint 85% (5h) / 90% (7j)</span>
                </div>
                <Toggle
                  checked={(cfg?.proxy?.autoSwitchThreshold5h ?? 0) > 0}
                  onchange={updateAutoSwitch}
                />
              </div>
            </Card>

            <Card hoverable={false}>
              <div class="option-row">
                <div class="option-info">
                  <span class="option-label">Rotation automatique</span>
                  <span class="option-desc">Alterner entre comptes a intervalle fixe</span>
                </div>
                <Toggle
                  checked={cfg?.proxy?.rotationEnabled ?? false}
                  onchange={updateRotation}
                />
              </div>
            </Card>

            {#if cfg?.proxy?.rotationEnabled}
              <Card hoverable={false}>
                <div class="option-row">
                  <div class="option-info">
                    <span class="option-label">Intervalle rotation</span>
                    <span class="option-desc">Minutes entre chaque changement</span>
                  </div>
                  <input
                    type="number"
                    class="option-input"
                    value={Math.round((cfg?.proxy?.rotationIntervalSecs ?? 3600) / 60)}
                    min="1"
                    max="120"
                    onchange={updateRotationInterval}
                  />
                </div>
              </Card>
            {/if}
          </div>
        {/if}
      </div>
    {:else if activeTab === "models"}
      <ModelMapping />
    {:else}
      <ImpersonationProfiles />
    {/if}
  </div>
</div>

<style>
  .proxy-screen {
    display: flex;
    flex-direction: column;
    gap: 20px;
    animation: fade-in 0.2s ease;
  }

  .screen-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .screen-title {
    font-size: 20px;
    font-weight: 700;
    color: var(--fg-primary);
  }

  .tab-bar {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--border);
    padding-bottom: 0;
  }

  .tab-item {
    padding: 8px 16px;
    font-size: 13px;
    font-weight: 500;
    color: var(--fg-secondary);
    border-radius: var(--radius-md) var(--radius-md) 0 0;
    cursor: pointer;
    background: none;
    border: none;
    position: relative;
    transition: all 0.15s ease;
  }

  .tab-item:hover {
    color: var(--fg-primary);
    background: var(--bg-card-hover);
  }

  .tab-item.active {
    color: var(--fg-accent);
  }

  .tab-item.active::after {
    content: "";
    position: absolute;
    bottom: -1px;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--accent);
    border-radius: 2px 2px 0 0;
  }

  .tab-content {
    min-height: 300px;
  }

  .strategy-section {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .strategy-options {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 4px;
  }

  .option-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .option-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
  }

  .option-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--fg-primary);
  }

  .option-desc {
    font-size: 11px;
    color: var(--fg-dim);
  }

  .option-input {
    width: 70px;
    padding: 4px 8px;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-primary);
    font-size: 13px;
    text-align: center;
  }

  .option-input:focus {
    outline: none;
    border-color: var(--accent);
  }
</style>
