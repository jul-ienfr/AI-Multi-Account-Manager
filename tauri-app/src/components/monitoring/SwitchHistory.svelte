<script lang="ts">
  import { onMount } from "svelte";
  import { getSwitchHistory } from "../../lib/tauri";
  import type { SwitchEntry } from "../../lib/types";
  import Card from "../ui/Card.svelte";
  import Button from "../ui/Button.svelte";
  import { ArrowRightLeft, RefreshCw } from "lucide-svelte";

  let switches: SwitchEntry[] = $state([]);
  let loading = $state(true);
  let error = $state("");

  // Aggregate per-account statistics
  let accountStats = $derived(() => {
    const stats = new Map<string, { total: number; from: number; to: number }>();
    for (const s of switches) {
      if (s.from) {
        const f = stats.get(s.from) ?? { total: 0, from: 0, to: 0 };
        f.from++;
        stats.set(s.from, f);
      }
      const t = stats.get(s.to) ?? { total: 0, from: 0, to: 0 };
      t.to++;
      stats.set(s.to, t);
    }
    return Array.from(stats.entries()).map(([key, v]) => ({ key, ...v }))
      .sort((a, b) => (b.from + b.to) - (a.from + a.to));
  });

  onMount(async () => {
    await load();
  });

  async function load() {
    loading = true;
    error = "";
    try {
      switches = await getSwitchHistory();
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  function formatTime(ts: string): string {
    if (!ts) return "";
    try {
      return new Date(ts).toLocaleString("fr-FR", {
        day: "2-digit", month: "2-digit",
        hour: "2-digit", minute: "2-digit", second: "2-digit",
      });
    } catch { return ts; }
  }

  function reasonLabel(reason: string): string {
    if (reason === "auto-switch") return "auto";
    if (reason === "quota") return "quota";
    return "manuel";
  }
</script>

<div class="switch-history">
  <div class="history-header">
    <span class="history-count">{switches.length} switches</span>
    <Button variant="ghost" size="sm" onclick={load}>
      <RefreshCw size={14} />
    </Button>
  </div>

  {#if loading}
    <div class="history-loading">Chargement...</div>
  {:else if error}
    <div class="history-error">{error}</div>
  {:else if switches.length === 0}
    <div class="history-empty">
      <ArrowRightLeft size={32} />
      <p>Aucun switch enregistre</p>
      <p class="hint">Les changements de compte apparaitront ici</p>
    </div>
  {:else}
    <!-- Stats table per account -->
    <Card hoverable={false}>
      <div class="stats-table-wrapper">
        <table class="stats-table">
          <thead>
            <tr>
              <th>Compte</th>
              <th>Switch depuis</th>
              <th>Switch vers</th>
              <th>Total</th>
            </tr>
          </thead>
          <tbody>
            {#each accountStats() as row}
              <tr>
                <td class="account-cell">{row.key}</td>
                <td class="num-cell">{row.from}</td>
                <td class="num-cell">{row.to}</td>
                <td class="num-cell">{row.from + row.to}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </Card>

    <!-- Last switches log -->
    <div class="last-switches">
      <div class="section-label">Derniers switches</div>
      <div class="switches-log">
        {#each switches.slice(0, 20) as sw}
          <div class="switch-item">
            <span class="sw-time">{formatTime(sw.timestamp)}</span>
            {#if sw.from}
              <span class="sw-from">{sw.from}</span>
              <ArrowRightLeft size={12} class="sw-arrow" />
            {/if}
            <span class="sw-to">{sw.to}</span>
            <span class="sw-reason" class:auto={sw.reason === "auto-switch"}>{reasonLabel(sw.reason)}</span>
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .switch-history {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .history-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .history-count {
    font-size: 13px;
    color: var(--fg-secondary);
    font-weight: 500;
  }

  .history-loading, .history-error, .history-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 48px;
    color: var(--fg-dim);
    text-align: center;
  }

  .history-error {
    color: var(--status-error);
  }

  .hint {
    font-size: 12px;
    opacity: 0.7;
  }

  .stats-table-wrapper {
    overflow-x: auto;
  }

  .stats-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  .stats-table th {
    text-align: left;
    padding: 6px 12px;
    color: var(--fg-dim);
    font-weight: 500;
    font-size: 11px;
    border-bottom: 1px solid var(--border);
  }

  .stats-table td {
    padding: 6px 12px;
    color: var(--fg-secondary);
    border-bottom: 1px solid color-mix(in srgb, var(--border) 40%, transparent);
  }

  .stats-table tr:last-child td {
    border-bottom: none;
  }

  .stats-table tr:hover td {
    background: var(--bg-card-hover);
    color: var(--fg-primary);
  }

  .account-cell {
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 11px;
    color: var(--fg-primary) !important;
  }

  .num-cell {
    text-align: right;
    font-variant-numeric: tabular-nums;
    width: 80px;
  }

  .section-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: 4px;
  }

  .switches-log {
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow-y: auto;
    max-height: 300px;
  }

  .switch-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 40%, transparent);
    font-size: 11px;
    transition: background 0.1s;
  }

  .switch-item:last-child { border-bottom: none; }
  .switch-item:hover { background: var(--bg-card-hover); }

  .sw-time {
    color: var(--fg-dim);
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 10px;
    flex-shrink: 0;
    width: 130px;
    font-variant-numeric: tabular-nums;
  }

  .sw-from {
    color: var(--fg-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    font-size: 11px;
  }

  .sw-to {
    color: var(--fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    font-weight: 500;
    font-size: 11px;
  }

  .sw-reason {
    font-size: 10px;
    padding: 1px 5px;
    border-radius: var(--radius-sm);
    background: var(--bg-card);
    color: var(--fg-dim);
    flex-shrink: 0;
  }

  .sw-reason.auto {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
</style>
