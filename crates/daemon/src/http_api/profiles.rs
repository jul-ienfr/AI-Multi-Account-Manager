//! Handlers profils — CRUD sur les snapshots de configuration nommés.
//!
//! Les profils sont stockés dans `~/.claude/multi-account/profiles/`.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use ai_core::profiles::ProfileManager;
use crate::dto::{ProfileInfoDto, SaveProfileData};
use super::{DaemonState, error_json, ok_json};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn profiles_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".claude")
        .join("multi-account")
        .join("profiles")
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /admin/api/profiles` — Liste tous les profils disponibles.
pub async fn list_profiles() -> impl IntoResponse {
    match ProfileManager::new(&profiles_dir()).list() {
        Ok(infos) => {
            let dtos: Vec<ProfileInfoDto> = infos
                .into_iter()
                .map(|info| ProfileInfoDto {
                    name: info.name,
                    created_at: info.created_at.to_rfc3339(),
                    size_bytes: info.size_bytes,
                })
                .collect();
            ok_json(dtos)
        }
        Err(e) => error_json(500, &e.to_string()),
    }
}

/// `POST /admin/api/profiles` — Sauvegarde (crée ou écrase) un profil.
pub async fn save_profile(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<SaveProfileData>,
) -> impl IntoResponse {
    match ProfileManager::new(&profiles_dir()).save(&body.name, &body.config) {
        Ok(_) => {
            // Broadcast ProfileUpdate to P2P peers
            if let Some(bus) = &state.sync_bus {
                let bus = bus.clone();
                let instance_id = bus.instance_id().to_string();
                let name = body.name.clone();
                let config_json = serde_json::to_string(&body.config).unwrap_or_default();
                tokio::spawn(async move {
                    let clock = bus.next_clock();
                    let msg = ai_sync::messages::SyncMessage::new(
                        &instance_id,
                        ai_sync::messages::SyncPayload::ProfileUpdate {
                            name,
                            config_json: Some(config_json),
                            clock,
                        },
                    );
                    let _ = bus.broadcast(msg).await;
                });
            }
            ok_json(json!({"ok": true}))
        }
        Err(e) => error_json(500, &e.to_string()),
    }
}

/// `GET /admin/api/profiles/:name` — Charge un profil par son nom.
pub async fn load_profile(Path(name): Path<String>) -> impl IntoResponse {
    match ProfileManager::new(&profiles_dir()).load(&name) {
        Ok(value) => ok_json(value),
        Err(_) => error_json(404, "profile not found"),
    }
}

/// `DELETE /admin/api/profiles/:name` — Supprime un profil par son nom.
pub async fn delete_profile(
    State(state): State<Arc<DaemonState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match ProfileManager::new(&profiles_dir()).delete(&name) {
        Ok(_) => {
            // Broadcast ProfileUpdate (delete) to P2P peers
            if let Some(bus) = &state.sync_bus {
                let bus = bus.clone();
                let instance_id = bus.instance_id().to_string();
                let name_clone = name.clone();
                tokio::spawn(async move {
                    let clock = bus.next_clock();
                    let msg = ai_sync::messages::SyncMessage::new(
                        &instance_id,
                        ai_sync::messages::SyncPayload::ProfileUpdate {
                            name: name_clone,
                            config_json: None,
                            clock,
                        },
                    );
                    let _ = bus.broadcast(msg).await;
                });
            }
            ok_json(json!({"ok": true}))
        }
        Err(e) => error_json(404, &e.to_string()),
    }
}
