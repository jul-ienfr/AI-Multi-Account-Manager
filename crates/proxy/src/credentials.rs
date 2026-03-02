//! Gestion des credentials multi-compte.
//!
//! Lit le fichier credentials-multi.json au format réel du AI Manager :
//! - Comptes OAuth : vscodeOauth / claudeAiOauth / setupToken → accessToken
//! - Comptes API : apiKey.key
//! - activeAccount : clé du compte actif

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Types JSON — correspondant au vrai format du fichier
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthSlot {
    #[serde(rename = "accessToken")]
    pub access_token: Option<String>,
    #[serde(rename = "refreshToken")]
    pub refresh_token: Option<String>,
    #[serde(rename = "expiresAt")]
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[allow(dead_code)]
pub struct ApiKeyData {
    pub key: Option<String>,
}

/// Données brutes d'un compte telles que dans le JSON.
/// Tous les champs sont optionnels car le format varie (OAuth vs API).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountData {
    #[serde(default)]
    pub email: Option<String>,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub account_type: Option<String>,   // "api" pour les comptes API, absent pour OAuth
    pub provider: Option<String>,       // "anthropic" | "gemini" | "openai" | ...
    pub priority: Option<u32>,
    pub plan_type: Option<String>,

    // OAuth slots (comptes Anthropic)
    pub vscode_oauth: Option<OAuthSlot>,
    pub claude_ai_oauth: Option<OAuthSlot>,
    pub setup_token: Option<OAuthSlot>,

    // Google OAuth slots (comptes Gemini) — ordre de priorité
    pub gemini_cli_oauth: Option<OAuthSlot>,
    pub gemini_code_assist_oauth: Option<OAuthSlot>,
    pub gcloud_adc_oauth: Option<OAuthSlot>,
    pub gcloud_legacy_oauth: Option<OAuthSlot>,

    // API key (comptes API tiers)
    pub api_key: Option<serde_json::Value>,  // peut être {key: "..."} ou une string
    pub api_url: Option<String>,
    pub auth_header: Option<String>,
    pub auth_type: Option<String>,
    pub api_format: Option<String>,          // "anthropic" | "openai"
    pub model_override: Option<String>,
    pub model_mappings: Option<HashMap<String, String>>,

    pub auto_switch_disabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsFile {
    #[serde(default)]
    pub accounts: HashMap<String, AccountData>,
    pub active_account: Option<String>,
    pub last_updated: Option<String>,
    pub version: Option<String>,
}

// ---------------------------------------------------------------------------
// Résultat simplifié pour le proxy
// ---------------------------------------------------------------------------

/// Infos extraites du compte actif, prêtes à utiliser par le proxy.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct ActiveAccount {
    pub email: String,
    pub token: String,
    pub account_type: String,     // "oauth" | "api"
    pub provider: String,         // "anthropic" par défaut
    pub api_url: Option<String>,
    pub auth_header: Option<String>,
    pub api_format: String,       // "anthropic" | "openai"
    pub model_override: Option<String>,
    pub model_mappings: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

pub struct CredentialsCache {
    path: PathBuf,
    data: RwLock<CredentialsFile>,
    #[allow(dead_code)]
    last_reload_ms: AtomicU64,
}

impl CredentialsCache {
    pub fn load(path: &Path) -> Arc<Self> {
        let data = Self::read_file(path).unwrap_or_default();
        Arc::new(Self {
            path: path.to_path_buf(),
            data: RwLock::new(data),
            last_reload_ms: AtomicU64::new(0),
        })
    }

    fn read_file(path: &Path) -> Option<CredentialsFile> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| warn!("Cannot read credentials: {e}"))
            .ok()?;
        serde_json::from_str(&content)
            .map_err(|e| warn!("Cannot parse credentials: {e}"))
            .ok()
    }

    /// Recharge le fichier depuis le disque.
    /// Si le parsing échoue (fichier corrompu / en cours d'écriture), le cache
    /// existant est conservé et un avertissement est émis — Fix H3.
    pub fn reload(&self) {
        match Self::read_file(&self.path) {
            Some(new_data) => {
                let n = new_data.accounts.len();
                *self.data.write().unwrap() = new_data;
                tracing::debug!("Credentials reloaded: {} account(s)", n);
            }
            None => {
                tracing::warn!(
                    "Credentials reload failed (parse error or missing file): \
                     keeping existing cache"
                );
                // Ne pas écraser le cache existant.
            }
        }
    }

    /// Reload seulement si le dernier reload date de plus de `min_interval_ms`.
    /// Utilise un CAS atomique pour éviter le thundering herd.
    /// Retourne `true` si un reload a effectivement eu lieu.
    #[allow(dead_code)]
    pub fn reload_if_stale(&self, min_interval_ms: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let last = self.last_reload_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) < min_interval_ms {
            return false;
        }
        // CAS : seul le premier thread qui gagne fait le reload
        if self
            .last_reload_ms
            .compare_exchange(last, now, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        self.reload();
        true
    }

    /// Retourne les infos du compte actif, prêtes pour le proxy.
    ///
    /// Toute la lecture (clé active + lookup dans accounts) est effectuée sous
    /// le MÊME read-guard afin d'éviter la race condition TOCTOU — Fix H2.
    ///
    /// Si le compte actif est absent de la map (supprimé lors d'un reload), un
    /// fallback retourne le premier compte valide (non supprimé, avec OAuth).
    pub fn get_active(&self) -> Option<ActiveAccount> {
        // Une seule acquisition du verrou pour toute la fonction.
        let guard = self.data.read().unwrap();

        // Chercher le compte désigné par active_account, tout sous le même guard.
        let (key, account) = guard
            .active_account
            .as_ref()
            .and_then(|active_key| {
                guard
                    .accounts
                    .get(active_key)
                    .map(|a| (active_key.clone(), a))
            })
            // Fallback H2 : si le compte actif a disparu, prendre le premier
            // compte valide (avec un token OAuth ou une clé API exploitable).
            .or_else(|| {
                guard.accounts.iter().find_map(|(k, a)| {
                    let has_oauth = a.claude_ai_oauth.as_ref()
                        .and_then(|s| s.access_token.as_deref())
                        .map(|t| !t.is_empty())
                        .unwrap_or(false)
                        || a.setup_token.as_ref()
                            .and_then(|s| s.access_token.as_deref())
                            .map(|t| !t.is_empty())
                            .unwrap_or(false)
                        || a.gemini_cli_oauth.as_ref()
                            .and_then(|s| s.access_token.as_deref())
                            .map(|t| !t.is_empty())
                            .unwrap_or(false)
                        || extract_api_key(a).is_some();
                    if has_oauth {
                        Some((k.clone(), a))
                    } else {
                        None
                    }
                })
            })?;

        let email = account.email.clone().unwrap_or(key);
        let provider = account.provider.clone().unwrap_or_else(|| "anthropic".to_string());

        // Compte API ?
        if account.account_type.as_deref() == Some("api") {
            let api_key = extract_api_key(account)?;
            let api_format = account.api_format.clone().unwrap_or_else(|| "anthropic".to_string());
            return Some(ActiveAccount {
                email,
                token: api_key,
                account_type: "api".to_string(),
                provider,
                api_url: account.api_url.clone(),
                auth_header: account.auth_header.clone(),
                api_format,
                model_override: account.model_override.clone(),
                model_mappings: account.model_mappings.clone().unwrap_or_default(),
            });
        }

        // Compte OAuth — router selon le provider
        let (token, api_url, api_format) = if provider == "gemini" {
            let t = extract_google_oauth_token(account)?;
            (t, Some("https://generativelanguage.googleapis.com".to_string()), "openai".to_string())
        } else {
            let t = extract_oauth_token(account)?;
            (t, None, "anthropic".to_string())
        };

        Some(ActiveAccount {
            email,
            token,
            account_type: "oauth".to_string(),
            provider,
            api_url,
            auth_header: None,
            api_format,
            model_override: None,
            model_mappings: HashMap::new(),
        })
    }
}

/// Extrait la clé API depuis le champ `apiKey` (objet {key: "..."} ou string).
fn extract_api_key(account: &AccountData) -> Option<String> {
    match &account.api_key {
        Some(serde_json::Value::Object(obj)) => {
            obj.get("key").and_then(|v| v.as_str()).map(|s| s.to_string())
        }
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Extrait le token OAuth Google depuis les slots Gemini (ordre de priorité).
fn extract_google_oauth_token(account: &AccountData) -> Option<String> {
    for slot in [
        &account.gemini_cli_oauth,
        &account.gemini_code_assist_oauth,
        &account.gcloud_adc_oauth,
        &account.gcloud_legacy_oauth,
    ]
    .into_iter()
    .flatten()
    {
        if let Some(ref token) = slot.access_token {
            if !token.is_empty() {
                return Some(token.clone());
            }
        }
    }
    None
}

/// Extrait le token OAuth depuis claudeAiOauth (slot unique) ou setupToken (fallback).
fn extract_oauth_token(account: &AccountData) -> Option<String> {
    // Slot principal : claudeAiOauth (vscodeOauth supprimé — fusionné côté Python)
    for slot in [&account.claude_ai_oauth, &account.setup_token]
        .into_iter()
        .flatten()
    {
        if let Some(ref token) = slot.access_token {
            if !token.is_empty() {
                return Some(token.clone());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Recherche du répertoire multi-account
// ---------------------------------------------------------------------------

pub fn find_multi_account_dir() -> PathBuf {
    let candidates: Vec<PathBuf> = vec![
        // Windows : USERPROFILE
        std::env::var("USERPROFILE")
            .ok()
            .map(|h| PathBuf::from(h).join(".claude").join("multi-account")),
        // Linux/macOS : HOME
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".claude").join("multi-account")),
        // WSL → Windows home
        detect_wsl_home(),
    ]
    .into_iter()
    .flatten()
    .collect();

    for path in candidates {
        if path.exists() {
            info!("credentials dir: {}", path.display());
            return path;
        }
    }

    // Fallback Windows
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return PathBuf::from(profile).join(".claude").join("multi-account");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".claude").join("multi-account")
}

fn detect_wsl_home() -> Option<PathBuf> {
    let version = std::fs::read_to_string("/proc/version").ok()?;
    if !version.to_lowercase().contains("microsoft") {
        return None;
    }
    let mnt_c = PathBuf::from("/mnt/c/Users");
    if !mnt_c.exists() {
        return None;
    }
    let system_users = ["Public", "Default", "All Users", "Default User"];
    std::fs::read_dir(&mnt_c)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| !system_users.contains(&name.as_str()))
        .find_map(|user| {
            let p = mnt_c.join(&user).join(".claude").join("multi-account");
            if p.exists() { Some(p) } else { None }
        })
}
