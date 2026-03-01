<script lang="ts">
  import { onMount } from "svelte";
  import { getImpersonationProfiles } from "../../lib/tauri";
  import type { ImpersonationProfile } from "../../lib/types";
  import Card from "../ui/Card.svelte";
  import Badge from "../ui/Badge.svelte";
  import Button from "../ui/Button.svelte";
  import { RefreshCw, ChevronDown, ChevronRight, Shield } from "lucide-svelte";

  const providerColors: Record<string, string> = {
    anthropic: "var(--provider-anthropic)",
    gemini: "var(--provider-gemini)",
    openai: "var(--provider-openai)",
    xai: "var(--provider-xai)",
    deepseek: "var(--provider-deepseek)",
    mistral: "var(--provider-mistral)",
    groq: "var(--provider-groq)",
  };

  let profiles: ImpersonationProfile[] = $state([]);
  let loading = $state(true);
  let error = $state("");
  let expanded = $state<Record<string, boolean>>({});

  onMount(async () => {
    await load();
  });

  async function load() {
    loading = true;
    error = "";
    try {
      profiles = await getImpersonationProfiles();
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  function toggle(name: string) {
    expanded = { ...expanded, [name]: !expanded[name] };
  }

  function formatDate(ts?: string): string {
    if (!ts) return "jamais";
    try {
      return new Date(ts).toLocaleString("fr-FR", {
        day: "2-digit", month: "2-digit", year: "2-digit",
        hour: "2-digit", minute: "2-digit",
      });
    } catch { return ts; }
  }

  function staticHeaderCount(p: ImpersonationProfile): number {
    return Object.keys(p.static_headers ?? {}).length;
  }

  function dynamicHeaderCount(p: ImpersonationProfile): number {
    return Object.keys(p.dynamic_headers ?? {}).length;
  }
</script>

<div class="profiles-list">
  <div class="profiles-header">
    <div class="profiles-title">
      <Shield size={16} />
      <span>Profils d'impersonation</span>
    </div>
    <Button variant="ghost" size="sm" onclick={load}>
      <RefreshCw size={14} />
    </Button>
  </div>

  {#if loading}
    <div class="profiles-state">Chargement des profils...</div>
  {:else if error}
    <div class="profiles-state error">{error}</div>
  {:else if profiles.length === 0}
    <div class="profiles-state">
      <Shield size={32} />
      <p>Aucun profil capture</p>
      <p class="hint">Les profils sont crees automatiquement lors des premieres requetes Claude Code</p>
    </div>
  {:else}
    {#each profiles as profile}
      <Card>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="profile-card">
          <div class="profile-header" onclick={() => toggle(profile.provider_name)} onkeydown={() => {}}>
            <div class="profile-title-row">
              <Badge color={providerColors[profile.provider_name] ?? "var(--fg-dim)"}>
                {profile.provider_name}
              </Badge>
              <span class="profile-meta">
                {#if profile.request_count != null}
                  {profile.request_count} requetes
                {/if}
              </span>
              <span class="profile-date">{formatDate(profile.last_capture ?? profile.captured_at)}</span>
            </div>
            <div class="profile-counts">
              <span class="count-badge">{staticHeaderCount(profile)} static</span>
              <span class="count-badge dyn">{dynamicHeaderCount(profile)} dynamic</span>
              {#if profile.always_streams}
                <span class="count-badge stream">streaming</span>
              {/if}
            </div>
            <button class="expand-btn" aria-label="Expand">
              {#if expanded[profile.provider_name]}
                <ChevronDown size={14} />
              {:else}
                <ChevronRight size={14} />
              {/if}
            </button>
          </div>

          {#if expanded[profile.provider_name]}
            <div class="profile-details">
              {#if staticHeaderCount(profile) > 0}
                <div class="header-section">
                  <div class="section-label">Headers statiques</div>
                  <div class="header-list">
                    {#each Object.entries(profile.static_headers ?? {}) as [name, value]}
                      <div class="header-item">
                        <span class="header-name">{name}</span>
                        <span class="header-value">{value}</span>
                      </div>
                    {/each}
                  </div>
                </div>
              {/if}

              {#if dynamicHeaderCount(profile) > 0}
                <div class="header-section">
                  <div class="section-label">Headers dynamiques</div>
                  <div class="header-list">
                    {#each Object.entries(profile.dynamic_headers ?? {}) as [name, dh]}
                      <div class="header-item">
                        <span class="header-name">{name}</span>
                        <span class="header-pattern">[{dh.pattern}]</span>
                        <span class="header-value">{dh.latest}</span>
                      </div>
                    {/each}
                  </div>
                </div>
              {/if}

              {#if profile.header_order && profile.header_order.length > 0}
                <div class="header-section">
                  <div class="section-label">Ordre ({profile.header_order.length} headers)</div>
                  <div class="order-list">
                    {#each profile.header_order as h}
                      <span class="order-item">{h}</span>
                    {/each}
                  </div>
                </div>
              {/if}
            </div>
          {/if}
        </div>
      </Card>
    {/each}
  {/if}
</div>

<style>
  .profiles-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .profiles-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .profiles-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .profiles-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 48px;
    color: var(--fg-dim);
    text-align: center;
    font-size: 13px;
  }

  .profiles-state.error { color: var(--status-error); }

  .hint {
    font-size: 11px;
    opacity: 0.7;
    max-width: 300px;
  }

  .profile-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .profile-header {
    display: flex;
    align-items: center;
    gap: 12px;
    cursor: pointer;
    user-select: none;
  }

  .profile-title-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
  }

  .profile-meta {
    font-size: 12px;
    color: var(--fg-dim);
  }

  .profile-date {
    font-size: 11px;
    color: var(--fg-dim);
    opacity: 0.7;
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }

  .profile-counts {
    display: flex;
    gap: 4px;
  }

  .count-badge {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    background: var(--bg-app);
    color: var(--fg-dim);
    border: 1px solid var(--border);
  }

  .count-badge.dyn {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }

  .count-badge.stream {
    color: var(--status-running);
    border-color: color-mix(in srgb, var(--status-running) 30%, transparent);
  }

  .expand-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    background: none;
    color: var(--fg-dim);
    cursor: pointer;
    border-radius: var(--radius-sm);
    flex-shrink: 0;
  }

  .expand-btn:hover {
    background: var(--bg-card-hover);
    color: var(--fg-primary);
  }

  .profile-details {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }

  .header-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .section-label {
    font-size: 10px;
    font-weight: 600;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .header-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 4px;
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 10px;
  }

  .header-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 6px;
    border-radius: var(--radius-sm);
  }

  .header-item:hover { background: var(--bg-card-hover); }

  .header-name {
    color: var(--accent);
    flex-shrink: 0;
    min-width: 120px;
  }

  .header-pattern {
    color: var(--fg-dim);
    flex-shrink: 0;
    font-size: 9px;
  }

  .header-value {
    color: var(--fg-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  .order-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .order-item {
    font-size: 10px;
    padding: 1px 6px;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-dim);
    font-family: "JetBrains Mono", "Fira Code", monospace;
  }
</style>
