//! Handlers REST — configuration de l'application.
//!
//! 2 handlers : get_config (GET /config), set_config (PUT /config).
//! Le patch est mergé en profondeur dans la config existante.

use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::Value;

use super::{DaemonState, error_json, ok_json};

// ---------------------------------------------------------------------------
// Helper deep-merge
// ---------------------------------------------------------------------------

/// Merge `patch` into `base` recursively.
///
/// Objects are merged key by key; any other type is overwritten.
fn merge_json(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                merge_json(base_map.entry(k).or_insert(Value::Null), v);
            }
        }
        (base, patch) => *base = patch,
    }
}

// ---------------------------------------------------------------------------
// 1. get_config
// ---------------------------------------------------------------------------

pub async fn get_config(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    match serde_json::to_value(&*state.config.read()) {
        Ok(value) => ok_json(value),
        Err(e) => error_json(500, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// 2. set_config
// ---------------------------------------------------------------------------

pub async fn set_config(
    State(state): State<Arc<DaemonState>>,
    Json(patch): Json<Value>,
) -> impl IntoResponse {
    // Serialize current config to Value
    let mut current = match serde_json::to_value(&*state.config.read()) {
        Ok(v) => v,
        Err(e) => return error_json(500, &e.to_string()),
    };

    // Deep-merge the patch
    merge_json(&mut current, patch);

    // Deserialize back to AppConfig
    let new_config: ai_core::config::AppConfig = match serde_json::from_value(current) {
        Ok(c) => c,
        Err(e) => return error_json(400, &e.to_string()),
    };

    // Write back
    *state.config.write() = new_config;

    // Persist to disk
    if let Err(e) = state.config.persist() {
        return error_json(500, &e.to_string());
    }

    // Broadcast ConfigUpdate to P2P peers
    if let Some(bus) = &state.sync_bus {
        let bus = bus.clone();
        let instance_id = bus.instance_id().to_string();
        let config_json = serde_json::to_string(&*state.config.read()).unwrap_or_default();
        tokio::spawn(async move {
            let clock = bus.next_clock();
            let msg = ai_sync::messages::SyncMessage::new(
                &instance_id,
                ai_sync::messages::SyncPayload::ConfigUpdate { config_json, clock },
            );
            let _ = bus.broadcast(msg).await;
        });
    }

    ok_json(serde_json::json!({"ok": true}))
}
