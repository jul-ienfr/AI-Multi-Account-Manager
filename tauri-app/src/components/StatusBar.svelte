<script lang="ts">
  import { config } from "../lib/stores/config";
  import { proxyStatus } from "../lib/stores/proxy";
  import { syncStore } from "../lib/stores/sync";
  import type { AppConfig, ProxyStatus } from "../lib/types";
  import Tooltip from "./ui/Tooltip.svelte";
  import { RefreshCw, Shuffle, RotateCw, Clock, Radio, Wifi } from "lucide-svelte";
  import { onMount } from "svelte";

  interface Props {
    onnavigate?: (screen: string) => void;
  }

  let { onnavigate }: Props = $props();

  let cfg: AppConfig | null = $state<AppConfig | null>(null);
  let proxy: { router: ProxyStatus; impersonator: ProxyStatus } = $state({ router: { running: false, port: 18080, uptimeSecs: 0, requestsTotal: 0, requestsActive: 0 }, impersonator: { running: false, port: 18081, uptimeSecs: 0, requestsTotal: 0, requestsActive: 0 } });
  let syncEnabled = $state(false);

  onMount(() => {
    const unsub1 = config.subscribe(c => { cfg = c; });
    const unsub2 = proxyStatus.subscribe(p => { proxy = p; });
    const unsub3 = syncStore.enabled.subscribe(e => { syncEnabled = e; });
    return () => { unsub1(); unsub2(); unsub3(); };
  });

  let autoRefresh = $derived((cfg as AppConfig | null)?.adaptiveRefresh ?? false);
  let autoSwitch = $derived(((cfg as AppConfig | null)?.proxy?.autoSwitchThreshold5h ?? 0) > 0);
  let rotation = $derived((cfg as AppConfig | null)?.proxy?.rotationEnabled ?? false);
  let schedule = $derived((cfg as AppConfig | null)?.schedule?.enabled ?? false);
  let routerOn = $derived(proxy.router.running);
  let impersonatorOn = $derived(proxy.impersonator.running);

  function nav(screen: string) {
    onnavigate?.(screen);
  }
</script>

<footer class="statusbar">
  <div class="statusbar-items">
    <Tooltip text="Auto-refresh: Rafraichissement automatique des quotas">
      <button class="status-item" class:active={autoRefresh} onclick={() => nav("settings")}>
        <RefreshCw size={12} />
        <span>Refresh</span>
        <span class="status-dot" class:on={autoRefresh}></span>
      </button>
    </Tooltip>

    <Tooltip text="Auto-switch: Changement automatique de compte">
      <button class="status-item" class:active={autoSwitch} onclick={() => nav("proxy")}>
        <Shuffle size={12} />
        <span>Switch</span>
        <span class="status-dot" class:on={autoSwitch}></span>
      </button>
    </Tooltip>

    <Tooltip text="Rotation automatique des comptes">
      <button class="status-item" class:active={rotation} onclick={() => nav("proxy")}>
        <RotateCw size={12} />
        <span>Rotation</span>
        <span class="status-dot" class:on={rotation}></span>
      </button>
    </Tooltip>

    <Tooltip text="Planning horaire d'activite">
      <button class="status-item" class:active={schedule} onclick={() => nav("settings")}>
        <Clock size={12} />
        <span>Schedule</span>
        <span class="status-dot" class:on={schedule}></span>
      </button>
    </Tooltip>

    <Tooltip text="Proxy Router / Impersonator">
      <button class="status-item" onclick={() => nav("proxy")}>
        <Radio size={12} />
        <span>R:{routerOn ? "ON" : "OFF"}</span>
        <span class="status-sep">/</span>
        <span>I:{impersonatorOn ? "ON" : "OFF"}</span>
        <span class="status-dot" class:on={routerOn || impersonatorOn}></span>
      </button>
    </Tooltip>

    <Tooltip text="Synchronisation P2P entre instances">
      <button class="status-item" class:active={syncEnabled} onclick={() => nav("settings")}>
        <Wifi size={12} />
        <span>P2P</span>
        <span class="status-dot" class:on={syncEnabled}></span>
      </button>
    </Tooltip>
  </div>
</footer>

<style>
  .statusbar {
    grid-column: 1 / -1;
    grid-row: 2;
    height: 36px;
    background: var(--bg-statusbar);
    border-top: 1px solid var(--border);
    display: flex;
    align-items: center;
    padding: 0 12px;
    user-select: none;
  }

  .statusbar-items {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
  }

  .status-item {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    font-size: 11px;
    color: var(--fg-dim);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: all 0.15s ease;
    background: none;
    border: none;
    font-family: inherit;
  }

  .status-item.active {
    color: var(--fg-secondary);
  }

  .status-item:hover {
    background: var(--bg-card-hover);
    color: var(--fg-secondary);
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-stopped);
    flex-shrink: 0;
  }

  .status-dot.on {
    background: var(--status-running);
    box-shadow: 0 0 6px var(--status-running);
  }

  .status-sep {
    color: var(--fg-dim);
    margin: 0 1px;
  }
</style>
