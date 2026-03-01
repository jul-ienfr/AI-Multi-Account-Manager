//! Handlers credentials — scan, import, find binary, capture token.

use std::sync::Arc;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;
use ai_core::credentials::{scan_local_credentials, AccountData, OAuthData};
use ai_core::capture::{find_claude_binary, capture_claude_token};
use crate::dto::{CaptureTokenData, ImportCredentialsData, ImportResult};
use super::{DaemonState, error_json, ok_json};

// ---------------------------------------------------------------------------
// scan_creds
// ---------------------------------------------------------------------------

/// `POST /credentials/scan` — scanne le filesystem local pour des credentials.
pub async fn scan_creds() -> impl IntoResponse {
    let creds = scan_local_credentials();
    ok_json(creds)
}

// ---------------------------------------------------------------------------
// import_creds
// ---------------------------------------------------------------------------

/// `POST /credentials/import` — importe des credentials scannés dans le cache.
pub async fn import_creds(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ImportCredentialsData>,
) -> impl IntoResponse {
    let mut count = 0usize;

    {
        let mut data = state.credentials.write();

        for cred in body.credentials {
            // Clé : email si disponible, sinon uuid tronqué
            let key = if let Some(ref email) = cred.email {
                email.clone()
            } else {
                format!("account-{}", &uuid::Uuid::new_v4().to_string()[..8])
            };

            // N'insère que si la clé est absente
            if data.accounts.contains_key(&key) {
                continue;
            }

            let expires_at = cred.expires_at_ms.and_then(|ms| {
                chrono::DateTime::from_timestamp_millis(ms)
            });

            let oauth = OAuthData {
                access_token: cred.access_token.clone(),
                refresh_token: cred.refresh_token.clone(),
                expires_at,
                token_type: None,
                scope: None,
                scopes: None,
                refresh_token_expires_at: None,
                organization_uuid: None,
            };

            let mut account = AccountData::default();
            account.email = cred.email.clone();
            account.name = cred.name.clone();
            account.provider = cred.provider.clone();
            account.claude_ai_oauth = Some(oauth.clone());
            account.oauth = Some(oauth);

            data.accounts.insert(key, account);
            count += 1;
        }
    }

    let _ = state.credentials.persist();

    ok_json(ImportResult { imported: count })
}

// ---------------------------------------------------------------------------
// find_binary
// ---------------------------------------------------------------------------

/// `GET /credentials/binary` — localise le binaire Claude CLI.
pub async fn find_binary() -> impl IntoResponse {
    match find_claude_binary(None).await {
        Some(path) => ok_json(json!({"path": path})),
        None => error_json(404, "Claude CLI binary not found"),
    }
}

// ---------------------------------------------------------------------------
// capture_token
// ---------------------------------------------------------------------------

/// `POST /credentials/capture` — lance `claude setup-token` et capture le token OAuth.
pub async fn capture_token(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<CaptureTokenData>,
) -> impl IntoResponse {
    let secs = body.timeout_secs.unwrap_or(30);
    let result = capture_claude_token(None, secs).await;

    // Auto-ajoute le compte si un token a été capturé et qu'un slot vide existe
    if result.access_token.is_some() {
        let key = if let Some(ref email) = result.email {
            email.clone()
        } else {
            format!("account-{}", &uuid::Uuid::new_v4().to_string()[..8])
        };

        {
            let mut data = state.credentials.write();

            if !data.accounts.contains_key(&key) {
                let oauth = OAuthData {
                    access_token: result.access_token.clone().unwrap_or_default(),
                    refresh_token: result.refresh_token.clone().unwrap_or_default(),
                    expires_at: None,
                    token_type: None,
                    scope: None,
                    scopes: None,
                    refresh_token_expires_at: None,
                    organization_uuid: None,
                };

                let mut account = AccountData::default();
                account.email = result.email.clone();
                account.provider = Some("anthropic".to_string());
                account.claude_ai_oauth = Some(oauth.clone());
                account.oauth = Some(oauth);

                data.accounts.insert(key, account);
            }
        }

        let _ = state.credentials.persist();
    }

    ok_json(&result)
}
