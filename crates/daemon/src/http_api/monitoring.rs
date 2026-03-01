//! Handlers monitoring — quota history, switch history, profiles, sessions, logs.

use std::io::{BufRead, BufReader};
use std::sync::Arc;
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{json, Value};
use super::{DaemonState, error_json, ok_json};

// ---------------------------------------------------------------------------
// Helper — lire les N dernières lignes d'un fichier JSONL
// ---------------------------------------------------------------------------

fn read_jsonl_last_n(path: &std::path::Path, n: usize) -> Vec<Value> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return vec![],
    };
    let reader = BufReader::new(file);
    let mut entries: Vec<Value> = reader
        .lines()
        .filter_map(|l| l.ok())
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect();
    let len = entries.len();
    if len > n {
        entries.drain(..len - n);
    }
    entries.reverse();
    entries
}

// ---------------------------------------------------------------------------
// quota_history
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct QuotaHistoryParams {
    pub key: String,
    pub period: Option<String>,
}

/// `GET /monitoring/quota-history` — historique quota pour un compte.
pub async fn quota_history(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<QuotaHistoryParams>,
) -> impl IntoResponse {
    let base = match state.credentials_path.parent() {
        Some(p) => p.to_path_buf(),
        None => return error_json(500, "invalid credentials_path"),
    };
    let path = base.join("quota_history.jsonl");

    if !path.exists() {
        return ok_json(json!([]));
    }

    // Calcul du cutoff selon la période demandée
    let hours: i64 = match params.period.as_deref() {
        Some("24h") => 24,
        Some("7d") => 7 * 24,
        Some("30d") => 30 * 24,
        _ => 7 * 24, // défaut 7 jours
    };
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(hours);

    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return ok_json(json!([])),
    };
    let reader = BufReader::new(file);

    let entries: Vec<Value> = reader
        .lines()
        .filter_map(|l| l.ok())
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
        .filter(|entry| {
            // Filtre par key
            let key_matches = entry
                .get("key")
                .and_then(|v| v.as_str())
                .map(|k| k == params.key)
                .unwrap_or(false);
            if !key_matches {
                return false;
            }
            // Filtre temporel
            if let Some(ts) = entry.get("timestamp").and_then(|v| v.as_str()) {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                    return dt.with_timezone(&chrono::Utc) >= cutoff;
                }
            }
            true
        })
        .collect();

    ok_json(entries)
}

// ---------------------------------------------------------------------------
// switch_history
// ---------------------------------------------------------------------------

/// `GET /monitoring/switch-history` — les 200 derniers switch de compte.
pub async fn switch_history(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let base = match state.credentials_path.parent() {
        Some(p) => p.to_path_buf(),
        None => return error_json(500, "invalid credentials_path"),
    };
    let path = base.join("switch_history.jsonl");

    if !path.exists() {
        return ok_json(json!([]));
    }

    let entries = read_jsonl_last_n(&path, 200);
    ok_json(entries)
}

// ---------------------------------------------------------------------------
// imp_profiles
// ---------------------------------------------------------------------------

/// `GET /monitoring/profiles` — profils d'impersonation.
pub async fn imp_profiles(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let base = match state.credentials_path.parent() {
        Some(p) => p.to_path_buf(),
        None => return error_json(500, "invalid credentials_path"),
    };

    let profiles_dir = base.join("profiles");
    let mut profiles: Vec<Value> = Vec::new();

    if profiles_dir.is_dir() {
        if let Ok(read_dir) = std::fs::read_dir(&profiles_dir) {
            for entry in read_dir.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(raw) = std::fs::read_to_string(&path) {
                    if let Ok(mut val) = serde_json::from_str::<Value>(&raw) {
                        // Ajoute le nom du provider depuis le stem du fichier
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            if let Some(obj) = val.as_object_mut() {
                                obj.insert(
                                    "provider_name".to_string(),
                                    Value::String(stem.to_string()),
                                );
                            }
                        }
                        profiles.push(val);
                    }
                }
            }
        }

        if !profiles.is_empty() {
            return ok_json(profiles);
        }

        // Dossier vide → tenter le fichier legacy
    }

    // Fallback legacy: cc-profile.json à la racine
    let legacy = base.join("cc-profile.json");
    if legacy.exists() {
        if let Ok(raw) = std::fs::read_to_string(&legacy) {
            if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                return ok_json(json!([val]));
            }
        }
    }

    ok_json(json!([]))
}

// ---------------------------------------------------------------------------
// sessions
// ---------------------------------------------------------------------------

/// `GET /monitoring/sessions` — sessions récentes (top 50 par start_time desc).
pub async fn sessions(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let base = match state.credentials_path.parent() {
        Some(p) => p.to_path_buf(),
        None => return error_json(500, "invalid credentials_path"),
    };

    let sessions_dir = base.join("sessions");

    if !sessions_dir.is_dir() {
        return ok_json(json!({"active": 0, "total_today": 0, "sessions": []}));
    }

    let mut list: Vec<Value> = Vec::new();

    if let Ok(read_dir) = std::fs::read_dir(&sessions_dir) {
        for entry in read_dir.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                    list.push(val);
                }
            }
        }
    }

    // Sort par start_time desc (chaîne RFC3339 — tri lexicographique suffisant)
    list.sort_by(|a, b| {
        let ta = a.get("start_time").and_then(|v| v.as_str()).unwrap_or("");
        let tb = b.get("start_time").and_then(|v| v.as_str()).unwrap_or("");
        tb.cmp(ta)
    });

    // Top 50
    list.truncate(50);
    let total = list.len();

    ok_json(json!({
        "active": 0,
        "total_today": total,
        "cost_today": 0.0,
        "cost_7d": 0.0,
        "sessions": list,
    }))
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LogsParams {
    pub filter: Option<String>,
}

/// `GET /monitoring/logs` — dernières entrées api_usage.jsonl.
pub async fn logs(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<LogsParams>,
) -> impl IntoResponse {
    let base = match state.credentials_path.parent() {
        Some(p) => p.to_path_buf(),
        None => return error_json(500, "invalid credentials_path"),
    };
    let path = base.join("api_usage.jsonl");

    if !path.exists() {
        return ok_json(json!([]));
    }

    let mut entries = read_jsonl_last_n(&path, 200);

    // Filtre optionnel case-insensitive
    if let Some(filter) = params.filter.as_deref() {
        let f = filter.to_lowercase();
        entries.retain(|entry| {
            serde_json::to_string(entry)
                .unwrap_or_default()
                .to_lowercase()
                .contains(&f)
        });
    }

    ok_json(entries)
}
