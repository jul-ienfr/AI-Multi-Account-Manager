<script lang="ts">
  import { config } from "../../lib/stores/config";
  import { syncStore } from "../../lib/stores/sync";
  import Toggle from "../ui/Toggle.svelte";
  import Card from "../ui/Card.svelte";
  import Button from "../ui/Button.svelte";
  import Badge from "../ui/Badge.svelte";
  import {
    Plus,
    Wifi,
    WifiOff,
    Trash2,
    RefreshCw,
    Eye,
    EyeOff,
    Key,
    Copy,
    Check,
    Zap,
    AlertCircle,
    Server,
    Terminal,
    Play,
    Square,
    Download,
  } from "lucide-svelte";
  import { onMount } from "svelte";
  import type { AppConfig, Peer, SshHostConfig } from "../../lib/types";
  import {
    getHostname,
    addSshHost,
    removeSshHost,
    testSshConnection,
    getSystemdStatus,
    installSystemdService,
    uninstallSystemdService,
  } from "../../lib/tauri";

  let cfg: AppConfig | null = $state(null);
  let peers: Peer[] = $state([]);

  // Peer add form
  let newHost = $state("");
  let newPort = $state(9090);

  // Key management
  let showKey = $state(false);
  let keyInput = $state("");
  let keyEditing = $state(false);
  let keyCopied = $state(false);

  // Peer test
  let testingPeer = $state<string | null>(null);
  let testResult = $state<{ host: string; ok: boolean; error?: string } | null>(null);

  // Instance hostname
  let hostname = $state("—");

  // SSH add form
  let sshUsername = $state("");
  let sshHost = $state("");
  let sshPort = $state(22);
  let sshIdentityPath = $state("");

  // SSH test
  let testingSsh = $state<string | null>(null);
  let sshTestResult = $state<{ id: string; ok: boolean; error?: string } | null>(null);

  // Systemd
  let systemdStatus = $state<string>("loading");
  let systemdBusy = $state(false);
  let systemdMessage = $state<string | null>(null);

  onMount(async () => {
    syncStore.load();
    const unsub1 = config.subscribe((c) => {
      cfg = c;
    });
    const unsub2 = syncStore.peers.subscribe((p) => {
      peers = p;
    });
    // Fetch hostname
    try {
      hostname = await getHostname();
    } catch { /* ignore */ }
    // Fetch systemd status
    try {
      systemdStatus = await getSystemdStatus();
    } catch { systemdStatus = "unavailable"; }
    return () => {
      unsub1();
      unsub2();
    };
  });

  async function updateP2PEnabled(checked: boolean) {
    if (!cfg?.sync) return;
    await config.save({ sync: { ...cfg.sync, enabled: checked } } as Partial<AppConfig>);
    await syncStore.load();
  }

  async function updateSyncOption(field: string, value: boolean) {
    if (!cfg?.sync) return;
    await config.save({
      sync: { ...cfg.sync, [field]: value },
    } as Partial<AppConfig>);
  }

  async function handleAddPeer() {
    if (!newHost) return;
    await syncStore.addPeer(newHost, newPort);
    newHost = "";
    newPort = 9090;
    await syncStore.load();
  }

  async function handleRemovePeer(id: string) {
    await syncStore.removePeer(id);
    await syncStore.load();
  }

  async function handleTestPeer(host: string, port: number) {
    const key = `${host}:${port}`;
    testingPeer = key;
    testResult = null;
    try {
      const ok = await syncStore.testPeer(host, port);
      testResult = { host: key, ok };
    } catch (e: unknown) {
      testResult = { host: key, ok: false, error: String(e) };
    } finally {
      testingPeer = null;
    }
  }

  async function handleGenerateKey() {
    const key = await syncStore.generateKey();
    if (cfg) {
      cfg = { ...cfg, sync: { ...cfg.sync, sharedKeyHex: key } };
    }
    showKey = true;
  }

  async function handleSaveKey() {
    if (!keyInput || keyInput.length !== 64) return;
    await syncStore.setKey(keyInput);
    if (cfg) {
      cfg = { ...cfg, sync: { ...cfg.sync, sharedKeyHex: keyInput } };
    }
    keyEditing = false;
    keyInput = "";
  }

  function handleCopyKey() {
    if (!cfg?.sync?.sharedKeyHex) return;
    navigator.clipboard.writeText(cfg.sync.sharedKeyHex);
    keyCopied = true;
    setTimeout(() => {
      keyCopied = false;
    }, 2000);
  }

  function maskedKey(hex: string | null): string {
    if (!hex) return "—";
    if (showKey) return hex;
    return hex.substring(0, 8) + "..." + hex.substring(hex.length - 8);
  }

  // --- SSH ---
  async function updateSshEnabled(checked: boolean) {
    if (!cfg?.sync) return;
    await config.save({ sync: { ...cfg.sync, sshEnabled: checked } } as Partial<AppConfig>);
  }

  async function handleAddSshHost() {
    if (!sshUsername || !sshHost) return;
    await addSshHost(sshHost, sshPort, sshUsername, sshIdentityPath || undefined);
    // Reload config to refresh ssh_hosts list
    await config.load();
    sshUsername = "";
    sshHost = "";
    sshPort = 22;
    sshIdentityPath = "";
  }

  async function handleRemoveSshHost(id: string) {
    await removeSshHost(id);
    await config.load();
  }

  async function handleTestSshHost(host: SshHostConfig) {
    testingSsh = host.id;
    sshTestResult = null;
    try {
      const ok = await testSshConnection(host.host, host.port, host.username, host.identityPath);
      sshTestResult = { id: host.id, ok };
    } catch (e: unknown) {
      sshTestResult = { id: host.id, ok: false, error: String(e) };
    } finally {
      testingSsh = null;
    }
  }

  // --- Systemd ---
  async function handleInstallSystemd() {
    systemdBusy = true;
    systemdMessage = null;
    try {
      const msg = await installSystemdService();
      systemdMessage = msg;
      systemdStatus = await getSystemdStatus();
    } catch (e: unknown) {
      systemdMessage = String(e);
    } finally {
      systemdBusy = false;
    }
  }

  async function handleUninstallSystemd() {
    systemdBusy = true;
    systemdMessage = null;
    try {
      const msg = await uninstallSystemdService();
      systemdMessage = msg;
      systemdStatus = await getSystemdStatus();
    } catch (e: unknown) {
      systemdMessage = String(e);
    } finally {
      systemdBusy = false;
    }
  }

  async function refreshSystemdStatus() {
    try {
      systemdStatus = await getSystemdStatus();
    } catch { systemdStatus = "unavailable"; }
  }
</script>

<div class="network-settings">
  <h3 class="section-title">Reseau & P2P</h3>

  {#if cfg}
    <div class="settings-group">
      <!-- P2P Toggle -->
      <Card hoverable={false}>
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">Synchronisation P2P</span>
            <span class="setting-desc">Partager les credentials entre instances</span>
          </div>
          <Toggle
            checked={cfg?.sync?.enabled ?? false}
            onchange={updateP2PEnabled}
          />
        </div>
      </Card>

      {#if cfg?.sync?.enabled}
        <!-- Port + Instance ID -->
        <Card hoverable={false}>
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-label">Port TCP</span>
              <span class="setting-desc">Port de la synchronisation P2P</span>
            </div>
            <code class="mono-value">{cfg?.sync?.port ?? 9090}</code>
          </div>
        </Card>

        <!-- Shared Key -->
        <Card hoverable={false}>
          <div class="key-section">
            <div class="setting-row">
              <div class="setting-info">
                <span class="setting-label">Cle partagee (NaCl SecretBox)</span>
                <span class="setting-desc">Cle de chiffrement P2P (32 bytes hex)</span>
              </div>
            </div>

            <div class="key-display">
              <code class="key-value">{maskedKey(cfg?.sync?.sharedKeyHex ?? null)}</code>
              <div class="key-actions">
                <button class="icon-btn" onclick={() => (showKey = !showKey)} title={showKey ? "Masquer" : "Afficher"}>
                  {#if showKey}
                    <EyeOff size={14} />
                  {:else}
                    <Eye size={14} />
                  {/if}
                </button>
                {#if cfg?.sync?.sharedKeyHex}
                  <button class="icon-btn" onclick={handleCopyKey} title="Copier">
                    {#if keyCopied}
                      <Check size={14} />
                    {:else}
                      <Copy size={14} />
                    {/if}
                  </button>
                {/if}
              </div>
            </div>

            <div class="key-buttons">
              <Button variant="primary" size="sm" onclick={handleGenerateKey}>
                <Key size={14} />
                Generer
              </Button>
              {#if !keyEditing}
                <Button variant="ghost" size="sm" onclick={() => { keyEditing = true; keyInput = ""; }}>
                  Saisir manuellement
                </Button>
              {:else}
                <div class="key-edit-row">
                  <input
                    type="text"
                    class="peer-input key-input"
                    placeholder="64 caracteres hex..."
                    maxlength={64}
                    bind:value={keyInput}
                  />
                  <Button variant="primary" size="sm" onclick={handleSaveKey} disabled={keyInput.length !== 64}>
                    Sauvegarder
                  </Button>
                  <Button variant="ghost" size="sm" onclick={() => { keyEditing = false; }}>
                    Annuler
                  </Button>
                </div>
              {/if}
            </div>
          </div>
        </Card>

        <!-- Peers list -->
        <div class="peers-section">
          <h4 class="subsection-title">Pairs connectes</h4>

          <div class="add-peer-row">
            <input
              type="text"
              class="peer-input"
              placeholder="Hote (ex: 192.168.1.10)"
              bind:value={newHost}
            />
            <input
              type="number"
              class="peer-input port-input"
              placeholder="Port"
              bind:value={newPort}
            />
            <Button variant="primary" size="sm" onclick={handleAddPeer}>
              <Plus size={14} />
              Ajouter
            </Button>
            {#if newHost}
              <Button
                variant="ghost"
                size="sm"
                onclick={() => handleTestPeer(newHost, newPort)}
                disabled={testingPeer !== null}
              >
                <Zap size={14} />
                Tester
              </Button>
            {/if}
          </div>

          {#if testResult}
            <div class="test-result" class:test-ok={testResult.ok} class:test-fail={!testResult.ok}>
              {#if testResult.ok}
                <Check size={14} /> Connexion reussie vers {testResult.host}
              {:else}
                <AlertCircle size={14} /> Echec: {testResult.error ?? "Connexion refusee"}
              {/if}
            </div>
          {/if}

          <div class="peer-list">
            {#each peers as peer (peer.id)}
              <div class="peer-item">
                <span class="peer-icon">
                  {#if peer.connected}
                    <Wifi size={14} />
                  {:else}
                    <WifiOff size={14} />
                  {/if}
                </span>
                <span class="peer-address">{peer.host}:{peer.port}</span>
                <Badge
                  color={peer.connected ? "var(--status-running)" : "var(--status-stopped)"}
                  small
                >
                  {peer.connected ? "Connecte" : "Deconnecte"}
                </Badge>
                {#if peer.lastSeen}
                  <span class="peer-seen">{peer.lastSeen}</span>
                {/if}
                <button
                  class="icon-btn icon-btn-danger"
                  title="Tester la connexion"
                  onclick={() => handleTestPeer(peer.host, peer.port)}
                  disabled={testingPeer === `${peer.host}:${peer.port}`}
                >
                  <Zap size={12} />
                </button>
                <button
                  class="icon-btn icon-btn-danger"
                  title="Supprimer"
                  onclick={() => handleRemovePeer(peer.id)}
                >
                  <Trash2 size={12} />
                </button>
              </div>
            {/each}

            {#if peers.length === 0}
              <p class="no-peers">Aucun pair configure</p>
            {/if}
          </div>
        </div>

        <!-- Sync Options -->
        <div class="options-section">
          <h4 class="subsection-title">Options de synchronisation</h4>

          <Card hoverable={false}>
            <div class="option-row">
              <div class="setting-info">
                <span class="setting-label">Synchroniser le compte actif</span>
                <span class="setting-desc">Propager les switchs de compte entre pairs</span>
              </div>
              <Toggle
                checked={cfg?.sync?.syncActiveAccount ?? true}
                onchange={(v: boolean) => updateSyncOption("syncActiveAccount", v)}
              />
            </div>
          </Card>

          <Card hoverable={false}>
            <div class="option-row">
              <div class="setting-info">
                <span class="setting-label">Synchroniser les quotas</span>
                <span class="setting-desc">Partager les mises a jour de quota entre pairs</span>
              </div>
              <Toggle
                checked={cfg?.sync?.syncQuota ?? true}
                onchange={(v: boolean) => updateSyncOption("syncQuota", v)}
              />
            </div>
          </Card>

          <Card hoverable={false}>
            <div class="option-row">
              <div class="setting-info">
                <span class="setting-label">Repartir les fetches de quota</span>
                <span class="setting-desc">Diviser les appels API quota entre pairs</span>
              </div>
              <Toggle
                checked={cfg?.sync?.splitQuotaFetch ?? true}
                onchange={(v: boolean) => updateSyncOption("splitQuotaFetch", v)}
              />
            </div>
          </Card>

          <Card hoverable={false}>
            <div class="option-row">
              <div class="setting-info">
                <span class="setting-label">Failover proxy automatique</span>
                <span class="setting-desc">Basculer vers un proxy pair si le local tombe</span>
              </div>
              <Toggle
                checked={cfg?.sync?.proxyFailover ?? true}
                onchange={(v: boolean) => updateSyncOption("proxyFailover", v)}
              />
            </div>
          </Card>
        </div>

        <!-- Daemon info -->
        <Card hoverable={false}>
          <div class="daemon-info">
            <div class="setting-info">
              <span class="setting-label">Mode daemon (headless)</span>
              <span class="setting-desc">
                Compatible serveur Ubuntu sans GUI — meme fichier settings.json
              </span>
            </div>
            <code class="mono-value code-block">ai-manager-daemon --sync-enabled --sync-port {cfg?.sync?.port ?? 9090} --sync-key &lt;base64&gt;</code>
          </div>
        </Card>

        <!-- Instance hostname -->
        <Card hoverable={false}>
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-label">Nom de cette instance</span>
              <span class="setting-desc">Utilise pour l'identification P2P et le proxy owner</span>
            </div>
            <code class="mono-value">{hostname}</code>
          </div>
        </Card>
      {/if}

      <!-- SSH Sync Section -->
      <div class="ssh-section">
        <h4 class="subsection-title">
          <Terminal size={16} />
          Synchronisation SSH
        </h4>

        <Card hoverable={false}>
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-label">Activer la sync SSH</span>
              <span class="setting-desc">Pousser les credentials vers des serveurs distants via SCP</span>
            </div>
            <Toggle
              checked={cfg?.sync?.sshEnabled ?? false}
              onchange={updateSshEnabled}
            />
          </div>
        </Card>

        {#if cfg?.sync?.sshEnabled}
          <!-- Add SSH host form -->
          <div class="ssh-add-form">
            <div class="ssh-form-row">
              <input
                type="text"
                class="peer-input"
                placeholder="Utilisateur"
                bind:value={sshUsername}
              />
              <span class="ssh-at">@</span>
              <input
                type="text"
                class="peer-input"
                placeholder="Hote (ex: 192.168.1.10)"
                bind:value={sshHost}
              />
              <input
                type="number"
                class="peer-input port-input"
                placeholder="22"
                bind:value={sshPort}
              />
            </div>
            <div class="ssh-form-row">
              <input
                type="text"
                class="peer-input"
                placeholder="Chemin cle privee (optionnel, ex: ~/.ssh/id_rsa)"
                bind:value={sshIdentityPath}
              />
              <Button variant="primary" size="sm" onclick={handleAddSshHost} disabled={!sshUsername || !sshHost}>
                <Plus size={14} />
                Ajouter
              </Button>
            </div>
          </div>

          {#if sshTestResult}
            <div class="test-result" class:test-ok={sshTestResult.ok} class:test-fail={!sshTestResult.ok}>
              {#if sshTestResult.ok}
                <Check size={14} /> Connexion SSH reussie vers {sshTestResult.id}
              {:else}
                <AlertCircle size={14} /> Echec SSH: {sshTestResult.error ?? "Connexion refusee"}
              {/if}
            </div>
          {/if}

          <!-- SSH hosts list -->
          <div class="peer-list">
            {#each cfg?.sync?.sshHosts ?? [] as host (host.id)}
              <div class="peer-item">
                <span class="peer-icon">
                  <Server size={14} />
                </span>
                <span class="peer-address">{host.username}@{host.host}:{host.port}</span>
                <Badge
                  color={host.enabled ? "var(--status-running)" : "var(--status-stopped)"}
                  small
                >
                  {host.enabled ? "Actif" : "Inactif"}
                </Badge>
                {#if host.identityPath}
                  <span class="peer-seen" title={host.identityPath}>
                    <Key size={10} /> {host.identityPath.split("/").pop()}
                  </span>
                {/if}
                <button
                  class="icon-btn"
                  title="Tester la connexion SSH"
                  onclick={() => handleTestSshHost(host)}
                  disabled={testingSsh === host.id}
                >
                  <Zap size={12} />
                </button>
                <button
                  class="icon-btn icon-btn-danger"
                  title="Supprimer"
                  onclick={() => handleRemoveSshHost(host.id)}
                >
                  <Trash2 size={12} />
                </button>
              </div>
            {/each}

            {#if (cfg?.sync?.sshHosts ?? []).length === 0}
              <p class="no-peers">Aucun hote SSH configure</p>
            {/if}
          </div>
        {/if}
      </div>

      <!-- Systemd auto-start section -->
      {#if systemdStatus !== "unavailable"}
        <div class="systemd-section">
          <h4 class="subsection-title">
            <Play size={16} />
            Lancement automatique (systemd)
          </h4>

          <Card hoverable={false}>
            <div class="setting-row">
              <div class="setting-info">
                <span class="setting-label">Service systemd</span>
                <span class="setting-desc">
                  Lancer automatiquement le daemon + proxy au demarrage du systeme
                </span>
              </div>
              <div class="systemd-status-row">
                <Badge
                  color={systemdStatus === "active" ? "var(--status-running)" : systemdStatus === "inactive" ? "var(--status-warning)" : "var(--status-stopped)"}
                  small
                >
                  {systemdStatus === "active" ? "Actif" : systemdStatus === "inactive" ? "Inactif" : systemdStatus === "not-found" ? "Non installe" : systemdStatus}
                </Badge>
                <button class="icon-btn" onclick={refreshSystemdStatus} title="Rafraichir le statut">
                  <RefreshCw size={12} />
                </button>
              </div>
            </div>
          </Card>

          <div class="systemd-actions">
            {#if systemdStatus === "not-found" || systemdStatus === "loading"}
              <Button variant="primary" size="sm" onclick={handleInstallSystemd} disabled={systemdBusy}>
                <Download size={14} />
                Installer le service
              </Button>
            {:else}
              {#if systemdStatus === "active"}
                <Button variant="ghost" size="sm" onclick={handleUninstallSystemd} disabled={systemdBusy}>
                  <Square size={14} />
                  Desinstaller
                </Button>
              {:else}
                <Button variant="primary" size="sm" onclick={handleInstallSystemd} disabled={systemdBusy}>
                  <Play size={14} />
                  Reinstaller et demarrer
                </Button>
                <Button variant="ghost" size="sm" onclick={handleUninstallSystemd} disabled={systemdBusy}>
                  <Trash2 size={14} />
                  Desinstaller
                </Button>
              {/if}
            {/if}
          </div>

          {#if systemdMessage}
            <div class="test-result" class:test-ok={systemdStatus === "active"} class:test-fail={systemdStatus !== "active"}>
              {systemdMessage}
            </div>
          {/if}

          <Card hoverable={false}>
            <div class="daemon-info">
              <span class="setting-desc">
                Le service systemd lancera <code>ai-manager-daemon --settings ~/path/settings.json</code> au demarrage.
                Le proxy et la sync P2P se lanceront automatiquement selon la configuration.
              </span>
            </div>
          </Card>
        </div>
      {/if}
    </div>
  {:else}
    <p class="loading-text">Chargement...</p>
  {/if}
</div>

<style>
  .network-settings {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .section-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--fg-primary);
    margin-bottom: 4px;
  }

  .subsection-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--fg-primary);
    margin-bottom: 8px;
  }

  .settings-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .setting-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .option-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .setting-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
  }

  .setting-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--fg-primary);
  }

  .setting-desc {
    font-size: 11px;
    color: var(--fg-dim);
  }

  .mono-value {
    font-size: 12px;
    font-family: "JetBrains Mono", "Fira Code", monospace;
    color: var(--fg-secondary);
    background: var(--bg-app);
    padding: 2px 8px;
    border-radius: var(--radius-sm);
  }

  /* Key section */
  .key-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .key-display {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .key-value {
    font-size: 11px;
    font-family: "JetBrains Mono", "Fira Code", monospace;
    color: var(--fg-secondary);
    background: var(--bg-app);
    padding: 4px 10px;
    border-radius: var(--radius-sm);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .key-actions {
    display: flex;
    gap: 4px;
  }

  .key-buttons {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }

  .key-edit-row {
    display: flex;
    gap: 6px;
    align-items: center;
    flex: 1;
  }

  .key-input {
    flex: 1;
    min-width: 200px;
  }

  .icon-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-card);
    color: var(--fg-dim);
    cursor: pointer;
    transition: all 0.15s;
  }

  .icon-btn:hover {
    color: var(--fg-primary);
    border-color: var(--accent);
  }

  .icon-btn-danger:hover {
    color: var(--status-stopped);
    border-color: var(--status-stopped);
  }

  .icon-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* Peers */
  .peers-section {
    margin-top: 8px;
  }

  .add-peer-row {
    display: flex;
    gap: 8px;
    margin-bottom: 12px;
    flex-wrap: wrap;
  }

  .peer-input {
    padding: 6px 10px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg-primary);
    font-size: 12px;
    flex: 1;
  }

  .peer-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .port-input {
    max-width: 80px;
    flex: 0 0 80px;
  }

  .peer-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .peer-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    font-size: 12px;
  }

  .peer-icon {
    display: flex;
    color: var(--fg-dim);
  }

  .peer-address {
    flex: 1;
    color: var(--fg-primary);
    font-family: "JetBrains Mono", "Fira Code", monospace;
  }

  .peer-seen {
    font-size: 11px;
    color: var(--fg-dim);
  }

  .no-peers {
    color: var(--fg-dim);
    font-size: 12px;
    padding: 12px;
    text-align: center;
  }

  /* Test result */
  .test-result {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-radius: var(--radius-sm);
    font-size: 12px;
    margin-bottom: 8px;
  }

  .test-ok {
    background: color-mix(in srgb, var(--status-running) 15%, transparent);
    color: var(--status-running);
    border: 1px solid color-mix(in srgb, var(--status-running) 30%, transparent);
  }

  .test-fail {
    background: color-mix(in srgb, var(--status-stopped) 15%, transparent);
    color: var(--status-stopped);
    border: 1px solid color-mix(in srgb, var(--status-stopped) 30%, transparent);
  }

  /* Options section */
  .options-section {
    margin-top: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* Daemon info */
  .daemon-info {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .code-block {
    display: block;
    font-size: 11px;
    padding: 8px 12px;
    word-break: break-all;
    white-space: pre-wrap;
    margin-top: 4px;
  }

  .loading-text {
    color: var(--fg-dim);
    font-size: 13px;
  }

  /* SSH section */
  .ssh-section {
    margin-top: 16px;
  }

  .subsection-title {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .ssh-add-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
    margin-bottom: 12px;
  }

  .ssh-form-row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }

  .ssh-at {
    color: var(--fg-dim);
    font-weight: 600;
    font-size: 13px;
  }

  /* Systemd section */
  .systemd-section {
    margin-top: 16px;
  }

  .systemd-status-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .systemd-actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
    margin-bottom: 8px;
  }
</style>
