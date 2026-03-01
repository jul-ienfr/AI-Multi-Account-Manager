//! Persistance des statistiques d'utilisation — switches, requêtes, uptime.
//!
//! Les stats sont stockées dans `~/.claude/multi-account/stats.json` et
//! rechargées à chaque lecture pour refléter les modifications d'autres
//! processus (ex. daemon de refresh).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::Result;

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

/// Statistiques globales d'utilisation de l'application.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    /// Nombre total de switches de compte (auto ou manuel).
    pub total_switches: u64,
    /// Nombre de switches par compte cible (`to_key`).
    pub switches_by_account: HashMap<String, u64>,
    /// Nombre total de requêtes traitées par le proxy.
    pub total_requests: u64,
    /// Horodatage du dernier switch (UTC).
    pub last_switch_at: Option<DateTime<Utc>>,
    /// Horodatage du démarrage de l'application (UTC).
    pub uptime_started_at: DateTime<Utc>,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            total_switches: 0,
            switches_by_account: HashMap::new(),
            total_requests: 0,
            last_switch_at: None,
            uptime_started_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// StatsManager
// ---------------------------------------------------------------------------

/// Gestionnaire de persistance des statistiques.
///
/// Les stats sont lues depuis le disque à chaque appel à [`StatsManager::load`]
/// et sauvegardées via [`StatsManager::save`].  Les méthodes
/// [`record_switch`][StatsManager::record_switch] et
/// [`record_request`][StatsManager::record_request] combinent les deux
/// opérations de façon atomique (load → mutate → save).
pub struct StatsManager {
    path: PathBuf,
}

impl StatsManager {
    /// Crée un `StatsManager` dont les stats sont stockées dans
    /// `base_dir/stats.json`.
    pub fn new(base_dir: &Path) -> Self {
        Self {
            path: base_dir.join("stats.json"),
        }
    }

    /// Crée un `StatsManager` pointant directement vers un fichier spécifique.
    ///
    /// Utile pour les tests.
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    /// Charge les stats depuis le disque.
    ///
    /// Si le fichier n'existe pas ou est corrompu, retourne des stats vides
    /// (avec `uptime_started_at` = maintenant).
    pub fn load(&self) -> Stats {
        if !self.path.exists() {
            debug!("Stats file not found at {:?}, using defaults", self.path);
            return Stats::default();
        }
        match std::fs::read_to_string(&self.path) {
            Ok(content) => match serde_json::from_str::<Stats>(&content) {
                Ok(stats) => {
                    debug!("Stats loaded from {:?}", self.path);
                    stats
                }
                Err(e) => {
                    warn!("Failed to parse stats file {:?}: {} — using defaults", self.path, e);
                    Stats::default()
                }
            },
            Err(e) => {
                warn!("Failed to read stats file {:?}: {} — using defaults", self.path, e);
                Stats::default()
            }
        }
    }

    /// Sauvegarde les stats sur le disque.
    ///
    /// # Errors
    /// Retourne [`CoreError::Io`] ou [`CoreError::Json`] en cas d'échec.
    pub fn save(&self, stats: &Stats) -> Result<()> {
        // S'assurer que le répertoire parent existe
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(stats)?;
        std::fs::write(&self.path, json)?;
        debug!("Stats saved to {:?}", self.path);
        Ok(())
    }

    /// Enregistre un switch de compte et sauvegarde sur disque.
    ///
    /// - Incrémente `total_switches`
    /// - Incrémente le compteur pour `to_key` dans `switches_by_account`
    /// - Met à jour `last_switch_at`
    ///
    /// Les erreurs de persistance sont loguées mais n'interrompent pas
    /// l'exécution (best-effort).
    pub fn record_switch(&self, from_key: &str, to_key: &str) {
        let mut stats = self.load();
        stats.total_switches += 1;
        *stats.switches_by_account.entry(to_key.to_string()).or_insert(0) += 1;
        stats.last_switch_at = Some(Utc::now());
        debug!("Recorded switch {} → {}, total={}", from_key, to_key, stats.total_switches);
        if let Err(e) = self.save(&stats) {
            warn!("Failed to persist stats after record_switch: {}", e);
        }
    }

    /// Incrémente le compteur de requêtes et sauvegarde sur disque.
    ///
    /// Les erreurs de persistance sont loguées mais n'interrompent pas
    /// l'exécution (best-effort).
    pub fn record_request(&self) {
        let mut stats = self.load();
        stats.total_requests += 1;
        if let Err(e) = self.save(&stats) {
            warn!("Failed to persist stats after record_request: {}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_manager() -> (StatsManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let mgr = StatsManager::new(tmp.path());
        (mgr, tmp)
    }

    #[test]
    fn test_load_defaults_when_file_missing() {
        let (mgr, _tmp) = make_manager();
        let stats = mgr.load();
        assert_eq!(stats.total_switches, 0);
        assert_eq!(stats.total_requests, 0);
        assert!(stats.last_switch_at.is_none());
        assert!(stats.switches_by_account.is_empty());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let (mgr, _tmp) = make_manager();
        let mut stats = Stats::default();
        stats.total_switches = 5;
        stats.total_requests = 100;
        stats.switches_by_account.insert("alice".to_string(), 3);
        stats.switches_by_account.insert("bob".to_string(), 2);
        stats.last_switch_at = Some(Utc::now());

        mgr.save(&stats).unwrap();

        let loaded = mgr.load();
        assert_eq!(loaded.total_switches, 5);
        assert_eq!(loaded.total_requests, 100);
        assert_eq!(loaded.switches_by_account["alice"], 3);
        assert_eq!(loaded.switches_by_account["bob"], 2);
        assert!(loaded.last_switch_at.is_some());
    }

    #[test]
    fn test_record_switch_increments_counters() {
        let (mgr, _tmp) = make_manager();

        mgr.record_switch("account_a", "account_b");
        mgr.record_switch("account_b", "account_a");
        mgr.record_switch("account_a", "account_b");

        let stats = mgr.load();
        assert_eq!(stats.total_switches, 3);
        assert_eq!(stats.switches_by_account["account_b"], 2);
        assert_eq!(stats.switches_by_account["account_a"], 1);
        assert!(stats.last_switch_at.is_some());
    }

    #[test]
    fn test_record_request_increments_counter() {
        let (mgr, _tmp) = make_manager();

        mgr.record_request();
        mgr.record_request();
        mgr.record_request();

        let stats = mgr.load();
        assert_eq!(stats.total_requests, 3);
        // Switches not touched
        assert_eq!(stats.total_switches, 0);
    }

    #[test]
    fn test_record_switch_sets_last_switch_at() {
        let (mgr, _tmp) = make_manager();
        let before = Utc::now();

        mgr.record_switch("src", "dst");

        let stats = mgr.load();
        let last = stats.last_switch_at.expect("last_switch_at should be set");
        assert!(last >= before, "last_switch_at should be >= before");
        assert!(last <= Utc::now(), "last_switch_at should be <= now");
    }

    #[test]
    fn test_load_returns_defaults_on_corrupt_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("stats.json");
        std::fs::write(&path, "{ this is not valid json !!!").unwrap();

        let mgr = StatsManager::with_path(path);
        // Should not panic; returns defaults
        let stats = mgr.load();
        assert_eq!(stats.total_switches, 0);
    }

    #[test]
    fn test_save_creates_parent_directory() {
        let tmp = TempDir::new().unwrap();
        let nested_path = tmp.path().join("nested").join("dir").join("stats.json");
        let mgr = StatsManager::with_path(nested_path.clone());

        let stats = Stats::default();
        mgr.save(&stats).unwrap();

        assert!(nested_path.exists());
    }

    #[test]
    fn test_uptime_started_at_preserved_across_saves() {
        let (mgr, _tmp) = make_manager();
        let mut stats = Stats::default();
        let original_start = stats.uptime_started_at;

        // Simulate some operations
        stats.total_switches = 1;
        mgr.save(&stats).unwrap();

        let loaded = mgr.load();
        // uptime_started_at should be preserved (same timestamp)
        assert_eq!(
            loaded.uptime_started_at.timestamp(),
            original_start.timestamp()
        );
    }

    #[test]
    fn test_multiple_accounts_tracked_independently() {
        let (mgr, _tmp) = make_manager();

        mgr.record_switch("x", "alice");
        mgr.record_switch("x", "alice");
        mgr.record_switch("x", "bob");
        mgr.record_switch("x", "charlie");
        mgr.record_switch("x", "alice");

        let stats = mgr.load();
        assert_eq!(stats.total_switches, 5);
        assert_eq!(stats.switches_by_account["alice"], 3);
        assert_eq!(stats.switches_by_account["bob"], 1);
        assert_eq!(stats.switches_by_account["charlie"], 1);
    }
}
