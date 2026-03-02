export interface OAuthSlot {
  accessToken?: string;
  refreshToken?: string;
  expiresAt?: number;
}

export interface AccountData {
  email?: string;
  name?: string;
  displayName?: string;
  accountType?: string;
  provider?: string;
  priority?: number;
  planType?: string;
  claudeAiOauth?: OAuthSlot;
  setupToken?: OAuthSlot;
  geminiCliOauth?: OAuthSlot;
  apiKey?: { key: string } | string;
  apiUrl?: string;
  autoSwitchDisabled?: boolean;
}

export type QuotaPhase = "Cruise" | "Watch" | "Alert" | "Critical";

export interface QuotaInfo {
  tokens5h: number;
  limit5h: number;
  tokens7d: number;
  limit7d: number;
  phase?: QuotaPhase;
  emaVelocity: number;
  timeToThreshold?: number;
  lastUpdated?: string;
  resetsAt5h?: string;
  resetsAt7d?: string;
}

export interface AccountState {
  key: string;
  data: AccountData;
  quota?: QuotaInfo;
  isActive: boolean;
  /** true si token révoqué (invalid_grant) — exclu de la rotation */
  revoked?: boolean;
  /** true si au moins un token OAuth ou clé API est présent */
  hasToken?: boolean;
}

export interface ProxyStatus {
  running: boolean;
  port: number;
  pid?: number;
  uptimeSecs: number;
  requestsTotal: number;
  requestsActive: number;
  backend?: string;
}

export interface Peer {
  id: string;
  host: string;
  port: number;
  connected: boolean;
  lastSeen?: string;
}

export interface ProxyConfig {
  routerPort: number | null;
  impersonatorPort: number | null;
  strategy: RoutingStrategy;
  autoSwitchThreshold5h: number;
  autoSwitchThreshold7d: number;
  autoSwitchGraceSecs: number;
  rotationEnabled: boolean;
  rotationIntervalSecs: number;
  modelOverrides: Record<string, string>;
  instances: ProxyInstanceConfig[];
}

export type ProxyKind = "router" | "impersonator" | "custom";

export interface ProxyInstanceConfig {
  id: string;
  name: string;
  kind: ProxyKind;
  port: number;
  autoStart: boolean;
  enabled: boolean;
  binaryPath?: string;
  setupTargets: string[];
  proxyOwner: string;
}

export interface ProxyInstanceState {
  config: ProxyInstanceConfig;
  status: ProxyStatus;
}

export interface SyncConfig {
  enabled: boolean;
  port: number;
  sharedKeyHex: string | null;
  peers: PeerConfigEntry[];
  syncActiveAccount: boolean;
  syncQuota: boolean;
  splitQuotaFetch: boolean;
  proxyFailover: boolean;
  sshEnabled: boolean;
  sshHosts: SshHostConfig[];
}

export interface SshHostConfig {
  id: string;
  host: string;
  port: number;
  username: string;
  identityPath?: string;
  enabled: boolean;
}

export interface PeerConfigEntry {
  id: string;
  host: string;
  port: number;
}

export interface AlertsConfig {
  soundEnabled: boolean;
  toastsEnabled: boolean;
  quotaAlertThreshold: number;
  quotaCriticalThreshold: number;
}

export interface ScheduleConfig {
  enabled: boolean;
  startTime: string;
  endTime: string;
}

export interface AppConfig {
  refreshIntervalSecs: number;
  adaptiveRefresh: boolean;
  proxy: ProxyConfig;
  sync: SyncConfig;
  alerts: AlertsConfig;
  schedule: ScheduleConfig;
}

export type ToastType = "info" | "success" | "warning" | "error" | "switch";

export interface Toast {
  id: string;
  type: ToastType;
  title: string;
  message?: string;
  duration?: number;
}

export type RoutingStrategy = "priority" | "quota-aware" | "round-robin" | "latency" | "usage-based";

export interface ModelTier {
  opus: string;
  sonnet: string;
  haiku: string;
}

export type Provider = "anthropic" | "gemini" | "openai" | "xai" | "deepseek" | "mistral" | "groq";

export interface SessionInfo {
  id: string;
  accountKey: string;
  startTime: string;
  requestCount: number;
  tokensUsed: number;
}

export interface LogEntry {
  timestamp: string;
  level: "info" | "warn" | "error" | "debug";
  category: string;
  message: string;
}

export interface SwitchEntry {
  timestamp: string;
  from?: string;
  to: string;
  reason: string;
}

export interface ImpersonationProfileHeader {
  latest: string;
  pattern: string;
  samples: string[];
}

export interface ImpersonationProfile {
  provider_name: string;
  provider?: string;
  version?: number;
  request_count?: number;
  last_capture?: string;
  captured_at?: string;
  static_headers?: Record<string, string>;
  dynamic_headers?: Record<string, ImpersonationProfileHeader>;
  header_order?: string[];
  body_field_whitelist?: string[];
  always_streams?: boolean;
}

export interface QuotaHistoryPoint {
  timestamp: string;
  tokens: number;
}

export interface ScannedCredential {
  sourcePath: string;
  email?: string;
  name?: string;
  accessToken: string;
  refreshToken: string;
  expiresAtMs?: number;
  provider?: string;
}

/// Result from `capture_oauth_token` (mirrors Rust CaptureResult).
export interface CaptureResult {
  accessToken?: string;
  refreshToken?: string;
  email?: string;
  success: boolean;
  error?: string;
  /** Raw stdout + stderr from the claude CLI process. */
  output: string;
  /** Path to the claude binary that was used. */
  binaryPath?: string;
}
