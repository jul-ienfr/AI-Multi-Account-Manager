//! Handlers intégrations — Claude Code, VS Code, Webhooks.
//!
//! Tous les handlers sont stateless (pas d'extractor `State`).

use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use ai_core::webhook::{WebhookEvent, WebhookKind, WebhookSender, WebhookTarget};
use crate::dto::{SetupPortData, TestWebhookData};
use super::{error_json, ok_json};

// ---------------------------------------------------------------------------
// Helpers chemins
// ---------------------------------------------------------------------------

/// Chemin vers `~/.claude/settings.json` (settings Claude Code).
fn claude_settings_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".claude")
        .join("settings.json")
}

/// Chemin vers le `settings.json` de VS Code (platform-aware).
fn vscode_settings_path() -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        Some(
            std::path::PathBuf::from(appdata)
                .join("Code")
                .join("User")
                .join("settings.json"),
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        Some(
            home.join(".config")
                .join("Code")
                .join("User")
                .join("settings.json"),
        )
    }
}

// ---------------------------------------------------------------------------
// Validation URL webhook (sans crate url)
// ---------------------------------------------------------------------------

/// Autorise uniquement HTTPS ou HTTP vers localhost/127.0.0.1/[::1].
/// Bloque les adresses privées non-localhost sur HTTP.
fn validate_webhook_url(url: &str) -> Result<(), String> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if url.starts_with("http://localhost")
        || url.starts_with("http://127.0.0.1")
        || url.starts_with("http://[::1]")
    {
        return Ok(());
    }
    Err("Only HTTPS or localhost HTTP allowed".into())
}

// ---------------------------------------------------------------------------
// Handlers Claude Code
// ---------------------------------------------------------------------------

/// `POST /admin/api/setup/claude-code` — Injecte `ANTHROPIC_BASE_URL` dans
/// `~/.claude/settings.json` pour rediriger Claude Code vers le proxy.
pub async fn setup_cc(Json(body): Json<SetupPortData>) -> impl IntoResponse {
    let path = claude_settings_path();

    // Lire la config existante ou démarrer avec un objet vide.
    let content = if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => return error_json(500, &e.to_string()),
        }
    } else {
        "{}".to_string()
    };

    let mut value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => serde_json::Value::Object(serde_json::Map::new()),
    };

    // S'assurer que value["env"] est un objet.
    if !value["env"].is_object() {
        value["env"] = json!({});
    }
    value["env"]["ANTHROPIC_BASE_URL"] =
        json!(format!("http://127.0.0.1:{}", body.port));

    // Créer les répertoires parents si nécessaire.
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return error_json(500, &e.to_string());
        }
    }

    match serde_json::to_string_pretty(&value) {
        Ok(json_str) => match std::fs::write(&path, json_str) {
            Ok(_) => ok_json(json!({"ok": true})),
            Err(e) => error_json(500, &e.to_string()),
        },
        Err(e) => error_json(500, &e.to_string()),
    }
}

/// `DELETE /admin/api/setup/claude-code` — Retire `ANTHROPIC_BASE_URL` de
/// `~/.claude/settings.json`.
pub async fn remove_cc() -> impl IntoResponse {
    let path = claude_settings_path();

    if !path.exists() {
        // Rien à faire.
        return ok_json(json!({"ok": true}));
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => return error_json(500, &e.to_string()),
    };

    let mut value: serde_json::Value =
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}));

    if let Some(env) = value["env"].as_object_mut() {
        env.remove("ANTHROPIC_BASE_URL");
    }

    match serde_json::to_string_pretty(&value) {
        Ok(json_str) => match std::fs::write(&path, json_str) {
            Ok(_) => ok_json(json!({"ok": true})),
            Err(e) => error_json(500, &e.to_string()),
        },
        Err(e) => error_json(500, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Handlers VS Code
// ---------------------------------------------------------------------------

/// `POST /admin/api/setup/vscode` — Configure `http.proxy` dans les settings
/// VS Code pour router via le proxy local.
pub async fn setup_vscode(Json(body): Json<SetupPortData>) -> impl IntoResponse {
    let path = match vscode_settings_path() {
        Some(p) => p,
        None => return error_json(404, "VS Code settings not found"),
    };

    let content = if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => return error_json(500, &e.to_string()),
        }
    } else {
        "{}".to_string()
    };

    let mut value: serde_json::Value =
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}));

    value["http.proxy"] = json!(format!("http://127.0.0.1:{}", body.port));

    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return error_json(500, &e.to_string());
        }
    }

    match serde_json::to_string_pretty(&value) {
        Ok(json_str) => match std::fs::write(&path, json_str) {
            Ok(_) => ok_json(json!({"ok": true})),
            Err(e) => error_json(500, &e.to_string()),
        },
        Err(e) => error_json(500, &e.to_string()),
    }
}

/// `DELETE /admin/api/setup/vscode` — Retire `http.proxy` des settings VS Code.
pub async fn remove_vscode() -> impl IntoResponse {
    let path = match vscode_settings_path() {
        Some(p) => p,
        None => return ok_json(json!({"ok": true})),
    };

    if !path.exists() {
        return ok_json(json!({"ok": true}));
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => return error_json(500, &e.to_string()),
    };

    let mut value: serde_json::Value =
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}));

    if let Some(obj) = value.as_object_mut() {
        obj.remove("http.proxy");
    }

    match serde_json::to_string_pretty(&value) {
        Ok(json_str) => match std::fs::write(&path, json_str) {
            Ok(_) => ok_json(json!({"ok": true})),
            Err(e) => error_json(500, &e.to_string()),
        },
        Err(e) => error_json(500, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Handler webhook test
// ---------------------------------------------------------------------------

/// `POST /admin/api/webhooks/test` — Envoie un événement de test vers l'URL
/// fournie (Discord, Slack ou Generic).
pub async fn test_webhook(Json(body): Json<TestWebhookData>) -> impl IntoResponse {
    // Valider l'URL.
    if let Err(e) = validate_webhook_url(&body.url) {
        return error_json(400, &e);
    }

    // Parser le kind.
    let kind = match body.kind.as_str() {
        "discord" => WebhookKind::Discord,
        "slack" => WebhookKind::Slack,
        "generic" => WebhookKind::Generic,
        other => {
            return error_json(
                400,
                &format!("Unknown webhook kind '{}': expected discord, slack or generic", other),
            );
        }
    };

    let target = WebhookTarget {
        url: body.url.clone(),
        kind,
        events: vec![], // filtre vide = accepte tout
    };

    // Événement de test : PhaseTransition factice.
    let event = WebhookEvent::PhaseTransition {
        key: "test".to_string(),
        from: "normal".to_string(),
        to: "warning".to_string(),
    };

    let sender = WebhookSender::new(vec![target]);
    sender.send(event).await;

    ok_json(json!({"ok": true}))
}
