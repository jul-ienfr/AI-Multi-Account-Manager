<script lang="ts">
  import { onMount } from "svelte";
  import { accounts } from "../../lib/stores/accounts";
  import { getQuotaHistory } from "../../lib/tauri";
  import type { AccountState } from "../../lib/types";
  import Button from "../ui/Button.svelte";
  import Card from "../ui/Card.svelte";
  import { Chart, registerables } from "chart.js";

  Chart.register(...registerables);

  let accountList: AccountState[] = $state([]);
  let selectedKey = $state("");
  let period: "24h" | "7d" | "30d" = $state("24h");
  let chartCanvas: HTMLCanvasElement | undefined = $state();
  let chart: Chart | null = null;

  onMount(() => {
    const unsub = accounts.subscribe(a => {
      accountList = a;
      if (!selectedKey && a.length > 0) {
        selectedKey = a[0].key;
      }
    });
    return () => {
      unsub();
      chart?.destroy();
    };
  });

  async function loadChart() {
    if (!selectedKey || !chartCanvas) return;

    try {
      const data = await getQuotaHistory(selectedKey, period);

      const labels = data.map(d => {
        const date = new Date(d.timestamp);
        return period === "24h"
          ? date.toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" })
          : date.toLocaleDateString("fr-FR", { day: "2-digit", month: "2-digit" });
      });

      const values = data.map(d => d.tokens);

      chart?.destroy();
      chart = new Chart(chartCanvas, {
        type: "line",
        data: {
          labels,
          datasets: [{
            label: "Tokens utilises",
            data: values,
            borderColor: "rgb(59, 130, 246)",
            backgroundColor: "rgba(59, 130, 246, 0.1)",
            fill: true,
            tension: 0.3,
            pointRadius: 2,
            pointHoverRadius: 5,
            borderWidth: 2,
          }],
        },
        options: {
          responsive: true,
          maintainAspectRatio: false,
          interaction: { intersect: false, mode: "index" },
          plugins: {
            legend: { display: false },
            tooltip: {
              backgroundColor: "rgba(18, 18, 26, 0.95)",
              borderColor: "rgba(30, 30, 46, 1)",
              borderWidth: 1,
              titleColor: "#e2e8f0",
              bodyColor: "#94a3b8",
              padding: 10,
              cornerRadius: 8,
            },
          },
          scales: {
            x: {
              grid: { color: "rgba(30, 30, 46, 0.5)" },
              ticks: { color: "#475569", font: { size: 11 } },
            },
            y: {
              grid: { color: "rgba(30, 30, 46, 0.5)" },
              ticks: { color: "#475569", font: { size: 11 } },
              beginAtZero: true,
            },
          },
        },
      });
    } catch (e) {
      console.error("Failed to load quota history:", e);
    }
  }

  $effect(() => {
    if (selectedKey && chartCanvas) {
      loadChart();
    }
  });
</script>

<div class="quota-chart">
  <div class="chart-controls">
    <select class="chart-select" bind:value={selectedKey}>
      {#each accountList as acc}
        <option value={acc.key}>{acc.data.displayName || acc.data.name || acc.key}</option>
      {/each}
    </select>

    <div class="period-group">
      {#each (["24h", "7d", "30d"] as const) as p}
        <Button
          variant={period === p ? "primary" : "ghost"}
          size="sm"
          onclick={() => { period = p; }}
        >
          {p}
        </Button>
      {/each}
    </div>
  </div>

  <Card hoverable={false}>
    <div class="chart-wrapper">
      <canvas bind:this={chartCanvas}></canvas>
    </div>
  </Card>
</div>

<style>
  .quota-chart {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .chart-controls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .chart-select {
    padding: 6px 12px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    color: var(--fg-primary);
    font-size: 13px;
    min-width: 180px;
  }

  .chart-select:focus {
    outline: none;
    border-color: var(--accent);
  }

  .period-group {
    display: flex;
    gap: 4px;
  }

  .chart-wrapper {
    height: 320px;
    position: relative;
  }
</style>
