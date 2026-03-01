//! API HTTP REST du daemon — Axum 0.8.
//!
//! Expose 57 endpoints sous `/admin/api/` répliquant les commandes Tauri.
//! Auth optionnelle via Bearer token (`Authorization: Bearer <token>`).
//! Endpoint santé `GET /admin/api/health` exempt d'auth.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::Response;
use axum::middleware::Next;
use axum::response::{IntoResponse, Json};
use axum::routing::{delete, get, post, put};
use axum::Router;
use parking_lot::RwLock;
use serde_json::json;
use tokio::sync::watch;
use tracing::info;

use ai_core::config::ConfigCache;
use ai_core::credentials::CredentialsCache;
use ai_core::event_log::EventLog;
use ai_core::quota::VelocityCalculator;
use ai_core::types::{Peer, ProxyInstanceRuntime, QuotaMetricsCache};

pub mod accounts;
pub mod config;
pub mod credentials;
pub mod integrations;
pub mod monitoring;
pub mod profiles;
pub mod proxy;
pub mod ssh;
pub mod stats;
pub mod sync;

// ---------------------------------------------------------------------------
// DaemonState
// ---------------------------------------------------------------------------

/// État partagé entre le daemon et le serveur HTTP API.
#[derive(Clone)]
pub struct DaemonState {
    pub credentials: Arc<CredentialsCache>,
    pub config: Arc<ConfigCache>,
    pub proxy_instances: Arc<RwLock<HashMap<String, Arc<ProxyInstanceRuntime>>>>,
    pub peers: Arc<RwLock<Vec<Peer>>>,
    pub velocity_calculators: Arc<RwLock<HashMap<String, VelocityCalculator>>>,
    pub quota_metrics: Arc<RwLock<HashMap<String, QuotaMetricsCache>>>,
    pub invalid_grant_accounts: Arc<RwLock<HashSet<String>>>,
    pub event_log: Arc<EventLog>,
    pub credentials_path: PathBuf,
    pub settings_path: PathBuf,
    pub http_client: reqwest::Client,
    /// Bearer token optionnel. Si None → accès libre.
    pub api_token: Option<String>,
    pub shutdown_tx: watch::Sender<bool>,
}

// ---------------------------------------------------------------------------
// Helpers réponses
// ---------------------------------------------------------------------------

/// Retourne une réponse JSON d'erreur.
pub fn error_json(status: u16, msg: &str) -> Response<Body> {
    let body = json!({"error": {"message": msg}}).to_string();
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

/// Retourne une réponse JSON 200 OK.
pub fn ok_json(value: impl serde::Serialize) -> Response<Body> {
    let body = serde_json::to_string(&value).unwrap_or_else(|e| {
        json!({"error": {"message": e.to_string()}}).to_string()
    });
    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Middleware auth
// ---------------------------------------------------------------------------

/// Middleware Bearer token. Vérifie `Authorization: Bearer <token>` si configuré.
pub async fn auth_middleware(
    State(state): State<Arc<DaemonState>>,
    req: Request,
    next: Next,
) -> Response<Body> {
    if let Some(ref expected) = state.api_token {
        let auth = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if auth != Some(expected.as_str()) {
            return error_json(401, "Unauthorized");
        }
    }
    next.run(req).await
}

// ---------------------------------------------------------------------------
// Health (pas d'auth)
// ---------------------------------------------------------------------------

pub async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

// ---------------------------------------------------------------------------
// serve() — construire le router et démarrer le serveur
// ---------------------------------------------------------------------------

/// Lance le serveur HTTP API sur `addr`.
///
/// S'arrête gracieusement quand `shutdown_rx` reçoit `true`.
pub async fn serve(
    state: Arc<DaemonState>,
    addr: SocketAddr,
    mut shutdown_rx: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    // Routes sous /admin/api/ (avec auth)
    let api = Router::new()
        // ── Comptes ──────────────────────────────────────────────────────
        .route("/accounts", get(accounts::list_accounts).post(accounts::add_account))
        .route("/accounts/active", get(accounts::get_active))
        .route("/accounts/capture-before-switch", post(accounts::capture_before_switch))
        .route("/accounts/:key", put(accounts::update_account).delete(accounts::delete_account))
        .route("/accounts/:key/switch", post(accounts::switch_account))
        .route("/accounts/:key/refresh", post(accounts::refresh_account))
        .route("/accounts/:key/revoke", post(accounts::revoke_account))
        // ── Config ───────────────────────────────────────────────────────
        .route("/config", get(config::get_config).put(config::set_config))
        // ── Proxy legacy ─────────────────────────────────────────────────
        .route("/proxy/status", get(proxy::proxy_status))
        .route("/proxy/start", post(proxy::proxy_start))
        .route("/proxy/stop", post(proxy::proxy_stop))
        .route("/proxy/restart", post(proxy::proxy_restart))
        // ── Proxy instances ──────────────────────────────────────────────
        .route("/proxy-instances", get(proxy::list_instances).post(proxy::add_instance))
        .route("/proxy-instances/probe", post(proxy::probe_instances))
        .route("/proxy-instances/:id", put(proxy::update_instance).delete(proxy::delete_instance))
        .route("/proxy-instances/:id/start", post(proxy::start_instance))
        .route("/proxy-instances/:id/stop", post(proxy::stop_instance))
        .route("/proxy-instances/:id/restart", post(proxy::restart_instance))
        .route("/proxy-binaries", get(proxy::list_binaries))
        // ── Sync P2P ─────────────────────────────────────────────────────
        .route("/sync/status", get(sync::sync_status))
        .route("/sync/key/generate", post(sync::gen_key))
        .route("/sync/key/set", post(sync::set_key))
        .route("/peers", get(sync::list_peers).post(sync::add_peer))
        .route("/peers/test", post(sync::test_peer))
        .route("/peers/:id", delete(sync::remove_peer))
        // ── SSH ──────────────────────────────────────────────────────────
        .route("/ssh/hostname", get(ssh::get_hostname))
        .route("/ssh-hosts", post(ssh::add_ssh_host))
        .route("/ssh-hosts/test", post(ssh::test_ssh))
        .route("/ssh-hosts/:id", delete(ssh::remove_ssh_host))
        // ── Monitoring ───────────────────────────────────────────────────
        .route("/monitoring/quota-history", get(monitoring::quota_history))
        .route("/monitoring/switch-history", get(monitoring::switch_history))
        .route("/monitoring/profiles", get(monitoring::imp_profiles))
        .route("/monitoring/sessions", get(monitoring::sessions))
        .route("/monitoring/logs", get(monitoring::logs))
        // ── Credentials ──────────────────────────────────────────────────
        .route("/credentials/scan", post(credentials::scan_creds))
        .route("/credentials/import", post(credentials::import_creds))
        .route("/credentials/binary", get(credentials::find_binary))
        .route("/credentials/capture", post(credentials::capture_token))
        // ── Profils ──────────────────────────────────────────────────────
        .route("/profiles", get(profiles::list_profiles).post(profiles::save_profile))
        .route("/profiles/:name", get(profiles::load_profile).delete(profiles::delete_profile))
        // ── Intégrations ─────────────────────────────────────────────────
        .route("/setup/claude-code", post(integrations::setup_cc).delete(integrations::remove_cc))
        .route("/setup/vscode", post(integrations::setup_vscode).delete(integrations::remove_vscode))
        .route("/webhooks/test", post(integrations::test_webhook))
        // ── Stats + Systemd ──────────────────────────────────────────────
        .route("/stats", get(stats::get_stats))
        .route("/systemd/status", get(stats::systemd_status))
        .route("/systemd/install", post(stats::systemd_install))
        .route("/systemd/uninstall", post(stats::systemd_uninstall))
        // Auth middleware sur toutes les routes /admin/api/*
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    // Router principal : health sans auth + api avec auth
    let app = Router::new()
        .route("/admin/api/health", get(health))
        .nest("/admin/api", api)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Daemon HTTP API listening on {}", addr);

    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(async move {
            shutdown_rx.changed().await.ok();
            info!("HTTP API shutdown initiated");
        })
        .await?;

    Ok(())
}
