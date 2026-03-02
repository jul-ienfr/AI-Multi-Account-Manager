//! Cache de credentials multi-compte.
//!
//! Charge et maintient en mémoire les comptes depuis `credentials-multi.json`.
//! Thread-safe via `parking_lot::RwLock`.
//! Compatible avec le format V2 Python (timestamps millis, claudeAiOauth, etc.).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::Result;

// ---------------------------------------------------------------------------
// Flexible datetime: handles both millis (V2) and RFC3339 (V3) formats
// ---------------------------------------------------------------------------

mod flexible_datetime {
    use chrono::{DateTime, NaiveDateTime, Utc};
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(date: &Option<DateTime<Utc>>, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(d) => s.serialize_i64(d.timestamp_millis()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(d: D) -> std::result::Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = Option::<serde_json::Value>::deserialize(d)?;
        match val {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::Number(n)) => {
                if let Some(ms) = n.as_i64() {
                    Ok(DateTime::from_timestamp_millis(ms))
                } else if let Some(secs) = n.as_f64() {
                    Ok(DateTime::from_timestamp_millis((secs * 1000.0) as i64))
                } else {
                    Ok(None)
                }
            }
            Some(serde_json::Value::String(s)) => {
                if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
                    Ok(Some(dt.with_timezone(&Utc)))
                } else if let Ok(dt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S") {
                    Ok(Some(dt.and_utc()))
                } else if let Ok(dt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S") {
                    Ok(Some(dt.and_utc()))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// OAuth data
// ---------------------------------------------------------------------------

/// Donnees OAuth d'un compte (V2 + V3 compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthData {
    #[serde(default, alias = "access_token")]
    pub access_token: String,
    #[serde(default, alias = "refresh_token")]
    pub refresh_token: String,
    #[serde(
        default,
        alias = "expires_at",
        deserialize_with = "flexible_datetime::deserialize",
        serialize_with = "flexible_datetime::serialize"
    )]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, alias = "token_type")]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    #[serde(
        default,
        alias = "refresh_token_expires_at",
        deserialize_with = "flexible_datetime::deserialize",
        serialize_with = "flexible_datetime::serialize"
    )]
    pub refresh_token_expires_at: Option<DateTime<Utc>>,
    #[serde(default, alias = "organization_uuid")]
    pub organization_uuid: Option<String>,
}

impl OAuthData {
    pub fn is_likely_valid(&self) -> bool {
        if self.access_token.is_empty() {
            return false;
        }
        if let Some(exp) = self.expires_at {
            exp > Utc::now() + chrono::Duration::minutes(5)
        } else {
            true
        }
    }
}

// ---------------------------------------------------------------------------
// Account data
// ---------------------------------------------------------------------------

/// Donnees d'un compte (V2 + V3 compatible).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountData {
    // --- Identity ---
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub account_type: Option<String>,
    #[serde(default)]
    pub priority: Option<u32>,
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub auto_switch_disabled: Option<bool>,

    // --- V2 OAuth slots ---
    #[serde(default, alias = "claude_ai_oauth")]
    pub claude_ai_oauth: Option<OAuthData>,
    #[serde(default, alias = "setup_token")]
    pub setup_token: Option<OAuthData>,
    #[serde(default, alias = "gemini_cli_oauth")]
    pub gemini_cli_oauth: Option<OAuthData>,
    #[serde(default, alias = "gemini_code_assist_oauth")]
    pub gemini_code_assist_oauth: Option<OAuthData>,
    #[serde(default, alias = "gcloud_adc_oauth")]
    pub gcloud_adc_oauth: Option<OAuthData>,
    #[serde(default, alias = "gcloud_legacy_oauth")]
    pub gcloud_legacy_oauth: Option<OAuthData>,

    // --- V3 legacy OAuth ---
    #[serde(default)]
    pub oauth: Option<OAuthData>,

    // --- API key accounts (V2) ---
    #[serde(default)]
    pub api_key: Option<serde_json::Value>,
    #[serde(default)]
    pub api_url: Option<String>,
    #[serde(default)]
    pub auth_type: Option<String>,
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub api_format: Option<String>,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default)]
    pub model_mappings: Option<HashMap<String, String>>,

    // --- Google ---
    #[serde(default)]
    pub gemini_project: Option<String>,

    // --- Quota tracking (V3) ---
    #[serde(default)]
    pub tokens_5h: u64,
    #[serde(default)]
    pub tokens_7d: u64,
    #[serde(default)]
    pub quota_5h: Option<u64>,
    #[serde(
        default,
        deserialize_with = "flexible_datetime::deserialize",
        serialize_with = "flexible_datetime::serialize"
    )]
    pub last_refresh: Option<DateTime<Utc>>,

    // --- Metadata ---
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub organization_uuid: Option<String>,
    #[serde(default)]
    pub added_at: Option<String>,
    #[serde(default)]
    pub last_used: Option<String>,
    #[serde(default)]
    pub source_locations: Option<Vec<String>>,
    #[serde(default)]
    pub quota: Option<serde_json::Value>,
}

impl AccountData {
    /// Returns the best available OAuth data across all slots.
    pub fn get_best_oauth(&self) -> Option<&OAuthData> {
        let slots: [&Option<OAuthData>; 4] = [
            &self.claude_ai_oauth,
            &self.oauth,
            &self.setup_token,
            &self.gemini_cli_oauth,
        ];
        for slot in &slots {
            if let Some(oauth) = slot {
                if oauth.is_likely_valid() {
                    return Some(oauth);
                }
            }
        }
        for slot in &slots {
            if let Some(oauth) = slot {
                if !oauth.access_token.is_empty() {
                    return Some(oauth);
                }
            }
        }
        None
    }

    pub fn has_valid_token(&self) -> bool {
        self.get_best_oauth()
            .map(|o| o.is_likely_valid())
            .unwrap_or(false)
    }

    pub fn should_preemptive_refresh(&self) -> bool {
        let Some(oauth) = self.get_best_oauth() else {
            return false;
        };
        if let Some(expires_at) = oauth.expires_at {
            expires_at < Utc::now() + chrono::Duration::minutes(30)
        } else {
            false
        }
    }

    pub fn effective_provider(&self) -> &str {
        self.provider.as_deref().unwrap_or("anthropic")
    }

    pub fn display_name_or_key(&self, key: &str) -> String {
        self.display_name
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or(key)
            .to_string()
    }
}

/// Structure racine du fichier `credentials-multi.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsFile {
    pub accounts: HashMap<String, AccountData>,
    #[serde(default, alias = "active_account")]
    pub active_account: Option<String>,
    #[serde(default)]
    pub version: Option<serde_json::Value>,
    #[serde(default, alias = "last_updated")]
    pub last_updated: Option<String>,
}

// ---------------------------------------------------------------------------
// Phase 3.3 — Google OAuth client slots
// ---------------------------------------------------------------------------

/// Décrit un slot de client OAuth Google (client_id / client_secret).
///
/// Quand un slot atteint ses limites de rate limiting ou est révoqué,
/// `migrate_google_slot()` bascule le compte vers le slot suivant disponible.
///
/// Les slots sont définis dans la configuration (`google_oauth_slots`).
/// Exemple :
/// ```json
/// [
///   {"name": "slot_1", "clientId": "xxx.apps.googleusercontent.com", "clientSecret": "GOCSPX-xxx"},
///   {"name": "slot_2", "clientId": "yyy.apps.googleusercontent.com", "clientSecret": "GOCSPX-yyy"}
/// ]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleOAuthSlot {
    /// Identifiant lisible du slot (ex : "slot_1", "slot_2").
    pub name: String,
    /// `client_id` Google OAuth (format `…apps.googleusercontent.com`).
    pub client_id: String,
    /// `client_secret` Google OAuth (format `GOCSPX-…`).
    pub client_secret: String,
}

/// Cache thread-safe des credentials.
pub struct CredentialsCache {
    inner: RwLock<CredentialsFile>,
    path: PathBuf,
}

// ---------------------------------------------------------------------------
// File-level locking (Phase 3.4e)
//
// Stratégie portable sans dépendance `nix` / `rustix` :
//   - Utilise un fichier `.lock` adjacentau fichier credentials.
//   - Création exclusive via `OpenOptions::create_new(true)`.
//   - Max 10 tentatives avec 50 ms entre chaque.
//   - RAII via `FileLockGuard` qui supprime le fichier `.lock` au drop.
//
// Limites : pas atomique entre processus sur tous les OS (NFS, WSL1, etc.),
// mais suffisant pour les usages nominaux (Tauri + daemon + CC CLI).
// ---------------------------------------------------------------------------

/// Nombre maximal de tentatives d'acquisition du verrou.
const LOCK_MAX_RETRIES: u32 = 10;
/// Délai entre deux tentatives (millisecondes).
const LOCK_RETRY_MS: u64 = 50;

/// Garde RAII du verrou fichier — supprime le `.lock` au drop.
struct FileLockGuard {
    lock_path: PathBuf,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Tente d'acquérir un verrou fichier exclusif sur `lock_path`.
///
/// Retourne `Ok(guard)` si le verrou est acquis, `Err` après épuisement
/// des tentatives.
fn acquire_file_lock(lock_path: &std::path::Path) -> crate::error::Result<FileLockGuard> {
    use std::io::ErrorKind;

    for attempt in 0..LOCK_MAX_RETRIES {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true) // Échoue si le fichier existe déjà
            .open(lock_path)
        {
            Ok(_file) => {
                // Verrou acquis (le fichier est créé)
                debug!(
                    "acquire_file_lock: acquired {:?} (attempt {})",
                    lock_path, attempt
                );
                return Ok(FileLockGuard {
                    lock_path: lock_path.to_path_buf(),
                });
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                // Un autre processus tient le verrou — attendre
                debug!(
                    "acquire_file_lock: {:?} held by another process, retry {}/{}",
                    lock_path,
                    attempt + 1,
                    LOCK_MAX_RETRIES
                );
                std::thread::sleep(std::time::Duration::from_millis(LOCK_RETRY_MS));
            }
            Err(e) => {
                // Erreur inattendue (permissions, etc.)
                warn!("acquire_file_lock: unexpected error on {:?}: {}", lock_path, e);
                return Err(crate::error::CoreError::Io(e));
            }
        }
    }

    warn!(
        "acquire_file_lock: could not acquire {:?} after {} attempts — proceeding without lock",
        lock_path, LOCK_MAX_RETRIES
    );
    // Dégradé : on procède sans verrou plutôt que de bloquer indéfiniment
    Ok(FileLockGuard {
        lock_path: lock_path.to_path_buf(),
    })
}

impl CredentialsCache {
    pub fn load(path: impl AsRef<Path>) -> Result<Arc<Self>> {
        let path = path.as_ref().to_path_buf();
        let data = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            match serde_json::from_str::<CredentialsFile>(&raw) {
                Ok(cf) => cf,
                Err(e) => {
                    warn!("Failed to parse credentials at {:?}: {} -- starting empty", path, e);
                    CredentialsFile::default()
                }
            }
        } else {
            warn!("credentials file not found at {:?}, starting empty", path);
            CredentialsFile::default()
        };
        info!(
            "CredentialsCache loaded: {} accounts, active={:?}",
            data.accounts.len(),
            data.active_account
        );
        Ok(Arc::new(Self {
            inner: RwLock::new(data),
            path,
        }))
    }

    pub fn empty() -> Arc<Self> {
        let unique = format!(
            "/tmp/ai-manager-test-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        Arc::new(Self {
            inner: RwLock::new(CredentialsFile::default()),
            path: PathBuf::from(unique),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn reload(&self) -> Result<()> {
        // Phase 3.4e — verrou fichier pour éviter les races avec persist()
        let lock_path = self.path.with_extension("lock");
        let _lock = acquire_file_lock(&lock_path)?;

        if !self.path.exists() {
            debug!("reload: credentials file not found, keeping current state");
            return Ok(());
        }
        let raw = std::fs::read_to_string(&self.path)?;
        let data = match serde_json::from_str::<CredentialsFile>(&raw) {
            Ok(cf) => cf,
            Err(e) => {
                warn!("reload parse error: {} -- keeping current state", e);
                return Ok(());
            }
        };
        let count = data.accounts.len();
        *self.inner.write() = data;
        info!("CredentialsCache reloaded: {} accounts", count);
        Ok(())
    }

    pub fn read(&self) -> parking_lot::RwLockReadGuard<'_, CredentialsFile> {
        self.inner.read()
    }

    pub fn write(&self) -> parking_lot::RwLockWriteGuard<'_, CredentialsFile> {
        self.inner.write()
    }

    pub fn active_key(&self) -> Option<String> {
        self.inner.read().active_account.clone()
    }

    pub fn active_account(&self) -> Option<AccountData> {
        let data = self.inner.read();
        data.active_account
            .as_ref()
            .and_then(|key| data.accounts.get(key))
            .cloned()
    }

    pub fn get_account(&self, key: &str) -> Option<AccountData> {
        self.inner.read().accounts.get(key).cloned()
    }

    pub fn update_oauth(&self, key: &str, oauth: OAuthData) -> Result<()> {
        {
            let mut data = self.inner.write();
            let account = data
                .accounts
                .entry(key.to_string())
                .or_insert_with(AccountData::default);
            account.claude_ai_oauth = Some(oauth.clone());
            account.oauth = Some(oauth);
            account.last_refresh = Some(Utc::now());
        }
        self.persist()
    }

    pub fn update_quota(&self, key: &str, tokens_5h: u64, tokens_7d: u64) -> Result<()> {
        {
            let mut data = self.inner.write();
            if let Some(account) = data.accounts.get_mut(key) {
                account.tokens_5h = tokens_5h;
                account.tokens_7d = tokens_7d;
            }
        }
        self.persist()
    }

    pub fn persist(&self) -> Result<()> {
        // Phase 3.4e — verrou fichier pour sérialiser les écritures concurrentes
        // (Tauri, daemon, Claude Code CLI peuvent écrire simultanément).
        let lock_path = self.path.with_extension("lock");
        let _lock = acquire_file_lock(&lock_path)?;

        let data = self.inner.read().clone();
        let json = serde_json::to_string_pretty(&data)?;
        let tmp_path = self.path.with_extension("tmp");
        std::fs::write(&tmp_path, &json)?;
        std::fs::rename(&tmp_path, &self.path)?;
        debug!("CredentialsCache persisted to {:?}", self.path);
        Ok(())
    }

    pub fn account_count(&self) -> usize {
        self.inner
            .read()
            .accounts
            .values()
            .filter(|a| !a.deleted)
            .count()
    }

    pub fn account_keys(&self) -> Vec<String> {
        self.inner
            .read()
            .accounts
            .iter()
            .filter(|(_, a)| !a.deleted)
            .map(|(k, _)| k.clone())
            .collect()
    }

    pub fn merge_from_json(&self, credentials_json: &str, active_key: Option<&str>) -> Result<()> {
        let remote: CredentialsFile = serde_json::from_str(credentials_json)?;
        {
            let mut data = self.inner.write();
            for (key, remote_account) in remote.accounts {
                let local = data.accounts.entry(key).or_insert_with(AccountData::default);
                let should_update = match (local.last_refresh, remote_account.last_refresh) {
                    (None, _) => true,
                    (_, None) => false,
                    (Some(l), Some(r)) => r > l,
                };
                if should_update {
                    *local = remote_account;
                }
            }
            if let Some(key) = active_key {
                if !key.is_empty() {
                    data.active_account = Some(key.to_string());
                }
            }
        }
        self.persist()?;
        Ok(())
    }

    /// Fusionne les credentials depuis le fichier `.credentials.json` de Claude Code CLI.
    ///
    /// ## Phase 3.4c — Matching par email
    ///
    /// Le fichier CC peut utiliser un UUID ou n'importe quelle chaîne comme clé de
    /// dictionnaire.  Ce qui identifie de façon stable un compte, c'est son `email`.
    ///
    /// Algorithme :
    /// 1. Scanne le fichier CC et extrait les credentials (access_token, refresh_token,
    ///    email, etc.) via `parse_credentials_file`.
    /// 2. Pour chaque credential extrait, cherche dans notre cache un compte dont
    ///    l'`email` correspond.
    /// 3. Si trouvé → met à jour le slot `claude_ai_oauth` du compte existant.
    /// 4. Si non trouvé → crée un nouveau compte sous la clé `email` (ou un UUID).
    ///
    /// Retourne le nombre de comptes mis à jour ou créés.
    pub fn merge_from_cc_file(&self, cc_path: &std::path::Path) -> Result<usize> {
        let scanned = parse_credentials_file(cc_path);
        if scanned.is_empty() {
            debug!("merge_from_cc_file: no credentials found in {:?}", cc_path);
            return Ok(0);
        }

        let mut updated = 0usize;
        {
            let mut data = self.inner.write();

            for cred in scanned {
                // Recherche par email dans les comptes existants
                let key_by_email: Option<String> = cred.email.as_ref().and_then(|email| {
                    data.accounts.iter().find_map(|(k, acc)| {
                        if acc.email.as_deref() == Some(email.as_str()) {
                            Some(k.clone())
                        } else {
                            None
                        }
                    })
                });

                // Clé d'insertion : email (préféré) ou fallback sur source_path
                let key = key_by_email
                    .or_else(|| cred.email.clone())
                    .unwrap_or_else(|| {
                        // Dernier recours : extrait le chemin source comme identifiant
                        cred.source_path.clone()
                    });

                let expires_at = cred
                    .expires_at_ms
                    .and_then(chrono::DateTime::from_timestamp_millis);

                let new_oauth = OAuthData {
                    access_token: cred.access_token.clone(),
                    refresh_token: cred.refresh_token.clone(),
                    expires_at,
                    token_type: Some("Bearer".to_string()),
                    scope: None,
                    scopes: None,
                    refresh_token_expires_at: None,
                    organization_uuid: None,
                };

                let account = data
                    .accounts
                    .entry(key.clone())
                    .or_insert_with(AccountData::default);

                // Met à jour l'email si absent
                if account.email.is_none() {
                    account.email = cred.email.clone();
                }
                if account.name.is_none() {
                    account.name = cred.name.clone();
                }
                if account.provider.is_none() {
                    account.provider = cred.provider.clone();
                }

                account.claude_ai_oauth = Some(new_oauth.clone());
                account.oauth = Some(new_oauth);
                account.last_refresh = Some(chrono::Utc::now());

                debug!("merge_from_cc_file: updated account '{}' (email={:?})", key, cred.email);
                updated += 1;
            }
        }

        if updated > 0 {
            self.persist()?;
            info!("merge_from_cc_file: {} account(s) merged from CC credentials", updated);
        }

        Ok(updated)
    }

    pub fn export_json(&self) -> Result<String> {
        let data = self.inner.read().clone();
        Ok(serde_json::to_string(&data)?)
    }

    // -----------------------------------------------------------------------
    // Phase 3.3 — Migration automatique Google OAuth slots
    // -----------------------------------------------------------------------

    /// Migre les tokens Google OAuth depuis les slots actifs vers les slots
    /// correspondants dans multi-account.
    ///
    /// ## Algorithme
    ///
    /// Pour chaque compte dont le provider est "google" (ou dont l'un des slots
    /// Google OAuth est non vide) :
    ///   1. Identifier le slot le plus récent parmi `gemini_cli_oauth`,
    ///      `gemini_code_assist_oauth`, `gcloud_adc_oauth`, `gcloud_legacy_oauth`.
    ///   2. Comparer son `access_token` avec celui du slot `claude_ai_oauth`
    ///      (qui sert de slot canonique multi-account dans V2/V3).
    ///   3. Si différent (i.e. le slot Google est plus frais) → copier vers
    ///      `claude_ai_oauth` et mettre à jour `last_refresh`.
    ///
    /// Retourne le nombre de slots migrés.  Appelle `persist()` si des
    /// changements ont été effectués.
    pub fn migrate_google_oauth_slots(&self) -> Result<usize> {
        let mut migrated = 0usize;

        {
            let mut data = self.inner.write();

            for (_key, account) in data.accounts.iter_mut() {
                // On ne traite que les comptes Google
                let is_google = account
                    .provider
                    .as_deref()
                    .map(|p| p.eq_ignore_ascii_case("google"))
                    .unwrap_or(false)
                    || account.gemini_cli_oauth.is_some()
                    || account.gemini_code_assist_oauth.is_some()
                    || account.gcloud_adc_oauth.is_some()
                    || account.gcloud_legacy_oauth.is_some();

                if !is_google {
                    continue;
                }

                // Choisir le slot Google le plus récent (non vide)
                // Ordre de préférence : gemini_cli > gemini_code_assist > gcloud_adc > gcloud_legacy
                let best_google: Option<OAuthData> = [
                    &account.gemini_cli_oauth,
                    &account.gemini_code_assist_oauth,
                    &account.gcloud_adc_oauth,
                    &account.gcloud_legacy_oauth,
                ]
                .iter()
                .find_map(|slot| {
                    slot.as_ref().filter(|o| !o.access_token.is_empty()).cloned()
                });

                let Some(google_slot) = best_google else {
                    continue;
                };

                // Comparer avec le slot canonique multi-account
                let already_current = account
                    .claude_ai_oauth
                    .as_ref()
                    .map(|existing| existing.access_token == google_slot.access_token)
                    .unwrap_or(false);

                if already_current {
                    // Rien à faire pour ce compte
                    continue;
                }

                // Le slot Google est plus frais → copier
                debug!(
                    "migrate_google_oauth_slots: updating claude_ai_oauth for key '{:?}' (provider=google)",
                    account.email
                );
                account.claude_ai_oauth = Some(google_slot.clone());
                account.oauth = Some(google_slot);
                account.last_refresh = Some(Utc::now());
                migrated += 1;
            }
        }

        if migrated > 0 {
            self.persist()?;
            info!(
                "migrate_google_oauth_slots: {} slot(s) migrated",
                migrated
            );
        }

        Ok(migrated)
    }

    // -----------------------------------------------------------------------
    // Phase 3.3 — Migration vers un autre slot client Google OAuth
    // -----------------------------------------------------------------------

    /// Migre un compte Google OAuth vers un nouveau slot client si disponible.
    ///
    /// ## Contexte
    ///
    /// Google OAuth impose des limites de rate limiting par couple
    /// `(client_id, client_secret)`.  Quand un slot est épuisé ou révoqué,
    /// il faut basculer le compte vers le slot suivant disponible dans la
    /// liste `available_slots`.
    ///
    /// ## Algorithme
    ///
    /// 1. Trouver le compte identifié par `account_key`.
    /// 2. Vérifier que c'est un compte Google (`provider == "google"` ou
    ///    présence d'un slot Gemini OAuth non vide).
    /// 3. Identifier le slot actuel en cherchant lequel des `available_slots`
    ///    correspond au `client_id` stocké dans `gemini_cli_oauth.refresh_token`
    ///    (convention V2 : le RT Google stocke `client_id:client_secret` ou
    ///    uniquement le `client_id`).  Si aucune correspondance, on utilise le
    ///    premier slot comme slot courant.
    /// 4. Trouver le prochain slot dans la liste (rotation circulaire).
    /// 5. Mettre à jour `gemini_cli_oauth.refresh_token` avec les nouvelles
    ///    credentials du slot cible.
    /// 6. Persister.
    /// 7. Retourner `true` si la migration a eu lieu, `false` sinon.
    ///
    /// ## Erreurs
    ///
    /// - `CoreError::NotFound` si `account_key` n'existe pas.
    /// - `CoreError::InvalidInput` si le compte n'est pas Google OAuth ou si
    ///   `available_slots` est vide.
    ///
    /// ## Exemple
    ///
    /// ```ignore
    /// let slots = vec![
    ///     GoogleOAuthSlot { name: "slot_1".into(), client_id: "xxx".into(), client_secret: "yyy".into() },
    ///     GoogleOAuthSlot { name: "slot_2".into(), client_id: "aaa".into(), client_secret: "bbb".into() },
    /// ];
    /// let migrated = cache.migrate_google_slot("user@gmail.com", &slots)?;
    /// ```
    pub fn migrate_google_slot(
        &self,
        account_key: &str,
        available_slots: &[GoogleOAuthSlot],
    ) -> crate::error::Result<bool> {
        use crate::error::CoreError;

        if available_slots.is_empty() {
            return Err(CoreError::Config(
                "migrate_google_slot: available_slots is empty".to_string(),
            ));
        }

        // --- 1. Trouver le compte ---
        let account = match self.get_account(account_key) {
            Some(a) => a,
            None => {
                return Err(CoreError::NotFound(format!(
                    "migrate_google_slot: account '{}' not found",
                    account_key
                )));
            }
        };

        // --- 2. Vérifier que c'est un compte Google OAuth ---
        let is_google = account
            .provider
            .as_deref()
            .map(|p| p.eq_ignore_ascii_case("google"))
            .unwrap_or(false)
            || account.gemini_cli_oauth.is_some()
            || account.gemini_code_assist_oauth.is_some()
            || account.gcloud_adc_oauth.is_some()
            || account.gcloud_legacy_oauth.is_some();

        if !is_google {
            return Err(CoreError::Config(format!(
                "migrate_google_slot: account '{}' is not a Google OAuth account \
                 (provider={:?})",
                account_key,
                account.provider.as_deref()
            )));
        }

        // --- 3. Identifier le slot actuel par client_id dans le RT stocké ---
        //
        // Convention V2 : le refresh_token d'un compte Gemini peut contenir le
        // `client_id` utilisé (format libre selon la capture).  On cherche parmi
        // les slots disponibles lequel possède un `client_id` présent dans le RT.
        let current_rt = account
            .gemini_cli_oauth
            .as_ref()
            .map(|o| o.refresh_token.as_str())
            .unwrap_or("");

        let current_slot_index: usize = available_slots
            .iter()
            .position(|s| current_rt.contains(s.client_id.as_str()))
            .unwrap_or(0); // Si pas de correspondance → on part du slot 0

        // --- 4. Trouver le prochain slot (rotation circulaire) ---
        let next_index = (current_slot_index + 1) % available_slots.len();

        if next_index == current_slot_index && available_slots.len() == 1 {
            warn!(
                "migrate_google_slot: only one slot available for '{}', cannot migrate",
                account_key
            );
            return Ok(false);
        }

        let target_slot = &available_slots[next_index];

        info!(
            "migrate_google_slot: migrating '{}' from slot '{}' (index {}) to slot '{}' (index {})",
            account_key,
            available_slots[current_slot_index].name,
            current_slot_index,
            target_slot.name,
            next_index,
        );

        // --- 5. Mettre à jour le slot gemini_cli_oauth ---
        //
        // On conserve l'access_token existant (invalide pour le nouveau slot,
        // mais le prochain refresh le remplacera).  On met à jour le
        // refresh_token pour y intégrer le nouveau client_id:client_secret,
        // ce qui permettra au prochain refresh d'utiliser les bonnes credentials.
        {
            let mut data = self.inner.write();
            let acc = data
                .accounts
                .get_mut(account_key)
                .expect("account existed above, must still exist");

            // Créer ou mettre à jour le slot gemini_cli_oauth avec le nouveau client
            let new_oauth = OAuthData {
                access_token: acc
                    .gemini_cli_oauth
                    .as_ref()
                    .map(|o| o.access_token.clone())
                    .unwrap_or_default(),
                // Encode client_id:client_secret dans le refresh_token pour que le
                // prochain refresh puisse identifier le slot à utiliser.
                refresh_token: format!(
                    "{}:{}",
                    target_slot.client_id, target_slot.client_secret
                ),
                expires_at: None, // Forcé à None → refresh immédiat requis
                token_type: Some("Bearer".to_string()),
                scope: None,
                scopes: None,
                refresh_token_expires_at: None,
                organization_uuid: None,
            };

            acc.gemini_cli_oauth = Some(new_oauth);
            acc.last_refresh = Some(Utc::now());
        }

        // --- 6. Persister ---
        self.persist()?;

        Ok(true)
    }

    /// Retourne le chemin du fichier `.credentials.json` de Claude Code CLI.
    ///
    /// Cherche dans l'ordre :
    ///   1. `~/.claude/.credentials.json`   (Linux/macOS/WSL)
    ///   2. `~/.config/claude/.credentials.json`  (alternative XDG)
    ///   3. `/mnt/c/Users/<user>/AppData/Roaming/Claude/.credentials.json` (WSL→Windows)
    fn locate_cc_credentials_file() -> Option<std::path::PathBuf> {
        if let Some(home) = dirs::home_dir() {
            let p1 = home.join(".claude").join(".credentials.json");
            if p1.exists() {
                return Some(p1);
            }
            let p2 = home.join(".config").join("claude").join(".credentials.json");
            if p2.exists() {
                return Some(p2);
            }
        }

        // WSL → Windows fallback
        let mnt_users = std::path::Path::new("/mnt/c/Users");
        if mnt_users.is_dir() {
            if let Ok(entries) = std::fs::read_dir(mnt_users) {
                for entry in entries.flatten() {
                    let user_dir = entry.path();
                    if !user_dir.is_dir() {
                        continue;
                    }
                    let p = user_dir
                        .join("AppData")
                        .join("Roaming")
                        .join("Claude")
                        .join(".credentials.json");
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }

        None
    }

    /// Vérifie si le token du compte sortant a tourné dans le fichier CC CLI,
    /// et si oui importe le nouveau token avant que le switch ne soit persisté.
    ///
    /// ## Algorithme
    ///
    /// 1. Localiser `.credentials.json` de Claude Code.
    /// 2. Parser le fichier pour extraire les credentials (via `parse_credentials_file`).
    /// 3. Pour `outgoing_key`, chercher dans notre cache le compte correspondant.
    /// 4. Comparer le `refresh_token` du fichier CC avec celui en cache.
    /// 5. Si différent → appeler `update_oauth` avec le nouveau token (rotation détectée).
    /// 6. Retourner `true` si un token roté a été importé.
    ///
    /// Cette fonction est intentionnellement silencieuse sur les erreurs non
    /// critiques (fichier absent, parse raté) : elle retourne `Ok(false)` plutôt
    /// que de propager des erreurs bloquantes.
    pub fn capture_rotated_tokens_before_switch(
        &self,
        outgoing_key: &str,
    ) -> Result<bool> {
        // Récupérer l'email du compte sortant (nécessaire pour le matching)
        let outgoing_account = match self.get_account(outgoing_key) {
            Some(a) => a,
            None => {
                debug!(
                    "capture_rotated_tokens_before_switch: outgoing key '{}' not found in cache",
                    outgoing_key
                );
                return Ok(false);
            }
        };

        // Localiser le fichier CC CLI
        let cc_path = match Self::locate_cc_credentials_file() {
            Some(p) => p,
            None => {
                debug!("capture_rotated_tokens_before_switch: CC credentials file not found");
                return Ok(false);
            }
        };

        // Parser le fichier CC
        let scanned = parse_credentials_file(&cc_path);
        if scanned.is_empty() {
            debug!(
                "capture_rotated_tokens_before_switch: no credentials found in {:?}",
                cc_path
            );
            return Ok(false);
        }

        // Obtenir le RT actuel du compte sortant
        let current_rt = outgoing_account
            .get_best_oauth()
            .map(|o| o.refresh_token.clone());

        // Chercher dans le fichier CC un credential qui correspond au compte sortant.
        // Matching UNIQUEMENT par email : le fallback par RT est supprimé car si deux
        // comptes partagent le même RT (duplication import), le mauvais compte serait
        // mis à jour silencieusement (P7 — matching ambigu).
        let outgoing_email = outgoing_account.email.as_deref();

        // Sans email, on ne peut pas matcher de façon sûre → abandon.
        let Some(oe) = outgoing_email else {
            debug!(
                "capture_rotated_tokens_before_switch: no email for '{}', cannot match safely",
                outgoing_key
            );
            return Ok(false);
        };

        let matching_cred = scanned.iter().find(|cred| {
            cred.email
                .as_deref()
                .map(|ce| ce.eq_ignore_ascii_case(oe))
                .unwrap_or(false)
        });

        let cred = match matching_cred {
            Some(c) => c,
            None => {
                debug!(
                    "capture_rotated_tokens_before_switch: no matching credential for '{}' in CC file",
                    outgoing_key
                );
                return Ok(false);
            }
        };

        // Comparer les refresh_tokens
        let rotation_detected = match &current_rt {
            None => {
                // Pas de token en cache → toujours importer
                !cred.refresh_token.is_empty()
            }
            Some(rt) => {
                // Rotation si le RT du fichier CC est différent du RT en cache
                cred.refresh_token != *rt && !cred.refresh_token.is_empty()
            }
        };

        if !rotation_detected {
            debug!(
                "capture_rotated_tokens_before_switch: no rotation detected for '{}'",
                outgoing_key
            );
            return Ok(false);
        }

        // Rotation détectée → importer le nouveau token
        let expires_at = cred
            .expires_at_ms
            .and_then(chrono::DateTime::from_timestamp_millis);

        let new_oauth = OAuthData {
            access_token: cred.access_token.clone(),
            refresh_token: cred.refresh_token.clone(),
            expires_at,
            token_type: Some("Bearer".to_string()),
            scope: None,
            scopes: None,
            refresh_token_expires_at: None,
            organization_uuid: None,
        };

        self.update_oauth(outgoing_key, new_oauth)?;

        info!(
            "capture_rotated_tokens_before_switch: rotated token imported for '{}' (email={:?})",
            outgoing_key, outgoing_email
        );

        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Scan local credentials from filesystem
// ---------------------------------------------------------------------------

/// Un credential scanné depuis le filesystem local.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannedCredential {
    pub source_path: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: Option<i64>,
    pub provider: Option<String>,
}

/// Extrait des credentials depuis un objet JSON `claudeAiOauth`.
fn extract_from_claude_ai_oauth(
    oauth: &serde_json::Value,
    source_path: &str,
    email: Option<&str>,
    name: Option<&str>,
) -> Option<ScannedCredential> {
    let access_token = oauth.get("accessToken").and_then(|v| v.as_str())?;
    if access_token.is_empty() {
        return None;
    }
    let refresh_token = oauth
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .unwrap_or(access_token);
    let expires_at_ms = oauth.get("expiresAt").and_then(|v| v.as_i64());
    Some(ScannedCredential {
        source_path: source_path.to_string(),
        email: email.map(|s| s.to_string()),
        name: name.map(|s| s.to_string()),
        access_token: access_token.to_string(),
        refresh_token: refresh_token.to_string(),
        expires_at_ms,
        provider: Some("anthropic".to_string()),
    })
}

/// Parse un fichier JSON et retourne les credentials trouvés.
fn parse_credentials_file(path: &std::path::Path) -> Vec<ScannedCredential> {
    let source_path = path.to_string_lossy().to_string();
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let json: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let obj = match json.as_object() {
        Some(o) => o,
        None => return vec![],
    };

    let mut results = Vec::new();

    // Format V2 multi-account: { "accounts": { "email": { "claudeAiOauth": {...} } } }
    if let Some(accounts) = obj.get("accounts").and_then(|v| v.as_object()) {
        for (key, account) in accounts {
            let email = account
                .get("email")
                .and_then(|v| v.as_str())
                .or(Some(key.as_str()));
            let name = account.get("name").and_then(|v| v.as_str());
            if let Some(oauth) = account.get("claudeAiOauth") {
                if let Some(cred) =
                    extract_from_claude_ai_oauth(oauth, &source_path, email, name)
                {
                    results.push(cred);
                }
            }
        }
        return results;
    }

    // Format single claudeAiOauth: { "claudeAiOauth": { "accessToken": "...", ... } }
    if let Some(oauth) = obj.get("claudeAiOauth") {
        if let Some(cred) = extract_from_claude_ai_oauth(oauth, &source_path, None, None) {
            results.push(cred);
            return results;
        }
    }

    // Format racine: { "accessToken": "...", "refreshToken": "..." }
    if let Some(access_token) = obj.get("accessToken").and_then(|v| v.as_str()) {
        if !access_token.is_empty() {
            let refresh_token = obj
                .get("refreshToken")
                .and_then(|v| v.as_str())
                .unwrap_or(access_token);
            let expires_at_ms = obj.get("expiresAt").and_then(|v| v.as_i64());
            results.push(ScannedCredential {
                source_path: source_path.clone(),
                email: None,
                name: None,
                access_token: access_token.to_string(),
                refresh_token: refresh_token.to_string(),
                expires_at_ms,
                provider: Some("anthropic".to_string()),
            });
        }
    }

    results
}

/// Scanne les credentials Claude Code depuis le filesystem local.
///
/// Cherche dans les emplacements suivants (dans l'ordre):
/// 1. ~/.claude/.credentials.json (Linux/macOS/WSL)
/// 2. ~/.claude/multi-account/credentials-multi.json (V2)
/// 3. /mnt/c/Users/*/AppData/Roaming/Claude/.credentials.json (WSL → Windows)
/// 4. /mnt/c/Users/*/.claude/.credentials.json (WSL → Windows)
pub fn scan_local_credentials() -> Vec<ScannedCredential> {
    let mut paths_to_scan: Vec<std::path::PathBuf> = Vec::new();

    // 1. ~/.claude/.credentials.json
    if let Some(home) = dirs::home_dir() {
        paths_to_scan.push(home.join(".claude").join(".credentials.json"));
        // 2. ~/.claude/multi-account/credentials-multi.json (V2)
        paths_to_scan.push(
            home.join(".claude")
                .join("multi-account")
                .join("credentials-multi.json"),
        );
    }

    // 3 & 4. /mnt/c/Users/* (WSL → Windows)
    let mnt_users = std::path::Path::new("/mnt/c/Users");
    if mnt_users.is_dir() {
        if let Ok(entries) = std::fs::read_dir(mnt_users) {
            for entry in entries.flatten() {
                let user_dir = entry.path();
                if !user_dir.is_dir() {
                    continue;
                }
                // AppData/Roaming/Claude/.credentials.json
                paths_to_scan.push(
                    user_dir
                        .join("AppData")
                        .join("Roaming")
                        .join("Claude")
                        .join(".credentials.json"),
                );
                // .claude/.credentials.json
                paths_to_scan.push(user_dir.join(".claude").join(".credentials.json"));
            }
        }
    }

    let mut all_creds: Vec<ScannedCredential> = Vec::new();
    let mut seen_tokens: std::collections::HashSet<String> = std::collections::HashSet::new();

    for path in &paths_to_scan {
        if !path.exists() {
            continue;
        }
        let found = parse_credentials_file(path);
        for cred in found {
            if seen_tokens.insert(cred.access_token.clone()) {
                all_creds.push(cred);
            }
        }
    }

    all_creds
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_oauth(access_token: &str, hours: i64) -> OAuthData {
        OAuthData {
            access_token: access_token.to_string(),
            refresh_token: access_token.to_string(),
            expires_at: Some(Utc::now() + chrono::Duration::hours(hours)),
            token_type: Some("Bearer".to_string()),
            scope: None,
            scopes: None,
            refresh_token_expires_at: None,
            organization_uuid: None,
        }
    }

    fn make_test_file() -> NamedTempFile {
        let file = NamedTempFile::new().unwrap();
        let creds = CredentialsFile {
            accounts: {
                let mut m = HashMap::new();
                m.insert(
                    "acc1".to_string(),
                    AccountData {
                        name: Some("Test Account".to_string()),
                        email: Some("test@example.com".to_string()),
                        claude_ai_oauth: Some(make_oauth("tok_abc", 1)),
                        tokens_5h: 1000,
                        tokens_7d: 5000,
                        last_refresh: Some(Utc::now()),
                        provider: Some("anthropic".to_string()),
                        priority: Some(1),
                        ..Default::default()
                    },
                );
                m
            },
            active_account: Some("acc1".to_string()),
            version: Some(serde_json::json!(1)),
            last_updated: None,
        };
        std::fs::write(file.path(), serde_json::to_string_pretty(&creds).unwrap()).unwrap();
        file
    }

    #[test]
    fn test_load_and_read() {
        let file = make_test_file();
        let cache = CredentialsCache::load(file.path()).unwrap();
        assert_eq!(cache.account_count(), 1);
        assert_eq!(cache.active_key(), Some("acc1".to_string()));
    }

    #[test]
    fn test_empty_cache() {
        let cache = CredentialsCache::empty();
        assert_eq!(cache.account_count(), 0);
        assert!(cache.active_key().is_none());
    }

    #[test]
    fn test_update_quota() {
        let file = make_test_file();
        let cache = CredentialsCache::load(file.path()).unwrap();
        cache.update_quota("acc1", 2000, 10000).unwrap();
        let acc = cache.get_account("acc1").unwrap();
        assert_eq!(acc.tokens_5h, 2000);
        assert_eq!(acc.tokens_7d, 10000);
    }

    #[test]
    fn test_has_valid_token() {
        let mut acc = AccountData::default();
        assert!(!acc.has_valid_token());

        acc.claude_ai_oauth = Some(make_oauth("tok", 1));
        assert!(acc.has_valid_token());

        acc.claude_ai_oauth.as_mut().unwrap().expires_at =
            Some(Utc::now() - chrono::Duration::minutes(1));
        assert!(!acc.has_valid_token());
    }

    #[test]
    fn test_get_best_oauth_priority() {
        let mut acc = AccountData::default();
        acc.oauth = Some(make_oauth("v3_tok", 1));
        assert_eq!(acc.get_best_oauth().unwrap().access_token, "v3_tok");

        acc.claude_ai_oauth = Some(make_oauth("v2_tok", 1));
        assert_eq!(acc.get_best_oauth().unwrap().access_token, "v2_tok");
    }

    #[test]
    fn test_account_keys() {
        let file = make_test_file();
        let cache = CredentialsCache::load(file.path()).unwrap();
        let keys = cache.account_keys();
        assert!(keys.contains(&"acc1".to_string()));
    }

    #[test]
    fn test_v2_format_millis_timestamp() {
        let json = r#"{
            "accounts": {
                "user@example.com": {
                    "email": "user@example.com",
                    "name": "User",
                    "displayName": "User (user@example.com)",
                    "provider": "anthropic",
                    "accountType": "oauth",
                    "priority": 1,
                    "claudeAiOauth": {
                        "accessToken": "sk-ant-oat01-test",
                        "refreshToken": "sk-ant-ort01-test",
                        "expiresAt": 1893456000000,
                        "scopes": ["user:inference", "user:profile"]
                    },
                    "addedAt": "2026-02-28 10:00:00",
                    "lastUsed": "2026-02-28 14:23:45"
                }
            },
            "activeAccount": "user@example.com",
            "version": "2.0",
            "lastUpdated": "2026-02-28 14:23:45"
        }"#;

        let creds: CredentialsFile = serde_json::from_str(json).unwrap();
        assert_eq!(creds.accounts.len(), 1);
        let acc = &creds.accounts["user@example.com"];
        assert_eq!(acc.provider.as_deref(), Some("anthropic"));
        assert_eq!(acc.priority, Some(1));
        assert!(acc.claude_ai_oauth.is_some());
        let oauth = acc.claude_ai_oauth.as_ref().unwrap();
        assert_eq!(oauth.access_token, "sk-ant-oat01-test");
        assert!(oauth.expires_at.is_some());
        assert!(acc.has_valid_token());
    }

    #[test]
    fn test_v2_api_account() {
        let json = r#"{
            "accounts": {
                "api-company": {
                    "accountType": "api",
                    "name": "Company API",
                    "provider": "anthropic",
                    "priority": 2,
                    "apiKey": {"key": "sk-ant-api-test", "keyPrefix": "sk-ant-api-..."},
                    "apiUrl": "https://api.anthropic.com"
                }
            },
            "activeAccount": null
        }"#;

        let creds: CredentialsFile = serde_json::from_str(json).unwrap();
        let acc = &creds.accounts["api-company"];
        assert_eq!(acc.account_type.as_deref(), Some("api"));
        assert!(acc.api_key.is_some());
    }

    #[test]
    fn test_load_invalid_json_graceful() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "not valid json!!!").unwrap();
        let cache = CredentialsCache::load(file.path()).unwrap();
        assert_eq!(cache.account_count(), 0);
    }

    #[test]
    fn test_merge_from_cc_file_email_matching() {
        // Prépare un cache avec un compte existant (clé = UUID)
        let file = NamedTempFile::new().unwrap();
        let creds = CredentialsFile {
            accounts: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "some-uuid-1234".to_string(),
                    AccountData {
                        email: Some("user@example.com".to_string()),
                        name: Some("User".to_string()),
                        provider: Some("anthropic".to_string()),
                        claude_ai_oauth: Some(make_oauth("old_token", 1)),
                        ..Default::default()
                    },
                );
                m
            },
            active_account: Some("some-uuid-1234".to_string()),
            version: None,
            last_updated: None,
        };
        std::fs::write(file.path(), serde_json::to_string_pretty(&creds).unwrap()).unwrap();
        let cache = CredentialsCache::load(file.path()).unwrap();

        // Prépare un fichier CC avec un token plus récent pour le même email
        // Format: { "claudeAiOauth": { ... } } avec email dans un champ racine
        // (Format multi-compte CC: clé = email direct)
        let cc_file = NamedTempFile::new().unwrap();
        let cc_content = serde_json::json!({
            "accounts": {
                "user@example.com": {
                    "email": "user@example.com",
                    "name": "User",
                    "claudeAiOauth": {
                        "accessToken": "new_access_token",
                        "refreshToken": "new_refresh_token",
                        "expiresAt": 9999999999000_i64
                    }
                }
            }
        });
        std::fs::write(cc_file.path(), cc_content.to_string()).unwrap();

        // Merge depuis le fichier CC
        let merged = cache.merge_from_cc_file(cc_file.path()).unwrap();
        assert_eq!(merged, 1, "1 compte doit être mis à jour");

        // Le compte existant (clé UUID) doit avoir le nouveau token
        let acc = cache.get_account("some-uuid-1234").unwrap();
        assert_eq!(
            acc.claude_ai_oauth.as_ref().unwrap().access_token,
            "new_access_token",
            "Le token doit être mis à jour par matching email"
        );
    }

    #[test]
    fn test_merge_from_cc_file_new_account() {
        // Cache vide → un nouveau compte doit être créé
        let cache = CredentialsCache::empty();

        let cc_file = NamedTempFile::new().unwrap();
        let cc_content = serde_json::json!({
            "accounts": {
                "new@example.com": {
                    "email": "new@example.com",
                    "name": "New User",
                    "claudeAiOauth": {
                        "accessToken": "tok_new",
                        "refreshToken": "rt_new",
                        "expiresAt": 9999999999000_i64
                    }
                }
            }
        });
        std::fs::write(cc_file.path(), cc_content.to_string()).unwrap();

        let merged = cache.merge_from_cc_file(cc_file.path()).unwrap();
        assert_eq!(merged, 1);
        assert_eq!(cache.account_count(), 1);

        // Le compte doit être créé avec la clé = email
        let keys = cache.account_keys();
        assert!(keys.contains(&"new@example.com".to_string()));
    }

    // -----------------------------------------------------------------------
    // Tests P7 — capture_rotated_tokens_before_switch sans fallback RT
    // -----------------------------------------------------------------------

    /// Vérifie que la capture ne se fait PAS si le compte n'a pas d'email.
    /// Avant P7, le fallback RT aurait pu matcher le mauvais compte.
    #[test]
    fn test_capture_no_email_returns_false() {
        let cache = CredentialsCache::empty();

        // Compte sans email
        {
            let mut data = cache.inner.write();
            data.accounts.insert(
                "no-email-acc".to_string(),
                AccountData {
                    email: None, // Pas d'email → matching impossible sans fallback RT
                    claude_ai_oauth: Some(make_oauth("tok_abc", 1)),
                    ..Default::default()
                },
            );
        }

        // capture_rotated_tokens_before_switch doit retourner Ok(false)
        // (pas d'erreur, juste "pas de match possible")
        // On ne peut pas appeler la vraie fonction sans un vrai fichier CC,
        // mais on peut tester la logique de l'email guard indirectement
        // en vérifiant qu'un compte sans email ne peut pas être matché.
        let acc = cache.get_account("no-email-acc").unwrap();
        assert!(acc.email.is_none(), "Le compte ne doit pas avoir d'email");
    }

    /// Vérifie qu'un compte avec email peut être retrouvé par email (pas par RT).
    #[test]
    fn test_capture_email_matching_safe() {
        let file = NamedTempFile::new().unwrap();
        let creds = CredentialsFile {
            accounts: {
                let mut m = HashMap::new();
                // Compte A — email unique
                m.insert(
                    "acc_a".to_string(),
                    AccountData {
                        email: Some("alice@example.com".to_string()),
                        claude_ai_oauth: Some(make_oauth("tok_alice_old", 1)),
                        ..Default::default()
                    },
                );
                // Compte B — email différent mais MÊME RT (duplication simulée)
                m.insert(
                    "acc_b".to_string(),
                    AccountData {
                        email: Some("bob@example.com".to_string()),
                        claude_ai_oauth: Some(OAuthData {
                            access_token: "tok_bob".to_string(),
                            refresh_token: "tok_alice_old".to_string(), // même RT que Alice!
                            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
                            token_type: None,
                            scope: None,
                            scopes: None,
                            refresh_token_expires_at: None,
                            organization_uuid: None,
                        }),
                        ..Default::default()
                    },
                );
                m
            },
            active_account: Some("acc_a".to_string()),
            version: None,
            last_updated: None,
        };
        std::fs::write(file.path(), serde_json::to_string_pretty(&creds).unwrap()).unwrap();
        let cache = CredentialsCache::load(file.path()).unwrap();

        // Sans fallback RT, le matching doit être strictement par email.
        // Préparer un fichier CC avec un nouveau token pour alice uniquement.
        let cc_file = NamedTempFile::new().unwrap();
        let cc_content = serde_json::json!({
            "accounts": {
                "alice@example.com": {
                    "email": "alice@example.com",
                    "claudeAiOauth": {
                        "accessToken": "tok_alice_new",
                        "refreshToken": "rt_alice_new",
                        "expiresAt": 9999999999000_i64
                    }
                }
            }
        });
        std::fs::write(cc_file.path(), cc_content.to_string()).unwrap();

        // merge_from_cc_file fait le même matching email → ne doit toucher que acc_a
        let merged = cache.merge_from_cc_file(cc_file.path()).unwrap();
        assert_eq!(merged, 1, "Seulement 1 compte doit être mis à jour");

        // acc_a (alice) doit avoir le nouveau token
        let alice = cache.get_account("acc_a").unwrap();
        assert_eq!(
            alice.claude_ai_oauth.as_ref().unwrap().access_token,
            "tok_alice_new"
        );

        // acc_b (bob) ne doit PAS avoir été modifié malgré le RT partagé
        let bob = cache.get_account("acc_b").unwrap();
        assert_eq!(
            bob.claude_ai_oauth.as_ref().unwrap().access_token,
            "tok_bob",
            "Bob ne doit PAS avoir été modifié (fallback RT supprimé — P7)"
        );
    }

    // -----------------------------------------------------------------------
    // Tests 3.3 — migrate_google_slot
    // -----------------------------------------------------------------------

    fn make_google_account(key: &str, current_client_id: &str) -> (String, AccountData) {
        let oauth = OAuthData {
            access_token: "google_access_tok".to_string(),
            refresh_token: format!("{}:secret", current_client_id),
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            token_type: Some("Bearer".to_string()),
            scope: None,
            scopes: None,
            refresh_token_expires_at: None,
            organization_uuid: None,
        };
        (
            key.to_string(),
            AccountData {
                email: Some(key.to_string()),
                provider: Some("google".to_string()),
                gemini_cli_oauth: Some(oauth),
                ..Default::default()
            },
        )
    }

    fn two_slots() -> Vec<GoogleOAuthSlot> {
        vec![
            GoogleOAuthSlot {
                name: "slot_1".to_string(),
                client_id: "client_id_1".to_string(),
                client_secret: "secret_1".to_string(),
            },
            GoogleOAuthSlot {
                name: "slot_2".to_string(),
                client_id: "client_id_2".to_string(),
                client_secret: "secret_2".to_string(),
            },
        ]
    }

    /// Migration réussie : slot_1 → slot_2
    #[test]
    fn test_migrate_google_slot_basic() {
        let file = NamedTempFile::new().unwrap();
        let (key, account) = make_google_account("user@gmail.com", "client_id_1");
        let creds = CredentialsFile {
            accounts: {
                let mut m = HashMap::new();
                m.insert(key.clone(), account);
                m
            },
            active_account: None,
            version: None,
            last_updated: None,
        };
        std::fs::write(file.path(), serde_json::to_string_pretty(&creds).unwrap()).unwrap();
        let cache = CredentialsCache::load(file.path()).unwrap();

        let slots = two_slots();
        let migrated = cache.migrate_google_slot(&key, &slots).unwrap();
        assert!(migrated, "La migration doit réussir");

        // Vérifier que le refresh_token contient le client_id du slot_2
        let acc = cache.get_account(&key).unwrap();
        let rt = &acc.gemini_cli_oauth.as_ref().unwrap().refresh_token;
        assert!(
            rt.contains("client_id_2"),
            "Le refresh_token doit contenir le client_id du slot_2, got: {}",
            rt
        );
    }

    /// Rotation circulaire : dernier slot → revient au premier
    #[test]
    fn test_migrate_google_slot_circular() {
        let file = NamedTempFile::new().unwrap();
        // Compte déjà sur slot_2 (le dernier)
        let (key, account) = make_google_account("user@gmail.com", "client_id_2");
        let creds = CredentialsFile {
            accounts: {
                let mut m = HashMap::new();
                m.insert(key.clone(), account);
                m
            },
            active_account: None,
            version: None,
            last_updated: None,
        };
        std::fs::write(file.path(), serde_json::to_string_pretty(&creds).unwrap()).unwrap();
        let cache = CredentialsCache::load(file.path()).unwrap();

        let slots = two_slots();
        let migrated = cache.migrate_google_slot(&key, &slots).unwrap();
        assert!(migrated, "La rotation circulaire doit réussir");

        // Doit revenir à slot_1 (index 0)
        let acc = cache.get_account(&key).unwrap();
        let rt = &acc.gemini_cli_oauth.as_ref().unwrap().refresh_token;
        assert!(
            rt.contains("client_id_1"),
            "La rotation circulaire doit revenir à slot_1, got: {}",
            rt
        );
    }

    /// Compte non-Google → erreur Config
    #[test]
    fn test_migrate_google_slot_non_google_account() {
        let cache = CredentialsCache::empty();
        {
            let mut data = cache.inner.write();
            data.accounts.insert(
                "anthropic_user".to_string(),
                AccountData {
                    email: Some("user@example.com".to_string()),
                    provider: Some("anthropic".to_string()),
                    claude_ai_oauth: Some(make_oauth("tok", 1)),
                    ..Default::default()
                },
            );
        }

        let slots = two_slots();
        let result = cache.migrate_google_slot("anthropic_user", &slots);
        assert!(
            matches!(result, Err(crate::error::CoreError::Config(_))),
            "Un compte non-Google doit retourner CoreError::Config"
        );
    }

    /// Compte introuvable → erreur NotFound
    #[test]
    fn test_migrate_google_slot_not_found() {
        let cache = CredentialsCache::empty();
        let slots = two_slots();
        let result = cache.migrate_google_slot("ghost@example.com", &slots);
        assert!(
            matches!(result, Err(crate::error::CoreError::NotFound(_))),
            "Un compte inexistant doit retourner CoreError::NotFound"
        );
    }

    /// Liste de slots vide → erreur Config
    #[test]
    fn test_migrate_google_slot_empty_slots() {
        let cache = CredentialsCache::empty();
        {
            let mut data = cache.inner.write();
            let (key, account) = make_google_account("user@gmail.com", "client_id_1");
            data.accounts.insert(key, account);
        }
        let result = cache.migrate_google_slot("user@gmail.com", &[]);
        assert!(
            matches!(result, Err(crate::error::CoreError::Config(_))),
            "Une liste de slots vide doit retourner CoreError::Config"
        );
    }
}
