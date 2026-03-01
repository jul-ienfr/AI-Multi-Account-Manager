<script lang="ts">
  import { onMount } from "svelte";
  import { getSessions } from "../../lib/tauri";
  import Card from "../ui/Card.svelte";
  import Badge from "../ui/Badge.svelte";
  import { DollarSign, RefreshCw, Users, Layers } from "lucide-svelte";

  interface SessionData {
    session_id: string;
    account_email: string;
    model: string;
    started_at: string;
    updated_at: string;
    total_input_tokens: number;
    total_output_tokens: number;
    cache_read_tokens: number;
    cache_creation_tokens: number;
    request_count: number;
    estimated_cost_usd: number;
    client_ip: string;
  }

  interface GroupedAccount {
    email: string;
    sessions: SessionData[];
    totalCost: number;
    totalInput: number;
    totalOutput: number;
    totalRequests: number;
  }

  let sessions: SessionData[] = $state([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let groupByAccount = $state(false);
  let refreshInterval: ReturnType<typeof setInterval> | null = null;

  let sorted = $derived(
    [...sessions].sort((a, b) => b.estimated_cost_usd - a.estimated_cost_usd)
  );

  let totalCost = $derived(
    sessions.reduce((sum, s) => sum + (s.estimated_cost_usd ?? 0), 0)
  );

  let grouped = $derived<GroupedAccount[]>(() => {
    const map = new Map<string, GroupedAccount>();
    for (const s of sessions) {
      const key = s.account_email ?? "inconnu";
      if (!map.has(key)) {
        map.set(key, {
          email: key,
          sessions: [],
          totalCost: 0,
          totalInput: 0,
          totalOutput: 0,
          totalRequests: 0,
        });
      }
      const g = map.get(key)!;
      g.sessions.push(s);
      g.totalCost += s.estimated_cost_usd ?? 0;
      g.totalInput += s.total_input_tokens ?? 0;
      g.totalOutput += s.total_output_tokens ?? 0;
      g.totalRequests += s.request_count ?? 0;
    }
    return [...map.values()].sort((a, b) => b.totalCost - a.totalCost);
  });

  async function load() {
    try {
      const raw = await getSessions();
      sessions = (raw as SessionData[]).filter(
        (s) => s && typeof s === "object"
      );
      error = null;
    } catch (e) {
      console.error("CostPanel: failed to load sessions", e);
      error = "Impossible de charger les sessions";
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    load();
    refreshInterval = setInterval(load, 10_000);
    return () => {
      if (refreshInterval !== null) clearInterval(refreshInterval);
    };
  });

  function formatCost(usd: number): string {
    if (usd === 0) return "$0.0000";
    if (usd < 0.0001) return `$${usd.toExponential(2)}`;
    return `$${usd.toFixed(4)}`;
  }

  function formatTokens(n: number): string {
    if (!n) return "0";
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
    return `${n}`;
  }

  function shortModel(model: string): string {
    if (!model) return "—";
    const parts = model.split("-");
    // Keep last 2-3 parts for readability
    return parts.slice(-2).join("-");
  }

  function formatDate(ts: string): string {
    if (!ts) return "—";
    return new Date(ts).toLocaleString("fr-FR", {
      day: "2-digit",
      month: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function costColor(usd: number): string {
    if (usd <= 0) return "var(--fg-dim)";
    if (usd < 0.01) return "var(--phase-cruise)";
    if (usd < 0.1) return "var(--phase-watch)";
    if (usd < 1.0) return "var(--phase-alert)";
    return "var(--phase-critical)";
  }
</script>

<div class="cost-panel">
  <div class="panel-toolbar">
    <div class="toolbar-left">
      <DollarSign size={16} />
      <span class="toolbar-title">Couts par session</span>
      {#if !loading}
        <Badge color="var(--fg-dim)" small>{sessions.length} session{sessions.length !== 1 ? "s" : ""}</Badge>
      {/if}
    </div>
    <div class="toolbar-right">
      <button
        class="toggle-btn"
        class:active={groupByAccount}
        onclick={() => (groupByAccount = !groupByAccount)}
        title={groupByAccount ? "Vue liste" : "Grouper par compte"}
      >
        {#if groupByAccount}
          <Layers size={14} />
          <span>Par compte</span>
        {:else}
          <Users size={14} />
          <span>Grouper</span>
        {/if}
      </button>
      <button class="refresh-btn" onclick={load} title="Rafraichir">
        <RefreshCw size={14} class={loading ? "spin" : ""} />
      </button>
    </div>
  </div>

  {#if loading}
    <div class="panel-placeholder">
      <RefreshCw size={20} class="spin" />
      <span>Chargement des sessions...</span>
    </div>
  {:else if error}
    <div class="panel-placeholder panel-error">
      <span>{error}</span>
    </div>
  {:else if sessions.length === 0}
    <div class="panel-placeholder">
      <DollarSign size={20} />
      <span>Aucune session enregistree</span>
    </div>
  {:else if groupByAccount}
    <!-- Vue groupee par compte -->
    <div class="cost-table-wrapper">
      <table class="cost-table">
        <thead>
          <tr>
            <th>Compte</th>
            <th class="num">Sessions</th>
            <th class="num">Requetes</th>
            <th class="num">Input</th>
            <th class="num">Output</th>
            <th class="num">Cout estimé</th>
          </tr>
        </thead>
        <tbody>
          {#each grouped as g (g.email)}
            <tr>
              <td class="account-cell">
                <span class="account-email" title={g.email}>{g.email}</span>
              </td>
              <td class="num">{g.sessions.length}</td>
              <td class="num">{g.totalRequests}</td>
              <td class="num mono">{formatTokens(g.totalInput)}</td>
              <td class="num mono">{formatTokens(g.totalOutput)}</td>
              <td class="num">
                <span class="cost-value" style="color: {costColor(g.totalCost)}">
                  {formatCost(g.totalCost)}
                </span>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <!-- Vue liste par session -->
    <div class="cost-table-wrapper">
      <table class="cost-table">
        <thead>
          <tr>
            <th>Compte</th>
            <th>Modele</th>
            <th class="num">Requetes</th>
            <th class="num">Input</th>
            <th class="num">Output</th>
            <th class="num">Cout estimé</th>
            <th class="num">Mise a jour</th>
          </tr>
        </thead>
        <tbody>
          {#each sorted as s (s.session_id)}
            <tr>
              <td class="account-cell">
                <span class="account-email" title={s.account_email}>{s.account_email ?? "—"}</span>
              </td>
              <td>
                <Badge color="var(--provider-anthropic)" small>
                  {shortModel(s.model)}
                </Badge>
              </td>
              <td class="num">{s.request_count ?? 0}</td>
              <td class="num mono">{formatTokens(s.total_input_tokens ?? 0)}</td>
              <td class="num mono">{formatTokens(s.total_output_tokens ?? 0)}</td>
              <td class="num">
                <span class="cost-value" style="color: {costColor(s.estimated_cost_usd ?? 0)}">
                  {formatCost(s.estimated_cost_usd ?? 0)}
                </span>
              </td>
              <td class="num dim">{formatDate(s.updated_at)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}

  <!-- Pied : total -->
  {#if sessions.length > 0}
    <div class="cost-footer">
      <Card hoverable={false} padding="12px 16px">
        <div class="footer-row">
          <span class="footer-label">Cout total estimé ({sessions.length} sessions)</span>
          <span class="footer-total" style="color: {costColor(totalCost)}">
            {formatCost(totalCost)}
          </span>
        </div>
      </Card>
    </div>
  {/if}
</div>

<style>
  .cost-panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    animation: fade-in 0.2s ease;
  }

  /* Toolbar */
  .panel-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .toolbar-left {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--fg-secondary);
  }

  .toolbar-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .toolbar-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .toggle-btn {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    font-size: 12px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-secondary);
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .toggle-btn:hover {
    background: var(--bg-card-hover);
    color: var(--fg-primary);
    border-color: var(--border-hover);
  }

  .toggle-btn.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-color: var(--accent);
    color: var(--fg-accent);
  }

  .refresh-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4px 6px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-secondary);
    cursor: pointer;
    transition: background 0.1s ease, color 0.1s ease;
  }

  .refresh-btn:hover {
    background: var(--bg-card-hover);
    color: var(--fg-primary);
  }

  /* Spinner */
  :global(.spin) {
    animation: spin 1s linear infinite;
  }

  /* Placeholder */
  .panel-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 48px;
    color: var(--fg-dim);
    font-size: 13px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
  }

  .panel-error {
    color: var(--status-error);
    border-color: color-mix(in srgb, var(--status-error) 30%, transparent);
  }

  /* Table */
  .cost-table-wrapper {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
  }

  .cost-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  .cost-table thead {
    border-bottom: 1px solid var(--border);
  }

  .cost-table th {
    padding: 10px 14px;
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    white-space: nowrap;
  }

  .cost-table th.num {
    text-align: right;
  }

  .cost-table td {
    padding: 9px 14px;
    color: var(--fg-primary);
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
  }

  .cost-table td.num {
    text-align: right;
  }

  .cost-table td.dim {
    color: var(--fg-dim);
  }

  .cost-table tbody tr:last-child td {
    border-bottom: none;
  }

  .cost-table tbody tr:hover td {
    background: var(--bg-card-hover);
  }

  .account-cell {
    max-width: 200px;
  }

  .account-email {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg-secondary);
    font-size: 12px;
  }

  .cost-value {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 12px;
  }

  .mono {
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--fg-secondary);
  }

  /* Footer */
  .cost-footer {
    margin-top: 4px;
  }

  .footer-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .footer-label {
    font-size: 12px;
    color: var(--fg-secondary);
  }

  .footer-total {
    font-size: 15px;
    font-weight: 700;
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-variant-numeric: tabular-nums;
  }
</style>
