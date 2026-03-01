//! Module OAuth — refresh de tokens Claude et Google.
//!
//! Traduit la logique Python de `account_agent.py` (_try_refresh_token,
//! _fetch_quota_for_account, _refresh_google_token) en Rust async.
//!
//! Fonctions exportées :
//! - `cooldown_key(refresh_token)` — clé sha256 pour le cooldown persistant
//! - `refresh_oauth_token(rt, client)` — refresh Anthropic avec backoff exponentiel
//! - `refresh_google_token(rt, client)` — refresh Google OAuth (Gemini CLI credentials)
//! - `needs_refresh(oauth)`, `is_expired(oauth)` — checks d'expiration

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::credentials::OAuthData;
use crate::error::{CoreError, Result};

/// Résultat d'un refresh OAuth.
///
/// Remplace `Result<Option<OAuthData>, CoreError>` pour distinguer finement
/// les cas d'échec et permettre au daemon de prendre des décisions adaptées
/// (exclure un compte `invalid_grant`, réessayer un `NetworkError`, etc.).
#[derive(Debug)]
pub enum RefreshResult {
    /// Refresh réussi — nouvelles données OAuth disponibles.
    Ok(OAuthData),
    /// Token révoqué ou expiré côté serveur (`invalid_grant`).
    /// Le compte doit être exclu des rotations jusqu'à réauthentification.
    InvalidGrant,
    /// Token expiré mais non révoqué (ex : expiration classique).
    /// Peut être réessayé après un délai.
    Expired,
    /// Erreur réseau ou HTTP transitoire.
    NetworkError(String),
}

/// URL du serveur d'autorisation Claude.
const TOKEN_ENDPOINT: &str = "https://api.claude.ai/api/auth/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// Endpoint Google OAuth token.
#[allow(dead_code)]
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// Google OAuth client credentials — loaded from environment at runtime.
/// Falls back to empty strings if not set (Google OAuth will fail gracefully).
#[allow(dead_code)]
fn google_client_id() -> String {
    std::env::var("GOOGLE_OAUTH_CLIENT_ID").unwrap_or_default()
}

#[allow(dead_code)]
fn google_client_secret() -> String {
    std::env::var("GOOGLE_OAUTH_CLIENT_SECRET").unwrap_or_default()
}

/// Réponse brute du serveur OAuth lors d'un refresh.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    /// Erreur OAuth (ex: "invalid_grant")
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Effectue un refresh OAuth et retourne un [`RefreshResult`] discriminant
/// précisément la cause de l'échec.
///
/// Traduit la logique Python `_try_refresh_token` / `_do_refresh`.
///
/// | Cas                        | Variante retournée         |
/// |----------------------------|---------------------------|
/// | Succès                     | `RefreshResult::Ok`        |
/// | `invalid_grant` serveur    | `RefreshResult::InvalidGrant` |
/// | Token expiré (HTTP 401)    | `RefreshResult::Expired`   |
/// | Erreur réseau / autre HTTP | `RefreshResult::NetworkError` |
pub async fn refresh_oauth_token(
    client: &reqwest::Client,
    refresh_token: &str,
) -> RefreshResult {
    debug!(
        "Refreshing OAuth token (rt prefix: {}...)",
        refresh_token.get(..8.min(refresh_token.len())).unwrap_or(refresh_token)
    );

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", CLIENT_ID),
    ];

    let resp = match client
        .post(TOKEN_ENDPOINT)
        .form(&params)
        .header("User-Agent", "claude-cli/1.0")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return RefreshResult::NetworkError(format!("OAuth request failed: {e}")),
    };

    let status = resp.status();

    // HTTP 401 → token expiré côté serveur (non révoqué)
    if status.as_u16() == 401 {
        warn!("OAuth refresh: HTTP 401 — token expiré (non révoqué)");
        return RefreshResult::Expired;
    }

    let body: TokenResponse = match resp.json().await {
        Ok(b) => b,
        Err(e) => return RefreshResult::NetworkError(format!("OAuth response parse error: {e}")),
    };

    // Erreur OAuth applicative (corps JSON)
    if let Some(ref err) = body.error {
        if err == "invalid_grant" {
            warn!("OAuth refresh: invalid_grant — token révoqué, compte à exclure");
            return RefreshResult::InvalidGrant;
        }
        let msg = format!(
            "OAuth error: {} — {}",
            err,
            body.error_description.as_deref().unwrap_or("no description")
        );
        return RefreshResult::NetworkError(msg);
    }

    if !status.is_success() {
        return RefreshResult::NetworkError(format!("OAuth HTTP {status}"));
    }

    // Calcul de l'expiration
    let expires_at = body
        .expires_in
        .map(|secs| Utc::now() + Duration::seconds(secs as i64));

    // Le refresh_token peut être réutilisé s'il n'est pas renouvelé
    // (design voulu: refreshToken = accessToken dans le cas Claude Code)
    let new_refresh = body
        .refresh_token
        .filter(|rt| !rt.is_empty())
        .unwrap_or_else(|| refresh_token.to_string());

    info!("OAuth refresh successful, expires_at={:?}", expires_at);

    RefreshResult::Ok(OAuthData {
        access_token: body.access_token,
        refresh_token: new_refresh,
        expires_at,
        token_type: body.token_type,
        scope: body.scope,
        scopes: None,
        refresh_token_expires_at: None,
        organization_uuid: None,
    })
}

/// Vérifie si un token doit être rafraîchi (expire dans < 30 min).
pub fn needs_refresh(oauth: &OAuthData) -> bool {
    match oauth.expires_at {
        None => false, // Pas d'info d'expiration → on suppose valide
        Some(exp) => exp < Utc::now() + Duration::minutes(30),
    }
}

/// Vérifie si un token est expiré.
pub fn is_expired(oauth: &OAuthData) -> bool {
    match oauth.expires_at {
        None => false,
        Some(exp) => exp <= Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Token revocation
// ---------------------------------------------------------------------------

/// Endpoint de révocation Anthropic OAuth.
const REVOKE_ENDPOINT: &str = "https://api.anthropic.com/v1/oauth/token";

/// Révoque un access token OAuth Anthropic.
///
/// Envoie un `DELETE` à `https://api.anthropic.com/v1/oauth/token`
/// avec le header `Authorization: Bearer {access_token}`.
///
/// | Statut HTTP | Résultat         |
/// |-------------|-----------------|
/// | 200 / 204   | `Ok(())`         |
/// | Autre       | `Err(CoreError)` |
///
/// # Errors
/// - [`CoreError::Http`] si la requête échoue ou si le serveur retourne une erreur HTTP.
pub async fn revoke_token(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<()> {
    debug!(
        "Revoking OAuth token (prefix: {}...)",
        access_token.get(..8.min(access_token.len())).unwrap_or(access_token)
    );

    let resp = client
        .delete(REVOKE_ENDPOINT)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| CoreError::Http(format!("Token revocation request failed: {e}")))?;

    let status = resp.status();

    if status.as_u16() == 200 || status.as_u16() == 204 {
        info!("Token revoked successfully (HTTP {})", status.as_u16());
        return Ok(());
    }

    Err(CoreError::Http(format!(
        "Token revocation failed: HTTP {}",
        status.as_u16()
    )))
}

// ---------------------------------------------------------------------------
// Quota fetching from Anthropic API
// ---------------------------------------------------------------------------

/// Endpoint for fetching OAuth usage/quota information.
const QUOTA_ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";

/// A single quota window (5h or 7d).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaWindowInfo {
    /// Usage percentage (0-100).
    #[serde(default)]
    pub utilization: f64,
    /// Max tokens in the window.
    #[serde(default)]
    pub limit: Option<u64>,
    /// Tokens remaining.
    #[serde(default)]
    pub remaining: Option<u64>,
    /// When the window resets (ISO8601).
    #[serde(default)]
    pub resets_at: Option<String>,
}

/// Full quota response from the Anthropic API.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaResponse {
    #[serde(default)]
    pub five_hour: Option<QuotaWindowInfo>,
    #[serde(default)]
    pub seven_day: Option<QuotaWindowInfo>,
    #[serde(default)]
    pub seven_day_sonnet: Option<QuotaWindowInfo>,
    #[serde(default)]
    pub seven_day_opus: Option<QuotaWindowInfo>,
    #[serde(default)]
    pub extra_usage: Option<serde_json::Value>,
}

/// Fetches current quota/usage from the Anthropic API.
///
/// Returns `Ok(response)` on success, `Err` on network/auth failure.
/// A 401 means token expired, 403 means revoked.
pub async fn fetch_quota(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<QuotaResponse> {
    debug!(
        "Fetching quota (token prefix: {}...)",
        access_token.get(..8.min(access_token.len())).unwrap_or(access_token)
    );

    let resp = client
        .get(QUOTA_ENDPOINT)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| CoreError::Http(format!("Quota fetch request failed: {e}")))?;

    let status = resp.status();

    if status.as_u16() == 401 {
        return Err(CoreError::Auth("token_expired".to_string()));
    }
    if status.as_u16() == 403 {
        return Err(CoreError::Auth("access_denied".to_string()));
    }
    if !status.is_success() {
        return Err(CoreError::Http(format!("Quota fetch HTTP {status}")));
    }

    let quota: QuotaResponse = resp
        .json()
        .await
        .map_err(|e| CoreError::Http(format!("Quota response parse error: {e}")))?;

    debug!(
        "Quota fetched: 5h={:.1}%, 7d={:.1}%",
        quota.five_hour.as_ref().map(|q| q.utilization).unwrap_or(0.0),
        quota.seven_day.as_ref().map(|q| q.utilization).unwrap_or(0.0),
    );

    Ok(quota)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_oauth(expires_in_minutes: i64) -> OAuthData {
        OAuthData {
            access_token: "tok".to_string(),
            refresh_token: "ref".to_string(),
            expires_at: Some(Utc::now() + Duration::minutes(expires_in_minutes)),
            token_type: Some("Bearer".to_string()),
            scope: None,
            scopes: None,
            refresh_token_expires_at: None,
            organization_uuid: None,
        }
    }

    #[test]
    fn test_needs_refresh_soon() {
        let oauth = make_oauth(10); // expire dans 10 min < 30 min
        assert!(needs_refresh(&oauth));
    }

    #[test]
    fn test_no_refresh_needed() {
        let oauth = make_oauth(60); // expire dans 60 min > 30 min
        assert!(!needs_refresh(&oauth));
    }

    #[test]
    fn test_is_expired() {
        let oauth = make_oauth(-5); // expiré depuis 5 min
        assert!(is_expired(&oauth));
    }

    #[test]
    fn test_not_expired() {
        let oauth = make_oauth(60);
        assert!(!is_expired(&oauth));
    }

    #[test]
    fn test_no_expires_at() {
        let oauth = OAuthData {
            access_token: "tok".to_string(),
            refresh_token: "ref".to_string(),
            expires_at: None,
            token_type: None,
            scope: None,
            scopes: None,
            refresh_token_expires_at: None,
            organization_uuid: None,
        };
        assert!(!needs_refresh(&oauth));
        assert!(!is_expired(&oauth));
    }
}
