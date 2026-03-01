<script lang="ts">
  import QuotaChart from "../components/monitoring/QuotaChart.svelte";
  import RequestFeed from "../components/monitoring/RequestFeed.svelte";
  import SessionList from "../components/monitoring/SessionList.svelte";
  import LogViewer from "../components/monitoring/LogViewer.svelte";
  import SwitchHistory from "../components/monitoring/SwitchHistory.svelte";
  import CostPanel from "../components/monitoring/CostPanel.svelte";
  import BackoffChart from "../components/monitoring/BackoffChart.svelte";
  import PeerTopology from "../components/monitoring/PeerTopology.svelte";

  type MonitorTab = "quotas" | "requests" | "sessions" | "switches" | "logs" | "costs" | "cooldowns" | "peers";
  let activeTab: MonitorTab = $state("quotas");

  const tabs: Array<{ id: MonitorTab; label: string }> = [
    { id: "quotas", label: "Quotas" },
    { id: "requests", label: "Requetes" },
    { id: "sessions", label: "Sessions" },
    { id: "costs", label: "Couts" },
    { id: "switches", label: "Switches" },
    { id: "cooldowns", label: "Cooldowns" },
    { id: "peers", label: "Pairs" },
    { id: "logs", label: "Journal" },
  ];
</script>

<div class="monitoring-screen">
  <header class="screen-header">
    <h1 class="screen-title">Monitoring</h1>
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
    {#if activeTab === "quotas"}
      <QuotaChart />
    {:else if activeTab === "requests"}
      <RequestFeed />
    {:else if activeTab === "sessions"}
      <SessionList />
    {:else if activeTab === "costs"}
      <CostPanel />
    {:else if activeTab === "switches"}
      <SwitchHistory />
    {:else if activeTab === "cooldowns"}
      <BackoffChart />
    {:else if activeTab === "peers"}
      <PeerTopology />
    {:else}
      <LogViewer />
    {/if}
  </div>
</div>

<style>
  .monitoring-screen {
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
</style>
