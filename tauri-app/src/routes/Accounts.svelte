<script lang="ts">
  import { accounts } from "../lib/stores/accounts";
  import AccountList from "../components/accounts/AccountList.svelte";
  import Button from "../components/ui/Button.svelte";
  import Dialog from "../components/ui/Dialog.svelte";
  import { RefreshCw, Plus, Users, Upload, ScanSearch, Zap } from "lucide-svelte";
  import { onMount } from "svelte";
  import type { AccountState, ScannedCredential, CaptureResult } from "../lib/types";
  import { scanLocalCredentials, importScannedCredentials, findClaudeBinary, captureOAuthToken } from "../lib/tauri";
  import { toast } from "../lib/stores/toast";

  let accountList: AccountState[] = $state([]);
  let loading = $state(false);
  let showAddDialog = $state(false);
  let showImportDialog = $state(false);
  let importToken = $state("");

  // Formulaire ajout compte
  let newKey = $state("");
  let newAccessToken = $state("");
  let newName = $state("");
  let newEmail = $state("");
  let newProvider = $state("anthropic");
  let newPriority = $state(1);
  let newPlanType = $state("");
  let adding = $state(false);

  // Auto-import
  let showAutoImportDialog = $state(false);
  let scanning = $state(false);
  let importing = $state(false);
  let scannedCredentials: ScannedCredential[] = $state([]);
  let selectedCredentials = $state<Set<string>>(new Set());

  // Setup automatique (OAuth capture)
  type SetupStep = "check" | "capturing" | "success" | "error";
  let showSetupDialog = $state(false);
  let setupStep = $state<SetupStep>("check");
  let setupBinaryPath = $state<string | null>(null);
  let setupError = $state<string | null>(null);
  let setupOutput = $state<string>("");
  let setupEmail = $state<string | null>(null);
  let checkingBinary = $state(false);

  onMount(() => {
    const unsub = accounts.subscribe((a) => {
      accountList = a;
    });
    return unsub;
  });

  async function handleRefreshAll() {
    loading = true;
    try {
      await accounts.load();
    } finally {
      loading = false;
    }
  }

  function resetForm() {
    newKey = "";
    newAccessToken = "";
    newName = "";
    newEmail = "";
    newProvider = "anthropic";
    newPriority = 1;
    newPlanType = "";
  }

  async function handleAdd() {
    if (!newKey.trim() || !newAccessToken.trim()) return;
    adding = true;
    try {
      await accounts.add(newKey.trim(), {
        name: newName.trim() || newKey.trim(),
        displayName: newName.trim() || newKey.trim(),
        email: newEmail.trim() || undefined,
        provider: newProvider as any,
        priority: newPriority,
        planType: newPlanType.trim() || undefined,
        claudeAiOauth: {
          accessToken: newAccessToken.trim(),
          refreshToken: newAccessToken.trim(),
        },
      });
      showAddDialog = false;
      resetForm();
    } finally {
      adding = false;
    }
  }

  function handleClose() {
    showAddDialog = false;
    resetForm();
  }

  // ---- Auto-import ----

  function credKey(c: ScannedCredential): string {
    // Clé stable pour identifier chaque credential dans la sélection
    return c.email ?? c.accessToken?.slice(0, 16) ?? c.sourcePath;
  }

  function labelFor(c: ScannedCredential): string {
    if (c.email) return c.email;
    if (c.name) return c.name;
    return `token-${c.accessToken?.slice(0, 8) ?? "???"}…`;
  }

  async function handleOpenAutoImport() {
    scanning = true;
    scannedCredentials = [];
    selectedCredentials = new Set();
    showAutoImportDialog = true;
    try {
      const found = await scanLocalCredentials();
      scannedCredentials = found;
      if (found.length === 0) {
        showAutoImportDialog = false;
        toast.info("Aucun token local trouvé", "Aucun fichier de credentials Claude n'a été détecté.");
      } else {
        // Tout pré-cocher
        selectedCredentials = new Set(found.map(credKey));
      }
    } catch (err) {
      showAutoImportDialog = false;
      toast.error("Erreur de scan", String(err));
    } finally {
      scanning = false;
    }
  }

  function closeAutoImportDialog() {
    showAutoImportDialog = false;
    scannedCredentials = [];
    selectedCredentials = new Set();
  }

  function toggleCredential(c: ScannedCredential) {
    const k = credKey(c);
    const next = new Set(selectedCredentials);
    if (next.has(k)) {
      next.delete(k);
    } else {
      next.add(k);
    }
    selectedCredentials = next;
  }

  function toggleAll() {
    if (selectedCredentials.size === scannedCredentials.length) {
      selectedCredentials = new Set();
    } else {
      selectedCredentials = new Set(scannedCredentials.map(credKey));
    }
  }

  async function handleImportSelected() {
    const toImport = scannedCredentials.filter((c) => selectedCredentials.has(credKey(c)));
    if (toImport.length === 0) return;
    importing = true;
    try {
      const count = await importScannedCredentials(toImport);
      await accounts.load();
      closeAutoImportDialog();
      toast.success(
        `${count} compte${count > 1 ? "s" : ""} importé${count > 1 ? "s" : ""}`,
        `Les tokens locaux ont été ajoutés avec succès.`
      );
    } catch (err) {
      toast.error("Erreur d'import", String(err));
    } finally {
      importing = false;
    }
  }

  // ---- Setup automatique (OAuth capture) ----

  function resetSetup() {
    setupStep = "check";
    setupBinaryPath = null;
    setupError = null;
    setupOutput = "";
    setupEmail = null;
    checkingBinary = false;
  }

  async function handleOpenSetup() {
    resetSetup();
    showSetupDialog = true;
    checkingBinary = true;
    try {
      const path = await findClaudeBinary();
      setupBinaryPath = path;
    } catch (err) {
      setupBinaryPath = null;
      setupError = String(err);
    } finally {
      checkingBinary = false;
    }
  }

  function closeSetupDialog() {
    showSetupDialog = false;
    resetSetup();
  }

  async function handleStartCapture() {
    setupStep = "capturing";
    setupOutput = "";
    setupError = null;
    try {
      const result: CaptureResult = await captureOAuthToken(60);
      setupOutput = result.output ?? "";
      if (result.success && result.accessToken) {
        setupEmail = result.email ?? null;
        setupStep = "success";
        // Reload account list
        await accounts.load();
      } else {
        setupError = result.error ?? "Aucun token capturé.";
        setupStep = "error";
      }
    } catch (err) {
      setupError = String(err);
      setupStep = "error";
    }
  }

  function handleSetupSuccess() {
    closeSetupDialog();
    toast.success(
      "Token capturé",
      setupEmail ? `Compte ${setupEmail} ajouté.` : "Compte ajouté avec succès."
    );
  }

  // Stats par phase
  let countActive = $derived(accountList.filter((a) => a.isActive).length);
  let countCruise = $derived(accountList.filter((a) => !a.quota || a.quota.phase === "Cruise").length);
  let countWatch = $derived(accountList.filter((a) => a.quota?.phase === "Watch").length);
  let countAlert = $derived(accountList.filter((a) => a.quota?.phase === "Alert").length);
  let countCritical = $derived(accountList.filter((a) => a.quota?.phase === "Critical").length);
  let allSelected = $derived(scannedCredentials.length > 0 && selectedCredentials.size === scannedCredentials.length);
  let someSelected = $derived(selectedCredentials.size > 0 && selectedCredentials.size < scannedCredentials.length);
</script>

<div class="accounts-screen">
  <!-- Header -->
  <header class="screen-header">
    <div class="header-left">
      <div class="screen-icon">
        <Users size={18} />
      </div>
      <div>
        <h1 class="screen-title">Comptes</h1>
        <p class="screen-subtitle">
          {accountList.length} compte{accountList.length !== 1 ? "s" : ""} configuré{accountList.length !== 1 ? "s" : ""}
        </p>
      </div>
    </div>
    <div class="screen-actions">
      <Button variant="ghost" size="sm" onclick={handleRefreshAll} disabled={loading}>
        <span class:spin={loading} style="display:flex"><RefreshCw size={14} /></span>
        Rafraîchir
      </Button>
      <Button variant="ghost" size="sm" onclick={handleOpenSetup}>
        <Zap size={14} />
        Setup auto
      </Button>
      <Button variant="ghost" size="sm" onclick={handleOpenAutoImport} disabled={scanning}>
        <span class:spin={scanning} style="display:flex"><ScanSearch size={14} /></span>
        Import auto
      </Button>
      <Button variant="ghost" size="sm" onclick={() => (showImportDialog = true)}>
        <Upload size={14} />
        Importer
      </Button>
      <Button variant="primary" size="sm" onclick={() => (showAddDialog = true)}>
        <Plus size={14} />
        Ajouter
      </Button>
    </div>
  </header>

  <!-- Stats rapides -->
  {#if accountList.length > 0}
    <div class="phase-stats">
      <div class="stat-pill">
        <span class="dot dot-active"></span>
        <span>{countActive} actif{countActive > 1 ? "s" : ""}</span>
      </div>
      <span class="stat-sep"></span>
      <div class="stat-pill">
        <span class="dot dot-cruise"></span>
        <span>{countCruise} Cruise</span>
      </div>
      {#if countWatch > 0}
        <div class="stat-pill">
          <span class="dot dot-watch"></span>
          <span>{countWatch} Watch</span>
        </div>
      {/if}
      {#if countAlert > 0}
        <div class="stat-pill">
          <span class="dot dot-alert"></span>
          <span>{countAlert} Alert</span>
        </div>
      {/if}
      {#if countCritical > 0}
        <div class="stat-pill urgent">
          <span class="dot dot-critical"></span>
          <span>{countCritical} Critical</span>
        </div>
      {/if}
    </div>
  {/if}

  <!-- Liste -->
  {#if accountList.length === 0}
    <div class="empty-state">
      <div class="empty-icon"><Users size={44} /></div>
      <p class="empty-title">Aucun compte configuré</p>
      <p class="empty-desc">Ajoutez votre premier compte Claude pour commencer</p>
      <Button variant="primary" size="md" onclick={() => (showAddDialog = true)}>
        <Plus size={14} />
        Ajouter un compte
      </Button>
    </div>
  {:else}
    <AccountList accounts={accountList} />
  {/if}
</div>

<!-- Dialog ajout compte -->
<Dialog bind:open={showAddDialog} title="Ajouter un compte" onclose={handleClose}>
  {#snippet children()}
    <div class="add-form">
      <div class="form-field">
        <label class="form-label" for="add-key">
          Identifiant <span class="req">*</span>
        </label>
        <input
          id="add-key"
          type="text"
          class="form-input"
          placeholder="ex: alice@example.com"
          bind:value={newKey}
          autocomplete="off"
        />
      </div>

      <div class="form-field">
        <label class="form-label" for="add-token">
          Access Token <span class="req">*</span>
        </label>
        <input
          id="add-token"
          type="password"
          class="form-input"
          placeholder="token OAuth..."
          bind:value={newAccessToken}
          autocomplete="new-password"
        />
      </div>

      <div class="form-row">
        <div class="form-field">
          <label class="form-label" for="add-name">Nom</label>
          <input id="add-name" type="text" class="form-input" placeholder="Alice" bind:value={newName} />
        </div>
        <div class="form-field">
          <label class="form-label" for="add-email">Email</label>
          <input id="add-email" type="email" class="form-input" placeholder="alice@example.com" bind:value={newEmail} />
        </div>
      </div>

      <div class="form-row">
        <div class="form-field">
          <label class="form-label" for="add-provider">Provider</label>
          <select id="add-provider" class="form-input form-select" bind:value={newProvider}>
            <option value="anthropic">Anthropic</option>
            <option value="gemini">Gemini</option>
            <option value="openai">OpenAI</option>
            <option value="xai">xAI</option>
            <option value="deepseek">DeepSeek</option>
            <option value="mistral">Mistral</option>
            <option value="groq">Groq</option>
          </select>
        </div>
        <div class="form-field">
          <label class="form-label" for="add-priority">Priorité</label>
          <input
            id="add-priority"
            type="number"
            class="form-input"
            min="0"
            max="99"
            bind:value={newPriority}
          />
        </div>
      </div>

      <div class="form-field">
        <label class="form-label" for="add-plan">Plan</label>
        <input id="add-plan" type="text" class="form-input" placeholder="pro, team, free..." bind:value={newPlanType} />
      </div>
    </div>
  {/snippet}

  {#snippet actions()}
    <Button variant="ghost" size="sm" onclick={handleClose}>Annuler</Button>
    <Button
      variant="primary"
      size="sm"
      onclick={handleAdd}
      disabled={adding || !newKey.trim() || !newAccessToken.trim()}
    >
      {adding ? "Ajout..." : "Ajouter"}
    </Button>
  {/snippet}
</Dialog>

<style>
  .accounts-screen {
    display: flex;
    flex-direction: column;
    gap: 20px;
    animation: fade-in 0.2s ease;
  }

  /* Header */
  .screen-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .screen-icon {
    width: 36px;
    height: 36px;
    border-radius: var(--radius-md);
    background: var(--accent-glow);
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--accent);
    flex-shrink: 0;
  }

  .screen-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--fg-primary);
    letter-spacing: -0.3px;
    line-height: 1.2;
  }

  .screen-subtitle {
    font-size: 12px;
    color: var(--fg-dim);
    margin-top: 1px;
  }

  .screen-actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  /* Spin pour RefreshCw */
  :global(.spin) {
    animation: spin 1s linear infinite;
  }

  /* Phase stats */
  .phase-stats {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .stat-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 10px;
    border-radius: 99px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    font-size: 11px;
    color: var(--fg-secondary);
    white-space: nowrap;
  }

  .stat-pill.urgent {
    border-color: color-mix(in srgb, var(--phase-critical) 30%, transparent);
    color: var(--phase-critical);
    background: color-mix(in srgb, var(--phase-critical) 8%, transparent);
  }

  .stat-sep {
    display: block;
    width: 1px;
    height: 14px;
    background: var(--border);
    flex-shrink: 0;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .dot-active {
    background: var(--status-running);
    box-shadow: 0 0 6px var(--status-running);
    animation: pulse-dot 2s ease-in-out infinite;
  }
  .dot-cruise { background: var(--phase-cruise); }
  .dot-watch { background: var(--phase-watch); }
  .dot-alert { background: var(--phase-alert); }
  .dot-critical { background: var(--phase-critical); }

  /* Empty state */
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 80px 20px;
    text-align: center;
    gap: 12px;
  }

  .empty-icon { color: var(--fg-dim); margin-bottom: 4px; }

  .empty-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--fg-secondary);
  }

  .empty-desc {
    font-size: 13px;
    color: var(--fg-dim);
    margin-bottom: 8px;
  }

  /* Form */
  .add-form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .form-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .form-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .form-label {
    font-size: 12px;
    font-weight: 500;
    color: var(--fg-secondary);
  }

  .req { color: var(--status-error); }

  .form-input {
    padding: 8px 12px;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    color: var(--fg-primary);
    font-size: 13px;
    font-family: inherit;
    width: 100%;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .form-input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-glow);
  }

  .form-input::placeholder { color: var(--fg-dim); }

  .form-select {
    cursor: pointer;
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%23475569' stroke-width='2'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 10px center;
    padding-right: 28px;
  }

  /* Auto-import dialog */
  .scan-intro {
    font-size: 12px;
    color: var(--fg-secondary);
    margin-bottom: 4px;
    line-height: 1.5;
  }

  .scan-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 280px;
    overflow-y: auto;
  }

  .scan-item {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--bg-app);
    cursor: pointer;
    transition: border-color 0.15s ease, background 0.15s ease;
  }

  .scan-item:hover {
    background: var(--bg-card-hover);
    border-color: var(--border-hover);
  }

  .scan-item.selected {
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }

  .scan-checkbox {
    width: 15px;
    height: 15px;
    margin-top: 1px;
    flex-shrink: 0;
    accent-color: var(--accent);
    cursor: pointer;
  }

  .scan-item-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .scan-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--fg-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .scan-source {
    font-size: 11px;
    color: var(--fg-dim);
    font-family: monospace;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .scan-provider-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 99px;
    font-size: 10px;
    font-weight: 500;
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent);
    text-transform: capitalize;
    align-self: flex-start;
  }

  .scan-select-all {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    font-size: 12px;
    color: var(--fg-secondary);
    cursor: pointer;
    user-select: none;
    border-bottom: 1px solid var(--border);
    margin-bottom: 4px;
  }

  .scan-select-all input {
    accent-color: var(--accent);
    cursor: pointer;
  }

  .scan-scanning {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 32px 0;
    color: var(--fg-dim);
    font-size: 13px;
  }

  .scan-spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  /* Setup automatique dialog */
  .setup-body {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .setup-step-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 2px;
  }

  .setup-check-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--bg-app);
    font-size: 13px;
  }

  .setup-check-row.ok {
    border-color: color-mix(in srgb, var(--status-running) 40%, transparent);
    background: color-mix(in srgb, var(--status-running) 6%, transparent);
  }

  .setup-check-row.fail {
    border-color: color-mix(in srgb, var(--status-error) 40%, transparent);
    background: color-mix(in srgb, var(--status-error) 6%, transparent);
  }

  .setup-check-icon {
    font-size: 16px;
    flex-shrink: 0;
  }

  .setup-check-path {
    font-family: monospace;
    font-size: 12px;
    color: var(--fg-secondary);
    word-break: break-all;
  }

  .setup-install-hint {
    font-size: 12px;
    color: var(--fg-secondary);
    line-height: 1.6;
  }

  .setup-install-hint a {
    color: var(--accent);
    text-decoration: underline;
  }

  .setup-capture-body {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    padding: 16px 0 8px;
  }

  .setup-spinner {
    width: 32px;
    height: 32px;
    border: 3px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .setup-capture-label {
    font-size: 13px;
    color: var(--fg-secondary);
    text-align: center;
  }

  .setup-capture-sublabel {
    font-size: 11px;
    color: var(--fg-dim);
    text-align: center;
    margin-top: -6px;
  }

  .setup-output {
    width: 100%;
    max-height: 140px;
    overflow-y: auto;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 10px 12px;
    font-family: monospace;
    font-size: 11px;
    color: var(--fg-dim);
    white-space: pre-wrap;
    word-break: break-all;
  }

  .setup-success-body {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 20px 0 8px;
    text-align: center;
  }

  .setup-success-icon {
    font-size: 40px;
    line-height: 1;
  }

  .setup-success-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--fg-primary);
  }

  .setup-success-email {
    font-size: 13px;
    color: var(--fg-secondary);
  }

  .setup-error-body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .setup-error-msg {
    font-size: 13px;
    color: var(--status-error);
    padding: 10px 14px;
    border-radius: var(--radius-md);
    border: 1px solid color-mix(in srgb, var(--status-error) 30%, transparent);
    background: color-mix(in srgb, var(--status-error) 6%, transparent);
    line-height: 1.5;
  }

  .setup-manual-hint {
    font-size: 12px;
    color: var(--fg-secondary);
    line-height: 1.6;
  }

  .setup-manual-hint code {
    font-family: monospace;
    font-size: 11px;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1px 5px;
    color: var(--fg-primary);
  }
</style>

<!-- Dialog import token -->
<Dialog bind:open={showImportDialog} title="Importer un token" onclose={() => { showImportDialog = false; importToken = ""; }}>
  {#snippet children()}
    <div class="add-form">
      <p style="font-size: 12px; color: var(--fg-secondary);">Collez un access token OAuth capturé depuis Claude Code pour l'importer comme nouveau compte.</p>
      <div class="form-field">
        <label class="form-label" for="import-token">Access Token <span class="req">*</span></label>
        <textarea id="import-token" class="form-input" rows="3" placeholder="Collez le token ici..." bind:value={importToken} style="resize:vertical;font-family:monospace;font-size:12px"></textarea>
      </div>
    </div>
  {/snippet}
  {#snippet actions()}
    <Button variant="ghost" size="sm" onclick={() => { showImportDialog = false; importToken = ""; }}>Annuler</Button>
    <Button variant="primary" size="sm" disabled={!importToken.trim()} onclick={async () => {
      const key = `imported-${Date.now()}`;
      await accounts.add(key, { name: key, claudeAiOauth: { accessToken: importToken.trim(), refreshToken: importToken.trim() } });
      showImportDialog = false;
      importToken = "";
    }}>Importer</Button>
  {/snippet}
</Dialog>

<!-- Dialog auto-import -->
<Dialog bind:open={showAutoImportDialog} title="Import automatique" onclose={closeAutoImportDialog}>
  {#snippet children()}
    {#if scanning}
      <div class="scan-scanning">
        <div class="scan-spinner"></div>
        <span>Scan des fichiers locaux en cours…</span>
      </div>
    {:else}
      <div class="add-form">
        <p class="scan-intro">
          Tokens Claude détectés sur ce poste. Sélectionnez ceux à importer.
        </p>
        <!-- Tout sélectionner -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="scan-select-all" onclick={toggleAll}>
          <input
            type="checkbox"
            checked={allSelected}
            indeterminate={someSelected}
            onclick={(e) => e.stopPropagation()}
            onchange={toggleAll}
          />
          <span>
            {allSelected ? "Tout désélectionner" : "Tout sélectionner"}
            &nbsp;·&nbsp;
            {selectedCredentials.size}/{scannedCredentials.length} sélectionné{selectedCredentials.size > 1 ? "s" : ""}
          </span>
        </div>
        <div class="scan-list">
          {#each scannedCredentials as cred (credKey(cred))}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
              class="scan-item"
              class:selected={selectedCredentials.has(credKey(cred))}
              onclick={() => toggleCredential(cred)}
            >
              <input
                type="checkbox"
                class="scan-checkbox"
                checked={selectedCredentials.has(credKey(cred))}
                onclick={(e) => e.stopPropagation()}
                onchange={() => toggleCredential(cred)}
              />
              <div class="scan-item-info">
                <span class="scan-label">{labelFor(cred)}</span>
                <span class="scan-source">{cred.sourcePath}</span>
                {#if cred.provider}
                  <span class="scan-provider-badge">{cred.provider}</span>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  {/snippet}
  {#snippet actions()}
    <Button variant="ghost" size="sm" onclick={closeAutoImportDialog} disabled={importing}>
      Annuler
    </Button>
    <Button
      variant="primary"
      size="sm"
      disabled={importing || scanning || selectedCredentials.size === 0}
      onclick={handleImportSelected}
    >
      {importing ? "Import…" : `Importer ${selectedCredentials.size > 0 ? selectedCredentials.size : ""} sélection${selectedCredentials.size > 1 ? "s" : ""}`}
    </Button>
  {/snippet}
</Dialog>

<!-- Dialog Setup automatique (OAuth capture) -->
<Dialog bind:open={showSetupDialog} title="Setup automatique" onclose={closeSetupDialog}>
  {#snippet children()}
    <div class="setup-body">

      {#if setupStep === "check"}
        <!-- Etape 1 : vérification du binaire claude -->
        <p class="setup-step-label">Etape 1 — Vérification</p>
        {#if checkingBinary}
          <div class="scan-scanning" style="padding:16px 0">
            <div class="scan-spinner"></div>
            <span>Recherche de claude dans le PATH…</span>
          </div>
        {:else if setupBinaryPath}
          <div class="setup-check-row ok">
            <span class="setup-check-icon">✓</span>
            <div>
              <div style="font-size:13px;font-weight:500;color:var(--fg-primary)">claude trouvé</div>
              <div class="setup-check-path">{setupBinaryPath}</div>
            </div>
          </div>
          <p class="setup-install-hint">
            Cliquez sur <strong>Continuer</strong> pour exécuter
            <code style="font-family:monospace;font-size:12px;background:var(--bg-app);border:1px solid var(--border);border-radius:4px;padding:1px 5px">claude setup-token</code>
            et capturer automatiquement votre token OAuth.
          </p>
        {:else}
          <div class="setup-check-row fail">
            <span class="setup-check-icon">✗</span>
            <div>
              <div style="font-size:13px;font-weight:500;color:var(--status-error)">claude introuvable</div>
              {#if setupError}
                <div class="setup-check-path" style="color:var(--fg-dim)">{setupError}</div>
              {/if}
            </div>
          </div>
          <p class="setup-install-hint">
            Claude CLI n'est pas installé ou n'est pas dans le PATH.<br />
            Installez-le depuis
            <a href="https://claude.ai/download" target="_blank" rel="noopener">claude.ai/download</a>
            puis relancez cette fenêtre.
          </p>
        {/if}

      {:else if setupStep === "capturing"}
        <!-- Etape 2 : capture en cours -->
        <p class="setup-step-label">Etape 2 — Capture en cours</p>
        <div class="setup-capture-body">
          <div class="setup-spinner"></div>
          <span class="setup-capture-label">Exécution de <code style="font-family:monospace;font-size:11px">claude setup-token</code>…</span>
          <span class="setup-capture-sublabel">Délai max : 60 secondes</span>
        </div>
        {#if setupOutput}
          <pre class="setup-output">{setupOutput}</pre>
        {/if}

      {:else if setupStep === "success"}
        <!-- Etape 3 : succès -->
        <p class="setup-step-label">Etape 3 — Succès</p>
        <div class="setup-success-body">
          <div class="setup-success-icon">✅</div>
          <div class="setup-success-title">Token capturé !</div>
          {#if setupEmail}
            <div class="setup-success-email">Compte ajouté : <strong>{setupEmail}</strong></div>
          {:else}
            <div class="setup-success-email">Le compte a été ajouté à la liste.</div>
          {/if}
        </div>

      {:else if setupStep === "error"}
        <!-- Erreur -->
        <p class="setup-step-label">Résultat — Fallback manuel</p>
        {#if setupError}
          <div class="setup-error-msg">{setupError}</div>
        {/if}
        {#if setupOutput}
          <pre class="setup-output">{setupOutput}</pre>
        {/if}
        <div class="setup-manual-hint">
          <strong>Instructions manuelles :</strong><br />
          1. Ouvrez un terminal et lancez <code>claude setup-token</code><br />
          2. Copiez le token affiché<br />
          3. Utilisez le bouton <strong>Importer</strong> pour coller le token manuellement
        </div>
      {/if}

    </div>
  {/snippet}

  {#snippet actions()}
    {#if setupStep === "check"}
      <Button variant="ghost" size="sm" onclick={closeSetupDialog}>Annuler</Button>
      {#if !checkingBinary && setupBinaryPath}
        <Button variant="primary" size="sm" onclick={handleStartCapture}>
          Continuer
        </Button>
      {:else if !checkingBinary && !setupBinaryPath}
        <Button variant="ghost" size="sm" onclick={handleOpenSetup}>Réessayer</Button>
      {/if}

    {:else if setupStep === "capturing"}
      <!-- Pas d'action pendant la capture -->
      <Button variant="ghost" size="sm" disabled>En cours…</Button>

    {:else if setupStep === "success"}
      <Button variant="primary" size="sm" onclick={handleSetupSuccess}>Fermer</Button>

    {:else if setupStep === "error"}
      <Button variant="ghost" size="sm" onclick={closeSetupDialog}>Fermer</Button>
      <Button variant="ghost" size="sm" onclick={resetSetup}>Réessayer</Button>
      <Button variant="primary" size="sm" onclick={() => { closeSetupDialog(); showImportDialog = true; }}>
        Importer manuellement
      </Button>
    {/if}
  {/snippet}
</Dialog>
