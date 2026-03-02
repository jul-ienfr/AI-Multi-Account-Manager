<script lang="ts">
  import type { ProxyInstanceState, ProxyInstanceConfig } from "../../lib/types";
  import { proxyInstances } from "../../lib/stores/proxy";
  import { setupClaudeCode, removeClaudeCodeSetup, setupVscodeProxy, removeVscodeProxy } from "../../lib/tauri";
  import Card from "../ui/Card.svelte";
  import Badge from "../ui/Badge.svelte";
  import Button from "../ui/Button.svelte";
  import { Power, RotateCw, Radio, Zap, Settings, Trash2, Pencil, Terminal, Code } from "lucide-svelte";

  interface DetectedBinary {
    id: string;
    name: string;
    path: string;
    defaultPort: number;
  }

  interface Props {
    instance: ProxyInstanceState;
    detectedBinaries?: DetectedBinary[];
  }

  let { instance, detectedBinaries = [] }: Props = $props();

  let loading = $state(false);
  let editing = $state(false);
  let editName = $state("");
  let editPort = $state(0);
  let editBinaryPath = $state("");

  let isRouter = $derived(instance.config.kind === "router");
  let isImpersonator = $derived(instance.config.kind === "impersonator");

  let engineName = $derived(() => {
    // Si un binaire externe est configuré, l'afficher en priorité
    if (instance.config.binaryPath) {
      const bin = detectedBinaries.find((b) => b.path === instance.config.binaryPath);
      if (bin) return bin.name;
      const parts = instance.config.binaryPath.replace(/\\/g, "/").split("/");
      return parts[parts.length - 1] || "Externe";
    }
    // Sinon utiliser le backend détecté par probe
    if (instance.status.backend) {
      const b = instance.status.backend;
      if (b === "python") return "V2 (Python)";
      if (b === "rust-auto") return "V3 (Rust)";
      if (b === "unknown") return "Externe";
      return `Externe (${b})`;
    }
    return "Integre";
  });

  function formatUptime(secs: number | undefined | null): string {
    if (secs == null || isNaN(secs) || secs <= 0) return "--";
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return `${h}h ${m}m`;
  }

  async function toggle() {
    loading = true;
    try {
      if (instance.status.running) {
        await proxyInstances.stop(instance.config.id);
      } else {
        await proxyInstances.start(instance.config.id);
      }
    } finally {
      loading = false;
    }
  }

  async function restart() {
    loading = true;
    try {
      await proxyInstances.restart(instance.config.id);
    } finally {
      loading = false;
    }
  }

  async function remove() {
    if (instance.status.running) {
      await proxyInstances.stop(instance.config.id);
    }
    await proxyInstances.remove(instance.config.id);
  }

  function startEdit() {
    editName = instance.config.name;
    editPort = instance.config.port;
    editBinaryPath = instance.config.binaryPath || "";
    editing = true;
  }

  async function saveEdit() {
    await proxyInstances.update(instance.config.id, {
      name: editName,
      port: editPort,
      binaryPath: editBinaryPath || null,
    } as any);
    editing = false;
  }

  function cancelEdit() {
    editing = false;
  }

  async function toggleSetupCC() {
    const hasCC = instance.config.setupTargets.includes("claude-code");
    if (hasCC) {
      await removeClaudeCodeSetup();
      await proxyInstances.update(instance.config.id, {
        setupTargets: instance.config.setupTargets.filter((t) => t !== "claude-code"),
      } as any);
    } else {
      await setupClaudeCode(instance.config.port);
      await proxyInstances.update(instance.config.id, {
        setupTargets: [...instance.config.setupTargets, "claude-code"],
      } as any);
    }
  }

  async function toggleSetupVSCode() {
    const hasVS = instance.config.setupTargets.includes("vscode");
    if (hasVS) {
      await removeVscodeProxy();
      await proxyInstances.update(instance.config.id, {
        setupTargets: instance.config.setupTargets.filter((t) => t !== "vscode"),
      } as any);
    } else {
      await setupVscodeProxy(instance.config.port);
      await proxyInstances.update(instance.config.id, {
        setupTargets: [...instance.config.setupTargets, "vscode"],
      } as any);
    }
  }
</script>

<Card>
  <div class="instance-card">
    <div class="instance-header">
      <span class="instance-icon">
        {#if isRouter}
          <Radio size={20} />
        {:else if isImpersonator}
          <Zap size={20} />
        {:else}
          <Settings size={20} />
        {/if}
      </span>

      {#if editing}
        <div class="edit-form">
          <div class="edit-row">
            <input
              class="edit-input"
              type="text"
              bind:value={editName}
              placeholder="Nom"
            />
            <input
              class="edit-input port-input"
              type="number"
              bind:value={editPort}
              min="1024"
              max="65535"
            />
          </div>
          <div class="edit-row">
            <select class="edit-input" bind:value={editBinaryPath}>
              <option value="">Integre</option>
              {#each detectedBinaries as bin}
                <option value={bin.path}>{bin.name}</option>
              {/each}
            </select>
            <Button size="sm" variant="primary" onclick={saveEdit}>OK</Button>
            <Button size="sm" variant="ghost" onclick={cancelEdit}>X</Button>
          </div>
        </div>
      {:else}
        <div class="instance-info">
          <h3 class="instance-name">{instance.config.name}</h3>
          <span class="instance-port">:{instance.config.port}</span>
        </div>
        <Badge color={instance.status.running ? "var(--status-running)" : "var(--status-stopped)"}>
          {instance.status.running ? "Actif" : "Arrete"}
        </Badge>
      {/if}
    </div>

    <div class="instance-stats">
      <div class="stat">
        <span class="stat-label">Moteur</span>
        <span class="stat-value engine-value">{engineName()}</span>
      </div>
      <div class="stat">
        <span class="stat-label">Uptime</span>
        <span class="stat-value">{instance.status.running ? formatUptime(instance.status.uptimeSecs) : "--"}</span>
      </div>
      <div class="stat">
        <span class="stat-label">Requetes</span>
        <span class="stat-value">{instance.status.requestsTotal ?? 0}</span>
      </div>
      <div class="stat">
        <span class="stat-label">Actives</span>
        <span class="stat-value">{instance.status.requestsActive ?? 0}</span>
      </div>
    </div>

    <div class="setup-row">
      <span class="setup-label">Setup:</span>
      <button
        class="setup-btn"
        class:active={instance.config.setupTargets.includes("claude-code")}
        onclick={toggleSetupCC}
        title="Injecter ANTHROPIC_BASE_URL dans Claude Code"
      >
        <Terminal size={12} />
        CC
      </button>
      <button
        class="setup-btn"
        class:active={instance.config.setupTargets.includes("vscode")}
        onclick={toggleSetupVSCode}
        title="Injecter http.proxy dans VS Code"
      >
        <Code size={12} />
        VSCode
      </button>
    </div>

    <div class="instance-actions">
      <Button
        variant={instance.status.running ? "secondary" : "primary"}
        size="sm"
        onclick={toggle}
        disabled={loading}
      >
        <Power size={14} />
        {instance.status.running ? "Arreter" : "Demarrer"}
      </Button>
      {#if instance.status.running}
        <Button variant="ghost" size="sm" onclick={restart} disabled={loading}>
          <RotateCw size={14} />
          Redemarrer
        </Button>
      {/if}
      <div class="actions-spacer"></div>
      <Button variant="ghost" size="sm" onclick={startEdit}>
        <Pencil size={14} />
      </Button>
      <Button variant="ghost" size="sm" onclick={remove}>
        <Trash2 size={14} />
      </Button>
    </div>
  </div>
</Card>

<style>
  .instance-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .instance-header {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .instance-icon {
    display: flex;
    color: var(--accent);
    flex-shrink: 0;
  }

  .instance-info {
    flex: 1;
    display: flex;
    align-items: baseline;
    gap: 4px;
  }

  .instance-name {
    font-size: 15px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .instance-port {
    font-size: 12px;
    color: var(--fg-dim);
    font-variant-numeric: tabular-nums;
  }

  .edit-form {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .edit-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .edit-input {
    flex: 1;
    min-width: 0;
    padding: 4px 8px;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-primary);
    font-size: 13px;
  }

  .edit-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .port-input { width: 80px; flex: none; text-align: center; }

  .instance-stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 12px;
    padding: 10px 12px;
    background: var(--bg-app);
    border-radius: var(--radius-md);
  }

  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .stat-label {
    font-size: 10px;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .stat-value {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-primary);
    font-variant-numeric: tabular-nums;
  }

  .engine-value {
    font-size: 11px;
    font-weight: 500;
  }

  .setup-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .setup-label {
    font-size: 11px;
    color: var(--fg-dim);
  }

  .setup-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 8px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--bg-app);
    color: var(--fg-dim);
    font-size: 11px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .setup-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .setup-btn.active {
    background: color-mix(in srgb, var(--status-running) 15%, transparent);
    border-color: var(--status-running);
    color: var(--status-running);
  }

  .instance-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .actions-spacer {
    flex: 1;
  }
</style>
