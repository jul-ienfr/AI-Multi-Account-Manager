//! DTOs JSON pour l'API HTTP du daemon.
//!
//! Miroir des structures `commands.rs` du crate Tauri, sans dépendance circulaire.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Account DTOs
// ---------------------------------------------------------------------------

/// État complet d'un compte (config + quota live).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStateDto {
    pub key: String,
    pub data: AccountDataDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<QuotaDto>,
    pub is_active: bool,
}

/// Données d'un compte (version allégée pour l'API).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountDataDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_ai_oauth: Option<OAuthSlotDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_token: Option<OAuthSlotDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gemini_cli_oauth: Option<OAuthSlotDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default)]
    pub auto_switch_disabled: bool,
    pub tokens_5h: u64,
    pub tokens_7d: u64,
    pub deleted: bool,
}

/// Slot OAuth (access_token tronqué, refresh_token masqué).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthSlotDto {
    /// Premiers 16 chars seulement.
    pub access_token: String,
    /// Toujours "***" pour ne pas exposer le secret.
    pub refresh_token: String,
    /// Expiration en millisecondes Unix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

/// Métriques de quota pour un compte.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuotaDto {
    pub tokens_5h: u64,
    pub limit_5h: u64,
    pub tokens_7d: u64,
    pub limit_7d: u64,
    pub phase: String,
    pub ema_velocity: f64,
    pub time_to_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at_5h: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at_7d: Option<String>,
}

/// Payload d'ajout d'un compte.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAccountData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Token OAuth (access_token pour comptes Claude)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Clé API (pour comptes API key)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default)]
    pub priority: u32,
}

/// Payload de mise à jour d'un compte.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAccountData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_switch_disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

// ---------------------------------------------------------------------------
// Profile DTOs
// ---------------------------------------------------------------------------

/// Métadonnées d'un profil de configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfoDto {
    pub name: String,
    pub created_at: String,
    pub size_bytes: u64,
}

// ---------------------------------------------------------------------------
// Stats DTOs
// ---------------------------------------------------------------------------

/// Statistiques globales du daemon.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StatsDto {
    pub total_switches: u64,
    pub switches_by_account: std::collections::HashMap<String, u64>,
    pub total_requests: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_switch_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_started_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Binary detection DTOs
// ---------------------------------------------------------------------------

/// Binaire proxy détecté sur le système.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedBinary {
    pub id: String,
    pub name: String,
    pub path: String,
    pub default_port: u16,
}

// ---------------------------------------------------------------------------
// Auth / API responses
// ---------------------------------------------------------------------------

/// Réponse d'erreur JSON standard.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub message: String,
}

/// Réponse de capture OAuth.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResultDto {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Payload pour test de webhook.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestWebhookData {
    pub url: String,
    pub kind: String,
}

/// Payload pour set sync key.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetKeyData {
    pub key: String,
}

/// Payload pour add peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddPeerData {
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Payload pour test peer connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestPeerData {
    pub host: String,
    pub port: u16,
}

/// Payload pour add SSH host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSshHostData {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_path: Option<String>,
}

/// Payload pour test SSH connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestSshData {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_path: Option<String>,
}

/// Payload pour setup Claude Code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupPortData {
    pub port: u16,
}

/// Payload pour save profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProfileData {
    pub name: String,
    pub config: serde_json::Value,
}

/// Payload pour capture_before_switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureBeforeSwitchData {
    pub outgoing_key: String,
}

/// Payload pour capture OAuth token.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTokenData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

/// Payload pour install systemd service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallSystemdData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_path: Option<String>,
}

/// Payload pour import scanned credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCredentialsData {
    pub credentials: Vec<ai_core::credentials::ScannedCredential>,
}

/// Payload proxy start/stop (kind optionnel).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProxyKindData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// Résultat import credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: usize,
}
