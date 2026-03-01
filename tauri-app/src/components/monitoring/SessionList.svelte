<script lang="ts">
  import { onMount } from "svelte";
  import { getSessions } from "../../lib/tauri";
  import Card from "../ui/Card.svelte";
  import Badge from "../ui/Badge.svelte";
  import { Clock, Hash, Coins } from "lucide-svelte";
  import type { SessionInfo } from "../../lib/types";

  let sessions: SessionInfo[] = $state([]);
  let loading = $state(true);

  onMount(async () => {
    try {
      const data = await getSessions();
      sessions = data as SessionInfo[];
    } catch (e) {
      console.error("Failed to load sessions:", e);
    } finally {
      loading = false;
    }
  });

  function formatDate(ts: string): string {
    return new Date(ts).toLocaleString("fr-FR", {
      day: "2-digit",
      month: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function formatTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
    return `${n}`;
  }
</script>

<div class="session-list">
  {#if loading}
    <div class="session-loading">
      <span class="animate-spin">
        <Clock size={20} />
      </span>
      <span>Chargement des sessions...</span>
    </div>
  {:else if sessions.length === 0}
    <div class="session-empty">
      <p>Aucune session enregistree</p>
    </div>
  {:else}
    <div class="session-grid">
      {#each sessions as session (session.id)}
        <Card>
          <div class="session-card">
            <div class="session-header">
              <span class="session-id" title={session.id}>
                #{session.id.slice(0, 8)}
              </span>
              <Badge color="var(--accent)" small>
                {session.accountKey}
              </Badge>
            </div>

            <div class="session-stats">
              <div class="session-stat">
                <Clock size={12} />
                <span>{formatDate(session.startTime)}</span>
              </div>
              <div class="session-stat">
                <Hash size={12} />
                <span>{session.requestCount} requetes</span>
              </div>
              <div class="session-stat">
                <Coins size={12} />
                <span>{formatTokens(session.tokensUsed)} tokens</span>
              </div>
            </div>
          </div>
        </Card>
      {/each}
    </div>
  {/if}
</div>

<style>
  .session-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .session-loading,
  .session-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 48px;
    color: var(--fg-dim);
    font-size: 13px;
  }

  .session-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
  }

  .session-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .session-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .session-id {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-primary);
    font-family: "JetBrains Mono", "Fira Code", monospace;
  }

  .session-stats {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .session-stat {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--fg-secondary);
  }
</style>
