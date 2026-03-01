<script lang="ts">
  // TODO: quand le backend exposera un endpoint pour les cooldowns OAuth,
  // remplacer la prop `cooldowns` par un appel invoke("get_oauth_cooldowns")
  // et rafraichir toutes les 5s avec setInterval.
  // Exemple d'interface attendue du backend :
  // invoke<CooldownEntry[]>("get_oauth_cooldowns")
  // Structure : { key, email, cooldown_until (ISO string), reason }

  export interface CooldownEntry {
    key: string;
    email: string;
    cooldown_until: string; // ISO 8601
    reason: string;
  }

  interface Props {
    cooldowns?: CooldownEntry[];
  }

  let { cooldowns = [] }: Props = $props();

  // Temps restant en secondes pour chaque entrée
  function remainingSecs(until: string): number {
    const ms = new Date(until).getTime() - Date.now();
    return Math.max(0, Math.floor(ms / 1000));
  }

  // Durée totale du cooldown en secondes (on approxime avec la valeur restante initiale
  // + on suppose un max de 300s = 5 min pour l'affichage relatif de la barre)
  const MAX_COOLDOWN_SECS = 300;

  function progressPct(until: string): number {
    const remaining = remainingSecs(until);
    return Math.max(0, Math.min(100, (remaining / MAX_COOLDOWN_SECS) * 100));
  }

  function formatRemaining(secs: number): string {
    if (secs <= 0) return "expire";
    const m = Math.floor(secs / 60);
    const s = secs % 60;
    if (m === 0) return `${s}s`;
    return `${m}m ${s}s`;
  }

  function formatUntil(until: string): string {
    return new Date(until).toLocaleTimeString("fr-FR", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  }

  // Couleur selon la gravite
  function severityColor(count: number): string {
    if (count === 0) return "var(--phase-cruise)";
    if (count <= 2) return "var(--phase-watch)";
    return "var(--phase-alert)";
  }

  function barColor(pct: number): string {
    if (pct < 30) return "var(--phase-cruise)";
    if (pct < 70) return "var(--phase-watch)";
    return "var(--phase-alert)";
  }

  // Ticker pour mettre a jour les temps restants
  import { onMount } from "svelte";

  let tick = $state(0);
  let timer: ReturnType<typeof setInterval> | null = null;

  onMount(() => {
    timer = setInterval(() => { tick += 1; }, 1000);
    return () => { if (timer) clearInterval(timer); };
  });

  // Recalculer les entrées a chaque tick
  let entries = $derived(
    // tick est lu pour forcer la recomputation chaque seconde
    tick >= 0
      ? cooldowns.map((c) => ({
          ...c,
          remaining: remainingSecs(c.cooldown_until),
          pct: progressPct(c.cooldown_until),
        }))
      : []
  );

  let active = $derived(entries.filter((e) => e.remaining > 0));
  let count = $derived(active.length);
  let headerColor = $derived(severityColor(count));
</script>

<div class="backoff-chart">
  <!-- En-tête avec indicateur global -->
  <div class="bc-header">
    <div class="bc-indicator" style="background: {headerColor}"></div>
    <span class="bc-title">Cooldowns OAuth</span>
    <span class="bc-count" style="color: {headerColor}">
      {count} actif{count !== 1 ? "s" : ""}
    </span>
  </div>

  {#if count === 0}
    <!-- Aucun cooldown -->
    <div class="bc-empty">
      <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="var(--phase-cruise)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/>
        <polyline points="22 4 12 14.01 9 11.01"/>
      </svg>
      <span>Aucun cooldown actif</span>
      <span class="bc-empty-hint">Tous les comptes OAuth sont disponibles</span>
    </div>
  {:else}
    <!-- Liste des cooldowns -->
    <div class="bc-list">
      {#each active as entry (entry.key)}
        <div class="bc-item">
          <div class="bc-item-header">
            <div class="bc-item-info">
              <span class="bc-email" title={entry.email}>{entry.email}</span>
              <span class="bc-reason">{entry.reason}</span>
            </div>
            <div class="bc-item-meta">
              <span class="bc-remaining" style="color: {barColor(entry.pct)}">
                {formatRemaining(entry.remaining)}
              </span>
              <span class="bc-until">jusqu'a {formatUntil(entry.cooldown_until)}</span>
            </div>
          </div>

          <!-- Barre de progression temporelle -->
          <div class="bc-bar-track">
            <div
              class="bc-bar-fill"
              style="width: {entry.pct}%; background: {barColor(entry.pct)}"
            ></div>
          </div>
        </div>
      {/each}
    </div>

    <!-- Entrées expirees (affichees en gris) -->
    {#if entries.some((e) => e.remaining === 0)}
      <div class="bc-expired-section">
        <span class="bc-expired-label">Expires recemment</span>
        {#each entries.filter((e) => e.remaining === 0) as entry (entry.key)}
          <div class="bc-item bc-item-expired">
            <div class="bc-item-header">
              <span class="bc-email dim">{entry.email}</span>
              <span class="bc-remaining dim">expire</span>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .backoff-chart {
    display: flex;
    flex-direction: column;
    gap: 12px;
    animation: fade-in 0.2s ease;
  }

  /* Header */
  .bc-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .bc-indicator {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    transition: background 0.3s ease;
  }

  .bc-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-primary);
    flex: 1;
  }

  .bc-count {
    font-size: 12px;
    font-weight: 600;
    transition: color 0.3s ease;
  }

  /* Empty state */
  .bc-empty {
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

  .bc-empty-hint {
    font-size: 12px;
    color: var(--fg-dim);
  }

  /* List */
  .bc-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .bc-item {
    padding: 12px 14px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    display: flex;
    flex-direction: column;
    gap: 8px;
    transition: border-color 0.15s ease;
  }

  .bc-item:hover {
    border-color: var(--border-hover);
    background: var(--bg-card-hover);
  }

  .bc-item-expired {
    opacity: 0.5;
  }

  .bc-item-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .bc-item-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .bc-item-meta {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 2px;
    flex-shrink: 0;
  }

  .bc-email {
    font-size: 13px;
    font-weight: 500;
    color: var(--fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
  }

  .bc-email.dim {
    color: var(--fg-dim);
  }

  .bc-reason {
    font-size: 11px;
    color: var(--fg-dim);
  }

  .bc-remaining {
    font-size: 13px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    font-family: "JetBrains Mono", "Fira Code", monospace;
    transition: color 0.3s ease;
  }

  .bc-remaining.dim {
    color: var(--fg-dim);
    font-weight: 400;
  }

  .bc-until {
    font-size: 11px;
    color: var(--fg-dim);
    font-variant-numeric: tabular-nums;
  }

  /* Barre de progression */
  .bc-bar-track {
    height: 4px;
    background: var(--border);
    border-radius: 2px;
    overflow: hidden;
  }

  .bc-bar-fill {
    height: 100%;
    border-radius: 2px;
    transition: width 1s linear, background 0.3s ease;
  }

  /* Section expirés */
  .bc-expired-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 4px;
  }

  .bc-expired-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--fg-dim);
    padding: 0 2px;
  }
</style>
