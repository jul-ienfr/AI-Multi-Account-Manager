//! Serveur proxy Axum — point d'entrée public.
//!
//! Lance le serveur Axum complet avec tous les handlers définis dans `handler.rs`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    routing::{get, post},
    Router,
};
use tokio::sync::watch;
use tracing::info;

use crate::handler::AppState;

/// Démarre le serveur proxy sur l'adresse donnée.
pub async fn start(addr: SocketAddr) -> anyhow::Result<()> {
    let multi_account_dir = crate::credentials::find_multi_account_dir();
    let credentials_path = multi_account_dir.join("credentials-multi.json");
    let settings_path = multi_account_dir.join("settings.json");

    info!(
        credentials = %credentials_path.display(),
        settings = %settings_path.display(),
        "proxy starting"
    );

    let creds = crate::credentials::CredentialsCache::load(&credentials_path);
    let model_config = Arc::new(crate::model_mapping::load_config_mappings(&settings_path));
    let imp = Arc::new(crate::impersonation::ImpersonationState::new(true, true));

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .danger_accept_invalid_certs(false)
        .build()?;

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    let upstream_url = "https://api.anthropic.com".to_string();

    let state = Arc::new(AppState {
        client: http_client,
        timeout_secs: 300,
        upstream_url,
        credentials: creds.clone(),
        imp: imp.clone(),
        model_config,
        shutdown_tx,
        provider_quota: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        verbose: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        rate_limiter: Arc::new(crate::rate_limiter::RateLimiter::new()),
        api_usage: Arc::new(crate::api_usage::ApiUsageTracker::new(&multi_account_dir)),
        session_writer: Arc::new(tokio::sync::Mutex::new(
            crate::session_writer::SessionWriter::new(&multi_account_dir),
        )),
        last_client_ip: Arc::new(parking_lot::Mutex::new(String::new())),
    });

    // Background: reload credentials every 30s
    {
        let creds = creds.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                creds.reload();
            }
        });
    }

    // Background: flush impersonation profiles every 30s
    {
        let profiles = imp.profiles.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                crate::cc_profile::flush_cache(&profiles);
            }
        });
    }

    let app = Router::new()
        .route("/_proxy/health", get(crate::handler::proxy_health))
        .route("/_proxy/status", get(crate::handler::proxy_status))
        .route("/_proxy/shutdown", post(crate::handler::proxy_shutdown))
        .route("/_proxy/profiles", get(crate::handler::proxy_profiles))
        .route("/_proxy/profiles/flush", post(crate::handler::proxy_flush))
        .route("/_proxy/verbose", post(crate::handler::proxy_verbose_toggle))
        .route("/_proxy/api/usage", get(crate::handler::proxy_api_usage))
        .fallback(crate::handler::handle_proxy)
        .with_state(state);

    info!("listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            shutdown_rx.changed().await.ok();
            info!("graceful shutdown initiated");
        })
        .await?;

    Ok(())
}

/// Chemin du répertoire multi-account (pour les tests / le binaire).
pub fn default_multi_account_dir() -> PathBuf {
    crate::credentials::find_multi_account_dir()
}
