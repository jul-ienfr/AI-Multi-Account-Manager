//! Commandes IPC Tauri -- exposees au frontend via `invoke()`.
//!
//! Toutes les commandes sont async et retournent `Result<T, String>`
//! pour une gestion d'erreur coherente cote JS.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::{debug, info};

use ai_manager_core::credentials::{AccountData, OAuthData};
use ai_manager_core::validator;
use ai_sync::compat::PeerProtocol;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// DTOs (matching frontend types.ts)
// ---------------------------------------------------------------------------

/// Account state as returned to the frontend (matches AccountState in types.ts).
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountStateDto {
    pub key: String,
    pub data: AccountDataDto,
    pub quota: Option<QuotaDto>,
    pub is_active: bool,
    /// true si le compte est en état invalid_grant (token révoqué).
    pub revoked: bool,
    /// true si le compte dispose d'au moins un token OAuth ou clé API.
    pub has_token: bool,
}

/// Account data DTO (matches AccountData in types.ts).
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountDataDto {
    pub email: Option<String>,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub account_type: Option<String>,
    pub provider: Option<String>,
    pub priority: Option<u32>,
    pub plan_type: Option<String>,
    pub claude_ai_oauth: Option<OAuthSlotDto>,
    pub setup_token: Option<OAuthSlotDto>,
    pub gemini_cli_oauth: Option<OAuthSlotDto>,
    pub api_key: Option<serde_json::Value>,
    pub api_url: Option<String>,
    pub auto_switch_disabled: Option<bool>,
}

/// Simplified OAuth slot for frontend display.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OAuthSlotDto {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

impl OAuthSlotDto {
    fn from_oauth(oauth: &OAuthData) -> Self {
        Self {
            access_token: if oauth.access_token.is_empty() {
                None
            } else {
                Some(format!("{}...", &oauth.access_token[..8.min(oauth.access_token.len())]))
            },
            refresh_token: if oauth.refresh_token.is_empty() {
                None
            } else {
                Some("***".to_string())
            },
            expires_at: oauth.expires_at.map(|dt| dt.timestamp_millis()),
        }
    }
}

/// Quota info DTO (matches QuotaInfo in types.ts).
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QuotaDto {
    pub tokens5h: u64,
    pub limit5h: u64,
    pub tokens7d: u64,
    pub limit7d: u64,
    pub phase: Option<String>,
    pub ema_velocity: f64,
    pub time_to_threshold: Option<f64>,
    pub last_updated: Option<String>,
    pub resets_at_5h: Option<String>,
    pub resets_at_7d: Option<String>,
}

impl AccountStateDto {
    pub fn from_entry(
        key: String,
        data: &AccountData,
        active_key: &Option<String>,
        metrics: Option<&crate::state::QuotaMetricsCache>,
        invalid_grant: bool,
    ) -> Self {
        // Parse V2 quota JSON field if present
        let (v2_5h_util, v2_7d_util, v2_5h_limit, v2_7d_limit) =
            Self::parse_v2_quota(&data.quota);

        // Extract resets_at from stored quota JSON
        let (resets_at_5h, resets_at_7d) = Self::parse_resets_at(&data.quota);

        // Determine tokens and limits — combine V2 JSON and V3 flat fields
        let limit5h = data.quota_5h
            .or(v2_5h_limit)
            .unwrap_or(45_000_000);
        let limit7d = v2_7d_limit.unwrap_or(limit5h * 4);

        // V3 fields take priority; fall back to V2 utilization-based calculation
        let tokens5h = if data.tokens_5h > 0 {
            data.tokens_5h
        } else if let Some(util) = v2_5h_util {
            ((util / 100.0) * limit5h as f64) as u64
        } else {
            0
        };
        let tokens7d = if data.tokens_7d > 0 {
            data.tokens_7d
        } else if let Some(util) = v2_7d_util {
            ((util / 100.0) * limit7d as f64) as u64
        } else {
            0
        };

        let pct = if limit5h > 0 {
            tokens5h as f64 / limit5h as f64
        } else {
            0.0
        };
        let phase = if pct >= 0.95 {
            "Critical"
        } else if pct >= 0.80 {
            "Alert"
        } else if pct >= 0.60 {
            "Watch"
        } else {
            "Cruise"
        };

        // Use cached metrics from velocity calculator if available
        let (ema_velocity, time_to_threshold, cached_resets_5h, cached_resets_7d) =
            if let Some(m) = metrics {
                (
                    m.ema_velocity,
                    m.time_to_threshold,
                    m.resets_at_5h.clone().or(resets_at_5h),
                    m.resets_at_7d.clone().or(resets_at_7d),
                )
            } else {
                (0.0, None, resets_at_5h, resets_at_7d)
            };

        // Always create quota DTO so UI always shows quota bars
        let quota = Some(QuotaDto {
            tokens5h,
            limit5h,
            tokens7d,
            limit7d,
            phase: Some(phase.to_string()),
            ema_velocity,
            time_to_threshold,
            last_updated: data.last_refresh.map(|t| t.to_rfc3339()),
            resets_at_5h: cached_resets_5h,
            resets_at_7d: cached_resets_7d,
        });

        let has_token = data.claude_ai_oauth.is_some()
            || data.setup_token.is_some()
            || data.api_key.is_some();

        Self {
            is_active: active_key.as_deref() == Some(key.as_str()),
            revoked: invalid_grant,
            has_token,
            data: AccountDataDto {
                email: data.email.clone(),
                name: data.name.clone(),
                display_name: data.display_name.clone(),
                account_type: data.account_type.clone(),
                provider: Some(data.effective_provider().to_string()),
                priority: data.priority,
                plan_type: data.plan_type.clone(),
                claude_ai_oauth: data.claude_ai_oauth.as_ref().map(OAuthSlotDto::from_oauth),
                setup_token: data.setup_token.as_ref().map(OAuthSlotDto::from_oauth),
                gemini_cli_oauth: data.gemini_cli_oauth.as_ref().map(OAuthSlotDto::from_oauth),
                api_key: data.api_key.clone(),
                api_url: data.api_url.clone(),
                auto_switch_disabled: data.auto_switch_disabled,
            },
            key,
            quota,
        }
    }

    /// Parse V2 quota JSON: { "five_hour": { "utilization": 45.2, "limit": ... }, "seven_day": {...} }
    fn parse_v2_quota(
        quota_json: &Option<serde_json::Value>,
    ) -> (Option<f64>, Option<f64>, Option<u64>, Option<u64>) {
        let Some(q) = quota_json.as_ref().and_then(|v| v.as_object()) else {
            return (None, None, None, None);
        };

        let five_hour = q.get("five_hour").and_then(|v| v.as_object());
        let seven_day = q.get("seven_day").and_then(|v| v.as_object());

        let util_5h = five_hour.and_then(|fh| fh.get("utilization").and_then(|u| u.as_f64()));
        let util_7d = seven_day.and_then(|sd| sd.get("utilization").and_then(|u| u.as_f64()));
        let limit_5h = five_hour.and_then(|fh| fh.get("limit").and_then(|l| l.as_u64()));
        let limit_7d = seven_day.and_then(|sd| sd.get("limit").and_then(|l| l.as_u64()));

        (util_5h, util_7d, limit_5h, limit_7d)
    }

    /// Extract resets_at from stored quota JSON.
    fn parse_resets_at(
        quota_json: &Option<serde_json::Value>,
    ) -> (Option<String>, Option<String>) {
        let Some(q) = quota_json.as_ref().and_then(|v| v.as_object()) else {
            return (None, None);
        };
        let r5h = q.get("five_hour")
            .and_then(|v| v.as_object())
            .and_then(|fh| fh.get("resets_at"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let r7d = q.get("seven_day")
            .and_then(|v| v.as_object())
            .and_then(|sd| sd.get("resets_at"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        (r5h, r7d)
    }
}

// ---------------------------------------------------------------------------
// Input payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAccountData {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub provider: Option<String>,
    pub priority: Option<u32>,
    pub plan_type: Option<String>,
    pub claude_ai_oauth: Option<OAuthInput>,
    pub api_key: Option<serde_json::Value>,
    pub api_url: Option<String>,
    pub auto_switch_disabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthInput {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

// ---------------------------------------------------------------------------
// Account commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_accounts(state: State<'_, AppState>) -> Result<Vec<AccountStateDto>, String> {
    let data = state.credentials.read();
    let active_key = &data.active_account;
    let metrics = state.quota_metrics.read();
    let invalid_grant = state.invalid_grant_accounts.read();
    let accounts: Vec<AccountStateDto> = data
        .accounts
        .iter()
        .filter(|(_, a)| !a.deleted)
        .map(|(k, a)| AccountStateDto::from_entry(
            k.clone(), a, active_key, metrics.get(k), invalid_grant.contains(k)
        ))
        .collect();
    debug!("get_accounts: {} accounts", accounts.len());
    Ok(accounts)
}

#[tauri::command]
pub async fn get_active_account(state: State<'_, AppState>) -> Result<Option<AccountStateDto>, String> {
    let data = state.credentials.read();
    let active_key = &data.active_account;
    let metrics = state.quota_metrics.read();
    let invalid_grant = state.invalid_grant_accounts.read();
    if let Some(key) = active_key {
        if let Some(account) = data.accounts.get(key) {
            return Ok(Some(AccountStateDto::from_entry(
                key.clone(),
                account,
                active_key,
                metrics.get(key),
                invalid_grant.contains(key),
            )));
        }
    }
    Ok(None)
}

#[tauri::command]
pub async fn switch_account(key: String, state: State<'_, AppState>) -> Result<(), String> {
    // Phase 3.4d — Vérification du token avant le switch.
    //
    // On récupère le compte cible et on vérifie :
    //   1. Si le token est valide (non expiré) → switch immédiat.
    //   2. Si le token est expiré → tentative de refresh rapide.
    //   3. Si le compte n'a pas de token OAuth (ex : compte API key) → switch autorisé.
    //   4. Si le refresh échoue → erreur retournée au frontend.
    let account_opt = {
        let data = state.credentials.read();
        if !data.accounts.contains_key(&key) {
            return Err(format!("Account not found: {}", key));
        }
        data.accounts.get(&key).cloned()
    };

    let account = account_opt.ok_or_else(|| format!("Account not found: {}", key))?;

    // Si le compte a un slot OAuth, vérifier la validité du token
    if let Some(oauth) = account.get_best_oauth() {
        let token_ok = oauth.is_likely_valid();

        if !token_ok {
            debug!("switch_account: token expired for {}, attempting refresh", key);

            // Tentative de refresh rapide
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("claude-cli/1.0")
                .build()
                .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

            match ai_manager_core::oauth::refresh_oauth_token(&client, &oauth.refresh_token)
                .await
            {
                ai_manager_core::oauth::RefreshResult::Ok(new_oauth) => {
                    // Refresh réussi — persiste le nouveau token avant le switch
                    state
                        .credentials
                        .update_oauth(&key, new_oauth)
                        .map_err(|e| format!("Failed to persist refreshed token: {}", e))?;
                    info!("switch_account: token refreshed for {} before switch", key);
                }
                ai_manager_core::oauth::RefreshResult::InvalidGrant => {
                    // Token révoqué — marquer dans invalid_grant_accounts
                    state.invalid_grant_accounts.write().insert(key.clone());
                    return Err(
                        "Token invalide ou expiré pour ce compte — réauthentification requise"
                            .to_string(),
                    );
                }
                ai_manager_core::oauth::RefreshResult::Expired
                | ai_manager_core::oauth::RefreshResult::NetworkError(_) => {
                    return Err(
                        "Token invalide ou expiré pour ce compte — réauthentification requise"
                            .to_string(),
                    );
                }
            }
        }
    }
    // Comptes sans OAuth (API key, etc.) : switch autorisé sans vérification de token

    // Phase 3.4a — Capture du token roté AVANT de persister le switch.
    //
    // Si le RT du compte sortant a changé dans le fichier `.credentials.json`
    // de Claude Code CLI (rotation détectée côté CLI), on importe le nouveau RT
    // avant d'écraser `active_account`.
    let current_key = state.credentials.active_key();
    if let Some(ref outgoing_key) = current_key {
        // Ne capturer que si on change vraiment de compte
        if outgoing_key != &key {
            match state
                .credentials
                .capture_rotated_tokens_before_switch(outgoing_key)
            {
                Ok(true) => {
                    info!(
                        "switch_account: rotated token captured for outgoing account '{}'",
                        outgoing_key
                    );
                }
                Ok(false) => {
                    debug!(
                        "switch_account: no token rotation detected for outgoing account '{}'",
                        outgoing_key
                    );
                }
                Err(e) => {
                    // Non bloquant : on log et on continue le switch
                    debug!(
                        "switch_account: capture_rotated_tokens_before_switch failed for '{}': {}",
                        outgoing_key, e
                    );
                }
            }
        }
    }

    {
        let mut data = state.credentials.write();
        data.active_account = Some(key.clone());
    }
    state
        .credentials
        .persist()
        .map_err(|e| e.to_string())?;
    info!("Switched to account: {}", key);

    // Record switch in history
    let history_path = state.credentials_path.parent()
        .map(|p| p.join("switch_history.jsonl"))
        .unwrap_or_default();
    let entry = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "to": key,
        "reason": "manual",
    });
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(&history_path) {
        use std::io::Write;
        let _ = writeln!(f, "{}", entry);
    }

    Ok(())
}

#[tauri::command]
pub async fn refresh_account(key: String, state: State<'_, AppState>) -> Result<(), String> {
    let account = state
        .credentials
        .get_account(&key)
        .ok_or_else(|| format!("Account not found: {}", key))?;

    let oauth = account
        .get_best_oauth()
        .ok_or_else(|| "Account has no OAuth data".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("claude-cli/1.0")
        .build()
        .map_err(|e| e.to_string())?;

    match ai_manager_core::oauth::refresh_oauth_token(&client, &oauth.refresh_token)
        .await
    {
        ai_manager_core::oauth::RefreshResult::Ok(new_oauth) => {
            state
                .credentials
                .update_oauth(&key, new_oauth)
                .map_err(|e| e.to_string())?;
            // Refresh réussi → retirer du HashSet invalid_grant si présent
            state.invalid_grant_accounts.write().remove(&key);
            info!("Token refreshed for account {}", key);
            Ok(())
        }
        ai_manager_core::oauth::RefreshResult::InvalidGrant => {
            state.invalid_grant_accounts.write().insert(key.clone());
            Err("Token refresh failed: invalid_grant — réauthentification requise".to_string())
        }
        ai_manager_core::oauth::RefreshResult::Expired => {
            Err("Token refresh failed: token expiré".to_string())
        }
        ai_manager_core::oauth::RefreshResult::NetworkError(msg) => {
            Err(format!("Token refresh failed: {}", msg))
        }
    }
}

#[tauri::command]
pub async fn add_account(
    key: String,
    data: AddAccountData,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if key.is_empty() {
        return Err("Account key cannot be empty".to_string());
    }

    // --- Validation Phase 2.3 ---
    // Valider l'email si fourni
    if let Some(ref email) = data.email {
        if !email.is_empty() {
            validator::validate_email(email)
                .map_err(|e| format!("Email invalide : {}", e))?;
        }
    }

    // Détecter le type de compte pour choisir la bonne validation
    let is_api_account = data.provider.as_deref() == Some("api")
        || data.api_key.is_some();

    if is_api_account {
        // Compte API key : valider la clé API
        if let Some(ref api_key_val) = data.api_key {
            let key_str = api_key_val.as_str().unwrap_or("");
            if !key_str.is_empty() {
                validator::validate_api_key(key_str)
                    .map_err(|e| format!("Clé API invalide : {}", e))?;
            }
        }
    } else {
        // Compte OAuth : valider l'access token si fourni
        if let Some(ref oauth_input) = data.claude_ai_oauth {
            if !oauth_input.access_token.is_empty() {
                validator::validate_access_token(&oauth_input.access_token)
                    .map_err(|e| format!("Access token invalide : {}", e))?;
            }
        }
    }
    // --- Fin validation Phase 2.3 ---

    let oauth = data.claude_ai_oauth.map(|o| OAuthData {
        access_token: o.access_token.clone(),
        refresh_token: o.refresh_token.unwrap_or_else(|| o.access_token),
        expires_at: None,
        token_type: Some("Bearer".to_string()),
        scope: None,
        scopes: None,
        refresh_token_expires_at: None,
        organization_uuid: None,
    });

    let account = AccountData {
        name: data.name,
        email: data.email,
        display_name: data.display_name,
        provider: data.provider,
        priority: data.priority,
        plan_type: data.plan_type,
        auto_switch_disabled: data.auto_switch_disabled,
        claude_ai_oauth: oauth.clone(),
        oauth,
        api_key: data.api_key,
        api_url: data.api_url,
        added_at: Some(chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()),
        ..Default::default()
    };

    {
        let mut creds = state.credentials.write();
        creds.accounts.insert(key.clone(), account);
    }
    state
        .credentials
        .persist()
        .map_err(|e| e.to_string())?;

    info!("Account added: {}", key);
    Ok(())
}

#[tauri::command]
pub async fn delete_account(key: String, state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut data = state.credentials.write();
        let account = data
            .accounts
            .get_mut(&key)
            .ok_or_else(|| format!("Account not found: {}", key))?;
        account.deleted = true;
        if data.active_account.as_deref() == Some(&key) {
            data.active_account = None;
        }
    }
    state
        .credentials
        .persist()
        .map_err(|e| e.to_string())?;
    info!("Account deleted: {}", key);
    Ok(())
}

/// Révoque le token OAuth d'un compte Anthropic.
///
/// Appelle `DELETE https://api.anthropic.com/v1/oauth/token` puis soft-delete
/// le compte (marque `deleted = true`) si la révocation réussit.
/// Si le serveur Anthropic retourne une erreur, l'opération est abandonnée et
/// une erreur descriptive est retournée au frontend.
#[tauri::command]
pub async fn revoke_account(key: String, state: State<'_, AppState>) -> Result<(), String> {
    // Récupérer l'access token du compte
    let access_token = {
        let data = state.credentials.read();
        let account = data
            .accounts
            .get(&key)
            .ok_or_else(|| format!("Compte non trouvé : {}", key))?;

        account
            .get_best_oauth()
            .map(|o| o.access_token.clone())
            .ok_or_else(|| "Ce compte ne possède pas de token OAuth".to_string())?
    };

    if access_token.is_empty() {
        return Err("L'access token est vide, impossible de révoquer".to_string());
    }

    // Construire le client HTTP
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("claude-cli/1.0")
        .build()
        .map_err(|e| format!("Impossible de créer le client HTTP : {}", e))?;

    // Appeler l'API de révocation Anthropic
    ai_manager_core::oauth::revoke_token(&client, &access_token)
        .await
        .map_err(|e| format!("Échec de la révocation : {}", e))?;

    // Succès — soft-delete du compte et désactivation si actif
    {
        let mut data = state.credentials.write();
        if let Some(account) = data.accounts.get_mut(&key) {
            account.deleted = true;
        }
        if data.active_account.as_deref() == Some(&key) {
            data.active_account = None;
        }
    }

    state
        .credentials
        .persist()
        .map_err(|e| format!("Erreur de persistance après révocation : {}", e))?;

    info!("Account {} revoked and soft-deleted", key);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAccountData {
    pub priority: Option<u32>,
    pub auto_switch_disabled: Option<bool>,
    pub display_name: Option<String>,
}

#[tauri::command]
pub async fn update_account(
    key: String,
    updates: UpdateAccountData,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut data = state.credentials.write();
        let account = data
            .accounts
            .get_mut(&key)
            .ok_or_else(|| format!("Account not found: {}", key))?;
        if let Some(p) = updates.priority {
            account.priority = Some(p);
        }
        if let Some(d) = updates.auto_switch_disabled {
            account.auto_switch_disabled = Some(d);
        }
        if let Some(n) = updates.display_name {
            account.display_name = Some(n);
        }
    }
    state
        .credentials
        .persist()
        .map_err(|e| e.to_string())?;
    info!("Account updated: {}", key);
    Ok(())
}

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let config = state.config.read();
    serde_json::to_value(&*config).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_config(
    config: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut cfg_json = serde_json::to_value(&*state.config.read())
            .map_err(|e| e.to_string())?;
        merge_json(&mut cfg_json, &config);
        let new_config: ai_manager_core::config::AppConfig =
            serde_json::from_value(cfg_json).map_err(|e| e.to_string())?;
        *state.config.write() = new_config;
    }
    state.config.persist().map_err(|e| e.to_string())?;
    Ok(())
}

fn merge_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    if let (Some(base_obj), Some(patch_obj)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in patch_obj {
            if v.is_object() && base_obj.get(k).map(|b| b.is_object()).unwrap_or(false) {
                merge_json(base_obj.get_mut(k).unwrap(), v);
            } else {
                base_obj.insert(k.clone(), v.clone());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Proxy commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_proxy_status(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let router = state.proxy_router.read().clone();
    let impersonator = state.proxy_impersonator.read().clone();
    Ok(serde_json::json!({
        "router": router,
        "impersonator": impersonator,
    }))
}

#[tauri::command]
pub async fn start_proxy(kind: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    let kind = kind.as_deref().unwrap_or("router").to_string();
    let (port, status_arc, task_arc) = match kind.as_str() {
        "impersonator" => (
            state.config.read().proxy.impersonator_port(),
            state.proxy_impersonator.clone(),
            state.proxy_impersonator_task.clone(),
        ),
        _ => (
            state.config.read().proxy.router_port(),
            state.proxy_router.clone(),
            state.proxy_router_task.clone(),
        ),
    };

    if status_arc.read().running {
        return Err(format!("Proxy {} is already running", kind));
    }

    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;

    let status_for_start = status_arc.clone();
    let status_for_end = status_arc.clone();
    let kind_label = kind.clone();

    let join = tokio::task::spawn(async move {
        {
            let mut s = status_for_start.write();
            s.running = true;
            s.port = addr.port();
        }
        if let Err(e) = proxy::server::start(addr).await {
            tracing::error!("Proxy {} error: {}", kind_label, e);
        }
        {
            let mut s = status_for_end.write();
            s.running = false;
            s.pid = None;
        }
    });

    *task_arc.lock() = Some(join.abort_handle());
    info!("Proxy {} started on port {}", kind, port);
    Ok(())
}

#[tauri::command]
pub async fn stop_proxy(kind: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    let kind = kind.as_deref().unwrap_or("router").to_string();
    let (status_arc, task_arc) = match kind.as_str() {
        "impersonator" => (
            state.proxy_impersonator.clone(),
            state.proxy_impersonator_task.clone(),
        ),
        _ => (
            state.proxy_router.clone(),
            state.proxy_router_task.clone(),
        ),
    };

    if let Some(handle) = task_arc.lock().take() {
        handle.abort();
    }
    {
        let mut s = status_arc.write();
        s.running = false;
        s.pid = None;
    }
    info!("Proxy {} stopped", kind);
    Ok(())
}

#[tauri::command]
pub async fn restart_proxy(kind: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    stop_proxy(kind.clone(), state.clone()).await?;
    start_proxy(kind, state).await
}

// ---------------------------------------------------------------------------
// Sync P2P commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_sync_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let cfg = state.config.read();
    let bus_opt = state.sync_bus.read().clone();
    let (peer_count, peers_list) = if let Some(bus) = bus_opt {
        let raw = bus.list_peers();
        let count = raw.len();
        let peers: Vec<_> = raw.into_iter().map(|(id, host, port)| {
            serde_json::json!({ "id": id, "host": host, "port": port, "connected": true })
        }).collect();
        (count, peers)
    } else {
        let peers = state.peers.read();
        let list: Vec<_> = peers.iter().map(|p| serde_json::json!({
            "id": p.id, "host": p.host, "port": p.port, "connected": p.connected
        })).collect();
        let count = list.len();
        (count, list)
    };
    Ok(serde_json::json!({
        "enabled": cfg.sync.enabled,
        "port": cfg.sync.port,
        "peer_count": peer_count,
        "peers": peers_list,
    }))
}

#[tauri::command]
pub async fn get_peers(state: State<'_, AppState>) -> Result<Vec<ai_manager_core::types::Peer>, String> {
    let bus_opt = state.sync_bus.read().clone();
    if let Some(bus) = bus_opt {
        let peers = bus.list_peers().into_iter().map(|(id, host, port)| {
            ai_manager_core::types::Peer { id, host, port, connected: true, last_seen: None }
        }).collect();
        Ok(peers)
    } else {
        Ok(state.peers.read().clone())
    }
}

#[tauri::command]
pub async fn add_peer(
    host: String,
    port: u16,
    id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let peer_id = id.unwrap_or_else(|| format!("{}:{}", host, port));
    let peer = ai_manager_core::types::Peer {
        id: peer_id.clone(),
        host: host.clone(),
        port,
        connected: false,
        last_seen: None,
    };
    {
        let mut peers = state.peers.write();
        peers.retain(|p| p.id != peer_id);
        peers.push(peer);
    }
    // Persist to config
    {
        let mut cfg = state.config.write();
        cfg.sync.peers.retain(|p| p.id != peer_id);
        cfg.sync.peers.push(ai_manager_core::config::PeerConfig {
            id: peer_id.clone(),
            host,
            port,
        });
    }
    let _ = state.config.persist();
    // Connect the peer via the real SyncBus if available
    let bus_opt = state.sync_bus.read().clone();
    if let Some(bus) = bus_opt {
        let peer_id2 = peer_id.clone();
        // Retrieve host/port from config (host was moved into cfg above)
        let (h, p) = {
            let cfg = state.config.read();
            cfg.sync.peers.iter()
                .find(|p| p.id == peer_id2)
                .map(|p| (p.host.clone(), p.port))
                .unwrap_or_default()
        };
        if !h.is_empty() {
            tauri::async_runtime::spawn(async move {
                bus.connect_peer(&peer_id2, &h, p, PeerProtocol::V3).await;
                tracing::info!("P2P: peer {} connected via bus", peer_id2);
            });
        }
    }
    info!("Peer added: {}", peer_id);
    Ok(())
}

#[tauri::command]
pub async fn remove_peer(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut peers = state.peers.write();
        peers.retain(|p| p.id != id);
    }
    {
        let mut cfg = state.config.write();
        cfg.sync.peers.retain(|p| p.id != id);
    }
    let _ = state.config.persist();
    let bus_opt = state.sync_bus.read().clone();
    if let Some(bus) = bus_opt {
        bus.remove_peer(&id);
    }
    info!("Peer removed: {}", id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: crée, démarre un SyncBus et un SyncCoordinator depuis la config.
// Stocke le sender shutdown du coordinateur dans state.sync_coordinator_shutdown.
// ---------------------------------------------------------------------------
async fn start_sync_bus_from_config(
    state: &AppState,
) -> Arc<ai_sync::bus::SyncBus> {
    let (port, key_bytes, peers_to_connect) = {
        let cfg = state.config.read();
        let port = cfg.sync.port;
        let key_bytes = crate::state::hex_to_bytes(
            cfg.sync.shared_key_hex.as_deref().unwrap_or(""),
        )
        .unwrap_or([0u8; 32]);
        let peers: Vec<(String, String, u16)> = cfg.sync.peers.iter()
            .map(|p| (p.id.clone(), p.host.clone(), p.port))
            .collect();
        (port, key_bytes, peers)
    };

    let instance_id = uuid::Uuid::new_v4().to_string();
    let bus = Arc::new(ai_sync::bus::SyncBus::new(instance_id.clone(), port, key_bytes));

    // 1. Démarrer l'écoute TCP entrante
    let bus_start = bus.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = bus_start.start().await {
            tracing::error!("SyncBus start error: {}", e);
        }
    });

    // 2. Connecter les pairs configurés (avec délai pour laisser le bind se faire)
    let bus_peers = bus.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        for (id, host, p) in peers_to_connect {
            bus_peers.connect_peer(&id, &host, p, PeerProtocol::V3).await;
        }
    });

    // 3. Spawn SyncCoordinator — reconcilie les credentials entre instances
    let coordinator = Arc::new(ai_sync::coordinator::SyncCoordinator::new(
        instance_id,
        bus.clone(),
        state.credentials.clone(),
    ));
    let (coord_tx, coord_rx) = tokio::sync::watch::channel(false);
    // Remplacer l'éventuel coordinateur précédent (signal d'arrêt implicite par drop)
    *state.sync_coordinator_shutdown.lock() = Some(coord_tx);
    tauri::async_runtime::spawn(async move {
        if let Err(e) = coordinator.run(coord_rx).await {
            tracing::error!("SyncCoordinator error: {}", e);
        }
    });

    bus
}

/// Active ou désactive la synchronisation P2P à chaud, sans redémarrer l'app.
#[tauri::command]
pub async fn toggle_sync(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut cfg = state.config.write();
        cfg.sync.enabled = enabled;
    }
    state.config.persist().map_err(|e| e.to_string())?;

    if !enabled {
        // Arrêter le coordinateur proprement, puis le bus
        if let Some(tx) = state.sync_coordinator_shutdown.lock().take() {
            let _ = tx.send(true);
        }
        *state.sync_bus.write() = None;
        info!("P2P sync disabled");
        return Ok(());
    }

    let bus = start_sync_bus_from_config(&state).await;
    *state.sync_bus.write() = Some(bus);
    info!("P2P sync enabled");
    Ok(())
}

/// Modifie le port TCP de la sync P2P et redémarre le bus si actif.
#[tauri::command]
pub async fn set_sync_port(
    port: u16,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut cfg = state.config.write();
        cfg.sync.port = port;
    }
    state.config.persist().map_err(|e| e.to_string())?;

    // Si la sync est activée, recréer le bus sur le nouveau port
    let enabled = state.config.read().sync.enabled;
    if enabled {
        *state.sync_bus.write() = None; // arrêter l'ancien bus
        let bus = start_sync_bus_from_config(&state).await;
        *state.sync_bus.write() = Some(bus);
        info!("P2P sync restarted on port {}", port);
    }
    Ok(())
}

#[tauri::command]
pub async fn generate_sync_key(
    state: State<'_, AppState>,
) -> Result<String, String> {
    // Generate 32 random bytes using two UUIDv4 (each is 16 random bytes)
    let u1 = uuid::Uuid::new_v4();
    let u2 = uuid::Uuid::new_v4();
    let mut key_bytes = [0u8; 32];
    key_bytes[..16].copy_from_slice(u1.as_bytes());
    key_bytes[16..].copy_from_slice(u2.as_bytes());
    let hex_key = bytes_to_hex(&key_bytes);
    // Save to config
    {
        let mut cfg = state.config.write();
        cfg.sync.shared_key_hex = Some(hex_key.clone());
    }
    let _ = state.config.persist();
    info!("New sync key generated");
    Ok(hex_key)
}

/// Encode bytes to lowercase hex string.
fn bytes_to_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[tauri::command]
pub async fn set_sync_key(
    key: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Validate: must be valid hex, 64 chars = 32 bytes
    if key.len() != 64 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Cle invalide: 64 caracteres hex attendus (32 bytes)".to_string());
    }
    {
        let mut cfg = state.config.write();
        cfg.sync.shared_key_hex = Some(key);
    }
    let _ = state.config.persist();
    info!("Sync key updated");
    Ok(())
}

#[tauri::command]
pub async fn test_peer_connection(
    host: String,
    port: u16,
) -> Result<bool, String> {
    use tokio::net::TcpStream;
    use std::time::Duration;
    match tokio::time::timeout(
        Duration::from_secs(3),
        TcpStream::connect(format!("{}:{}", host, port)),
    ).await {
        Ok(Ok(_)) => Ok(true),
        Ok(Err(e)) => Err(format!("Connexion refusee: {}", e)),
        Err(_) => Err("Timeout (3s)".to_string()),
    }
}

// ---------------------------------------------------------------------------
// SSH Sync commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_hostname() -> Result<String, String> {
    // Try gethostname, fallback to "unknown"
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    Ok(hostname)
}

#[tauri::command]
pub async fn add_ssh_host(
    host: String,
    port: u16,
    username: String,
    identity_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id = format!("{}@{}", username, host);
    let ssh_host = ai_manager_core::config::SshHostConfig {
        id: id.clone(),
        host,
        port,
        username,
        identity_path,
        enabled: true,
    };
    {
        let mut cfg = state.config.write();
        cfg.sync.ssh_hosts.retain(|h| h.id != id);
        cfg.sync.ssh_hosts.push(ssh_host);
    }
    let _ = state.config.persist();
    info!("SSH host added: {}", id);
    Ok(())
}

#[tauri::command]
pub async fn remove_ssh_host(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut cfg = state.config.write();
        cfg.sync.ssh_hosts.retain(|h| h.id != id);
    }
    let _ = state.config.persist();
    info!("SSH host removed: {}", id);
    Ok(())
}

#[tauri::command]
pub async fn test_ssh_connection(
    host: String,
    port: u16,
    username: String,
    identity_path: Option<String>,
) -> Result<bool, String> {
    let ssh_config = ai_sync::ssh::SshConfig::new(host, port, username, identity_path)
        .map_err(|e| format!("Config SSH invalide: {}", e))?;
    let ssh_sync = ai_sync::ssh::SshSync::new(ssh_config);
    ssh_sync.test_connection().await
        .map(|_| true)
        .map_err(|e| format!("Connexion SSH echouee: {}", e))
}

// ---------------------------------------------------------------------------
// Monitoring commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_quota_history(
    key: String,
    period: Option<String>,  // "24h" | "7d" | "30d" | None
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let history_path = state.credentials_path.parent()
        .map(|p| p.join("quota_history.jsonl"))
        .unwrap_or_default();

    let mut entries: Vec<serde_json::Value> = Vec::new();

    if history_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&history_path) {
            let cutoff = match period.as_deref() {
                Some("7d") => chrono::Utc::now() - chrono::Duration::days(7),
                Some("30d") => chrono::Utc::now() - chrono::Duration::days(30),
                _ => chrono::Utc::now() - chrono::Duration::hours(24), // default 24h
            };
            let cutoff_str = cutoff.to_rfc3339();

            for line in content.lines() {
                if line.trim().is_empty() { continue; }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                    // Filter by account key
                    if val.get("key").and_then(|k| k.as_str()) != Some(&key) { continue; }
                    // Filter by period
                    if let Some(ts) = val.get("timestamp").and_then(|t| t.as_str()) {
                        if ts < cutoff_str.as_str() { continue; }
                    }
                    // Return in frontend-expected format: {timestamp, tokens}
                    let ts = val.get("timestamp").and_then(|t| t.as_str()).unwrap_or("").to_string();
                    let tokens = val.get("tokens5h").and_then(|t| t.as_u64()).unwrap_or(0);
                    entries.push(serde_json::json!({ "timestamp": ts, "tokens": tokens }));
                }
            }
        }
    }

    // If no history data, return current snapshot as single point
    if entries.is_empty() {
        if let Some(a) = state.credentials.get_account(&key) {
            entries.push(serde_json::json!({
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "tokens": a.tokens_5h,
            }));
        }
    }

    Ok(entries)
}

#[tauri::command]
pub async fn get_switch_history(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let history_path = state.credentials_path.parent()
        .map(|p| p.join("switch_history.jsonl"))
        .unwrap_or_default();

    let mut switches: Vec<serde_json::Value> = Vec::new();

    if history_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&history_path) {
            for line in content.lines().rev().take(200) {
                if line.trim().is_empty() { continue; }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                    switches.push(val);
                }
            }
        }
    }

    Ok(switches)
}

#[tauri::command]
pub async fn get_impersonation_profiles(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let profiles_dir = state.credentials_path.parent()
        .map(|p| p.join("profiles"))
        .unwrap_or_default();

    let mut profiles: Vec<serde_json::Value> = Vec::new();

    if profiles_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&profiles_dir) {
            for entry in entries.flatten() {
                if entry.path().extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(data) = std::fs::read_to_string(entry.path()) {
                        if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&data) {
                            // Add provider name from filename
                            if let Some(stem) = entry.path().file_stem().and_then(|s| s.to_str()) {
                                val["provider_name"] = serde_json::Value::String(stem.to_string());
                            }
                            profiles.push(val);
                        }
                    }
                }
            }
        }
    }

    // Also check for legacy cc-profile.json
    let legacy = state.credentials_path.parent()
        .map(|p| p.join("cc-profile.json"))
        .unwrap_or_default();
    if legacy.exists() && profiles.is_empty() {
        if let Ok(data) = std::fs::read_to_string(&legacy) {
            if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&data) {
                val["provider_name"] = serde_json::Value::String("anthropic".to_string());
                profiles.push(val);
            }
        }
    }

    Ok(profiles)
}

#[tauri::command]
pub async fn get_sessions(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    // Lire les sessions depuis le répertoire multi-account/sessions/
    let sessions_dir = state.credentials_path.parent()
        .map(|p| p.join("sessions"))
        .unwrap_or_default();
    let mut active: Vec<serde_json::Value> = Vec::new();
    let mut total_today = 0u32;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    if sessions_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                if entry.path().extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(data) = std::fs::read_to_string(entry.path()) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                            if val.get("start_time").and_then(|t| t.as_str()).map(|t| t.starts_with(&today)).unwrap_or(false) {
                                total_today += 1;
                            }
                            active.push(val);
                        }
                    }
                }
            }
        }
    }
    // Trier par timestamp décroissant, garder les 50 dernières
    active.sort_by(|a, b| {
        let ta = a.get("start_time").and_then(|t| t.as_str()).unwrap_or("");
        let tb = b.get("start_time").and_then(|t| t.as_str()).unwrap_or("");
        tb.cmp(ta)
    });
    active.truncate(50);

    Ok(serde_json::json!({
        "active": active,
        "total_today": total_today,
        "cost_today": 0.0,
        "cost_7d": 0.0,
    }))
}

#[tauri::command]
pub async fn get_logs(
    filter: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    // Lire les logs depuis api_usage.jsonl
    let usage_path = state.credentials_path.parent()
        .map(|p| p.join("api_usage.jsonl"))
        .unwrap_or_default();

    let mut logs: Vec<serde_json::Value> = Vec::new();

    if usage_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&usage_path) {
            for line in content.lines().rev().take(200) {
                if line.trim().is_empty() { continue; }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(ref f) = filter {
                        let line_str = line.to_lowercase();
                        if !line_str.contains(&f.to_lowercase()) {
                            continue;
                        }
                    }
                    logs.push(val);
                }
            }
        }
    }

    Ok(logs)
}

// ---------------------------------------------------------------------------
// Dynamic proxy instance commands
// ---------------------------------------------------------------------------

use ai_manager_core::types::{ProxyInstanceConfig, ProxyInstanceState, ProxyStatus as CoreProxyStatus};
use crate::state::ProxyInstanceRuntime;

/// Probe un port local pour vérifier si un proxy y tourne (V2 ou V3).
///
/// Envoie un HTTP GET à `http://127.0.0.1:{port}/_proxy/health` avec un
/// timeout de 2 secondes.  Si la réponse contient `{"status": "ok"}`,
/// retourne le champ `backend` (ex: "python", "rust-auto").
async fn probe_proxy_health(port: u16) -> Option<String> {
    let url = format!("http://127.0.0.1:{}/_proxy/health", port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    if body.get("status").and_then(|v| v.as_str()) == Some("ok") {
        Some(
            body.get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
        )
    } else {
        None
    }
}

#[tauri::command]
pub async fn get_proxy_instances(
    state: State<'_, AppState>,
) -> Result<Vec<ProxyInstanceState>, String> {
    let cfg = state.config.read();
    let instances_rt = state.proxy_instances.read();

    let mut result = Vec::new();
    for inst_cfg in &cfg.proxy.instances {
        let status = if let Some(rt) = instances_rt.get(&inst_cfg.id) {
            rt.status.read().clone()
        } else {
            CoreProxyStatus { port: inst_cfg.port, ..Default::default() }
        };
        result.push(ProxyInstanceState {
            config: inst_cfg.clone(),
            status,
        });
    }
    Ok(result)
}

/// Probe tous les ports proxy configurés et met à jour le statut des instances
/// dont le proxy n'a pas été démarré par cette application (proxys externes, V2, P2P).
#[tauri::command]
pub async fn probe_proxy_instances(
    state: State<'_, AppState>,
) -> Result<Vec<ProxyInstanceState>, String> {
    let configs: Vec<ProxyInstanceConfig> = {
        state.config.read().proxy.instances.clone()
    };

    // Probe en parallèle tous les ports non gérés par cette instance
    let mut futures = Vec::new();
    for inst_cfg in &configs {
        let port = inst_cfg.port;
        let id = inst_cfg.id.clone();
        // Probe seulement si on ne l'a pas démarré nous-même (ni child process ni tokio task)
        let has_child = {
            let instances_rt = state.proxy_instances.read();
            instances_rt
                .get(&id)
                .map(|rt| rt.child_process.lock().is_some() || rt.task_handle.lock().is_some())
                .unwrap_or(false)
        };
        if !has_child {
            futures.push(async move { (id, port, probe_proxy_health(port).await) });
        }
    }

    let results = futures::future::join_all(futures).await;

    // Mettre à jour les statuts
    {
        let instances_rt = state.proxy_instances.read();
        for (id, port, backend) in &results {
            if let Some(rt) = instances_rt.get(id) {
                let mut s = rt.status.write();
                if let Some(backend_name) = backend {
                    s.running = true;
                    s.port = *port;
                    s.backend = Some(backend_name.clone());
                    debug!("Proxy '{}' detected externally on port {} (backend: {})", id, port, backend_name);
                } else if s.backend.is_some() && !s.running {
                    // Reset backend si le proxy externe n'est plus là
                    s.backend = None;
                }
            }
        }
    }

    // Retourner la liste mise à jour
    get_proxy_instances(state).await
}

#[tauri::command]
pub async fn add_proxy_instance(
    config: ProxyInstanceConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Add to config
    {
        let mut cfg = state.config.write();
        if cfg.proxy.instances.iter().any(|i| i.id == config.id) {
            return Err(format!("Proxy instance '{}' already exists", config.id));
        }
        cfg.proxy.instances.push(config.clone());
    }
    state.config.persist().map_err(|e| e.to_string())?;

    // Create runtime entry
    {
        let runtime = std::sync::Arc::new(ProxyInstanceRuntime {
            status: parking_lot::RwLock::new(CoreProxyStatus {
                port: config.port,
                ..Default::default()
            }),
            task_handle: parking_lot::Mutex::new(None),
            child_process: parking_lot::Mutex::new(None),
        });
        state.proxy_instances.write().insert(config.id.clone(), runtime);
    }

    info!("Proxy instance added: {} ({})", config.name, config.id);
    Ok(())
}

#[tauri::command]
pub async fn update_proxy_instance(
    id: String,
    updates: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut cfg = state.config.write();
        let inst = cfg.proxy.instances.iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| format!("Proxy instance '{}' not found", id))?;

        if let Some(name) = updates.get("name").and_then(|v| v.as_str()) {
            inst.name = name.to_string();
        }
        if let Some(port) = updates.get("port").and_then(|v| v.as_u64()) {
            inst.port = port as u16;
        }
        if let Some(enabled) = updates.get("enabled").and_then(|v| v.as_bool()) {
            inst.enabled = enabled;
        }
        if let Some(auto_start) = updates.get("autoStart").and_then(|v| v.as_bool()) {
            inst.auto_start = auto_start;
        }
        if let Some(targets) = updates.get("setupTargets").and_then(|v| v.as_array()) {
            inst.setup_targets = targets.iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect();
        }
        // Handle binaryPath: null clears it, string sets it
        if updates.get("binaryPath").is_some() {
            inst.binary_path = updates.get("binaryPath").and_then(|v| v.as_str()).map(|s| s.to_string());
        }
    }
    state.config.persist().map_err(|e| e.to_string())?;
    info!("Proxy instance updated: {}", id);
    Ok(())
}

#[tauri::command]
pub async fn delete_proxy_instance(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Stop if running
    {
        let instances = state.proxy_instances.read();
        if let Some(rt) = instances.get(&id) {
            // Kill child process if any
            if let Some(mut child) = rt.child_process.lock().take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            if let Some(handle) = rt.task_handle.lock().take() {
                handle.abort();
            }
        }
    }
    // Remove runtime
    state.proxy_instances.write().remove(&id);
    // Remove from config
    {
        let mut cfg = state.config.write();
        cfg.proxy.instances.retain(|i| i.id != id);
    }
    state.config.persist().map_err(|e| e.to_string())?;
    info!("Proxy instance deleted: {}", id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Security helpers (P3 / P4)
// ---------------------------------------------------------------------------

/// P3 — Valide que `path` pointe vers un binaire proxy reconnu.
///
/// Effectue une canonicalisation (résout `..` et symlinks) puis vérifie :
/// 1. Le fichier existe et est un fichier régulier.
/// 2. Son nom contient l'un des tokens attendus ("proxy", "claude-router", "ai-manager").
///
/// Retourne le `PathBuf` canonicalisé prêt à être passé à `Command::new`.
fn validate_binary_path(path: &str) -> Result<std::path::PathBuf, String> {
    let raw = std::path::PathBuf::from(path);
    // Canonicaliser : résout les symlinks et les segments `..`
    let canonical = raw
        .canonicalize()
        .map_err(|e| format!("Invalid binary path '{}': {}", raw.display(), e))?;
    // Vérifier que c'est bien un fichier régulier
    if !canonical.is_file() {
        return Err(format!("Binary not found or not a file: {}", canonical.display()));
    }
    // Vérifier que le nom du binaire appartient aux exécutables proxy autorisés
    let filename = canonical
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    let allowed = ["proxy", "claude-router", "ai-manager"];
    if !allowed.iter().any(|tok| filename.contains(tok)) {
        return Err(format!(
            "Binary '{}' is not a recognized proxy binary (expected name to contain one of: {})",
            filename,
            allowed.join(", ")
        ));
    }
    Ok(canonical)
}

/// P4 — Valide une URL webhook pour éviter tout SSRF.
///
/// Règles :
/// - Seuls les schémas `https` et `http` (localhost uniquement) sont acceptés.
/// - Les plages d'adresses IP privées RFC-1918 et link-local sont bloquées.
/// - Les schémas file://, ftp://, etc. sont rejetés.
fn validate_webhook_url(url: &str) -> Result<(), String> {
    // Extraction schéma (avant "://")
    let scheme_end = url
        .find("://")
        .ok_or_else(|| format!("Invalid webhook URL (missing scheme): {}", url))?;
    let scheme = &url[..scheme_end];

    // Extraction host (entre "://" et premier "/" ou fin)
    let after_scheme = &url[scheme_end + 3..];
    let host_end = after_scheme
        .find(|c| c == '/' || c == '?' || c == '#' || c == ':')
        .unwrap_or(after_scheme.len());
    let host = &after_scheme[..host_end];

    match scheme {
        "https" => { /* toujours autorisé */ }
        "http" => {
            // HTTP uniquement pour le développement local
            if host != "localhost" && host != "127.0.0.1" {
                return Err(
                    "HTTP webhooks are only allowed for localhost. Use HTTPS for remote URLs."
                        .to_string(),
                );
            }
        }
        other => {
            return Err(format!("Unsupported webhook URL scheme: '{}'", other));
        }
    }

    // Bloquer les plages d'IP privées / réservées (RFC 1918 + link-local + loopback non-localhost)
    let private_prefixes = [
        "10.",
        "172.16.", "172.17.", "172.18.", "172.19.",
        "172.20.", "172.21.", "172.22.", "172.23.",
        "172.24.", "172.25.", "172.26.", "172.27.",
        "172.28.", "172.29.", "172.30.", "172.31.",
        "192.168.",
        "169.254.", // link-local
        "0.",       // 0.0.0.0/8
        "100.64.",  // Shared address space (RFC 6598)
    ];
    for prefix in &private_prefixes {
        if host.starts_with(prefix) {
            return Err(format!(
                "Private/reserved IP address '{}' is not allowed in webhook URLs",
                host
            ));
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn start_proxy_instance(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (port, binary_path) = {
        let cfg = state.config.read();
        let inst = cfg.proxy.instances.iter()
            .find(|i| i.id == id)
            .ok_or_else(|| format!("Proxy instance '{}' not found", id))?;
        (inst.port, inst.binary_path.clone())
    };

    let instances = state.proxy_instances.read();
    let runtime = instances.get(&id)
        .ok_or_else(|| format!("Runtime for '{}' not found", id))?
        .clone();

    if runtime.status.read().running {
        return Err(format!("Proxy '{}' is already running", id));
    }

    if let Some(bin_path) = binary_path {
        // External binary mode: spawn process
        // P3 — Valider le chemin avant exécution pour éviter l'exécution de binaires arbitraires.
        let canonical_bin = validate_binary_path(&bin_path)?;

        let child = std::process::Command::new(&canonical_bin)
            .args(["--port", &port.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start {}: {}", canonical_bin.display(), e))?;

        let pid = child.id();
        {
            let mut s = runtime.status.write();
            s.running = true;
            s.port = port;
            s.pid = Some(pid);
        }
        *runtime.child_process.lock() = Some(child);
        info!("Proxy '{}' started (external pid={}) on port {}", id, pid, port);
    } else {
        // Built-in proxy mode: tokio task
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
            .parse()
            .map_err(|e: std::net::AddrParseError| e.to_string())?;

        let status_ref = runtime.clone();
        let id_label = id.clone();

        let join = tokio::task::spawn(async move {
            {
                let mut s = status_ref.status.write();
                s.running = true;
                s.port = addr.port();
            }
            if let Err(e) = proxy::server::start(addr).await {
                tracing::error!("Proxy {} error: {}", id_label, e);
            }
            {
                let mut s = status_ref.status.write();
                s.running = false;
                s.pid = None;
            }
        });

        *runtime.task_handle.lock() = Some(join.abort_handle());
        info!("Proxy instance '{}' started (built-in) on port {}", id, port);
    }

    Ok(())
}

#[tauri::command]
pub async fn stop_proxy_instance(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let instances = state.proxy_instances.read();
    let runtime = instances.get(&id)
        .ok_or_else(|| format!("Proxy instance '{}' not found", id))?;

    // Kill child process if any
    if let Some(mut child) = runtime.child_process.lock().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    // Abort tokio task if any
    if let Some(handle) = runtime.task_handle.lock().take() {
        handle.abort();
    }
    {
        let mut s = runtime.status.write();
        s.running = false;
        s.pid = None;
    }
    info!("Proxy instance '{}' stopped", id);
    Ok(())
}

#[tauri::command]
pub async fn restart_proxy_instance(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    stop_proxy_instance(id.clone(), state.clone()).await?;
    start_proxy_instance(id, state).await
}

// ---------------------------------------------------------------------------
// Proxy binary detection
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedBinary {
    pub id: String,
    pub name: String,
    pub path: String,
    pub default_port: u16,
}

#[tauri::command]
pub async fn detect_proxy_binaries() -> Result<Vec<DetectedBinary>, String> {
    let mut binaries = Vec::new();

    // Determine root directory: parent of the executable, then go up to find "AI Manager"
    let exe_path = std::env::current_exe().unwrap_or_default();
    let exe_dir = exe_path.parent().unwrap_or_else(|| std::path::Path::new("."));

    // Walk up to find "AI Manager" folder (or use exe dir's grandparent)
    let mut root = exe_dir.to_path_buf();
    for _ in 0..5 {
        if root.file_name().and_then(|n| n.to_str()) == Some("AI Manager") {
            break;
        }
        if let Some(p) = root.parent() {
            root = p.to_path_buf();
        } else {
            break;
        }
    }

    // anthrouter: check multiple build output paths
    let anthrouter_paths = [
        "anthrouter/target/x86_64-pc-windows-gnu/release/anthrouter.exe",
        "anthrouter/target/release/anthrouter.exe",
        "anthrouter/target/release/anthrouter",
        "anthrouter/anthrouter.exe",
        "anthrouter/anthrouter",
    ];
    for rel in &anthrouter_paths {
        let p = root.join(rel);
        if p.exists() {
            binaries.push(DetectedBinary {
                id: "router-rust".to_string(),
                name: "anthrouter".to_string(),
                path: p.to_string_lossy().to_string(),
                default_port: 18080,
            });
            break;
        }
    }

    // Other legacy binaries (standalone executables)
    let legacy_candidates = [
        ("impersonator-rust", "claude-impersonator", "claude-impersonator.exe", 18081u16),
        ("translator-rust", "claude-translator", "claude-translator.exe", 18082),
    ];
    for (id, name, rel_path, port) in &legacy_candidates {
        let full_path = root.join(rel_path);
        if full_path.exists() {
            binaries.push(DetectedBinary {
                id: id.to_string(),
                name: name.to_string(),
                path: full_path.to_string_lossy().to_string(),
                default_port: *port,
            });
            continue;
        }
        // Try without .exe (Linux)
        let linux_path = root.join(rel_path.trim_end_matches(".exe"));
        if linux_path.exists() {
            binaries.push(DetectedBinary {
                id: id.to_string(),
                name: name.to_string(),
                path: linux_path.to_string_lossy().to_string(),
                default_port: *port,
            });
        }
    }

    Ok(binaries)
}

// ---------------------------------------------------------------------------
// Setup injection commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn setup_claude_code(port: u16) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let settings_path = home.join(".claude").join("settings.json");

    let mut settings: serde_json::Value = if settings_path.exists() {
        let raw = std::fs::read_to_string(&settings_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let url = format!("http://127.0.0.1:{}", port);
    settings.as_object_mut().unwrap()
        .entry("env")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut().unwrap()
        .insert("ANTHROPIC_BASE_URL".to_string(), serde_json::Value::String(url.clone()));

    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&settings_path, json).map_err(|e| e.to_string())?;
    info!("Claude Code setup: ANTHROPIC_BASE_URL={}", url);
    Ok(())
}

#[tauri::command]
pub async fn remove_claude_code_setup() -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let settings_path = home.join(".claude").join("settings.json");

    if !settings_path.exists() {
        return Ok(());
    }

    let raw = std::fs::read_to_string(&settings_path).map_err(|e| e.to_string())?;
    let mut settings: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));

    if let Some(env) = settings.get_mut("env").and_then(|v| v.as_object_mut()) {
        env.remove("ANTHROPIC_BASE_URL");
    }

    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&settings_path, json).map_err(|e| e.to_string())?;
    info!("Claude Code setup removed");
    Ok(())
}

#[tauri::command]
pub async fn setup_vscode_proxy(port: u16) -> Result<(), String> {
    let settings_path = find_vscode_settings()?;

    let mut settings: serde_json::Value = if settings_path.exists() {
        let raw = std::fs::read_to_string(&settings_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let url = format!("http://127.0.0.1:{}", port);
    settings.as_object_mut().unwrap()
        .insert("http.proxy".to_string(), serde_json::Value::String(url.clone()));

    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&settings_path, json).map_err(|e| e.to_string())?;
    info!("VS Code proxy setup: http.proxy={}", url);
    Ok(())
}

#[tauri::command]
pub async fn remove_vscode_proxy() -> Result<(), String> {
    let settings_path = find_vscode_settings()?;

    if !settings_path.exists() {
        return Ok(());
    }

    let raw = std::fs::read_to_string(&settings_path).map_err(|e| e.to_string())?;
    let mut settings: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));

    if let Some(obj) = settings.as_object_mut() {
        obj.remove("http.proxy");
    }

    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&settings_path, json).map_err(|e| e.to_string())?;
    info!("VS Code proxy removed");
    Ok(())
}

fn find_vscode_settings() -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").map_err(|_| "APPDATA not set")?;
        Ok(std::path::PathBuf::from(appdata).join("Code").join("User").join("settings.json"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = dirs::home_dir().ok_or("Cannot find home directory")?;
        Ok(home.join(".config").join("Code").join("User").join("settings.json"))
    }
}

// ---------------------------------------------------------------------------
// Auto-import local credentials
// ---------------------------------------------------------------------------

/// Scanne les credentials locaux et retourne la liste des tokens trouvés.
#[tauri::command]
pub async fn scan_local_credentials() -> Vec<ai_manager_core::credentials::ScannedCredential> {
    ai_manager_core::credentials::scan_local_credentials()
}

/// Importe une liste de credentials scannés dans le cache.
/// Retourne le nombre de comptes importés.
#[tauri::command]
pub async fn import_scanned_credentials(
    state: tauri::State<'_, AppState>,
    credentials: Vec<ai_manager_core::credentials::ScannedCredential>,
) -> Result<usize, String> {
    use ai_manager_core::credentials::{AccountData, OAuthData};

    let mut imported = 0;
    {
        let mut data = state.credentials.write();
        for cred in credentials {
            let key = cred.email.clone().unwrap_or_else(|| {
                format!("imported-{}", &cred.access_token[..8.min(cred.access_token.len())])
            });

            let expires_at = cred
                .expires_at_ms
                .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms));

            let oauth = OAuthData {
                access_token: cred.access_token.clone(),
                refresh_token: cred.refresh_token.clone(),
                expires_at,
                token_type: Some("Bearer".to_string()),
                scope: None,
                scopes: None,
                refresh_token_expires_at: None,
                organization_uuid: None,
            };

            let existing = data.accounts.entry(key.clone()).or_insert_with(AccountData::default);
            if existing.claude_ai_oauth.is_none() || existing.email.is_none() {
                existing.email = cred.email;
                existing.name = cred.name;
                existing.provider = cred.provider.or(Some("anthropic".to_string()));
                existing.claude_ai_oauth = Some(oauth.clone());
                existing.oauth = Some(oauth);
                imported += 1;
            }
        }
    }
    let _ = state.credentials.persist();
    Ok(imported)
}

// ---------------------------------------------------------------------------
// Profiles (Phase 6.2)
// ---------------------------------------------------------------------------

/// Helper — construit le répertoire des profils.
fn profiles_dir() -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".claude").join("multi-account").join("profiles")
}

/// DTO pour un profil retourné au frontend.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfoDto {
    pub name: String,
    pub created_at: String,
    pub size_bytes: u64,
}

/// Liste tous les profils disponibles.
#[tauri::command]
pub async fn list_profiles() -> Result<Vec<ProfileInfoDto>, String> {
    use ai_manager_core::profiles::ProfileManager;
    let mgr = ProfileManager::new(&profiles_dir());
    let list = mgr.list().map_err(|e| e.to_string())?;
    let dtos = list
        .into_iter()
        .map(|p| ProfileInfoDto {
            name: p.name,
            created_at: p.created_at.to_rfc3339(),
            size_bytes: p.size_bytes,
        })
        .collect();
    Ok(dtos)
}

/// Sauvegarde un profil de configuration.
#[tauri::command]
pub async fn save_profile(name: String, config: serde_json::Value) -> Result<(), String> {
    use ai_manager_core::profiles::ProfileManager;
    let mgr = ProfileManager::new(&profiles_dir());
    mgr.save(&name, &config).map_err(|e| e.to_string())
}

/// Charge un profil de configuration.
#[tauri::command]
pub async fn load_profile(name: String) -> Result<serde_json::Value, String> {
    use ai_manager_core::profiles::ProfileManager;
    let mgr = ProfileManager::new(&profiles_dir());
    mgr.load(&name).map_err(|e| e.to_string())
}

/// Supprime un profil de configuration.
#[tauri::command]
pub async fn delete_profile(name: String) -> Result<(), String> {
    use ai_manager_core::profiles::ProfileManager;
    let mgr = ProfileManager::new(&profiles_dir());
    mgr.delete(&name).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Stats (Phase 6.6)
// ---------------------------------------------------------------------------

/// Helper — construit le répertoire multi-account.
fn multi_account_base_dir() -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".claude").join("multi-account")
}

/// DTO des stats retourné au frontend.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatsDto {
    pub total_switches: u64,
    pub switches_by_account: std::collections::HashMap<String, u64>,
    pub total_requests: u64,
    pub last_switch_at: Option<String>,
    pub uptime_started_at: String,
}

/// Retourne les statistiques d'utilisation persistées.
#[tauri::command]
pub async fn get_stats() -> Result<StatsDto, String> {
    use ai_manager_core::stats::StatsManager;
    let mgr = StatsManager::new(&multi_account_base_dir());
    let stats = mgr.load();
    Ok(StatsDto {
        total_switches: stats.total_switches,
        switches_by_account: stats.switches_by_account,
        total_requests: stats.total_requests,
        last_switch_at: stats.last_switch_at.map(|dt| dt.to_rfc3339()),
        uptime_started_at: stats.uptime_started_at.to_rfc3339(),
    })
}

// ---------------------------------------------------------------------------
// Systemd integration
// ---------------------------------------------------------------------------

/// Retourne le statut du service systemd `ai-manager-daemon`.
///
/// Valeurs possibles :
/// - `"active"` — le service tourne
/// - `"inactive"` — le service existe mais est arrêté
/// - `"not-found"` — le service n'est pas installé
/// - `"unavailable"` — systemd n'est pas disponible sur cette plateforme
/// - autre chaîne — sortie brute de `systemctl is-active` (ex. "failed")
#[tauri::command]
pub async fn get_systemd_status() -> Result<String, String> {
    #[cfg(unix)]
    {
        let output = std::process::Command::new("systemctl")
            .args(["is-active", "ai-manager-daemon"])
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                // systemctl is-active retourne "active", "inactive", "failed", etc.
                // Si le service n'existe pas, la sortie est "inactive" avec exit code ≠ 0.
                // On distingue "not-found" en vérifiant stderr.
                let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
                if stderr.contains("not found") || stderr.contains("no such") {
                    Ok("not-found".to_string())
                } else if stdout.is_empty() {
                    // systemctl absent ou inaccessible
                    Ok("unavailable".to_string())
                } else {
                    Ok(stdout)
                }
            }
            Err(e) => {
                // systemctl introuvable (pas de systemd sur ce système)
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok("unavailable".to_string())
                } else {
                    Err(format!("systemctl error: {}", e))
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        Ok("unavailable".to_string())
    }
}

/// Installe le service systemd user `ai-manager-daemon`.
///
/// Crée le fichier `~/.config/systemd/user/ai-manager-daemon.service`,
/// fait `daemon-reload` et `enable --now` le service.
///
/// `daemon_path` : chemin absolu vers le binaire ai-manager-daemon.
/// Si None, tente de le trouver automatiquement.
#[tauri::command]
pub async fn install_systemd_service(
    daemon_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    #[cfg(unix)]
    {
        // Resolve daemon binary path
        let bin = if let Some(p) = daemon_path {
            std::path::PathBuf::from(p)
        } else {
            // Try common locations
            let candidates = [
                dirs::home_dir().map(|h| h.join(".cargo/bin/ai-manager-daemon")),
                Some(std::path::PathBuf::from("/usr/local/bin/ai-manager-daemon")),
                Some(std::path::PathBuf::from("/usr/bin/ai-manager-daemon")),
            ];
            candidates.iter()
                .filter_map(|c| c.as_ref())
                .find(|p| p.exists())
                .cloned()
                .ok_or_else(|| "Binaire ai-manager-daemon introuvable. Specifiez le chemin manuellement.".to_string())?
        };

        if !bin.exists() {
            return Err(format!("Binaire introuvable: {}", bin.display()));
        }

        // Build settings.json path
        let settings_path = state.config.path();

        // Generate systemd unit file
        let unit = format!(
            r#"[Unit]
Description=AI Manager Daemon - Multi-Account Manager
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={bin} --settings {settings}
Restart=on-failure
RestartSec=10
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
"#,
            bin = bin.display(),
            settings = settings_path.display(),
        );

        // Write unit file
        let systemd_dir = dirs::home_dir()
            .ok_or_else(|| "Impossible de determiner le repertoire home".to_string())?
            .join(".config/systemd/user");
        std::fs::create_dir_all(&systemd_dir)
            .map_err(|e| format!("Impossible de creer {}: {}", systemd_dir.display(), e))?;

        let unit_path = systemd_dir.join("ai-manager-daemon.service");
        std::fs::write(&unit_path, &unit)
            .map_err(|e| format!("Impossible d'ecrire {}: {}", unit_path.display(), e))?;

        info!("Systemd unit written to {}", unit_path.display());

        // daemon-reload
        let reload = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output()
            .map_err(|e| format!("systemctl daemon-reload echoue: {}", e))?;
        if !reload.status.success() {
            let stderr = String::from_utf8_lossy(&reload.stderr);
            return Err(format!("daemon-reload echoue: {}", stderr));
        }

        // enable --now
        let enable = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", "ai-manager-daemon"])
            .output()
            .map_err(|e| format!("systemctl enable echoue: {}", e))?;
        if !enable.status.success() {
            let stderr = String::from_utf8_lossy(&enable.stderr);
            return Err(format!("enable --now echoue: {}", stderr));
        }

        // Enable lingering so the user service starts at boot even without login
        let _ = std::process::Command::new("loginctl")
            .args(["enable-linger"])
            .output();

        info!("Systemd service installed and started");
        Ok(format!("Service installe et demarre: {}", unit_path.display()))
    }

    #[cfg(not(unix))]
    {
        let _ = (daemon_path, state);
        Err("Systemd n'est pas disponible sur cette plateforme".to_string())
    }
}

/// Désinstalle le service systemd user `ai-manager-daemon`.
///
/// Stop + disable le service, puis supprime le fichier unit.
#[tauri::command]
pub async fn uninstall_systemd_service() -> Result<String, String> {
    #[cfg(unix)]
    {
        // stop + disable
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "stop", "ai-manager-daemon"])
            .output();
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "ai-manager-daemon"])
            .output();

        // Remove unit file
        let unit_path = dirs::home_dir()
            .ok_or_else(|| "Impossible de determiner le repertoire home".to_string())?
            .join(".config/systemd/user/ai-manager-daemon.service");

        if unit_path.exists() {
            std::fs::remove_file(&unit_path)
                .map_err(|e| format!("Impossible de supprimer {}: {}", unit_path.display(), e))?;
        }

        // daemon-reload
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();

        info!("Systemd service uninstalled");
        Ok("Service desinstalle".to_string())
    }

    #[cfg(not(unix))]
    {
        Err("Systemd n'est pas disponible sur cette plateforme".to_string())
    }
}

// ---------------------------------------------------------------------------
// Phase 2.1 — OAuth capture via Claude CLI
// ---------------------------------------------------------------------------

/// Find the `claude` CLI binary on the system.
///
/// Returns the resolved path, or an error message if not found.
#[tauri::command]
pub async fn find_claude_binary() -> Result<String, String> {
    ai_manager_core::capture::find_claude_binary(None)
        .await
        .ok_or_else(|| {
            "Claude CLI introuvable dans le PATH. Installez-le depuis https://claude.ai/download".to_string()
        })
}

/// Launch `claude setup-token`, capture the OAuth token from its output, and
/// (on success) automatically add the account to the credentials store.
#[tauri::command]
pub async fn capture_oauth_token(
    timeout_secs: Option<u64>,
    state: State<'_, AppState>,
) -> Result<ai_manager_core::capture::CaptureResult, String> {
    let secs = timeout_secs.unwrap_or(60);
    let result = ai_manager_core::capture::capture_claude_token(None, secs).await;

    // If we captured a token, automatically add the account
    if result.success {
        if let Some(ref token) = result.access_token {
            let key = result
                .email
                .clone()
                .unwrap_or_else(|| format!("captured-{}", &token[..8.min(token.len())]));

            let oauth = OAuthData {
                access_token: token.clone(),
                refresh_token: result.refresh_token.clone().unwrap_or_else(|| token.clone()),
                expires_at: None,
                token_type: Some("Bearer".to_string()),
                scope: None,
                scopes: None,
                refresh_token_expires_at: None,
                organization_uuid: None,
            };

            {
                let mut data = state.credentials.write();
                let existing = data
                    .accounts
                    .entry(key.clone())
                    .or_insert_with(AccountData::default);
                // Only overwrite if the slot is empty (never stomp existing data)
                if existing.claude_ai_oauth.is_none() {
                    existing.email = result.email.clone();
                    existing.name = result.email.clone();
                    existing.provider = Some("anthropic".to_string());
                    existing.claude_ai_oauth = Some(oauth.clone());
                    existing.oauth = Some(oauth);
                    tracing::info!("capture_oauth_token: added account '{}'", key);
                } else {
                    tracing::debug!(
                        "capture_oauth_token: account '{}' already exists, skipping auto-add",
                        key
                    );
                }
            }
            let _ = state.credentials.persist();
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Webhook testing
// ---------------------------------------------------------------------------

/// Envoie un message de test vers un webhook Discord/Slack/Generic.
///
/// Paramètres :
/// - `url`  : URL complète du webhook
/// - `kind` : "discord" | "slack" | "generic"
///
/// Retourne `Ok(())` en cas de succès, ou `Err(message)` en cas d'erreur.
#[tauri::command]
pub async fn test_webhook(url: String, kind: String) -> Result<(), String> {
    use ai_manager_core::webhook::{WebhookKind, WebhookTarget, WebhookSender, WebhookEvent};

    // P4 — Valider l'URL pour prévenir le SSRF avant toute requête sortante.
    validate_webhook_url(&url)?;

    let webhook_kind = match kind.to_lowercase().as_str() {
        "discord" => WebhookKind::Discord,
        "slack"   => WebhookKind::Slack,
        _         => WebhookKind::Generic,
    };

    let target = WebhookTarget {
        url: url.clone(),
        kind: webhook_kind,
        events: vec![],  // pas de filtre pour le test
    };

    let sender = WebhookSender::new(vec![target]);
    sender.send(WebhookEvent::QuotaWarning {
        key: "test-account".to_string(),
        pct: 42.0,
        phase: "Test".to_string(),
    }).await;

    info!("test_webhook: message envoyé à {}", url);
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 3.4a — Capture token roté avant switch (commande frontale)
// ---------------------------------------------------------------------------

/// Capture le token roté du compte sortant avant un switch manuel.
///
/// Appelle `CredentialsCache::capture_rotated_tokens_before_switch(outgoing_key)`.
/// Retourne `true` si un token roté a été détecté et importé, `false` sinon.
///
/// Cette commande est silencieuse sur les erreurs non critiques (fichier absent,
/// parse raté) : elle retourne `Ok(false)` plutôt que de propager une erreur
/// bloquante. Les erreurs critiques sont tout de même remontées en `Err(String)`.
#[tauri::command]
pub async fn capture_before_switch(
    outgoing_key: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    state
        .credentials
        .capture_rotated_tokens_before_switch(&outgoing_key)
        .map_err(|e| format!("capture_before_switch failed for '{}': {}", outgoing_key, e))
}
