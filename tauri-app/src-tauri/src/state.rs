//! AppState — état global de l'application Tauri.
//!
//! Partagé entre tous les commands IPC via `tauri::State<AppState>`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use tracing::info;

use ai_manager_core::credentials::CredentialsCache;
use ai_manager_core::config::ConfigCache;
use ai_manager_core::event_log::EventLog;
use ai_manager_core::quota::VelocityCalculator;
use ai_manager_core::types::{Peer, ProxyStatus};

/// Runtime state for a single proxy instance.
pub struct ProxyInstanceRuntime {
    pub status: RwLock<ProxyStatus>,
    pub task_handle: Mutex<Option<tokio::task::AbortHandle>>,
    pub child_process: Mutex<Option<std::process::Child>>,
}

/// Cached quota metrics per account (populated by refresh loop).
#[derive(Debug, Clone, Default)]
pub struct QuotaMetricsCache {
    pub ema_velocity: f64,
    pub time_to_threshold: Option<f64>,
    pub resets_at_5h: Option<String>,
    pub resets_at_7d: Option<String>,
}

/// État global de l'application.
pub struct AppState {
    pub credentials: Arc<CredentialsCache>,
    pub config: Arc<ConfigCache>,
    /// Legacy proxy status (for backward compat with existing commands).
    pub proxy_router: Arc<RwLock<ProxyStatus>>,
    pub proxy_impersonator: Arc<RwLock<ProxyStatus>>,
    pub proxy_router_task: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    pub proxy_impersonator_task: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    /// Dynamic proxy instances runtime state.
    pub proxy_instances: Arc<RwLock<HashMap<String, Arc<ProxyInstanceRuntime>>>>,
    /// Pairs P2P connectés.
    pub peers: Arc<RwLock<Vec<Peer>>>,
    /// Chemin vers credentials-multi.json.
    pub credentials_path: PathBuf,
    /// Chemin vers settings.json.
    pub settings_path: PathBuf,
    /// Velocity calculators per account (populated by refresh loop).
    pub velocity_calculators: Arc<RwLock<HashMap<String, VelocityCalculator>>>,
    /// Cached quota metrics per account.
    pub quota_metrics: Arc<RwLock<HashMap<String, QuotaMetricsCache>>>,
    /// Comptes dont le dernier refresh OAuth a retourné `invalid_grant`.
    ///
    /// Ces comptes sont exclus de l'auto-switch et de la rotation jusqu'à ce
    /// qu'un nouveau refresh_token soit détecté (rotation de RT par Claude).
    /// Partagé entre la boucle de refresh (`quota_refresh_loop`) et le
    /// `SwitchController`.
    pub invalid_grant_accounts: Arc<RwLock<HashSet<String>>>,
    /// Journal d'événements applicatifs (Phase 6.5).
    pub event_log: Arc<EventLog>,
}

impl AppState {
    /// Initialise l'état depuis les chemins de fichiers par défaut.
    pub fn init() -> anyhow::Result<Self> {
        // Credentials: même chemin que la V2 Python → ~/.claude/multi-account/
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let multi_account_dir = home.join(".claude").join("multi-account");
        std::fs::create_dir_all(&multi_account_dir)?;
        let credentials_path = multi_account_dir.join("credentials-multi.json");

        // Config V3 dans son propre dossier
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ai-manager");
        std::fs::create_dir_all(&config_dir)?;
        let settings_path = config_dir.join("settings.json");

        let credentials = CredentialsCache::load(&credentials_path)?;
        let config = ConfigCache::load(&settings_path)?;

        // --- Migration: rename "Impersonator" → "Anthrouter" ---
        {
            let mut cfg = config.write();
            let mut migrated = false;
            for inst in cfg.proxy.instances.iter_mut() {
                if inst.id == "impersonator" && inst.name == "Impersonator" {
                    inst.name = "Anthrouter".to_string();
                    migrated = true;
                }
            }
            drop(cfg);
            if migrated {
                let _ = config.persist();
                info!("Migrated proxy instance name: Impersonator → Anthrouter");
            }
        }

        // Initialise le journal d'événements
        let event_log = Arc::new(EventLog::new(&multi_account_dir));

        // Initialize proxy instances runtime from config
        let mut instances_map = HashMap::new();
        for inst_cfg in &config.read().proxy.instances {
            let runtime = Arc::new(ProxyInstanceRuntime {
                status: RwLock::new(ProxyStatus {
                    port: inst_cfg.port,
                    ..Default::default()
                }),
                task_handle: Mutex::new(None),
                child_process: Mutex::new(None),
            });
            instances_map.insert(inst_cfg.id.clone(), runtime);
        }

        info!(
            "AppState initialized: {} accounts, {} proxy instances, credentials={:?}",
            credentials.account_count(),
            instances_map.len(),
            credentials_path
        );

        Ok(Self {
            credentials,
            config,
            proxy_router: Arc::new(RwLock::new(ProxyStatus::default())),
            proxy_impersonator: Arc::new(RwLock::new(ProxyStatus::default())),
            proxy_router_task: Arc::new(Mutex::new(None)),
            proxy_impersonator_task: Arc::new(Mutex::new(None)),
            proxy_instances: Arc::new(RwLock::new(instances_map)),
            peers: Arc::new(RwLock::new(Vec::new())),
            credentials_path,
            settings_path,
            velocity_calculators: Arc::new(RwLock::new(HashMap::new())),
            quota_metrics: Arc::new(RwLock::new(HashMap::new())),
            invalid_grant_accounts: Arc::new(RwLock::new(HashSet::new())),
            event_log,
        })
    }
}
