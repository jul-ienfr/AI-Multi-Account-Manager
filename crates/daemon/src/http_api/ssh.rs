//! Handlers SSH — gestion des hôtes SSH distants et tests de connectivité.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use ai_core::config::SshHostConfig;

use crate::dto::{AddSshHostData, TestSshData};
use super::{DaemonState, ok_json};

// ---------------------------------------------------------------------------
// get_hostname
// ---------------------------------------------------------------------------

/// `GET /ssh/hostname` — retourne le nom d'hôte local (stateless).
pub async fn get_hostname() -> impl IntoResponse {
    let name = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    ok_json(json!({"hostname": name}))
}

// ---------------------------------------------------------------------------
// add_ssh_host
// ---------------------------------------------------------------------------

/// `POST /ssh-hosts` — ajoute ou remplace un hôte SSH distant.
///
/// Upsert : si un hôte avec le même `host` et `port` existe déjà, il est remplacé.
pub async fn add_ssh_host(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<AddSshHostData>,
) -> impl IntoResponse {
    let cfg = SshHostConfig {
        id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
        host: body.host,
        port: body.port,
        username: body.username,
        identity_path: body.identity_path,
        enabled: true,
    };

    {
        let mut guard = state.config.write();
        // Upsert : remplace si même host+port, sinon ajoute.
        let existing = guard
            .sync
            .ssh_hosts
            .iter()
            .position(|h| h.host == cfg.host && h.port == cfg.port);
        if let Some(idx) = existing {
            guard.sync.ssh_hosts[idx] = cfg.clone();
        } else {
            guard.sync.ssh_hosts.push(cfg.clone());
        }
    }

    let _ = state.config.persist();

    ok_json(json!({"ok": true, "id": cfg.id}))
}

// ---------------------------------------------------------------------------
// remove_ssh_host
// ---------------------------------------------------------------------------

/// `DELETE /ssh-hosts/:id` — supprime un hôte SSH par identifiant.
pub async fn remove_ssh_host(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    state.config.write().sync.ssh_hosts.retain(|h| h.id != id);
    let _ = state.config.persist();

    ok_json(json!({"ok": true}))
}

// ---------------------------------------------------------------------------
// test_ssh
// ---------------------------------------------------------------------------

/// `POST /ssh-hosts/test` — teste la connexion SSH vers un hôte (stateless).
pub async fn test_ssh(Json(body): Json<TestSshData>) -> impl IntoResponse {
    let config_result = ai_sync::ssh::SshConfig::new(
        body.host,
        body.port,
        body.username,
        body.identity_path,
    );

    let reachable = match config_result {
        Err(_) => false,
        Ok(config) => ai_sync::ssh::SshSync::new(config)
            .test_connection()
            .await
            .is_ok(),
    };

    ok_json(json!({"reachable": reachable}))
}
