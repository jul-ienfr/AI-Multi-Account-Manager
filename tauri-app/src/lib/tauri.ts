import type { AccountState, AppConfig, ProxyStatus, ProxyInstanceState, ProxyInstanceConfig, Peer, QuotaInfo, AccountData, QuotaHistoryPoint, SwitchEntry, ImpersonationProfile, ScannedCredential, CaptureResult } from "./types";

const API_BASE = "/ai-manager/admin/api";
const WS_URL = "/ai-manager/admin/ws";

// --- HTTP helpers ---

async function get<T>(path: string, params?: Record<string, string | undefined>): Promise<T> {
  let url = `${API_BASE}/${path}`;
  if (params) {
    const qs = new URLSearchParams(
      Object.fromEntries(Object.entries(params).filter(([, v]) => v !== undefined)) as Record<string, string>
    ).toString();
    if (qs) url += `?${qs}`;
  }
  const res = await fetch(url);
  if (!res.ok) throw new Error(`GET ${path} failed: ${res.status} ${res.statusText}`);
  return res.json() as Promise<T>;
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${API_BASE}/${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) throw new Error(`POST ${path} failed: ${res.status} ${res.statusText}`);
  return res.json() as Promise<T>;
}

async function put<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${API_BASE}/${path}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) throw new Error(`PUT ${path} failed: ${res.status} ${res.statusText}`);
  return res.json() as Promise<T>;
}

async function del<T>(path: string): Promise<T> {
  const res = await fetch(`${API_BASE}/${path}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`DELETE ${path} failed: ${res.status} ${res.statusText}`);
  return res.json() as Promise<T>;
}

// --- WebSocket event bus ---

type EventCallback = (payload: unknown) => void;

let ws: WebSocket | null = null;
const subscribers = new Map<string, Set<EventCallback>>();

function getWs(): WebSocket {
  if (ws && ws.readyState === WebSocket.OPEN) return ws;

  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const url = `${protocol}//${location.host}${WS_URL}`;

  ws = new WebSocket(url);

  ws.addEventListener("message", (ev) => {
    try {
      const msg = JSON.parse(ev.data as string) as { event: string; payload: unknown };
      const cbs = subscribers.get(msg.event);
      if (cbs) cbs.forEach((cb) => cb(msg.payload));
    } catch {
      // ignore malformed messages
    }
  });

  ws.addEventListener("close", () => {
    ws = null;
    if (subscribers.size > 0) {
      setTimeout(() => getWs(), 2000);
    }
  });

  return ws;
}

function subscribeEvent(event: string, cb: EventCallback): () => void {
  if (!subscribers.has(event)) subscribers.set(event, new Set());
  subscribers.get(event)!.add(cb);
  getWs();
  return () => {
    subscribers.get(event)?.delete(cb);
    if (subscribers.get(event)?.size === 0) subscribers.delete(event);
  };
}

// --- Comptes --- GET /accounts, POST /accounts, PUT /accounts/:key, DELETE /accounts/:key
export const getAccounts = () => get<AccountState[]>("accounts");
export const getActiveAccount = () => get<AccountState | null>("accounts/active");
export const switchAccount = (key: string) => post<void>(`accounts/${encodeURIComponent(key)}/switch`);
export const refreshAccount = (key: string) => post<void>(`accounts/${encodeURIComponent(key)}/refresh`);
export const revokeAccount = (key: string) => post<void>(`accounts/${encodeURIComponent(key)}/revoke`);
export const addAccount = (_key: string, data: Partial<AccountData>) =>
  post<void>("accounts", data);
export const updateAccount = (key: string, updates: { priority?: number; autoSwitchDisabled?: boolean; displayName?: string }) =>
  put<void>(`accounts/${encodeURIComponent(key)}`, updates);
export const deleteAccount = (key: string) => del<void>(`accounts/${encodeURIComponent(key)}`);
export const captureBeforeSwitch = (outgoingKey: string) =>
  post<boolean>("accounts/capture-before-switch", { outgoingKey });

// --- Config --- GET /config, PUT /config
export const getConfig = () => get<AppConfig>("config");
export const setConfig = (config: Partial<AppConfig>) => put<void>("config", config);

// --- Proxy legacy --- GET /proxy/status, POST /proxy/start|stop|restart
type ProxyInstance = { id: string; kind: string; pid: number | null; port: number; running: boolean; uptimeSecs: number; requestsTotal: number; requestsActive: number; backend?: string };
export const getProxyStatus = () =>
  get<{ instances: ProxyInstance[]; instances_count: number }>("proxy/status").then((r) => {
    const byKind = (kind: string): ProxyStatus => {
      const inst = r.instances.find((i) => i.kind === kind);
      return {
        running: inst?.running ?? false,
        port: inst?.port ?? 0,
        pid: inst?.pid ?? undefined,
        uptimeSecs: inst?.uptimeSecs ?? 0,
        requestsTotal: inst?.requestsTotal ?? 0,
        requestsActive: inst?.requestsActive ?? 0,
        backend: inst?.backend,
      };
    };
    return { router: byKind("router"), impersonator: byKind("impersonator") };
  });
export const startProxy = (kind?: "router" | "impersonator") => post<void>("proxy/start", { kind });
export const stopProxy = (kind?: "router" | "impersonator") => post<void>("proxy/stop", { kind });
export const restartProxy = (kind?: "router" | "impersonator") => post<void>("proxy/restart", { kind });

// --- Proxy instances --- GET/POST /proxy-instances, PUT/DELETE /proxy-instances/:id, etc.
export const getProxyInstances = () => get<ProxyInstanceState[]>("proxy-instances");
export const addProxyInstance = (config: ProxyInstanceConfig) => post<void>("proxy-instances", config);
export const updateProxyInstance = (id: string, updates: Partial<ProxyInstanceConfig>) =>
  put<void>(`proxy-instances/${encodeURIComponent(id)}`, updates);
export const deleteProxyInstance = (id: string) => del<void>(`proxy-instances/${encodeURIComponent(id)}`);
export const startProxyInstance = (id: string) => post<void>(`proxy-instances/${encodeURIComponent(id)}/start`);
export const stopProxyInstance = (id: string) => post<void>(`proxy-instances/${encodeURIComponent(id)}/stop`);
export const restartProxyInstance = (id: string) => post<void>(`proxy-instances/${encodeURIComponent(id)}/restart`);
export const probeProxyInstances = () => post<ProxyInstanceState[]>("proxy-instances/probe");
export const detectProxyBinaries = () =>
  get<{ binaries: Array<{ id: string; name: string; path: string; defaultPort: number }> }>("proxy-binaries")
    .then((r) => r.binaries);

// --- Setup injection ---
export const setupClaudeCode = (port: number) => post<void>("setup/claude-code", { port });
export const removeClaudeCodeSetup = () => del<void>("setup/claude-code");
export const setupVscodeProxy = (port: number) => post<void>("setup/vscode", { port });
export const removeVscodeProxy = () => del<void>("setup/vscode");

// --- Systemd ---
export const getSystemdStatus = () =>
  get<{ status: string }>("systemd/status").then((r) => r.status);
export const installSystemdService = (daemonPath?: string) =>
  post<{ ok: boolean; message: string }>("systemd/install", { daemonPath }).then((r) => r.message);
export const uninstallSystemdService = () =>
  post<{ ok: boolean; message: string }>("systemd/uninstall").then((r) => r.message);

// --- Sync P2P ---
export const getSyncStatus = () => get<{ enabled: boolean; peers: number }>("sync/status");
export const generateSyncKey = () =>
  post<{ key: string }>("sync/key/generate").then((r) => r.key);
export const setSyncKey = (key: string) => post<void>("sync/key/set", { key });

// Helpers config-based (enable/disable sync, change port via PUT /config)
export const toggleSync = async (enabled: boolean) => {
  const cfg = await getConfig();
  await setConfig({ ...cfg, sync: { ...cfg.sync, enabled } });
};
export const setSyncPort = async (port: number) => {
  const cfg = await getConfig();
  await setConfig({ ...cfg, sync: { ...cfg.sync, port } });
};

// --- Peers ---
export const getPeers = () => get<Peer[]>("peers");
export const addPeer = (host: string, port: number, id?: string) => post<void>("peers", { host, port, id });
export const removePeer = (id: string) => del<void>(`peers/${encodeURIComponent(id)}`);
export const testPeerConnection = (host: string, port: number) =>
  post<{ reachable: boolean }>("peers/test", { host, port }).then((r) => r.reachable);

// --- SSH Sync ---
export const getHostname = () =>
  get<{ hostname: string }>("ssh/hostname").then((r) => r.hostname);
export const addSshHost = (host: string, port: number, username: string, identityPath?: string) =>
  post<void>("ssh-hosts", { host, port, username, identityPath });
export const removeSshHost = (id: string) => del<void>(`ssh-hosts/${encodeURIComponent(id)}`);
export const testSshConnection = (host: string, port: number, username: string, identityPath?: string) =>
  post<{ reachable: boolean }>("ssh-hosts/test", { host, port, username, identityPath }).then((r) => r.reachable);

// --- Monitoring ---
export const getQuotaHistory = (key: string, period?: "24h" | "7d" | "30d") =>
  get<QuotaHistoryPoint[]>("monitoring/quota-history", { key, period });
export const getSwitchHistory = () => get<SwitchEntry[]>("monitoring/switch-history");
export const getImpersonationProfiles = () => get<ImpersonationProfile[]>("monitoring/profiles");
export const getSessions = () => get<unknown[]>("monitoring/sessions");
export const getLogs = (filter?: string) => get<unknown[]>("monitoring/logs", filter ? { filter } : undefined);

// --- Credentials ---
export const scanLocalCredentials = () => post<ScannedCredential[]>("credentials/scan");
export const importScannedCredentials = (credentials: ScannedCredential[]) =>
  post<number>("credentials/import", { credentials });
export const findClaudeBinary = () => get<string>("credentials/binary");
export const captureOAuthToken = (timeoutSecs?: number) =>
  post<CaptureResult>("credentials/capture", { timeoutSecs });

// --- Profils ---
export const getProfiles = () => get<Array<{ name: string; createdAt: string; sizeBytes: number }>>("profiles");
export const saveProfile = (name: string, config: unknown) => post<void>("profiles", { name, config });
export const loadProfile = (name: string) => get<unknown>(`profiles/${encodeURIComponent(name)}`);
export const deleteProfile = (name: string) => del<void>(`profiles/${encodeURIComponent(name)}`);

// --- Stats ---
export const getStats = () => get<unknown>("stats");

// --- Health ---
export const getHealth = () => get<{ status: string }>("health");

// --- Event listeners (WebSocket) ---
// Returns an unsubscribe function (same contract as Tauri's listen())

export const onQuotaUpdate = (cb: (data: { key: string; quota: QuotaInfo }) => void): Promise<() => void> =>
  Promise.resolve(subscribeEvent("quota_update", (payload) => cb(payload as { key: string; quota: QuotaInfo })));

export const onToast = (cb: (toast: { type: string; title: string; message?: string }) => void): Promise<() => void> =>
  Promise.resolve(subscribeEvent("toast", (payload) => cb(payload as { type: string; title: string; message?: string })));

export const onProxyStatus = (cb: (status: ProxyStatus) => void): Promise<() => void> =>
  Promise.resolve(subscribeEvent("proxy_status", (payload) => cb(payload as ProxyStatus)));

export const onAccountSwitch = (cb: (key: string) => void): Promise<() => void> =>
  Promise.resolve(subscribeEvent("account_switch", (payload) => cb(payload as string)));
