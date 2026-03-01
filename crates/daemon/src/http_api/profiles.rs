//! Handlers profils — CRUD sur les snapshots de configuration nommés.
//!
//! Tous les handlers sont stateless (pas d'extractor `State`).
//! Les profils sont stockés dans `~/.claude/multi-account/profiles/`.

use axum::extract::Path;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use ai_core::profiles::ProfileManager;
use crate::dto::{ProfileInfoDto, SaveProfileData};
use super::{error_json, ok_json};

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
pub async fn save_profile(Json(body): Json<SaveProfileData>) -> impl IntoResponse {
    match ProfileManager::new(&profiles_dir()).save(&body.name, &body.config) {
        Ok(_) => ok_json(json!({"ok": true})),
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
pub async fn delete_profile(Path(name): Path<String>) -> impl IntoResponse {
    match ProfileManager::new(&profiles_dir()).delete(&name) {
        Ok(_) => ok_json(json!({"ok": true})),
        Err(e) => error_json(404, &e.to_string()),
    }
}
