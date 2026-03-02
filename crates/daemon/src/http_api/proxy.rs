//! Handlers proxy — legacy (start/stop/status) + instances dynamiques.
//!
//! 13 handlers au total :
//!   - Proxy legacy  : `proxy_status`, `proxy_start`, `proxy_stop`, `proxy_restart`
//!   - Instances     : `list_instances`, `add_instance`, `update_instance`,
//!                     `delete_instance`, `start_instance`, `stop_instance`,
//!                     `restart_instance`, `probe_instances`
//!   - Binaires      : `list_binaries`

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use parking_lot::{Mutex, RwLock};
use serde_json::{json, Value};
use tracing::{debug, info};

use ai_core::types::{ProxyInstanceConfig, ProxyInstanceRuntime, ProxyInstanceState, ProxyStatus};

use crate::dto::{DetectedBinary, ProxyKindData};
use super::{DaemonState, error_json, ok_json};

// ---------------------------------------------------------------------------
// Helper — validate binary path (allowlist strict)
// ---------------------------------------------------------------------------

fn validate_binary_path(path: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(path);
    let canonical = p
        .canonicalize()
        .map_err(|e| format!("Cannot resolve binary: {e}"))?;
    if !canonical.is_file() {
        return Err(format!(
            "Binary not found or not a file: {}",
            canonical.display()
        ));
    }
    let name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let allowed = [
        "anthrouter",
        "anthrouter.exe",
        "claude-router-auto",
        "claude-router-auto.exe",
        "claude-impersonator",
        "claude-impersonator.exe",
        "claude-translator",
        "claude-translator.exe",
        "ai-proxy",
        "ai-proxy.exe",
    ];
    if !allowed.contains(&name) {
        return Err(format!("Binary not in allowlist: {name}"));
    }
    Ok(canonical)
}

// ---------------------------------------------------------------------------
// Helper — probe un port proxy via HTTP GET /_proxy/health
// ---------------------------------------------------------------------------

async fn probe_proxy_health(client: &reqwest::Client, port: u16) -> Option<String> {
    let url = format!("http://127.0.0.1:{}/_proxy/health", port);
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    if body.get("status").and_then(|v| v.as_str()) == Some("ok") {
        Some(
            body.get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
        )
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Helper interne — stop par id
// ---------------------------------------------------------------------------

async fn stop_instance_by_id(state: &DaemonState, id: &str) -> Result<(), String> {
    let instances = state.proxy_instances.read();
    let runtime: Arc<ProxyInstanceRuntime> = match instances.get(id) {
        Some(rt) => rt.clone(),
        None => return Err(format!("Proxy instance '{}' not found", id)),
    };

    if let Some(mut child) = runtime.child_process.lock().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(handle) = runtime.task_handle.lock().take() {
        handle.abort();
    }
    {
        let mut s = runtime.status.write();
        s.running = false;
        s.pid = None;
    }
    info!("Proxy instance '{}' stopped", id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper interne — start par id
// ---------------------------------------------------------------------------

async fn start_instance_by_id(state: &DaemonState, id: &str) -> Result<(), String> {
    let (port, binary_path) = {
        let cfg = state.config.read();
        let inst = cfg
            .proxy
            .instances
            .iter()
            .find(|i| i.id == id)
            .ok_or_else(|| format!("Proxy instance '{}' not found", id))?;
        (inst.port, inst.binary_path.clone())
    };

    let runtime = {
        let instances = state.proxy_instances.read();
        instances
            .get(id)
            .ok_or_else(|| format!("Runtime for '{}' not found", id))?
            .clone()
    };

    if runtime.status.read().running {
        return Err(format!("Proxy '{}' is already running", id));
    }

    if let Some(bin_path) = binary_path {
        // Mode binaire externe
        let canonical_bin = validate_binary_path(&bin_path)?;

        let child = std::process::Command::new(&canonical_bin)
            .args(["--port", &port.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to start {}: {}", canonical_bin.display(), e))?;

        let pid = child.id();
        {
            let mut s = runtime.status.write();
            s.running = true;
            s.port = port;
            s.pid = Some(pid);
        }
        *runtime.child_process.lock() = Some(child);
        info!(
            "Proxy '{}' started (external pid={}) on port {}",
            id, pid, port
        );
    } else {
        // Mode proxy built-in (tokio task)
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
            .parse()
            .map_err(|e: std::net::AddrParseError| e.to_string())?;

        let status_ref = runtime.clone();
        let id_label = id.to_string();

        let join = tokio::task::spawn(async move {
            {
                let mut s = status_ref.status.write();
                s.running = true;
                s.port = addr.port();
            }
            if let Err(e) = ai_proxy::server::start(addr).await {
                tracing::error!("Proxy {} error: {}", id_label, e);
            }
            {
                let mut s = status_ref.status.write();
                s.running = false;
                s.pid = None;
            }
        });

        *runtime.task_handle.lock() = Some(join.abort_handle());
        info!("Proxy instance '{}' started (built-in) on port {}", id, port);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Proxy Legacy — 4 handlers
// ---------------------------------------------------------------------------

/// GET /proxy/status — Statut agrégé basé sur les proxy instances configurées.
pub async fn proxy_status(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let cfg = state.config.read();
    let instances = &cfg.proxy.instances;
    let instances_rt = state.proxy_instances.read();

    let instances_summary: Vec<Value> = instances
        .iter()
        .map(|inst| {
            let status = instances_rt
                .get(&inst.id)
                .map(|rt| rt.status.read().clone())
                .unwrap_or_else(|| ProxyStatus {
                    port: inst.port,
                    ..Default::default()
                });
            json!({
                "id": inst.id,
                "name": inst.name,
                "kind": inst.kind,
                "port": inst.port,
                "running": status.running,
                "pid": status.pid,
            })
        })
        .collect();

    ok_json(json!({
        "instances_count": instances.len(),
        "instances": instances_summary,
    }))
}

/// POST /proxy/start — Démarre la première instance du kind demandé.
pub async fn proxy_start(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ProxyKindData>,
) -> impl IntoResponse {
    let id = {
        let cfg = state.config.read();
        let found = cfg.proxy.instances.iter().find(|inst| {
            if let Some(ref k) = body.kind {
                format!("{:?}", inst.kind).to_lowercase() == k.to_lowercase()
            } else {
                true
            }
        });
        match found {
            None => return error_json(404, "instance not found"),
            Some(inst) => {
                let instances_rt = state.proxy_instances.read();
                if let Some(rt) = instances_rt.get(&inst.id) {
                    if rt.status.read().running {
                        return error_json(409, "already running");
                    }
                }
                inst.id.clone()
            }
        }
    };

    match start_instance_by_id(&state, &id).await {
        Ok(()) => ok_json(json!({"ok": true})),
        Err(e) => error_json(500, &e),
    }
}

/// POST /proxy/stop — Arrête la première instance du kind demandé.
pub async fn proxy_stop(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ProxyKindData>,
) -> impl IntoResponse {
    let id = {
        let cfg = state.config.read();
        let found = cfg.proxy.instances.iter().find(|inst| {
            if let Some(ref k) = body.kind {
                format!("{:?}", inst.kind).to_lowercase() == k.to_lowercase()
            } else {
                true
            }
        });
        match found {
            None => return error_json(404, "instance not found"),
            Some(inst) => {
                let instances_rt = state.proxy_instances.read();
                if let Some(rt) = instances_rt.get(&inst.id) {
                    if !rt.status.read().running {
                        return error_json(409, "not running");
                    }
                }
                inst.id.clone()
            }
        }
    };

    match stop_instance_by_id(&state, &id).await {
        Ok(()) => ok_json(json!({"ok": true})),
        Err(e) => error_json(500, &e),
    }
}

/// POST /proxy/restart — Stop puis start la première instance du kind demandé.
pub async fn proxy_restart(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ProxyKindData>,
) -> impl IntoResponse {
    let id = {
        let cfg = state.config.read();
        let found = cfg.proxy.instances.iter().find(|inst| {
            if let Some(ref k) = body.kind {
                format!("{:?}", inst.kind).to_lowercase() == k.to_lowercase()
            } else {
                true
            }
        });
        match found {
            None => return error_json(404, "instance not found"),
            Some(inst) => inst.id.clone(),
        }
    };

    if let Err(e) = stop_instance_by_id(&state, &id).await {
        return error_json(500, &e);
    }
    match start_instance_by_id(&state, &id).await {
        Ok(()) => ok_json(json!({"ok": true})),
        Err(e) => error_json(500, &e),
    }
}

// ---------------------------------------------------------------------------
// Proxy Instances — 9 handlers
// ---------------------------------------------------------------------------

/// GET /proxy-instances — Liste toutes les instances avec leur statut runtime.
pub async fn list_instances(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let cfg = state.config.read();
    let instances_rt = state.proxy_instances.read();

    let result: Vec<ProxyInstanceState> = cfg
        .proxy
        .instances
        .iter()
        .map(|inst| {
            let status = instances_rt
                .get(&inst.id)
                .map(|rt| rt.status.read().clone())
                .unwrap_or_else(|| ProxyStatus {
                    port: inst.port,
                    ..Default::default()
                });
            ProxyInstanceState {
                config: inst.clone(),
                status,
            }
        })
        .collect();

    ok_json(result)
}

/// POST /proxy-instances — Ajoute une nouvelle instance.
pub async fn add_instance(
    State(state): State<Arc<DaemonState>>,
    Json(config): Json<ProxyInstanceConfig>,
) -> impl IntoResponse {
    {
        let mut cfg = state.config.write();
        if cfg.proxy.instances.iter().any(|i| i.id == config.id) {
            return error_json(409, &format!("Proxy instance '{}' already exists", config.id));
        }
        cfg.proxy.instances.push(config.clone());
    }

    if let Err(e) = state.config.persist() {
        return error_json(500, &e.to_string());
    }

    {
        let runtime = Arc::new(ProxyInstanceRuntime {
            status: RwLock::new(ProxyStatus {
                port: config.port,
                ..Default::default()
            }),
            task_handle: Mutex::new(None),
            child_process: Mutex::new(None),
        });
        state
            .proxy_instances
            .write()
            .insert(config.id.clone(), runtime);
    }

    info!("Proxy instance added: {} ({})", config.name, config.id);
    ok_json(json!({"ok": true}))
}

/// PUT /proxy-instances/:id — Met à jour les champs d'une instance (patch partiel).
pub async fn update_instance(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Json(updates): Json<Value>,
) -> impl IntoResponse {
    {
        let mut cfg = state.config.write();
        let inst = match cfg.proxy.instances.iter_mut().find(|i| i.id == id) {
            Some(i) => i,
            None => return error_json(404, &format!("Proxy instance '{}' not found", id)),
        };

        if let Some(name) = updates.get("name").and_then(|v| v.as_str()) {
            inst.name = name.to_string();
        }
        if let Some(port) = updates.get("port").and_then(|v| v.as_u64()) {
            inst.port = port as u16;
        }
        if let Some(enabled) = updates.get("enabled").and_then(|v| v.as_bool()) {
            inst.enabled = enabled;
        }
        if let Some(auto_start) = updates.get("autoStart").and_then(|v| v.as_bool()) {
            inst.auto_start = auto_start;
        }
        if let Some(targets) = updates.get("setupTargets").and_then(|v| v.as_array()) {
            inst.setup_targets = targets
                .iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect();
        }
        // binaryPath: null efface, string met à jour
        if updates.get("binaryPath").is_some() {
            inst.binary_path = updates
                .get("binaryPath")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }

    if let Err(e) = state.config.persist() {
        return error_json(500, &e.to_string());
    }

    info!("Proxy instance updated: {}", id);
    ok_json(json!({"ok": true}))
}

/// DELETE /proxy-instances/:id — Supprime une instance (la stoppe si running).
pub async fn delete_instance(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Stop si en cours
    {
        let instances = state.proxy_instances.read();
        if let Some(rt_ref) = instances.get(&id) {
            let rt: Arc<ProxyInstanceRuntime> = rt_ref.clone();
            drop(instances);
            let child_opt = rt.child_process.lock().take();
            if let Some(mut child) = child_opt {
                let _ = child.kill();
                let _ = child.wait();
            }
            let handle_opt = rt.task_handle.lock().take();
            if let Some(handle) = handle_opt {
                handle.abort();
            }
        }
    }

    // Retirer du runtime
    state.proxy_instances.write().remove(&id);

    // Retirer de la config
    {
        let mut cfg = state.config.write();
        cfg.proxy.instances.retain(|i| i.id != id);
    }

    if let Err(e) = state.config.persist() {
        return error_json(500, &e.to_string());
    }

    info!("Proxy instance deleted: {}", id);
    ok_json(json!({"ok": true}))
}

/// POST /proxy-instances/:id/start — Démarre une instance spécifique.
pub async fn start_instance(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match start_instance_by_id(&state, &id).await {
        Ok(()) => ok_json(json!({"ok": true})),
        Err(e) => {
            if e.contains("not found") {
                error_json(404, &e)
            } else if e.contains("already running") {
                error_json(409, &e)
            } else {
                error_json(500, &e)
            }
        }
    }
}

/// POST /proxy-instances/:id/stop — Arrête une instance spécifique.
pub async fn stop_instance(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match stop_instance_by_id(&state, &id).await {
        Ok(()) => ok_json(json!({"ok": true})),
        Err(e) => {
            if e.contains("not found") {
                error_json(404, &e)
            } else {
                error_json(500, &e)
            }
        }
    }
}

/// POST /proxy-instances/:id/restart — Stop puis start une instance spécifique.
pub async fn restart_instance(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = stop_instance_by_id(&state, &id).await {
        return error_json(if e.contains("not found") { 404 } else { 500 }, &e);
    }
    match start_instance_by_id(&state, &id).await {
        Ok(()) => ok_json(json!({"ok": true})),
        Err(e) => error_json(500, &e),
    }
}

/// POST /proxy-instances/probe — Probe tous les ports configurés et met à jour les statuts.
pub async fn probe_instances(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let configs: Vec<ai_core::types::ProxyInstanceConfig> = {
        state.config.read().proxy.instances.clone()
    };

    // Identifie les instances sans handle local (process externe / V2 / P2P)
    let mut probe_targets: Vec<(String, u16)> = Vec::new();
    for inst_cfg in &configs {
        let has_local_handle = {
            let instances_rt = state.proxy_instances.read();
            instances_rt
                .get(&inst_cfg.id)
                .map(|rt| {
                    rt.child_process.lock().is_some() || rt.task_handle.lock().is_some()
                })
                .unwrap_or(false)
        };
        if !has_local_handle {
            probe_targets.push((inst_cfg.id.clone(), inst_cfg.port));
        }
    }

    // Probe en parallèle
    let client = state.http_client.clone();
    let mut futures: Vec<
        std::pin::Pin<Box<dyn std::future::Future<Output = (String, u16, Option<String>)> + Send>>,
    > = Vec::new();

    for (id, port) in probe_targets {
        let client_ref = client.clone();
        futures.push(Box::pin(async move {
            let backend = probe_proxy_health(&client_ref, port).await;
            (id, port, backend)
        }));
    }

    let results = futures::future::join_all(futures).await;

    // Mettre à jour les statuts
    {
        let instances_rt = state.proxy_instances.read();
        for (id, port, backend) in &results {
            if let Some(rt) = instances_rt.get(id) {
                let mut s = rt.status.write();
                if let Some(backend_name) = backend {
                    s.running = true;
                    s.port = *port;
                    s.backend = Some(backend_name.clone());
                    debug!(
                        "Proxy '{}' detected externally on port {} (backend: {})",
                        id, port, backend_name
                    );
                } else {
                    // Plus de réponse : marque stopped uniquement si pas démarré localement
                    s.running = false;
                    s.backend = None;
                }
            }
        }
    }

    // Retourne la liste mise à jour
    list_instances(State(state)).await
}

/// GET /proxy-binaries — Détecte les binaires proxy disponibles sur le système.
pub async fn list_binaries() -> impl IntoResponse {
    let exe_path = std::env::current_exe().unwrap_or_default();
    let exe_dir = exe_path.parent().unwrap_or_else(|| std::path::Path::new("."));

    // Remonte jusqu'à trouver le dossier racine "AI Manager" (max 5 niveaux)
    let mut root = exe_dir.to_path_buf();
    for _ in 0..5 {
        if root
            .file_name()
            .and_then(|n| n.to_str())
            == Some("AI Manager")
        {
            break;
        }
        if let Some(p) = root.parent() {
            root = p.to_path_buf();
        } else {
            break;
        }
    }

    let mut binaries: Vec<DetectedBinary> = Vec::new();

    // anthrouter: check multiple build output paths
    let anthrouter_paths = [
        "anthrouter/target/x86_64-pc-windows-gnu/release/anthrouter.exe",
        "anthrouter/target/release/anthrouter.exe",
        "anthrouter/target/release/anthrouter",
        "anthrouter/anthrouter.exe",
        "anthrouter/anthrouter",
    ];
    for rel in &anthrouter_paths {
        let p = root.join(rel);
        if p.exists() {
            binaries.push(DetectedBinary {
                id: "router-rust".to_string(),
                name: "anthrouter".to_string(),
                path: p.to_string_lossy().to_string(),
                default_port: 18080,
            });
            break;
        }
    }

    // Other legacy binaries (standalone executables)
    let legacy_candidates = [
        ("impersonator-rust", "claude-impersonator", "claude-impersonator.exe", 18081u16),
        ("translator-rust", "claude-translator", "claude-translator.exe", 18082),
    ];
    for (id, name, rel_path, port) in &legacy_candidates {
        let full_path = root.join(rel_path);
        if full_path.exists() {
            binaries.push(DetectedBinary {
                id: id.to_string(),
                name: name.to_string(),
                path: full_path.to_string_lossy().to_string(),
                default_port: *port,
            });
            continue;
        }
        let linux_path = root.join(rel_path.trim_end_matches(".exe"));
        if linux_path.exists() {
            binaries.push(DetectedBinary {
                id: id.to_string(),
                name: name.to_string(),
                path: linux_path.to_string_lossy().to_string(),
                default_port: *port,
            });
        }
    }

    ok_json(json!({"binaries": binaries}))
}
