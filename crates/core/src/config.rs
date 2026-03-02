//! Configuration de l'application — chargée depuis `settings.json`.
//!
//! Supporte le hot-reload via `notify` (utilisé par le daemon watchdog).
//! Thread-safe via `parking_lot::RwLock`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::Result;

/// Configuration globale de l'application.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    /// Intervalle de refresh OAuth en secondes.
    pub refresh_interval_secs: u64,
    /// Activer le refresh adaptatif (basé sur la phase quota).
    pub adaptive_refresh: bool,
    /// Configuration du proxy.
    pub proxy: ProxyConfig,
    /// Configuration de la synchronisation P2P.
    pub sync: SyncConfig,
    /// Configuration des alertes.
    pub alerts: AlertsConfig,
    /// Schedule d'activité (plage horaire).
    pub schedule: ScheduleConfig,
    /// Cibles webhook Discord/Slack/Generic.
    #[serde(default)]
    pub webhooks: Vec<crate::webhook::WebhookTarget>,
    /// Google OAuth credentials pour le flux Gemini (optionnel).
    #[serde(default)]
    pub gemini_oauth: GeminiOAuthConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            refresh_interval_secs: 60,
            adaptive_refresh: true,
            proxy: ProxyConfig::default(),
            sync: SyncConfig::default(),
            alerts: AlertsConfig::default(),
            schedule: ScheduleConfig::default(),
            webhooks: Vec::new(),
            gemini_oauth: GeminiOAuthConfig::default(),
        }
    }
}

/// Credentials Google OAuth pour le flux Gemini CLI.
/// Configurable via les Paramètres de l'application.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GeminiOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
}

/// Configuration du proxy (router + anthrouter + custom instances).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxyConfig {
    /// Port du routeur (legacy — use instances instead).
    pub router_port: Option<u16>,
    /// Port de l'impersonateur (legacy — use instances instead).
    pub impersonator_port: Option<u16>,
    /// Stratégie de routing.
    pub strategy: RoutingStrategy,
    /// Seuil 5h pour l'auto-switch (fraction, ex: 0.85).
    pub auto_switch_threshold_5h: f64,
    /// Seuil 7d pour l'auto-switch.
    pub auto_switch_threshold_7d: f64,
    /// Période de grâce avant switch (secondes).
    pub auto_switch_grace_secs: u64,
    /// Activer la rotation automatique.
    pub rotation_enabled: bool,
    /// Intervalle de rotation (secondes).
    pub rotation_interval_secs: u64,
    /// Overrides de modèles {client_model: anthropic_model}.
    pub model_overrides: HashMap<String, String>,
    /// Dynamic proxy instances.
    pub instances: Vec<crate::types::ProxyInstanceConfig>,
}

impl ProxyConfig {
    pub fn router_port(&self) -> u16 {
        self.router_port.unwrap_or(8080)
    }
    pub fn impersonator_port(&self) -> u16 {
        self.impersonator_port.unwrap_or(8081)
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        use crate::types::{ProxyInstanceConfig, ProxyKind};
        Self {
            router_port: Some(8080),
            impersonator_port: Some(8081),
            strategy: RoutingStrategy::default(),
            auto_switch_threshold_5h: 0.85,
            auto_switch_threshold_7d: 0.90,
            auto_switch_grace_secs: 30,
            rotation_enabled: false,
            rotation_interval_secs: 3600,
            model_overrides: HashMap::new(),
            instances: vec![
                ProxyInstanceConfig {
                    id: "router".to_string(),
                    name: "Router".to_string(),
                    kind: ProxyKind::Router,
                    port: 8080,
                    auto_start: false,
                    enabled: true,
                    binary_path: None,
                    setup_targets: vec!["claude-code".to_string()],
                    proxy_owner: "auto".to_string(),
                },
                ProxyInstanceConfig {
                    id: "impersonator".to_string(),
                    name: "Anthrouter".to_string(),
                    kind: ProxyKind::Impersonator,
                    port: 8081,
                    auto_start: false,
                    enabled: true,
                    binary_path: None,
                    setup_targets: vec![],
                    proxy_owner: "auto".to_string(),
                },
            ],
        }
    }
}

/// Stratégie de routing entre comptes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStrategy {
    /// Compte prioritaire fixe.
    #[default]
    Priority,
    /// Basé sur le quota restant.
    QuotaAware,
    /// Round-robin entre comptes.
    RoundRobin,
    /// Basé sur la latence mesurée.
    Latency,
    /// Basé sur l'utilisation historique.
    Usage,
}

impl RoutingStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            RoutingStrategy::Priority => "priority",
            RoutingStrategy::QuotaAware => "quota_aware",
            RoutingStrategy::RoundRobin => "round_robin",
            RoutingStrategy::Latency => "latency",
            RoutingStrategy::Usage => "usage",
        }
    }
}

impl std::fmt::Display for RoutingStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Configuration de la synchronisation P2P.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncConfig {
    /// Activer la sync P2P.
    pub enabled: bool,
    /// Port TCP de la sync.
    pub port: u16,
    /// Clé partagée hexadécimale (32 bytes = 64 hex chars).
    pub shared_key_hex: Option<String>,
    /// Pairs configurés manuellement [{id, host, port}].
    pub peers: Vec<PeerConfig>,
    /// Synchroniser le compte actif entre instances.
    pub sync_active_account: bool,
    /// Synchroniser les mises à jour de quota.
    pub sync_quota: bool,
    /// Répartir les fetches de quota entre pairs.
    pub split_quota_fetch: bool,
    /// Failover proxy automatique si le proxy local est down.
    pub proxy_failover: bool,
    /// Activer la synchronisation SSH.
    pub ssh_enabled: bool,
    /// Hôtes SSH distants pour push credentials.
    pub ssh_hosts: Vec<SshHostConfig>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 9090,
            shared_key_hex: None,
            peers: Vec::new(),
            sync_active_account: true,
            sync_quota: true,
            split_quota_fetch: true,
            proxy_failover: true,
            ssh_enabled: false,
            ssh_hosts: Vec::new(),
        }
    }
}

/// Configuration d'un hôte SSH distant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostConfig {
    /// Identifiant unique (ex: "user@host").
    pub id: String,
    /// Nom d'hôte ou IP.
    pub host: String,
    /// Port SSH (défaut: 22).
    pub port: u16,
    /// Nom d'utilisateur SSH.
    pub username: String,
    /// Chemin vers la clé privée (optionnel).
    pub identity_path: Option<String>,
    /// Activer cet hôte.
    pub enabled: bool,
}

/// Configuration d'un pair P2P.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerConfig {
    pub id: String,
    pub host: String,
    pub port: u16,
}

/// Configuration des alertes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlertsConfig {
    /// Activer les sons d'alerte.
    pub sound_enabled: bool,
    /// Activer les toasts.
    pub toasts_enabled: bool,
    /// Seuil d'alerte quota 5h (fraction, ex: 0.80).
    pub quota_alert_threshold: f64,
    /// Seuil critique quota 5h.
    pub quota_critical_threshold: f64,
}

impl Default for AlertsConfig {
    fn default() -> Self {
        Self {
            sound_enabled: false,
            toasts_enabled: true,
            quota_alert_threshold: 0.80,
            quota_critical_threshold: 0.95,
        }
    }
}

/// Schedule d'activité (plage horaire).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScheduleConfig {
    /// Activer le schedule.
    pub enabled: bool,
    /// Heure de début (format HH:MM).
    pub start_time: String,
    /// Heure de fin (format HH:MM).
    pub end_time: String,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_time: "09:00".to_string(),
            end_time: "18:00".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// ConfigCache
// ---------------------------------------------------------------------------

/// Cache thread-safe de la configuration.
pub struct ConfigCache {
    inner: RwLock<AppConfig>,
    path: PathBuf,
}

impl ConfigCache {
    /// Charge la configuration depuis un fichier settings.json.
    pub fn load(path: impl AsRef<Path>) -> Result<Arc<Self>> {
        let path = path.as_ref().to_path_buf();
        let config = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            serde_json::from_str::<AppConfig>(&raw).unwrap_or_else(|e| {
                warn!("Failed to parse config at {:?}: {} — using defaults", path, e);
                AppConfig::default()
            })
        } else {
            debug!("Config file not found at {:?}, using defaults", path);
            AppConfig::default()
        };

        info!(
            "ConfigCache loaded: strategy={}, refresh={}s",
            config.proxy.strategy, config.refresh_interval_secs
        );

        Ok(Arc::new(Self {
            inner: RwLock::new(config),
            path,
        }))
    }

    /// Crée un cache avec la configuration par défaut.
    pub fn default_config() -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(AppConfig::default()),
            path: PathBuf::from("/tmp/settings.json"),
        })
    }

    /// Recharge depuis le disque.
    pub fn reload(&self) -> Result<()> {
        if !self.path.exists() {
            debug!("reload: config file not found, keeping defaults");
            return Ok(());
        }
        let raw = std::fs::read_to_string(&self.path)?;
        let new_config = serde_json::from_str::<AppConfig>(&raw).unwrap_or_else(|e| {
            warn!("Config reload parse error: {} — keeping current config", e);
            self.inner.read().clone()
        });
        *self.inner.write() = new_config;
        info!("ConfigCache reloaded");
        Ok(())
    }

    /// Accès en lecture.
    pub fn read(&self) -> parking_lot::RwLockReadGuard<'_, AppConfig> {
        self.inner.read()
    }

    /// Accès en écriture.
    pub fn write(&self) -> parking_lot::RwLockWriteGuard<'_, AppConfig> {
        self.inner.write()
    }

    /// Persiste la configuration sur disque.
    pub fn persist(&self) -> Result<()> {
        let config = self.inner.read().clone();
        let json = serde_json::to_string_pretty(&config)?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, &self.path)?;
        debug!("ConfigCache persisted to {:?}", self.path);
        Ok(())
    }

    /// Retourne la stratégie de routing.
    pub fn routing_strategy(&self) -> RoutingStrategy {
        self.inner.read().proxy.strategy
    }

    /// Retourne l'intervalle de refresh.
    pub fn refresh_interval(&self) -> Duration {
        Duration::from_secs(self.inner.read().refresh_interval_secs)
    }

    /// Retourne le chemin du fichier settings.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.refresh_interval_secs, 60);
        assert!(config.adaptive_refresh);
        assert_eq!(config.proxy.router_port(), 8080);
        assert_eq!(config.proxy.impersonator_port(), 8081);
        assert_eq!(config.proxy.strategy, RoutingStrategy::Priority);
        assert!(!config.sync.enabled);
        assert_eq!(config.sync.port, 9090);
    }

    #[test]
    fn test_load_defaults_when_missing() {
        let cache = ConfigCache::load("/nonexistent/settings.json").unwrap();
        let config = cache.read();
        assert_eq!(config.refresh_interval_secs, 60);
    }

    #[test]
    fn test_load_from_file() {
        let file = NamedTempFile::new().unwrap();
        let json = r#"{
            "refreshIntervalSecs": 120,
            "adaptiveRefresh": false,
            "proxy": {
                "routerPort": 9000,
                "strategy": "round_robin"
            }
        }"#;
        std::fs::write(file.path(), json).unwrap();
        let cache = ConfigCache::load(file.path()).unwrap();
        let config = cache.read();
        assert_eq!(config.refresh_interval_secs, 120);
        assert!(!config.adaptive_refresh);
        assert_eq!(config.proxy.router_port(), 9000);
        assert_eq!(config.proxy.strategy, RoutingStrategy::RoundRobin);
    }

    #[test]
    fn test_reload() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), r#"{"refreshIntervalSecs": 30}"#).unwrap();
        let cache = ConfigCache::load(file.path()).unwrap();
        assert_eq!(cache.read().refresh_interval_secs, 30);

        std::fs::write(file.path(), r#"{"refreshIntervalSecs": 90}"#).unwrap();
        cache.reload().unwrap();
        assert_eq!(cache.read().refresh_interval_secs, 90);
    }

    #[test]
    fn test_persist() {
        let file = NamedTempFile::new().unwrap();
        let cache = ConfigCache::load(file.path()).unwrap();
        {
            let mut cfg = cache.write();
            cfg.refresh_interval_secs = 45;
        }
        cache.persist().unwrap();

        let cache2 = ConfigCache::load(file.path()).unwrap();
        assert_eq!(cache2.read().refresh_interval_secs, 45);
    }

    #[test]
    fn test_routing_strategy_display() {
        assert_eq!(RoutingStrategy::Priority.to_string(), "priority");
        assert_eq!(RoutingStrategy::QuotaAware.to_string(), "quota_aware");
        assert_eq!(RoutingStrategy::RoundRobin.to_string(), "round_robin");
        assert_eq!(RoutingStrategy::Latency.to_string(), "latency");
        assert_eq!(RoutingStrategy::Usage.to_string(), "usage");
    }

    #[test]
    fn test_refresh_interval_duration() {
        let cache = ConfigCache::default_config();
        assert_eq!(cache.refresh_interval(), Duration::from_secs(60));
    }

    #[test]
    fn test_malformed_json_uses_defaults() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "not valid json !!!").unwrap();
        let cache = ConfigCache::load(file.path()).unwrap();
        // Uses defaults when JSON is invalid
        assert_eq!(cache.read().refresh_interval_secs, 60);
    }
}
