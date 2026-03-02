import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AccountState, AppConfig, ProxyStatus, ProxyInstanceState, ProxyInstanceConfig, Peer, QuotaInfo, AccountData, QuotaHistoryPoint, SwitchEntry, ImpersonationProfile, ScannedCredential, CaptureResult } from "./types";

// --- Comptes ---
export const getAccounts = () => invoke<AccountState[]>("get_accounts");
export const getActiveAccount = () => invoke<AccountState | null>("get_active_account");
export const switchAccount = (key: string) => invoke<void>("switch_account", { key });
export const refreshAccount = (key: string) => invoke<void>("refresh_account", { key });
export const revokeAccount = (key: string) => invoke<void>("revoke_account", { key });
export const addAccount = (key: string, data: Partial<AccountData>) =>
  invoke<void>("add_account", { key, data });
export const updateAccount = (key: string, updates: { priority?: number; autoSwitchDisabled?: boolean; displayName?: string; planType?: string; geminiProject?: string }) =>
  invoke<void>("update_account", { key, updates });
export const geminiOAuthFlow = () =>
  invoke<{ email: string; accessToken: string; refreshToken: string; expiresAtMs: number | null; accountKey: string }>("gemini_oauth_flow");
export const deleteAccount = (key: string) => invoke<void>("delete_account", { key });
export const captureBeforeSwitch = (outgoingKey: string) =>
  invoke<boolean>("capture_before_switch", { outgoingKey });

// --- Config ---
export const getConfig = () => invoke<AppConfig>("get_config");
export const setConfig = (config: Partial<AppConfig>) => invoke<void>("set_config", { config });

// --- Proxy legacy ---
export const getProxyStatus = () =>
  invoke<{ router: ProxyStatus; impersonator: ProxyStatus }>("get_proxy_status");
export const startProxy = (kind?: "router" | "impersonator") => invoke<void>("start_proxy", { kind });
export const stopProxy = (kind?: "router" | "impersonator") => invoke<void>("stop_proxy", { kind });
export const restartProxy = (kind?: "router" | "impersonator") => invoke<void>("restart_proxy", { kind });

// --- Proxy instances ---
export const getProxyInstances = () => invoke<ProxyInstanceState[]>("get_proxy_instances");
export const addProxyInstance = (config: ProxyInstanceConfig) => invoke<void>("add_proxy_instance", { config });
export const updateProxyInstance = (id: string, updates: Partial<ProxyInstanceConfig>) =>
  invoke<void>("update_proxy_instance", { id, updates });
export const deleteProxyInstance = (id: string) => invoke<void>("delete_proxy_instance", { id });
export const startProxyInstance = (id: string) => invoke<void>("start_proxy_instance", { id });
export const stopProxyInstance = (id: string) => invoke<void>("stop_proxy_instance", { id });
export const restartProxyInstance = (id: string) => invoke<void>("restart_proxy_instance", { id });
export const probeProxyInstances = () => invoke<ProxyInstanceState[]>("probe_proxy_instances");
export const detectProxyBinaries = () =>
  invoke<Array<{ id: string; name: string; path: string; defaultPort: number }>>("detect_proxy_binaries");

// --- Setup injection ---
export const setupClaudeCode = (port: number) => invoke<void>("setup_claude_code", { port });
export const removeClaudeCodeSetup = () => invoke<void>("remove_claude_code_setup");
export const setupVscodeProxy = (port: number) => invoke<void>("setup_vscode_proxy", { port });
export const removeVscodeProxy = () => invoke<void>("remove_vscode_proxy");

// --- Systemd ---
export const getSystemdStatus = () => invoke<string>("get_systemd_status");
export const installSystemdService = (daemonPath?: string) =>
  invoke<string>("install_systemd_service", { daemonPath });
export const uninstallSystemdService = () => invoke<string>("uninstall_systemd_service");

// --- Sync P2P ---
export const getSyncStatus = () => invoke<{ enabled: boolean; peers: number }>("get_sync_status");
export const generateSyncKey = () => invoke<string>("generate_sync_key");
export const setSyncKey = (key: string) => invoke<void>("set_sync_key", { key });
export const toggleSync = (enabled: boolean) => invoke<void>("toggle_sync", { enabled });
export const setSyncPort = (port: number) => invoke<void>("set_sync_port", { port });

// --- Peers ---
export const getPeers = () => invoke<Peer[]>("get_peers");
export const addPeer = (host: string, port: number, id?: string) => invoke<void>("add_peer", { host, port, id });
export const removePeer = (id: string) => invoke<void>("remove_peer", { id });
export const testPeerConnection = (host: string, port: number) =>
  invoke<boolean>("test_peer_connection", { host, port }).catch(() => false);

// --- SSH Sync ---
export const getHostname = () => invoke<string>("get_hostname");
export const addSshHost = (host: string, port: number, username: string, identityPath?: string) =>
  invoke<void>("add_ssh_host", { host, port, username, identityPath });
export const removeSshHost = (id: string) => invoke<void>("remove_ssh_host", { id });
export const testSshConnection = (host: string, port: number, username: string, identityPath?: string) =>
  invoke<boolean>("test_ssh_connection", { host, port, username, identityPath }).catch(() => false);

// --- Monitoring ---
export const getQuotaHistory = (key: string, period?: "24h" | "7d" | "30d") =>
  invoke<QuotaHistoryPoint[]>("get_quota_history", { key, period });
export const getSwitchHistory = () => invoke<SwitchEntry[]>("get_switch_history");
export const getImpersonationProfiles = () => invoke<ImpersonationProfile[]>("get_impersonation_profiles");
export const getSessions = () => invoke<unknown[]>("get_sessions");
export const getLogs = (filter?: string) => invoke<unknown[]>("get_logs", { filter });

// --- Credentials ---
export const scanLocalCredentials = () => invoke<ScannedCredential[]>("scan_local_credentials");
export const importScannedCredentials = (credentials: ScannedCredential[]) =>
  invoke<number>("import_scanned_credentials", { credentials });
export const findClaudeBinary = () => invoke<string>("find_claude_binary");
export const captureOAuthToken = (timeoutSecs?: number) =>
  invoke<CaptureResult>("capture_oauth_token", { timeoutSecs });

// --- Profils ---
export const getProfiles = () => invoke<Array<{ name: string; createdAt: string; sizeBytes: number }>>("list_profiles");
export const saveProfile = (name: string, config: unknown) => invoke<void>("save_profile", { name, config });
export const loadProfile = (name: string) => invoke<unknown>("load_profile", { name });
export const deleteProfile = (name: string) => invoke<void>("delete_profile", { name });

// --- Stats ---
export const getStats = () => invoke<unknown>("get_stats");

// --- Health ---
export const getHealth = async () => ({ status: "ok" });

// --- Event listeners (Tauri events via listen()) ---
export const onQuotaUpdate = (cb: (data: { key: string; quota: QuotaInfo }) => void): Promise<() => void> =>
  listen("quota_update", (event) => cb(event.payload as { key: string; quota: QuotaInfo }));

export const onToast = (cb: (toast: { type: string; title: string; message?: string }) => void): Promise<() => void> =>
  listen("toast", (event) => cb(event.payload as { type: string; title: string; message?: string }));

export const onProxyStatus = (cb: (status: ProxyStatus) => void): Promise<() => void> =>
  listen("proxy_status", (event) => cb(event.payload as ProxyStatus));

export const onAccountSwitch = (cb: (key: string) => void): Promise<() => void> =>
  listen("account_switch", (event) => cb(event.payload as string));

export const onSyncRefresh = (cb: () => void): Promise<() => void> =>
  listen("sync_refresh", () => cb());
