//! Gestion des profils de configuration — snapshots nommés de la config.
//!
//! Les profils sont stockés dans `~/.claude/multi-account/profiles/{name}.json`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::{CoreError, Result};

// ---------------------------------------------------------------------------
// Types publics
// ---------------------------------------------------------------------------

/// Informations d'un profil (retournées par `list()`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileInfo {
    /// Nom du profil (sans extension).
    pub name: String,
    /// Date de création / dernière modification du fichier.
    pub created_at: DateTime<Utc>,
    /// Taille du fichier JSON en octets.
    pub size_bytes: u64,
}

// ---------------------------------------------------------------------------
// ProfileManager
// ---------------------------------------------------------------------------

/// Gestionnaire de profils de configuration.
///
/// Un profil = snapshot nommé de la configuration, persisté sur disque
/// sous forme de fichier JSON dans `<dir>/{name}.json`.
pub struct ProfileManager {
    dir: PathBuf,
}

impl ProfileManager {
    /// Crée un `ProfileManager` dont les profils sont stockés dans `base_dir`.
    ///
    /// Le répertoire est créé si nécessaire lors de la première opération
    /// d'écriture.
    pub fn new(base_dir: &Path) -> Self {
        Self {
            dir: base_dir.to_path_buf(),
        }
    }

    /// Chemin complet vers le fichier JSON d'un profil donné.
    fn profile_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{}.json", name))
    }

    /// Sauvegarde un profil sur disque.
    ///
    /// Si un profil du même nom existe déjà, il est écrasé.
    ///
    /// # Errors
    /// Retourne [`CoreError::Io`] si la création du répertoire ou l'écriture
    /// du fichier échoue, [`CoreError::Json`] en cas d'erreur de sérialisation.
    pub fn save(&self, name: &str, config: &serde_json::Value) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.profile_path(name);
        let json = serde_json::to_string_pretty(config)?;
        std::fs::write(&path, json)?;
        info!("Profile '{}' saved to {:?}", name, path);
        Ok(())
    }

    /// Charge un profil depuis le disque.
    ///
    /// # Errors
    /// Retourne [`CoreError::NotFound`] si le fichier n'existe pas,
    /// [`CoreError::Io`] ou [`CoreError::Json`] en cas d'erreur de lecture.
    pub fn load(&self, name: &str) -> Result<serde_json::Value> {
        let path = self.profile_path(name);
        if !path.exists() {
            return Err(CoreError::NotFound(format!("Profile '{}' not found", name)));
        }
        let content = std::fs::read_to_string(&path)?;
        let value: serde_json::Value = serde_json::from_str(&content)?;
        debug!("Profile '{}' loaded from {:?}", name, path);
        Ok(value)
    }

    /// Liste tous les profils disponibles, triés par nom.
    ///
    /// # Errors
    /// Retourne [`CoreError::Io`] si le répertoire est inaccessible.
    pub fn list(&self) -> Result<Vec<ProfileInfo>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut profiles = Vec::new();

        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();

            // Ne garder que les fichiers .json
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            let metadata = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    warn!("Cannot read metadata for {:?}: {}", path, e);
                    continue;
                }
            };

            let size_bytes = metadata.len();
            let created_at = metadata
                .modified()
                .ok()
                .and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|d| {
                            DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                                .unwrap_or_else(Utc::now)
                        })
                })
                .unwrap_or_else(Utc::now);

            profiles.push(ProfileInfo {
                name,
                created_at,
                size_bytes,
            });
        }

        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    /// Supprime un profil du disque.
    ///
    /// # Errors
    /// Retourne [`CoreError::NotFound`] si le profil n'existe pas,
    /// [`CoreError::Io`] en cas d'erreur de suppression.
    pub fn delete(&self, name: &str) -> Result<()> {
        let path = self.profile_path(name);
        if !path.exists() {
            return Err(CoreError::NotFound(format!("Profile '{}' not found", name)));
        }
        std::fs::remove_file(&path)?;
        info!("Profile '{}' deleted", name);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_manager() -> (ProfileManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let mgr = ProfileManager::new(tmp.path());
        (mgr, tmp)
    }

    #[test]
    fn test_save_and_load_profile() {
        let (mgr, _tmp) = make_manager();
        let config = serde_json::json!({"key": "value", "num": 42});

        mgr.save("test", &config).unwrap();
        let loaded = mgr.load("test").unwrap();
        assert_eq!(loaded["key"], "value");
        assert_eq!(loaded["num"], 42);
    }

    #[test]
    fn test_load_nonexistent_returns_not_found() {
        let (mgr, _tmp) = make_manager();
        let err = mgr.load("nonexistent").unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn test_list_empty_dir() {
        let (mgr, _tmp) = make_manager();
        let profiles = mgr.list().unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn test_list_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let mgr = ProfileManager::new(&tmp.path().join("profiles"));
        let profiles = mgr.list().unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn test_save_creates_directory_automatically() {
        let tmp = TempDir::new().unwrap();
        let profiles_dir = tmp.path().join("nested").join("profiles");
        let mgr = ProfileManager::new(&profiles_dir);

        let config = serde_json::json!({"nested": true});
        mgr.save("nested_test", &config).unwrap();

        assert!(profiles_dir.join("nested_test.json").exists());
    }

    #[test]
    fn test_list_returns_correct_metadata() {
        let (mgr, _tmp) = make_manager();
        let config_a = serde_json::json!({"profile": "a"});
        let config_b = serde_json::json!({"profile": "b", "extra": "data"});

        mgr.save("alpha", &config_a).unwrap();
        mgr.save("beta", &config_b).unwrap();

        let profiles = mgr.list().unwrap();
        assert_eq!(profiles.len(), 2);
        // Sorted alphabetically
        assert_eq!(profiles[0].name, "alpha");
        assert_eq!(profiles[1].name, "beta");
        // Sizes must be > 0
        assert!(profiles[0].size_bytes > 0);
        assert!(profiles[1].size_bytes > 0);
        // beta has more content → larger file
        assert!(profiles[1].size_bytes > profiles[0].size_bytes);
    }

    #[test]
    fn test_delete_profile() {
        let (mgr, _tmp) = make_manager();
        let config = serde_json::json!({"deleteme": true});

        mgr.save("to_delete", &config).unwrap();
        mgr.delete("to_delete").unwrap();

        let err = mgr.load("to_delete").unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[test]
    fn test_delete_nonexistent_returns_not_found() {
        let (mgr, _tmp) = make_manager();
        let err = mgr.delete("ghost").unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[test]
    fn test_overwrite_existing_profile() {
        let (mgr, _tmp) = make_manager();

        let v1 = serde_json::json!({"version": 1});
        let v2 = serde_json::json!({"version": 2, "new_field": "added"});

        mgr.save("evolving", &v1).unwrap();
        mgr.save("evolving", &v2).unwrap();

        let loaded = mgr.load("evolving").unwrap();
        assert_eq!(loaded["version"], 2);
        assert_eq!(loaded["new_field"], "added");
    }

    #[test]
    fn test_list_ignores_non_json_files() {
        let (mgr, tmp) = make_manager();
        // Save a real profile
        mgr.save("real", &serde_json::json!({})).unwrap();
        // Create a non-JSON file in the profiles dir
        std::fs::write(tmp.path().join("ignored.txt"), "not json").unwrap();

        let profiles = mgr.list().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "real");
    }
}
