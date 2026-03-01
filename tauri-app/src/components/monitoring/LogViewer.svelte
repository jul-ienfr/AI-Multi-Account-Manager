<script lang="ts">
  import { onMount } from "svelte";
  import { getLogs } from "../../lib/tauri";
  import Badge from "../ui/Badge.svelte";
  import Button from "../ui/Button.svelte";
  import { Search, Download, Trash2 } from "lucide-svelte";

  interface LogLine {
    raw: string;
    timestamp: string;
    level: string;
    message: string;
  }

  let logs: LogLine[] = $state([]);
  let searchQuery = $state("");
  let levelFilter = $state("all");
  let autoScroll = $state(true);
  let logEl: HTMLDivElement | undefined = $state();

  const levelColors: Record<string, string> = {
    info: "var(--accent)",
    warn: "var(--status-warning)",
    error: "var(--status-error)",
    debug: "var(--fg-dim)",
  };

  let filtered = $derived(
    logs.filter(l => {
      if (levelFilter !== "all" && l.level !== levelFilter) return false;
      if (searchQuery && !l.raw.toLowerCase().includes(searchQuery.toLowerCase())) return false;
      return true;
    })
  );

  function parseEntry(raw: unknown): LogLine {
    if (typeof raw === 'object' && raw !== null) {
      const obj = raw as Record<string, unknown>;
      // api_usage.jsonl entry
      const ts = String(obj.timestamp ?? obj.time ?? '');
      const model = String(obj.model ?? '');
      const email = String(obj.account_email ?? obj.email ?? '');
      const tokens = Number(obj.output_tokens ?? obj.total_tokens ?? 0);
      const message = email ? `${email} | ${model} | ${tokens}t` : JSON.stringify(raw);
      return { raw: JSON.stringify(raw), timestamp: ts, level: 'info', message };
    }
    // Fallback for string lines
    const str = String(raw);
    const match = str.match(/^\[([^\]]+)\]\s*(\w+)\s*(.*)$/);
    if (match) {
      return { raw: str, timestamp: match[1], level: match[2].toLowerCase(), message: match[3] };
    }
    return { raw: str, timestamp: '', level: 'info', message: str };
  }

  onMount(async () => {
    try {
      const entries = await getLogs(undefined);
      logs = entries.map(parseEntry);
    } catch (e) {
      console.error("Failed to load logs:", e);
    }
  });

  async function refresh() {
    try {
      const entries = await getLogs(levelFilter === "all" ? undefined : levelFilter);
      logs = entries.map(parseEntry);
    } catch (e) {
      console.error("Failed to refresh logs:", e);
    }
  }

  function clear() {
    logs = [];
  }

  $effect(() => {
    if (autoScroll && logEl && filtered.length > 0) {
      logEl.scrollTop = logEl.scrollHeight;
    }
  });
</script>

<div class="log-viewer">
  <div class="log-controls">
    <div class="log-search">
      <Search size={14} />
      <input
        type="text"
        class="log-search-input"
        placeholder="Rechercher dans les logs..."
        bind:value={searchQuery}
      />
    </div>

    <div class="log-filters">
      {#each (["all", "info", "warn", "error", "debug"] as const) as level}
        <Button
          variant={levelFilter === level ? "primary" : "ghost"}
          size="sm"
          onclick={() => { levelFilter = level; }}
        >
          {level === "all" ? "Tous" : level.toUpperCase()}
        </Button>
      {/each}
    </div>

    <div class="log-actions">
      <label class="auto-scroll-toggle">
        <input type="checkbox" bind:checked={autoScroll} />
        <span>Auto-scroll</span>
      </label>
      <Button variant="ghost" size="sm" onclick={refresh}>
        <Download size={14} />
      </Button>
      <Button variant="ghost" size="sm" onclick={clear}>
        <Trash2 size={14} />
      </Button>
    </div>
  </div>

  <div class="log-output" bind:this={logEl}>
    {#each filtered as log, i}
      <div class="log-line">
        <span class="log-num">{i + 1}</span>
        {#if log.timestamp}
          <span class="log-ts">{log.timestamp}</span>
        {/if}
        <Badge color={levelColors[log.level] ?? "var(--fg-dim)"} small>
          {log.level}
        </Badge>
        <span class="log-msg">{log.message}</span>
      </div>
    {/each}

    {#if filtered.length === 0}
      <div class="log-empty">Aucun log a afficher</div>
    {/if}
  </div>
</div>

<style>
  .log-viewer {
    display: flex;
    flex-direction: column;
    gap: 12px;
    height: 100%;
  }

  .log-controls {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }

  .log-search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    color: var(--fg-dim);
    flex: 1;
    min-width: 200px;
  }

  .log-search:focus-within {
    border-color: var(--accent);
  }

  .log-search-input {
    flex: 1;
    background: none;
    border: none;
    outline: none;
    color: var(--fg-primary);
    font-size: 12px;
  }

  .log-filters {
    display: flex;
    gap: 2px;
  }

  .log-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .auto-scroll-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--fg-secondary);
    cursor: pointer;
    user-select: none;
  }

  .auto-scroll-toggle input {
    accent-color: var(--accent);
  }

  .log-output {
    flex: 1;
    overflow-y: auto;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 11px;
    min-height: 300px;
    max-height: 500px;
  }

  .log-line {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 40%, transparent);
    transition: background 0.1s ease;
  }

  .log-line:hover {
    background: var(--bg-card-hover);
  }

  .log-num {
    color: var(--fg-dim);
    width: 32px;
    text-align: right;
    flex-shrink: 0;
    opacity: 0.5;
    font-size: 10px;
  }

  .log-ts {
    color: var(--fg-dim);
    flex-shrink: 0;
    font-size: 10px;
  }

  .log-msg {
    color: var(--fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  .log-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 48px;
    color: var(--fg-dim);
    font-size: 12px;
  }
</style>
