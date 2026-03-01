<script lang="ts">
  import { onMount } from "svelte";
  import { getPeers } from "../../lib/tauri";
  import Badge from "../ui/Badge.svelte";
  import { Network, List, RefreshCw } from "lucide-svelte";

  // Le backend retourne des pairs avec : id, addr (ou host+port), last_seen, status
  // On gere les deux formes pour compatibilite
  interface PeerRaw {
    id: string;
    // forme "addr" directe
    addr?: string;
    // forme host+port separee
    host?: string;
    port?: number;
    connected?: boolean;
    last_seen?: string;
    lastSeen?: string;
    // statut explicite ou derive
    status?: "ALIVE" | "SUSPECT" | "DEAD" | string;
  }

  interface PeerNode {
    id: string;
    addr: string;
    status: "ALIVE" | "SUSPECT" | "DEAD";
    lastSeen: string | null;
    latencyMs: number | null;
  }

  type ViewMode = "list" | "graph";

  let peers: PeerNode[] = $state([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let viewMode: ViewMode = $state("list");
  let refreshInterval: ReturnType<typeof setInterval> | null = null;

  // SVG dimensions
  const SVG_W = 500;
  const SVG_H = 340;
  const CENTER_X = SVG_W / 2;
  const CENTER_Y = SVG_H / 2;
  const ORBIT_R = 120;
  const NODE_R = 22;
  const CENTER_R = 28;

  function normalizeStatus(raw: PeerRaw): "ALIVE" | "SUSPECT" | "DEAD" {
    if (raw.status === "ALIVE" || raw.status === "alive") return "ALIVE";
    if (raw.status === "SUSPECT" || raw.status === "suspect") return "SUSPECT";
    if (raw.status === "DEAD" || raw.status === "dead") return "DEAD";
    // Derive depuis connected si status absent
    if (raw.connected === true) return "ALIVE";
    if (raw.connected === false) return "DEAD";
    return "SUSPECT";
  }

  function normalizeAddr(raw: PeerRaw): string {
    if (raw.addr) return raw.addr;
    if (raw.host && raw.port) return `${raw.host}:${raw.port}`;
    if (raw.host) return raw.host;
    return raw.id;
  }

  function normalizePeer(raw: PeerRaw): PeerNode {
    return {
      id: raw.id,
      addr: normalizeAddr(raw),
      status: normalizeStatus(raw),
      lastSeen: raw.last_seen ?? raw.lastSeen ?? null,
      latencyMs: null, // non fourni par le backend actuellement
    };
  }

  async function load() {
    try {
      const raw = await getPeers();
      peers = (raw as PeerRaw[]).map(normalizePeer);
      error = null;
    } catch (e) {
      console.error("PeerTopology: failed to load peers", e);
      error = "Impossible de charger les pairs";
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    load();
    refreshInterval = setInterval(load, 5_000);
    return () => {
      if (refreshInterval !== null) clearInterval(refreshInterval);
    };
  });

  // Couleurs selon statut
  function statusColor(status: string): string {
    if (status === "ALIVE") return "var(--phase-cruise)";
    if (status === "SUSPECT") return "var(--status-warning)";
    return "var(--status-error)";
  }

  function statusLabel(status: string): string {
    if (status === "ALIVE") return "Actif";
    if (status === "SUSPECT") return "Suspect";
    return "Hors ligne";
  }

  function formatLastSeen(ts: string | null): string {
    if (!ts) return "jamais";
    const diff = Date.now() - new Date(ts).getTime();
    const s = Math.floor(diff / 1000);
    if (s < 60) return `il y a ${s}s`;
    const m = Math.floor(s / 60);
    if (m < 60) return `il y a ${m}min`;
    return `il y a ${Math.floor(m / 60)}h`;
  }

  // Calcul des positions SVG en cercle autour du centre
  function peerPosition(index: number, total: number): { x: number; y: number } {
    if (total === 0) return { x: CENTER_X, y: CENTER_Y };
    const angle = (2 * Math.PI * index) / total - Math.PI / 2;
    return {
      x: CENTER_X + ORBIT_R * Math.cos(angle),
      y: CENTER_Y + ORBIT_R * Math.sin(angle),
    };
  }

  // Comptes de statuts pour la legende
  let aliveCount = $derived(peers.filter((p) => p.status === "ALIVE").length);
  let suspectCount = $derived(peers.filter((p) => p.status === "SUSPECT").length);
  let deadCount = $derived(peers.filter((p) => p.status === "DEAD").length);
</script>

<div class="peer-topology">
  <!-- Toolbar -->
  <div class="pt-toolbar">
    <div class="pt-toolbar-left">
      <Network size={16} />
      <span class="pt-title">Topologie reseau</span>
      {#if !loading}
        <div class="pt-stats">
          {#if aliveCount > 0}
            <span class="stat-pill" style="color: var(--phase-cruise)">{aliveCount} actif{aliveCount !== 1 ? "s" : ""}</span>
          {/if}
          {#if suspectCount > 0}
            <span class="stat-pill" style="color: var(--status-warning)">{suspectCount} suspect{suspectCount !== 1 ? "s" : ""}</span>
          {/if}
          {#if deadCount > 0}
            <span class="stat-pill" style="color: var(--status-error)">{deadCount} hors ligne</span>
          {/if}
        </div>
      {/if}
    </div>
    <div class="pt-toolbar-right">
      <!-- Toggle mode -->
      <div class="mode-toggle">
        <button
          class="mode-btn"
          class:active={viewMode === "list"}
          onclick={() => (viewMode = "list")}
          title="Vue liste"
        >
          <List size={14} />
        </button>
        <button
          class="mode-btn"
          class:active={viewMode === "graph"}
          onclick={() => (viewMode = "graph")}
          title="Vue graphe"
        >
          <Network size={14} />
        </button>
      </div>
      <button class="refresh-btn" onclick={load} title="Rafraichir">
        <RefreshCw size={14} />
      </button>
    </div>
  </div>

  {#if loading}
    <div class="pt-placeholder">
      <RefreshCw size={20} class="spin" />
      <span>Chargement des pairs...</span>
    </div>
  {:else if error}
    <div class="pt-placeholder pt-error">
      <span>{error}</span>
    </div>
  {:else if peers.length === 0}
    <div class="pt-placeholder">
      <Network size={24} />
      <span>Aucun pair configure</span>
      <span class="pt-placeholder-hint">Ajoutez des pairs dans les parametres de synchronisation</span>
    </div>
  {:else if viewMode === "list"}
    <!-- Vue liste -->
    <div class="pt-table-wrapper">
      <table class="pt-table">
        <thead>
          <tr>
            <th>Adresse</th>
            <th>Statut</th>
            <th>Latence</th>
            <th>Derniere activite</th>
            <th class="id-col">ID</th>
          </tr>
        </thead>
        <tbody>
          {#each peers as peer (peer.id)}
            <tr>
              <td class="addr-cell">
                <span class="status-dot" style="background: {statusColor(peer.status)}"></span>
                <span class="addr-text">{peer.addr}</span>
              </td>
              <td>
                <Badge color={statusColor(peer.status)} small>
                  {statusLabel(peer.status)}
                </Badge>
              </td>
              <td class="dim">
                {peer.latencyMs !== null ? `${peer.latencyMs}ms` : "—"}
              </td>
              <td class="dim">{formatLastSeen(peer.lastSeen)}</td>
              <td class="id-col mono dim" title={peer.id}>{peer.id.slice(0, 12)}…</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <!-- Vue graphe (SVG pur) -->
    <div class="pt-graph-wrapper">
      <svg
        viewBox="0 0 {SVG_W} {SVG_H}"
        class="pt-svg"
        role="img"
        aria-label="Topologie reseau des pairs"
      >
        <!-- Lignes de connexion (pairs → centre) -->
        {#each peers as peer, i (peer.id)}
          {@const pos = peerPosition(i, peers.length)}
          <line
            x1={CENTER_X}
            y1={CENTER_Y}
            x2={pos.x}
            y2={pos.y}
            stroke={statusColor(peer.status)}
            stroke-width={peer.status === "ALIVE" ? 1.5 : 1}
            stroke-opacity={peer.status === "ALIVE" ? 0.5 : 0.2}
            stroke-dasharray={peer.status === "DEAD" ? "4 4" : "none"}
          />
        {/each}

        <!-- Orbite circulaire (guide visuel) -->
        <circle
          cx={CENTER_X}
          cy={CENTER_Y}
          r={ORBIT_R}
          fill="none"
          stroke="var(--border)"
          stroke-width="1"
          stroke-dasharray="3 6"
          opacity="0.4"
        />

        <!-- Nœuds pairs -->
        {#each peers as peer, i (peer.id)}
          {@const pos = peerPosition(i, peers.length)}
          <g class="peer-node" transform="translate({pos.x},{pos.y})">
            <!-- Halo pour pairs ALIVE -->
            {#if peer.status === "ALIVE"}
              <circle
                cx="0"
                cy="0"
                r={NODE_R + 6}
                fill={statusColor(peer.status)}
                opacity="0.08"
              />
            {/if}
            <!-- Cercle principal -->
            <circle
              cx="0"
              cy="0"
              r={NODE_R}
              fill="var(--bg-card)"
              stroke={statusColor(peer.status)}
              stroke-width="2"
            />
            <!-- Icone statut -->
            <text
              x="0"
              y="0"
              text-anchor="middle"
              dominant-baseline="central"
              font-size="11"
              font-weight="700"
              fill={statusColor(peer.status)}
            >
              {peer.status === "ALIVE" ? "●" : peer.status === "SUSPECT" ? "◐" : "○"}
            </text>
            <!-- Adresse sous le nœud -->
            <text
              x="0"
              y={NODE_R + 14}
              text-anchor="middle"
              dominant-baseline="central"
              font-size="9"
              fill="var(--fg-secondary)"
            >
              {peer.addr.length > 18 ? peer.addr.slice(0, 18) + "…" : peer.addr}
            </text>
          </g>
        {/each}

        <!-- Nœud central (nous) -->
        <g transform="translate({CENTER_X},{CENTER_Y})">
          <circle
            cx="0"
            cy="0"
            r={CENTER_R + 8}
            fill="var(--accent)"
            opacity="0.08"
          />
          <circle
            cx="0"
            cy="0"
            r={CENTER_R}
            fill="var(--bg-card)"
            stroke="var(--accent)"
            stroke-width="2.5"
          />
          <text
            x="0"
            y="-5"
            text-anchor="middle"
            dominant-baseline="central"
            font-size="10"
            font-weight="700"
            fill="var(--fg-accent)"
          >
            Vous
          </text>
          <text
            x="0"
            y="8"
            text-anchor="middle"
            dominant-baseline="central"
            font-size="9"
            fill="var(--fg-dim)"
          >
            (local)
          </text>
        </g>
      </svg>

      <!-- Legende -->
      <div class="graph-legend">
        <span class="legend-item">
          <span class="legend-dot" style="background: var(--phase-cruise)"></span>
          Actif
        </span>
        <span class="legend-item">
          <span class="legend-dot" style="background: var(--status-warning)"></span>
          Suspect
        </span>
        <span class="legend-item">
          <span class="legend-dot" style="background: var(--status-error)"></span>
          Hors ligne
        </span>
        <span class="legend-item">
          <span class="legend-line-dashed"></span>
          Connexion inactive
        </span>
      </div>
    </div>
  {/if}
</div>

<style>
  .peer-topology {
    display: flex;
    flex-direction: column;
    gap: 12px;
    animation: fade-in 0.2s ease;
  }

  /* Toolbar */
  .pt-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .pt-toolbar-left {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--fg-secondary);
    flex-wrap: wrap;
  }

  .pt-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .pt-stats {
    display: flex;
    gap: 8px;
  }

  .stat-pill {
    font-size: 12px;
    font-weight: 600;
  }

  .pt-toolbar-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .mode-toggle {
    display: flex;
    gap: 2px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 2px;
  }

  .mode-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4px 8px;
    border-radius: 4px;
    color: var(--fg-dim);
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .mode-btn:hover {
    color: var(--fg-primary);
    background: var(--bg-card-hover);
  }

  .mode-btn.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
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
  .pt-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 48px 24px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
    color: var(--fg-dim);
    font-size: 13px;
    text-align: center;
  }

  .pt-placeholder-hint {
    font-size: 12px;
    color: var(--fg-dim);
  }

  .pt-error {
    color: var(--status-error);
    border-color: color-mix(in srgb, var(--status-error) 30%, transparent);
  }

  /* Table */
  .pt-table-wrapper {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
  }

  .pt-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  .pt-table thead {
    border-bottom: 1px solid var(--border);
  }

  .pt-table th {
    padding: 10px 14px;
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    white-space: nowrap;
  }

  .pt-table td {
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
    color: var(--fg-primary);
  }

  .pt-table td.dim {
    color: var(--fg-dim);
    font-size: 12px;
  }

  .pt-table tbody tr:last-child td {
    border-bottom: none;
  }

  .pt-table tbody tr:hover td {
    background: var(--bg-card-hover);
  }

  .addr-cell {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .status-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .addr-text {
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 12px;
  }

  .id-col {
    max-width: 120px;
  }

  .mono {
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 11px;
  }

  /* Graphe SVG */
  .pt-graph-wrapper {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
  }

  .pt-svg {
    width: 100%;
    max-width: 500px;
    height: auto;
    display: block;
  }

  .peer-node {
    cursor: default;
  }

  .peer-node:hover circle:not(:first-child) {
    filter: brightness(1.15);
  }

  /* Legende */
  .graph-legend {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
    justify-content: center;
  }

  .legend-item {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--fg-dim);
  }

  .legend-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .legend-line-dashed {
    display: inline-block;
    width: 20px;
    height: 2px;
    border-top: 2px dashed var(--fg-dim);
  }
</style>
