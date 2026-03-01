//! Types de base partagés dans tout le crate core.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Phase de quota
// ---------------------------------------------------------------------------

/// Phase de quota d'un compte (basée sur TTT — time to threshold).
///
/// Thresholds (depuis VelocityController Python) :
/// - Cruise  : TTT > 20 min
/// - Watch   : TTT 5–20 min
/// - Alert   : TTT 2–5 min
/// - Critical: TTT < 2 min ou quota >= threshold
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QuotaPhase {
    #[default]
    Cruise,
    Watch,
    Alert,
    Critical,
}

impl QuotaPhase {
    /// Priorité numérique (plus élevé = plus urgent).
    pub fn priority(&self) -> u8 {
        match self {
            QuotaPhase::Cruise => 0,
            QuotaPhase::Watch => 1,
            QuotaPhase::Alert => 2,
            QuotaPhase::Critical => 3,
        }
    }

    /// Intervalle de refresh recommandé en secondes.
    pub fn refresh_interval_secs(&self) -> u64 {
        match self {
            QuotaPhase::Cruise => 120,
            QuotaPhase::Watch => 60,
            QuotaPhase::Alert => 30,
            QuotaPhase::Critical => 3,
        }
    }

    /// Retourne le nom lisible de la phase.
    pub fn as_str(&self) -> &'static str {
        match self {
            QuotaPhase::Cruise => "cruise",
            QuotaPhase::Watch => "watch",
            QuotaPhase::Alert => "alert",
            QuotaPhase::Critical => "critical",
        }
    }
}

impl std::fmt::Display for QuotaPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Info quota
// ---------------------------------------------------------------------------

/// Informations de quota pour un compte Claude.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuotaInfo {
    /// Tokens consommés sur la fenêtre 5 heures.
    pub tokens_5h: u64,
    /// Limite de tokens sur la fenêtre 5 heures.
    pub limit_5h: u64,
    /// Tokens consommés sur la fenêtre 7 jours.
    pub tokens_7d: u64,
    /// Limite de tokens sur la fenêtre 7 jours.
    pub limit_7d: u64,
    /// Phase actuelle (déterminée par TTT et EMA).
    pub phase: Option<QuotaPhase>,
    /// Vélocité EMA des tokens (tokens/min).
    pub ema_velocity: f64,
    /// Temps estimé avant d'atteindre le seuil (minutes). None si ema=0.
    pub time_to_threshold: Option<f64>,
    /// Horodatage de la dernière mise à jour.
    pub last_updated: Option<DateTime<Utc>>,
}

impl QuotaInfo {
    /// Pourcentage d'utilisation sur la fenêtre 5h (0.0–100.0).
    pub fn usage_pct_5h(&self) -> f64 {
        if self.limit_5h == 0 {
            return 0.0;
        }
        (self.tokens_5h as f64 / self.limit_5h as f64 * 100.0).min(100.0)
    }

    /// Pourcentage d'utilisation sur la fenêtre 7d (0.0–100.0).
    pub fn usage_pct_7d(&self) -> f64 {
        if self.limit_7d == 0 {
            return 0.0;
        }
        (self.tokens_7d as f64 / self.limit_7d as f64 * 100.0).min(100.0)
    }

    /// Quota restant (0.0–1.0) — facteur le plus contraignant entre 5h et 7d.
    pub fn remaining_factor(&self) -> f64 {
        let r5 = if self.limit_5h > 0 {
            1.0 - (self.tokens_5h as f64 / self.limit_5h as f64)
        } else {
            1.0
        };
        let r7 = if self.limit_7d > 0 {
            1.0 - (self.tokens_7d as f64 / self.limit_7d as f64)
        } else {
            1.0
        };
        r5.min(r7).max(0.0)
    }
}

// ---------------------------------------------------------------------------
// Statut proxy
// ---------------------------------------------------------------------------

/// Statut d'un proxy (router ou impersonator).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStatus {
    /// Le proxy est-il en cours d'exécution ?
    pub running: bool,
    /// Port d'écoute.
    pub port: u16,
    /// PID du processus (None si non démarré).
    pub pid: Option<u32>,
    /// Durée de vie en secondes.
    pub uptime_secs: u64,
    /// Nombre total de requêtes traitées.
    pub requests_total: u64,
    /// Nombre de requêtes en cours.
    pub requests_active: u32,
    /// Backend identifié par le health check (ex: "rust-auto", "python").
    /// None si le proxy a été démarré localement sans probe, ou si le probe n'a
    /// pas encore été effectué.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
}

impl Default for ProxyStatus {
    fn default() -> Self {
        Self {
            running: false,
            port: 0,
            pid: None,
            uptime_secs: 0,
            requests_total: 0,
            requests_active: 0,
            backend: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Proxy instance configuration
// ---------------------------------------------------------------------------

/// Type de proxy instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    Router,
    Impersonator,
    Custom,
}

impl Default for ProxyKind {
    fn default() -> Self {
        ProxyKind::Custom
    }
}

/// Configuration persistée d'une instance proxy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyInstanceConfig {
    /// Unique ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Kind: router, impersonator, or custom.
    pub kind: ProxyKind,
    /// Listening port.
    pub port: u16,
    /// Whether auto-start is enabled.
    pub auto_start: bool,
    /// Whether this instance is enabled.
    pub enabled: bool,
    /// Path to external proxy binary (None = built-in V3 proxy).
    #[serde(default)]
    pub binary_path: Option<String>,
    /// Setup injection targets (e.g., ["claude-code", "vscode", "gemini-cli"]).
    #[serde(default)]
    pub setup_targets: Vec<String>,
    /// Proxy owner: "auto" (premier arrivé), hostname de cette instance, ou hostname d'un pair.
    #[serde(default = "default_proxy_owner")]
    pub proxy_owner: String,
}

fn default_proxy_owner() -> String {
    "auto".to_string()
}

/// Runtime state of a proxy instance (config + live status).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyInstanceState {
    pub config: ProxyInstanceConfig,
    pub status: ProxyStatus,
}

// ---------------------------------------------------------------------------
// Peer P2P
// ---------------------------------------------------------------------------

/// Un pair de synchronisation P2P.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    /// Identifiant unique du pair.
    pub id: String,
    /// Adresse hôte (IP ou hostname).
    pub host: String,
    /// Port de synchronisation.
    pub port: u16,
    /// Le pair est-il actuellement connecté ?
    pub connected: bool,
    /// Horodatage de la dernière communication.
    pub last_seen: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// Providers supportés par le système.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    Gemini,
    OpenAI,
    XAI,
    DeepSeek,
    Mistral,
    Groq,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::Gemini => "gemini",
            Provider::OpenAI => "openai",
            Provider::XAI => "xai",
            Provider::DeepSeek => "deepseek",
            Provider::Mistral => "mistral",
            Provider::Groq => "groq",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "anthropic" => Some(Provider::Anthropic),
            "gemini" => Some(Provider::Gemini),
            "openai" => Some(Provider::OpenAI),
            "xai" => Some(Provider::XAI),
            "deepseek" => Some(Provider::DeepSeek),
            "mistral" => Some(Provider::Mistral),
            "groq" => Some(Provider::Groq),
            _ => None,
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quota_phase_priority() {
        assert!(QuotaPhase::Critical.priority() > QuotaPhase::Alert.priority());
        assert!(QuotaPhase::Alert.priority() > QuotaPhase::Watch.priority());
        assert!(QuotaPhase::Watch.priority() > QuotaPhase::Cruise.priority());
    }

    #[test]
    fn test_quota_phase_display() {
        assert_eq!(QuotaPhase::Cruise.to_string(), "cruise");
        assert_eq!(QuotaPhase::Watch.to_string(), "watch");
        assert_eq!(QuotaPhase::Alert.to_string(), "alert");
        assert_eq!(QuotaPhase::Critical.to_string(), "critical");
    }

    #[test]
    fn test_quota_info_usage_pct() {
        let q = QuotaInfo {
            tokens_5h: 7000,
            limit_5h: 10000,
            tokens_7d: 5000,
            limit_7d: 100000,
            ..Default::default()
        };
        assert!((q.usage_pct_5h() - 70.0).abs() < 0.01);
        assert!((q.usage_pct_7d() - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_quota_info_remaining_factor() {
        let q = QuotaInfo {
            tokens_5h: 9500,
            limit_5h: 10000,
            tokens_7d: 1000,
            limit_7d: 10000,
            ..Default::default()
        };
        // La fenêtre 5h est la plus contraignante (95% utilisé → 5% restant = 0.05)
        let r = q.remaining_factor();
        assert!((r - 0.05).abs() < 0.01, "remaining_factor = {r}");
    }

    #[test]
    fn test_quota_info_no_limit() {
        let q = QuotaInfo::default();
        assert_eq!(q.usage_pct_5h(), 0.0);
        assert_eq!(q.usage_pct_7d(), 0.0);
        assert_eq!(q.remaining_factor(), 1.0);
    }

    #[test]
    fn test_provider_roundtrip() {
        assert_eq!(Provider::from_str("anthropic"), Some(Provider::Anthropic));
        assert_eq!(Provider::from_str("gemini"), Some(Provider::Gemini));
        assert_eq!(Provider::from_str("openai"), Some(Provider::OpenAI));
        assert_eq!(Provider::from_str("xai"), Some(Provider::XAI));
        assert_eq!(Provider::from_str("deepseek"), Some(Provider::DeepSeek));
        assert_eq!(Provider::from_str("mistral"), Some(Provider::Mistral));
        assert_eq!(Provider::from_str("groq"), Some(Provider::Groq));
        assert_eq!(Provider::from_str("unknown"), None);
    }

    #[test]
    fn test_quota_phase_refresh_interval() {
        assert_eq!(QuotaPhase::Cruise.refresh_interval_secs(), 120);
        assert_eq!(QuotaPhase::Watch.refresh_interval_secs(), 60);
        assert_eq!(QuotaPhase::Alert.refresh_interval_secs(), 30);
        assert_eq!(QuotaPhase::Critical.refresh_interval_secs(), 3);
    }
}
