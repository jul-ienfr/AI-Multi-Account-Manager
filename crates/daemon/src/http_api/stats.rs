//! Handlers stats et gestion du service systemd.
//!
//! Tous les handlers sont stateless (pas d'extractor `State`).

use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use ai_core::stats::StatsManager;
use crate::dto::{InstallSystemdData, StatsDto};
use super::{error_json, ok_json};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn multi_account_base_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".claude")
        .join("multi-account")
}

// ---------------------------------------------------------------------------
// Handler stats
// ---------------------------------------------------------------------------

/// `GET /admin/api/stats` — Retourne les statistiques globales du daemon.
pub async fn get_stats() -> impl IntoResponse {
    let stats = StatsManager::new(&multi_account_base_dir()).load();
    let dto = StatsDto {
        total_switches: stats.total_switches,
        switches_by_account: stats.switches_by_account,
        total_requests: stats.total_requests,
        last_switch_at: stats.last_switch_at.map(|dt| dt.to_rfc3339()),
        uptime_started_at: Some(stats.uptime_started_at.to_rfc3339()),
    };
    ok_json(dto)
}

// ---------------------------------------------------------------------------
// Handlers systemd
// ---------------------------------------------------------------------------

/// `GET /admin/api/systemd/status` — Vérifie si le service systemd est actif.
pub async fn systemd_status() -> impl IntoResponse {
    #[cfg(unix)]
    {
        match std::process::Command::new("systemctl")
            .args(["--user", "is-active", "ai-manager-daemon"])
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let status_str = match stdout.trim() {
                    "active" => "active",
                    "inactive" => "inactive",
                    _ => "not-found",
                };
                ok_json(json!({"status": status_str}))
            }
            Err(_) => ok_json(json!({"status": "not-found"})),
        }
    }

    #[cfg(not(unix))]
    {
        ok_json(json!({"status": "unavailable"}))
    }
}

/// `POST /admin/api/systemd/install` — Installe et active le service systemd
/// user pour le daemon.
pub async fn systemd_install(Json(body): Json<InstallSystemdData>) -> impl IntoResponse {
    #[cfg(unix)]
    {
        // Résoudre le chemin du daemon.
        let daemon_path = if let Some(p) = body.daemon_path {
            std::path::PathBuf::from(p)
        } else {
            // Auto-detect dans les emplacements courants.
            let candidates = [
                dirs::home_dir()
                    .map(|h| h.join(".cargo").join("bin").join("ai-daemon")),
                Some(std::path::PathBuf::from("/usr/local/bin/ai-daemon")),
                Some(std::path::PathBuf::from("/usr/bin/ai-daemon")),
            ];

            let found = candidates
                .into_iter()
                .flatten()
                .find(|p| p.exists());

            match found {
                Some(p) => p,
                None => {
                    return error_json(
                        500,
                        "ai-daemon binary not found; specify daemon_path explicitly",
                    );
                }
            }
        };

        let daemon_path_str = daemon_path.display().to_string();

        // Contenu du unit file.
        let unit_content = format!(
            "[Unit]\n\
             Description=AI Manager v3 Daemon\n\
             After=network.target\n\
             \n\
             [Service]\n\
             Type=notify\n\
             ExecStart={} start\n\
             Restart=on-failure\n\
             RestartSec=5\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            daemon_path_str
        );

        // Chemin du unit file utilisateur.
        let unit_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join(".config")
            .join("systemd")
            .join("user");

        if let Err(e) = std::fs::create_dir_all(&unit_dir) {
            return error_json(500, &e.to_string());
        }

        let unit_path = unit_dir.join("ai-manager-daemon.service");
        if let Err(e) = std::fs::write(&unit_path, &unit_content) {
            return error_json(500, &e.to_string());
        }

        // daemon-reload
        if let Err(e) = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()
        {
            return error_json(500, &format!("daemon-reload failed: {}", e));
        }

        // enable --now
        if let Err(e) = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", "ai-manager-daemon"])
            .status()
        {
            return error_json(500, &format!("enable --now failed: {}", e));
        }

        // loginctl enable-linger (best-effort, ignore errors)
        let _ = std::process::Command::new("loginctl")
            .arg("enable-linger")
            .status();

        ok_json(json!({"ok": true, "message": "service installed"}))
    }

    #[cfg(not(unix))]
    {
        let _ = body; // éviter warning unused
        error_json(501, "systemd not available on Windows")
    }
}

/// `POST /admin/api/systemd/uninstall` — Désactive et supprime le service
/// systemd user du daemon.
pub async fn systemd_uninstall() -> impl IntoResponse {
    #[cfg(unix)]
    {
        // stop
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "stop", "ai-manager-daemon"])
            .status();

        // disable
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "ai-manager-daemon"])
            .status();

        // Supprimer le unit file.
        let unit_path = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join(".config")
            .join("systemd")
            .join("user")
            .join("ai-manager-daemon.service");

        if unit_path.exists() {
            if let Err(e) = std::fs::remove_file(&unit_path) {
                return error_json(500, &e.to_string());
            }
        }

        // daemon-reload
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();

        ok_json(json!({"ok": true}))
    }

    #[cfg(not(unix))]
    {
        error_json(501, "unavailable")
    }
}
