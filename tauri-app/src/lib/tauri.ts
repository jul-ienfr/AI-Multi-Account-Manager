import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AccountState, AppConfig, ProxyStatus, ProxyInstanceState, ProxyInstanceConfig, Peer, QuotaInfo, AccountData, QuotaHistoryPoint, SwitchEntry, ImpersonationProfile, ScannedCredential, CaptureResult } from "./types";

// Comptes
export const getAccounts = () => invoke<AccountState[]>("get_accounts");
export const getActiveAccount = () => invoke<AccountState | null>("get_active_account");
export const switchAccount = (key: string) => invoke<void>("switch_account", { key });
export const refreshAccount = (key: string) => invoke<void>("refresh_account", { key });
export const addAccount = (key: string, data: Partial<AccountData>) => invoke<void>("add_account", { key, data });
export const updateAccount = (key: string, updates: { priority?: number; autoSwitchDisabled?: boolean; displayName?: string }) => invoke<void>("update_account", { key, updates });
export const deleteAccount = (key: string) => invoke<void>("delete_account", { key });

// Config
export const getConfig = () => invoke<AppConfig>("get_config");
export const setConfig = (config: Partial<AppConfig>) => invoke<void>("set_config", { config });

// Proxy (legacy)
export const getProxyStatus = () => invoke<{ router: ProxyStatus; impersonator: ProxyStatus }>("get_proxy_status");
export const startProxy = (kind: "router" | "impersonator") => invoke<void>("start_proxy", { kind });
export const stopProxy = (kind: "router" | "impersonator") => invoke<void>("stop_proxy", { kind });
export const restartProxy = (kind: "router" | "impersonator") => invoke<void>("restart_proxy", { kind });

// Proxy instances (dynamic)
export const getProxyInstances = () => invoke<ProxyInstanceState[]>("get_proxy_instances");
export const addProxyInstance = (config: ProxyInstanceConfig) => invoke<void>("add_proxy_instance", { config });
export const updateProxyInstance = (id: string, updates: Partial<ProxyInstanceConfig>) => invoke<void>("update_proxy_instance", { id, updates });
export const deleteProxyInstance = (id: string) => invoke<void>("delete_proxy_instance", { id });
export const startProxyInstance = (id: string) => invoke<void>("start_proxy_instance", { id });
export const stopProxyInstance = (id: string) => invoke<void>("stop_proxy_instance", { id });
export const restartProxyInstance = (id: string) => invoke<void>("restart_proxy_instance", { id });
export const detectProxyBinaries = () => invoke<Array<{ id: string; name: string; path: string; defaultPort: number }>>("detect_proxy_binaries");
export const probeProxyInstances = () => invoke<ProxyInstanceState[]>("probe_proxy_instances");

// Setup injection
export const setupClaudeCode = (port: number) => invoke<void>("setup_claude_code", { port });
export const removeClaudeCodeSetup = () => invoke<void>("remove_claude_code_setup");
export const setupVscodeProxy = (port: number) => invoke<void>("setup_vscode_proxy", { port });
export const removeVscodeProxy = () => invoke<void>("remove_vscode_proxy");

// Systemd (daemon auto-start)
export const getSystemdStatus = () => invoke<string>("get_systemd_status");
export const installSystemdService = (daemonPath?: string) =>
  invoke<string>("install_systemd_service", { daemonPath });
export const uninstallSystemdService = () => invoke<string>("uninstall_systemd_service");

// Sync
export const getSyncStatus = () => invoke<{ enabled: boolean; peers: number }>("get_sync_status");
export const getPeers = () => invoke<Peer[]>("get_peers");
export const addPeer = (host: string, port: number) => invoke<void>("add_peer", { host, port });
export const removePeer = (id: string) => invoke<void>("remove_peer", { id });
export const generateSyncKey = () => invoke<string>("generate_sync_key");
export const setSyncKey = (key: string) => invoke<void>("set_sync_key", { key });
export const testPeerConnection = (host: string, port: number) => invoke<boolean>("test_peer_connection", { host, port });

// SSH Sync
export const getHostname = () => invoke<string>("get_hostname");
export const addSshHost = (host: string, port: number, username: string, identityPath?: string) =>
  invoke<void>("add_ssh_host", { host, port, username, identityPath });
export const removeSshHost = (id: string) => invoke<void>("remove_ssh_host", { id });
export const testSshConnection = (host: string, port: number, username: string, identityPath?: string) =>
  invoke<boolean>("test_ssh_connection", { host, port, username, identityPath });

// Monitoring
export const getQuotaHistory = (key: string, period?: "24h" | "7d" | "30d") =>
  invoke<QuotaHistoryPoint[]>("get_quota_history", { key, period: period ?? "24h" });
export const getSessions = () => invoke<unknown[]>("get_sessions");
export const getLogs = (filter?: string) => invoke<unknown[]>("get_logs", { filter });

export const getSwitchHistory = () =>
  invoke<SwitchEntry[]>("get_switch_history");

export const getImpersonationProfiles = () =>
  invoke<ImpersonationProfile[]>("get_impersonation_profiles");

// Scan & import automatique de credentials locaux
export const scanLocalCredentials = () =>
  invoke<ScannedCredential[]>("scan_local_credentials");
export const importScannedCredentials = (credentials: ScannedCredential[]) =>
  invoke<number>("import_scanned_credentials", { credentials });

// OAuth capture via Claude CLI
export const findClaudeBinary = () =>
  invoke<string>("find_claude_binary");
export const captureOAuthToken = (timeoutSecs?: number) =>
  invoke<CaptureResult>("capture_oauth_token", { timeoutSecs });

// Phase 3.4a — Capture token roté avant switch
export const captureBeforeSwitch = (outgoingKey: string): Promise<boolean> =>
  invoke<boolean>("capture_before_switch", { outgoingKey });

// Event listeners
export const onQuotaUpdate = (cb: (data: { key: string; quota: QuotaInfo }) => void) =>
  listen("quota_update", (e) => cb(e.payload as { key: string; quota: QuotaInfo }));
export const onToast = (cb: (toast: { type: string; title: string; message?: string }) => void) =>
  listen("toast", (e) => cb(e.payload as { type: string; title: string; message?: string }));
export const onProxyStatus = (cb: (status: ProxyStatus) => void) =>
  listen("proxy_status", (e) => cb(e.payload as ProxyStatus));
export const onAccountSwitch = (cb: (key: string) => void) =>
  listen("account_switch", (e) => cb(e.payload as string));
