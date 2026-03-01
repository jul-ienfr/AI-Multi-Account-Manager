<script lang="ts">
  import { onMount } from "svelte";
  import Badge from "../ui/Badge.svelte";
  import { ArrowUpRight, ArrowDownLeft, Filter, RefreshCw } from "lucide-svelte";
  import { getLogs } from "../../lib/tauri";

  interface RequestEntry {
    id: string;
    timestamp: string;
    method: string;
    path: string;
    status: number;
    provider: string;
    account: string;
    duration: number;
    tokens?: number;
  }

  let requests: RequestEntry[] = $state([]);
  let filterProvider = $state("all");
  let filterStatus = $state("all");
  let feedEl: HTMLDivElement | undefined = $state();
  let autoScroll = $state(true);

  const providerColors: Record<string, string> = {
    anthropic: "var(--provider-anthropic)",
    gemini: "var(--provider-gemini)",
    openai: "var(--provider-openai)",
    xai: "var(--provider-xai)",
    deepseek: "var(--provider-deepseek)",
    mistral: "var(--provider-mistral)",
    groq: "var(--provider-groq)",
  };

  let filtered = $derived(
    requests.filter(r => {
      if (filterProvider !== "all" && r.provider !== filterProvider) return false;
      if (filterStatus === "success" && (r.status < 200 || r.status >= 300)) return false;
      if (filterStatus === "error" && r.status < 400) return false;
      return true;
    })
  );

  function statusColor(status: number): string {
    if (status >= 200 && status < 300) return "var(--phase-cruise)";
    if (status >= 400 && status < 500) return "var(--status-warning)";
    if (status >= 500) return "var(--status-error)";
    return "var(--fg-dim)";
  }

  function formatTime(ts: string): string {
    return new Date(ts).toLocaleTimeString("fr-FR", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  }

  async function loadRequests() {
    try {
      const raw = await getLogs();
      // api_usage.jsonl entries have: timestamp, account_email, model, input_tokens, output_tokens, client_ip, client_fmt
      requests = (raw as Record<string, unknown>[]).flatMap((entry, i) => {
        if (!entry || typeof entry !== 'object') return [];
        const ts = String(entry.timestamp ?? entry.time ?? '');
        const email = String(entry.account_email ?? entry.email ?? 'unknown');
        const model = String(entry.model ?? '');
        const tokens = Number(entry.output_tokens ?? entry.tokens_output ?? entry.total_tokens ?? 0);
        // Derive provider from model name
        let provider = 'anthropic';
        if (model.includes('gemini')) provider = 'gemini';
        else if (model.includes('gpt')) provider = 'openai';
        else if (model.includes('grok')) provider = 'xai';
        else if (model.includes('deepseek')) provider = 'deepseek';
        else if (model.includes('mistral')) provider = 'mistral';
        else if (model.includes('llama') || model.includes('groq')) provider = 'groq';
        return [{
          id: String(i),
          timestamp: ts,
          method: 'POST',
          path: '/v1/messages',
          status: 200,
          provider,
          account: email,
          duration: 0,
          tokens: tokens || undefined,
        }] satisfies RequestEntry[];
      });
      // Newest first
      requests = requests.reverse();
    } catch (e) {
      console.error("Failed to load requests:", e);
    }
  }

  onMount(async () => {
    await loadRequests();
  });

  $effect(() => {
    if (autoScroll && feedEl && filtered.length > 0) {
      feedEl.scrollTop = feedEl.scrollHeight;
    }
  });
</script>

<div class="request-feed">
  <div class="feed-controls">
    <div class="feed-filters">
      <Filter size={14} />
      <select class="feed-select" bind:value={filterProvider}>
        <option value="all">Tous les providers</option>
        <option value="anthropic">Anthropic</option>
        <option value="gemini">Gemini</option>
        <option value="openai">OpenAI</option>
        <option value="xai">xAI</option>
        <option value="deepseek">DeepSeek</option>
        <option value="mistral">Mistral</option>
        <option value="groq">Groq</option>
      </select>

      <select class="feed-select" bind:value={filterStatus}>
        <option value="all">Tous les statuts</option>
        <option value="success">Succes (2xx)</option>
        <option value="error">Erreurs (4xx/5xx)</option>
      </select>
    </div>

    <label class="auto-scroll-toggle">
      <input type="checkbox" bind:checked={autoScroll} />
      <span>Auto-scroll</span>
    </label>

    <button class="refresh-btn" onclick={loadRequests} title="Rafraichir">
      <RefreshCw size={14} />
    </button>
  </div>

  <div class="feed-list" bind:this={feedEl}>
    {#each filtered as req (req.id)}
      <div class="feed-item">
        <span class="feed-time">{formatTime(req.timestamp)}</span>
        <span class="feed-method">
          {#if req.method === "POST"}
            <ArrowUpRight size={12} />
          {:else}
            <ArrowDownLeft size={12} />
          {/if}
          {req.method}
        </span>
        <span class="feed-path" title={req.path}>{req.path}</span>
        <Badge color={statusColor(req.status)} small>{req.status}</Badge>
        <Badge color={providerColors[req.provider] ?? "var(--fg-dim)"} small>
          {req.provider}
        </Badge>
        <span class="feed-duration">{req.duration}ms</span>
        {#if req.tokens}
          <span class="feed-tokens">{req.tokens}t</span>
        {/if}
      </div>
    {/each}

    {#if filtered.length === 0}
      <div class="feed-empty">
        <p>Aucune requete a afficher</p>
        <p class="feed-empty-hint">Les requetes apparaitront ici en temps reel</p>
      </div>
    {/if}
  </div>
</div>

<style>
  .request-feed {
    display: flex;
    flex-direction: column;
    gap: 12px;
    height: 100%;
  }

  .feed-controls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .feed-filters {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--fg-dim);
  }

  .feed-select {
    padding: 4px 8px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-primary);
    font-size: 12px;
  }

  .feed-select:focus {
    outline: none;
    border-color: var(--accent);
  }

  .auto-scroll-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--fg-secondary);
    cursor: pointer;
    user-select: none;
  }

  .auto-scroll-toggle input {
    accent-color: var(--accent);
  }

  .feed-list {
    flex: 1;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
    min-height: 300px;
    max-height: 500px;
  }

  .feed-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    transition: background 0.1s ease;
  }

  .feed-item:last-child {
    border-bottom: none;
  }

  .feed-item:hover {
    background: var(--bg-card-hover);
  }

  .feed-time {
    color: var(--fg-dim);
    font-variant-numeric: tabular-nums;
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 11px;
    flex-shrink: 0;
    width: 70px;
  }

  .feed-method {
    display: flex;
    align-items: center;
    gap: 3px;
    color: var(--fg-secondary);
    font-weight: 600;
    width: 50px;
    flex-shrink: 0;
  }

  .feed-path {
    flex: 1;
    color: var(--fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 11px;
  }

  .feed-duration {
    color: var(--fg-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
    width: 50px;
    text-align: right;
  }

  .feed-tokens {
    color: var(--fg-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
    width: 45px;
    text-align: right;
  }

  .feed-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 48px;
    text-align: center;
    color: var(--fg-dim);
  }

  .feed-empty-hint {
    font-size: 12px;
    margin-top: 4px;
    color: var(--fg-dim);
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
</style>
