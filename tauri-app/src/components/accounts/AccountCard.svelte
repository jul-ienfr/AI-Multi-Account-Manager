<script lang="ts">
  import type { AccountState } from "../../lib/types";
  import { accounts } from "../../lib/stores/accounts";
  import QuotaRing from "./QuotaRing.svelte";
  import Badge from "../ui/Badge.svelte";
  import Card from "../ui/Card.svelte";
  import {
    RefreshCw, MoreVertical, Trash2, ArrowRightLeft,
    ShieldOff, Shield, SlidersHorizontal, Key, KeyRound,
    Ban, RotateCw
  } from "lucide-svelte";

  interface Props {
    account: AccountState;
  }

  let { account }: Props = $props();

  let showContextMenu = $state(false);
  let contextPos = $state({ x: 0, y: 0 });
  let showPriorityInput = $state(false);
  let priorityValue = $state(50);

  const providerColors: Record<string, string> = {
    anthropic: "var(--provider-anthropic)",
    gemini: "var(--provider-gemini)",
    openai: "var(--provider-openai)",
    xai: "var(--provider-xai)",
    deepseek: "var(--provider-deepseek)",
    mistral: "var(--provider-mistral)",
    groq: "var(--provider-groq)",
  };

  const phaseColors: Record<string, string> = {
    Cruise: "var(--phase-cruise)",
    Watch: "var(--phase-watch)",
    Alert: "var(--phase-alert)",
    Critical: "var(--phase-critical)",
  };

  let quota5hPercent = $derived(
    account.quota && account.quota.limit5h > 0
      ? account.quota.tokens5h / account.quota.limit5h
      : 0
  );

  let quota7dPercent = $derived(
    account.quota && account.quota.limit7d > 0
      ? account.quota.tokens7d / account.quota.limit7d
      : 0
  );

  let providerColor = $derived(
    providerColors[account.data.provider ?? "anthropic"] ?? "var(--fg-dim)"
  );

  let phaseColor = $derived(
    phaseColors[account.quota?.phase ?? "Cruise"] ?? "var(--phase-cruise)"
  );

  let isPulse = $derived(
    account.quota?.phase === "Alert" || account.quota?.phase === "Critical"
  );

  let displayName = $derived(
    account.data.displayName || account.data.name || account.key
  );

  let isAutoSwitchDisabled = $derived(
    account.data.autoSwitchDisabled === true
  );

  let isApiAccount = $derived(
    account.data.accountType === "api"
  );

  function formatTokens(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
    if (n >= 1_000) return (n / 1_000).toFixed(0) + "k";
    return String(n);
  }

  function formatTTT(mins?: number): string {
    if (mins == null || mins <= 0) return "";
    if (mins < 60) return `~${Math.round(mins)}m`;
    const h = Math.floor(mins / 60);
    const m = Math.round(mins % 60);
    return `~${h}h${m > 0 ? m + "m" : ""}`;
  }

  function formatResetCountdown(isoDate?: string): string {
    if (!isoDate) return "";
    const resetTime = new Date(isoDate).getTime();
    const now = Date.now();
    const diffMs = resetTime - now;
    if (diffMs <= 0) return "reset";
    const mins = Math.floor(diffMs / 60000);
    if (mins < 60) return `${mins}m`;
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return `${h}h${m > 0 ? m.toString().padStart(2, "0") + "m" : ""}`;
  }

  let tttDisplay = $derived(formatTTT(account.quota?.timeToThreshold));
  let velocityDisplay = $derived(
    account.quota?.emaVelocity && account.quota.emaVelocity > 0.001
      ? `${account.quota.emaVelocity.toFixed(2)}%/min`
      : ""
  );
  let reset5hDisplay = $derived(formatResetCountdown(account.quota?.resetsAt5h));
  let reset7dDisplay = $derived(formatResetCountdown(account.quota?.resetsAt7d));

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    const menuW = 220;
    const menuH = 320;
    const x = Math.min(e.clientX, window.innerWidth - menuW - 8);
    const y = Math.min(e.clientY, window.innerHeight - menuH - 8);
    contextPos = { x: Math.max(4, x), y: Math.max(4, y) };
    showContextMenu = true;
    showPriorityInput = false;
  }

  function closeContextMenu() {
    showContextMenu = false;
    showPriorityInput = false;
  }

  async function handleSwitch() {
    closeContextMenu();
    await accounts.switch(account.key);
  }

  async function handleRefresh() {
    closeContextMenu();
    await accounts.refresh(account.key);
  }

  async function handleDelete() {
    closeContextMenu();
    await accounts.delete(account.key);
  }

  async function handleToggleAutoSwitch() {
    closeContextMenu();
    await accounts.updateAccount(account.key, {
      autoSwitchDisabled: !isAutoSwitchDisabled,
    });
  }

  function handlePriority() {
    priorityValue = account.data.priority ?? 50;
    showPriorityInput = true;
  }

  async function savePriority() {
    await accounts.updateAccount(account.key, { priority: priorityValue });
    closeContextMenu();
  }

  async function handleRefreshToken() {
    closeContextMenu();
    await accounts.refresh(account.key);
  }
</script>

<svelte:window onclick={closeContextMenu} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="account-card-wrapper" oncontextmenu={handleContextMenu} ondblclick={handleSwitch}>
  <Card active={account.isActive}>
    <div class="card-layout">
      <div class="card-left">
        <QuotaRing
          percent={quota5hPercent}
          phase={account.quota?.phase}
          size={44}
          strokeWidth={3.5}
        />
      </div>

      <div class="card-center">
        <div class="card-header">
          <div class="card-name-row">
            {#if account.isActive}
              <span class="active-dot" class:pulse={isPulse}></span>
            {/if}
            <span class="card-name">{displayName}</span>
          </div>
          {#if account.data.email}
            <span class="card-email">{account.data.email}</span>
          {/if}
        </div>

        <div class="card-badges">
          <Badge color={providerColor}>
            {account.data.provider ?? "anthropic"}
          </Badge>

          {#if account.quota?.phase}
            <Badge color={phaseColor}>
              {#if isPulse}
                <span class="pulse-badge"></span>
              {/if}
              {account.quota.phase}
            </Badge>
          {/if}

          {#if account.data.priority != null}
            <Badge color="var(--fg-dim)" small>
              P{account.data.priority}
            </Badge>
          {/if}

          {#if account.data.planType}
            <Badge color="var(--accent)" small>
              {account.data.planType}
            </Badge>
          {/if}

          {#if isAutoSwitchDisabled}
            <Badge color="var(--status-error)" small>
              exclu
            </Badge>
          {/if}
        </div>

        {#if account.quota}
          <div class="quota-bar-row">
            <span class="quota-bar-label">5h</span>
            <div class="quota-bar-track">
              <div
                class="quota-bar-fill"
                style="width: {Math.min(quota5hPercent, 1) * 100}%; background: {phaseColor}"
              ></div>
            </div>
            <span class="quota-bar-value">{Math.round(quota5hPercent * 100)}%</span>
            <span class="quota-bar-extra">
              {#if reset5hDisplay}Reset: {reset5hDisplay}{/if}
              {#if velocityDisplay} · ↗ {velocityDisplay}{/if}
              {#if tttDisplay} · TTT {tttDisplay}{/if}
            </span>
          </div>
          <div class="quota-bar-row">
            <span class="quota-bar-label">7j</span>
            <div class="quota-bar-track">
              <div
                class="quota-bar-fill"
                style="width: {Math.min(quota7dPercent, 1) * 100}%; background: {phaseColor}"
              ></div>
            </div>
            <span class="quota-bar-value">{Math.round(quota7dPercent * 100)}%</span>
            <span class="quota-bar-extra">
              {formatTokens(account.quota.tokens7d ?? 0)}/{formatTokens(account.quota.limit7d ?? 0)}
              {#if reset7dDisplay} · Reset: {reset7dDisplay}{/if}
            </span>
          </div>
          {#if account.quota.lastUpdated}
            <span class="last-updated">MAJ {new Date(account.quota.lastUpdated).toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" })}</span>
          {/if}
        {/if}
      </div>

      <div class="card-actions">
        {#if !account.isActive}
          <button class="action-btn switch-btn" onclick={handleSwitch} aria-label="Activer ce compte" title="Activer ce compte">
            <ArrowRightLeft size={14} />
          </button>
        {/if}
        <button class="action-btn" onclick={handleRefresh} aria-label="Rafraichir">
          <RefreshCw size={14} />
        </button>
        <button class="action-btn" onclick={(e) => { e.stopPropagation(); handleContextMenu(e); }} aria-label="Plus d'options">
          <MoreVertical size={14} />
        </button>
      </div>
    </div>
  </Card>
</div>

{#if showContextMenu}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="context-menu"
    style="left: {contextPos.x}px; top: {contextPos.y}px"
    onclick={(e) => e.stopPropagation()}
    onkeydown={() => {}}
  >
    <button class="context-item" onclick={handleSwitch}>
      <ArrowRightLeft size={14} />
      <span>Activer {account.data.email ?? account.key}</span>
    </button>

    <div class="context-divider"></div>

    <button class="context-item" onclick={handleToggleAutoSwitch}>
      {#if isAutoSwitchDisabled}
        <Shield size={14} />
        <span>Inclure dans l'auto-switch</span>
      {:else}
        <ShieldOff size={14} />
        <span>Exclure de l'auto-switch</span>
      {/if}
    </button>

    {#if !showPriorityInput}
      <button class="context-item" onclick={handlePriority}>
        <SlidersHorizontal size={14} />
        <span>Priorite ({account.data.priority ?? 50})</span>
      </button>
    {:else}
      <div class="context-priority">
        <SlidersHorizontal size={14} />
        <input
          class="priority-input"
          type="number"
          min="1"
          max="99"
          bind:value={priorityValue}
          onkeydown={(e) => { if (e.key === 'Enter') savePriority(); }}
        />
        <button class="priority-ok" onclick={savePriority}>OK</button>
      </div>
    {/if}

    <div class="context-divider"></div>

    <button class="context-item" onclick={handleRefresh}>
      <RefreshCw size={14} />
      <span>Rafraichir le quota</span>
    </button>

    {#if !isApiAccount}
      <button class="context-item" onclick={handleRefreshToken}>
        <RotateCw size={14} />
        <span>Rafraichir le token</span>
      </button>

      <button class="context-item" onclick={closeContextMenu}>
        <KeyRound size={14} />
        <span>Setup Token</span>
      </button>
    {/if}

    <div class="context-divider"></div>

    {#if !isApiAccount}
      <button class="context-item danger" onclick={closeContextMenu}>
        <Ban size={14} />
        <span>Revoquer</span>
      </button>
    {/if}

    <button class="context-item danger" onclick={handleDelete}>
      <Trash2 size={14} />
      <span>Supprimer</span>
    </button>
  </div>
{/if}

<style>
  .account-card-wrapper {
    position: relative;
  }

  .card-layout {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }

  .card-left {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding-top: 2px;
  }

  .card-center {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .card-header {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .card-name-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .card-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-email {
    font-size: 11px;
    color: var(--fg-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .active-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--status-running);
    box-shadow: 0 0 6px var(--status-running);
    flex-shrink: 0;
  }

  .active-dot.pulse {
    animation: pulse-dot 1.5s ease infinite;
  }

  .card-badges {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
  }

  .pulse-badge {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    animation: pulse-dot 1.5s ease infinite;
  }

  .quota-bar-row {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .quota-bar-label {
    font-size: 10px;
    color: var(--fg-dim);
    width: 14px;
    flex-shrink: 0;
  }

  .quota-bar-track {
    flex: 1;
    height: 3px;
    background: var(--border);
    border-radius: 2px;
    overflow: hidden;
  }

  .quota-bar-fill {
    height: 100%;
    border-radius: 2px;
    transition: width 0.5s ease, background 0.3s ease;
  }

  .quota-bar-value {
    font-size: 10px;
    color: var(--fg-secondary);
    width: 26px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .quota-bar-extra {
    font-size: 9px;
    color: var(--fg-dim);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .last-updated {
    font-size: 9px;
    color: var(--fg-dim);
  }

  .card-actions {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex-shrink: 0;
  }

  .action-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    color: var(--fg-dim);
    cursor: pointer;
    background: none;
    border: none;
  }

  .action-btn:hover {
    color: var(--fg-primary);
    background: var(--bg-card-hover);
  }

  .switch-btn {
    color: var(--accent);
  }

  .switch-btn:hover {
    color: var(--fg-primary);
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }

  .action-btn:hover {
    background: var(--bg-card-hover);
    color: var(--fg-primary);
  }

  /* Context menu */
  .context-menu {
    position: fixed;
    z-index: 100;
    min-width: 210px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 4px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
    animation: fade-in 0.1s ease;
  }

  .context-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 5px 10px;
    font-size: 12px;
    color: var(--fg-secondary);
    border-radius: var(--radius-sm);
    cursor: pointer;
    background: none;
    border: none;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .context-item:hover {
    background: var(--bg-card-hover);
    color: var(--fg-primary);
  }

  .context-item.danger {
    color: var(--status-error);
  }
  .context-item.danger:hover {
    background: color-mix(in srgb, var(--status-error) 10%, transparent);
  }

  .context-divider {
    height: 1px;
    background: var(--border);
    margin: 3px 0;
  }

  .context-priority {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    color: var(--fg-secondary);
  }

  .priority-input {
    width: 50px;
    padding: 2px 6px;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-primary);
    font-size: 12px;
    text-align: center;
  }

  .priority-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .priority-ok {
    padding: 2px 8px;
    font-size: 11px;
    background: var(--accent);
    color: var(--bg-app);
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-weight: 600;
  }
</style>
