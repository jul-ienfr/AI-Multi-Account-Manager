//! Handlers REST — gestion des comptes.
//!
//! 9 handlers : list, get_active, add, update, delete, switch, refresh, revoke,
//! capture_before_switch.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use chrono::DateTime;
use serde_json::json;

use ai_core::credentials::{AccountData, OAuthData};
use ai_core::oauth::{refresh_oauth_token, revoke_token, RefreshResult};

use crate::dto::{
    AccountDataDto, AccountStateDto, AddAccountData, CaptureBeforeSwitchData, OAuthSlotDto,
    QuotaDto, UpdateAccountData,
};
use super::{DaemonState, error_json, ok_json};

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn account_to_dto(account: &AccountData) -> AccountDataDto {
    AccountDataDto {
        email: account.email.clone(),
        name: account.name.clone(),
        display_name: account.display_name.clone(),
        account_type: account.account_type.clone(),
        provider: account.provider.clone(),
        priority: account.priority,
        plan_type: account.plan_type.clone(),
        claude_ai_oauth: account.claude_ai_oauth.as_ref().map(oauth_to_dto),
        setup_token: account.setup_token.as_ref().map(oauth_to_dto),
        gemini_cli_oauth: account.gemini_cli_oauth.as_ref().map(oauth_to_dto),
        api_key: account.api_key.clone(),
        api_url: account.api_url.clone(),
        auto_switch_disabled: account.auto_switch_disabled.unwrap_or(false),
        tokens_5h: account.tokens_5h,
        tokens_7d: account.tokens_7d,
        deleted: account.deleted,
    }
}

fn oauth_to_dto(oauth: &OAuthData) -> OAuthSlotDto {
    OAuthSlotDto {
        access_token: oauth.access_token.chars().take(16).collect(),
        refresh_token: "***".to_string(),
        expires_at: oauth.expires_at.map(|dt| dt.timestamp_millis()),
    }
}

fn quota_dto_from_state(state: &DaemonState, key: &str) -> Option<QuotaDto> {
    let metrics = state.quota_metrics.read();
    metrics.get(key).map(|m| QuotaDto {
        tokens_5h: 0,
        limit_5h: 0,
        tokens_7d: 0,
        limit_7d: 0,
        phase: String::new(),
        ema_velocity: m.ema_velocity,
        time_to_threshold: m.time_to_threshold,
        resets_at_5h: m.resets_at_5h.clone(),
        resets_at_7d: m.resets_at_7d.clone(),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// 1. list_accounts
// ---------------------------------------------------------------------------

pub async fn list_accounts(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let data = state.credentials.read();
    let active_key = data.active_account.as_deref();

    let accounts: Vec<AccountStateDto> = data
        .accounts
        .iter()
        .filter(|(_, account)| !account.deleted)
        .map(|(key, account)| AccountStateDto {
            key: key.clone(),
            data: account_to_dto(account),
            quota: quota_dto_from_state(&state, key),
            is_active: active_key == Some(key.as_str()),
        })
        .collect();

    ok_json(accounts)
}

// ---------------------------------------------------------------------------
// 2. get_active
// ---------------------------------------------------------------------------

pub async fn get_active(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let data = state.credentials.read();
    let result: Option<AccountStateDto> = data
        .active_account
        .as_ref()
        .and_then(|key| {
            data.accounts.get(key).map(|account| AccountStateDto {
                key: key.clone(),
                data: account_to_dto(account),
                quota: quota_dto_from_state(&state, key),
                is_active: true,
            })
        });

    ok_json(result)
}

// ---------------------------------------------------------------------------
// 3. add_account
// ---------------------------------------------------------------------------

pub async fn add_account(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<AddAccountData>,
) -> impl IntoResponse {
    let key = body
        .email
        .clone()
        .unwrap_or_else(|| format!("account-{}", &uuid::Uuid::new_v4().to_string()[..8]));

    let mut account = AccountData {
        email: body.email,
        name: body.name,
        display_name: body.display_name,
        account_type: body.account_type,
        provider: body.provider,
        priority: if body.priority > 0 { Some(body.priority) } else { None },
        api_url: body.api_url,
        ..AccountData::default()
    };

    // OAuth access token fourni ?
    if let Some(access_token) = body.access_token {
        let expires_at = body.expires_at.and_then(DateTime::from_timestamp_millis);
        let oauth = OAuthData {
            access_token,
            refresh_token: body.refresh_token.unwrap_or_default(),
            expires_at,
            token_type: Some("Bearer".to_string()),
            scope: None,
            scopes: None,
            refresh_token_expires_at: None,
            organization_uuid: None,
        };
        account.claude_ai_oauth = Some(oauth.clone());
        account.oauth = Some(oauth);
    }

    // API key fournie ?
    if let Some(api_key) = body.api_key {
        account.api_key = Some(serde_json::Value::String(api_key));
    }

    state.credentials.write().accounts.insert(key, account);

    state
        .credentials
        .persist()
        .map_err(|e| error_json(500, &e.to_string()))
        .map(|_| ok_json(json!({"ok": true})))
        .unwrap_or_else(|e| e)
}

// ---------------------------------------------------------------------------
// 4. update_account
// ---------------------------------------------------------------------------

pub async fn update_account(
    State(state): State<Arc<DaemonState>>,
    Path(key): Path<String>,
    Json(body): Json<UpdateAccountData>,
) -> impl IntoResponse {
    {
        let mut data = state.credentials.write();
        let account = match data.accounts.get_mut(&key) {
            Some(a) => a,
            None => return error_json(404, "account not found"),
        };

        if let Some(priority) = body.priority {
            account.priority = Some(priority);
        }
        if let Some(disabled) = body.auto_switch_disabled {
            account.auto_switch_disabled = Some(disabled);
        }
        if let Some(display_name) = body.display_name {
            account.display_name = Some(display_name);
        }
    }

    state
        .credentials
        .persist()
        .map_err(|e| error_json(500, &e.to_string()))
        .map(|_| ok_json(json!({"ok": true})))
        .unwrap_or_else(|e| e)
}

// ---------------------------------------------------------------------------
// 5. delete_account
// ---------------------------------------------------------------------------

pub async fn delete_account(
    State(state): State<Arc<DaemonState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    {
        let mut data = state.credentials.write();

        if !data.accounts.contains_key(&key) {
            return error_json(404, "account not found");
        }

        // Mark the account as deleted
        if let Some(account) = data.accounts.get_mut(&key) {
            account.deleted = true;
        }

        // Clear active account if it's the one being deleted
        if data.active_account.as_deref() == Some(key.as_str()) {
            data.active_account = None;
        }
    }

    state
        .credentials
        .persist()
        .map_err(|e| error_json(500, &e.to_string()))
        .map(|_| ok_json(json!({"ok": true})))
        .unwrap_or_else(|e| e)
}

// ---------------------------------------------------------------------------
// 6. switch_account
// ---------------------------------------------------------------------------

pub async fn switch_account(
    State(state): State<Arc<DaemonState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    // Verify account exists and is not deleted
    {
        let data = state.credentials.read();
        match data.accounts.get(&key) {
            Some(account) if account.deleted => {
                return error_json(404, "account not found or deleted");
            }
            None => return error_json(404, "account not found"),
            _ => {}
        }
    }

    // Capture rotated tokens from the outgoing active account before switching
    if let Some(active_key) = state.credentials.active_key() {
        if active_key != key {
            if let Err(e) = state.credentials.capture_rotated_tokens_before_switch(&active_key) {
                tracing::warn!("capture_rotated_tokens_before_switch failed: {}", e);
            }
        }
    }

    // Update active account
    state.credentials.write().active_account = Some(key.clone());

    state
        .credentials
        .persist()
        .map_err(|e| error_json(500, &e.to_string()))
        .map(|_| ok_json(json!({"ok": true, "switched_to": key})))
        .unwrap_or_else(|e| e)
}

// ---------------------------------------------------------------------------
// 7. refresh_account
// ---------------------------------------------------------------------------

pub async fn refresh_account(
    State(state): State<Arc<DaemonState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    // Get best available OAuth for this account
    let refresh_token = {
        let account = match state.credentials.get_account(&key) {
            Some(a) => a,
            None => return error_json(404, "account not found"),
        };
        match account.get_best_oauth() {
            Some(oauth) => oauth.refresh_token.clone(),
            None => return error_json(422, "no oauth token available for this account"),
        }
    };

    match refresh_oauth_token(&state.http_client, &refresh_token).await {
        RefreshResult::Ok(new_oauth) => {
            if let Err(e) = state.credentials.update_oauth(&key, new_oauth) {
                return error_json(500, &e.to_string());
            }
            ok_json(json!({"ok": true}))
        }
        RefreshResult::InvalidGrant => {
            state.invalid_grant_accounts.write().insert(key.clone());
            error_json(422, "invalid_grant")
        }
        RefreshResult::Expired => {
            error_json(502, "token_expired")
        }
        RefreshResult::NetworkError(msg) => {
            error_json(502, &format!("refresh failed: {}", msg))
        }
    }
}

// ---------------------------------------------------------------------------
// 8. revoke_account
// ---------------------------------------------------------------------------

pub async fn revoke_account(
    State(state): State<Arc<DaemonState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    // Get access token
    let access_token = {
        let account = match state.credentials.get_account(&key) {
            Some(a) => a,
            None => return error_json(404, "account not found"),
        };
        match account.get_best_oauth() {
            Some(oauth) => oauth.access_token.clone(),
            None => return error_json(422, "no oauth token available for this account"),
        }
    };

    // Revoke the token
    if let Err(e) = revoke_token(&state.http_client, &access_token).await {
        return error_json(502, &e.to_string());
    }

    // Mark account as deleted
    {
        let mut data = state.credentials.write();
        if let Some(account) = data.accounts.get_mut(&key) {
            account.deleted = true;
        }
        if data.active_account.as_deref() == Some(key.as_str()) {
            data.active_account = None;
        }
    }

    state
        .credentials
        .persist()
        .map_err(|e| error_json(500, &e.to_string()))
        .map(|_| ok_json(json!({"ok": true})))
        .unwrap_or_else(|e| e)
}

// ---------------------------------------------------------------------------
// 9. capture_before_switch
// ---------------------------------------------------------------------------

pub async fn capture_before_switch(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<CaptureBeforeSwitchData>,
) -> impl IntoResponse {
    let result = state
        .credentials
        .capture_rotated_tokens_before_switch(&body.outgoing_key)
        .unwrap_or(false);

    ok_json(json!({"found": result}))
}
