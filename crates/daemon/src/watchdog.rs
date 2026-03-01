//! Watcher fichier credentials — surveille `credentials-multi.json`
//! et déclenche un reload du cache lors de modifications externes.
//!
//! Traduit la logique Python du watchdog de fichier credentials en Rust
//! avec la crate `notify` (inotify/FSEvents/ReadDirectoryChangesW selon la plateforme).
//!
//! ## Phase 3.4c — Matching par email
//!
//! Quand une modification du fichier `.credentials.json` de Claude Code CLI est détectée,
//! le reconciliement est fait par le champ `email` de chaque compte (et non par la clé
//! du dictionnaire, qui peut être un UUID arbitraire dans le fichier CC).
//!
//! Le chemin du fichier CC peut être passé via `cc_credentials_path`.  Quand ce chemin
//! est renseigné et que l'événement filesystem correspond à ce fichier, on appelle
//! `CredentialsCache::merge_from_cc_file` qui effectue la réconciliation par email.
//! Sinon, on appelle le `reload()` classique (remplacement intégral du cache).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use ai_core::credentials::CredentialsCache;

/// Délai de debounce : évite des reloads multiples pour une seule sauvegarde.
const DEBOUNCE_MS: u64 = 500;

/// Surveille `credentials-multi.json` (et optionnellement le `.credentials.json` de CC)
/// et recharge le cache lors de modifications.
pub struct CredentialsWatchdog {
    credentials: Arc<CredentialsCache>,
    /// Chemin de notre propre fichier `credentials-multi.json`.
    path: PathBuf,
    /// Chemin optionnel du fichier `.credentials.json` de Claude Code CLI.
    /// Quand ce fichier change, la réconciliation est faite par email (3.4c).
    cc_credentials_path: Option<PathBuf>,
}

impl CredentialsWatchdog {
    /// Crée un nouveau watchdog (surveille uniquement `credentials-multi.json`).
    pub fn new(credentials: Arc<CredentialsCache>, path: PathBuf) -> Self {
        Self {
            credentials,
            path,
            cc_credentials_path: None,
        }
    }

    /// Crée un watchdog qui surveille aussi le `.credentials.json` de Claude Code CLI.
    ///
    /// Quand ce fichier change, la réconciliation est faite par email (3.4c) plutôt
    /// que par la clé du dictionnaire (qui peut être un UUID arbitraire dans CC).
    pub fn with_cc_credentials(mut self, cc_path: PathBuf) -> Self {
        self.cc_credentials_path = Some(cc_path);
        self
    }

    /// Lance la surveillance.
    ///
    /// S'arrête quand `shutdown` reçoit `true`.
    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        info!("CredentialsWatchdog started watching {:?}", self.path);
        if let Some(ref cc) = self.cc_credentials_path {
            info!("CredentialsWatchdog also watching CC file {:?} (email-matching)", cc);
        }

        let (notify_tx, mut notify_rx) = mpsc::channel::<Result<Event, notify::Error>>(32);

        // Crée le watcher dans un thread tokio blocking.
        // On surveille les répertoires parents de *tous* les fichiers d'intérêt
        // (certains éditeurs sauvegardent atomiquement via rename).
        let path = self.path.clone();
        let cc_path = self.cc_credentials_path.clone();
        let watcher_result: anyhow::Result<RecommendedWatcher> = tokio::task::spawn_blocking(
            move || {
                let tx = notify_tx.clone();
                let mut watcher = notify::recommended_watcher(move |res| {
                    if tx.blocking_send(res).is_err() {
                        debug!("CredentialsWatchdog: notify channel closed");
                    }
                })
                .map_err(|e| anyhow::anyhow!("watcher creation: {e}"))?;

                // Répertoire du fichier principal
                let dir = path.parent().unwrap_or(std::path::Path::new("."));
                watcher
                    .watch(dir, RecursiveMode::NonRecursive)
                    .map_err(|e| anyhow::anyhow!("watch main: {e}"))?;

                // Répertoire du fichier CC (si différent)
                if let Some(ref cc) = cc_path {
                    let cc_dir = cc.parent().unwrap_or(std::path::Path::new("."));
                    if cc_dir != dir {
                        watcher
                            .watch(cc_dir, RecursiveMode::NonRecursive)
                            .map_err(|e| anyhow::anyhow!("watch cc: {e}"))?;
                    }
                }

                Ok(watcher)
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking: {e}"))
        .and_then(|r| r);

        let _watcher = match watcher_result {
            Ok(w) => w,
            Err(e) => {
                error!("CredentialsWatchdog: failed to start file watcher: {}", e);
                return;
            }
        };

        let mut last_reload = tokio::time::Instant::now();

        loop {
            tokio::select! {
                // Événement filesystem
                Some(event_result) = notify_rx.recv() => {
                    match event_result {
                        Ok(event) => {
                            // Détermine quel fichier est concerné
                            let is_main = self.is_main_credentials_event(&event);
                            let is_cc = self.is_cc_credentials_event(&event);

                            if is_main || is_cc {
                                // Debounce : ignore les événements trop rapprochés
                                let now = tokio::time::Instant::now();
                                if now.duration_since(last_reload).as_millis() < DEBOUNCE_MS as u128 {
                                    debug!("CredentialsWatchdog: debounced event");
                                    continue;
                                }
                                last_reload = now;

                                // Petite pause pour que l'écriture soit terminée
                                tokio::time::sleep(Duration::from_millis(50)).await;

                                if is_cc {
                                    // Phase 3.4c — réconciliation par email depuis le fichier CC
                                    if let Some(ref cc_path) = self.cc_credentials_path {
                                        info!("CredentialsWatchdog: CC credentials changed, merging by email");
                                        match self.credentials.merge_from_cc_file(cc_path) {
                                            Ok(merged) => {
                                                info!(
                                                    "CredentialsWatchdog: CC merge successful ({} account(s) updated)",
                                                    merged
                                                );
                                            }
                                            Err(e) => {
                                                warn!("CredentialsWatchdog: CC merge failed: {}", e);
                                            }
                                        }
                                    }
                                } else {
                                    // Reload classique de notre propre fichier
                                    info!("CredentialsWatchdog: credentials file changed, reloading");
                                    match self.credentials.reload() {
                                        Ok(_) => {
                                            info!(
                                                "CredentialsWatchdog: reload successful ({} accounts)",
                                                self.credentials.account_count()
                                            );
                                        }
                                        Err(e) => {
                                            warn!("CredentialsWatchdog: reload failed: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!("CredentialsWatchdog: notify error: {}", e);
                        }
                    }
                }

                // Signal d'arrêt
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("CredentialsWatchdog: shutdown signal received");
                        break;
                    }
                }
            }
        }

        info!("CredentialsWatchdog stopped");
    }

    /// Vérifie si l'événement concerne notre fichier `credentials-multi.json`.
    fn is_main_credentials_event(&self, event: &Event) -> bool {
        match event.kind {
            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {}
            _ => return false,
        }
        event.paths.iter().any(|p| {
            p.file_name() == self.path.file_name() || p == &self.path
        })
    }

    /// Vérifie si l'événement concerne le fichier `.credentials.json` de Claude Code.
    fn is_cc_credentials_event(&self, event: &Event) -> bool {
        let Some(ref cc_path) = self.cc_credentials_path else {
            return false;
        };
        match event.kind {
            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {}
            _ => return false,
        }
        event.paths.iter().any(|p| {
            p.file_name() == cc_path.file_name() || p == cc_path
        })
    }

    /// Alias pour la compatibilité avec l'ancien code (délègue à `is_main_credentials_event`).
    #[allow(dead_code)]
    fn is_credentials_event(&self, event: &Event) -> bool {
        self.is_main_credentials_event(event)
    }

    /// Retourne le chemin surveillé.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Retourne le chemin du fichier CC (si configuré).
    pub fn cc_credentials_path(&self) -> Option<&PathBuf> {
        self.cc_credentials_path.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use ai_core::credentials::CredentialsCache;
    use tempfile::NamedTempFile;

    #[test]
    fn test_watchdog_new() {
        let cache = CredentialsCache::empty();
        let path = PathBuf::from("/tmp/credentials-multi.json");
        let wd = CredentialsWatchdog::new(cache, path.clone());
        assert_eq!(wd.path(), &path);
        assert!(wd.cc_credentials_path().is_none());
    }

    #[test]
    fn test_watchdog_with_cc_credentials() {
        let cache = CredentialsCache::empty();
        let path = PathBuf::from("/tmp/credentials-multi.json");
        let cc_path = PathBuf::from("/home/user/.claude/.credentials.json");
        let wd = CredentialsWatchdog::new(cache, path.clone())
            .with_cc_credentials(cc_path.clone());
        assert_eq!(wd.path(), &path);
        assert_eq!(wd.cc_credentials_path(), Some(&cc_path));
    }

    #[tokio::test]
    async fn test_watchdog_run_shutdown() {
        let cache = CredentialsCache::empty();
        let path = PathBuf::from("/tmp/credentials-multi.json");
        let wd = Arc::new(CredentialsWatchdog::new(cache, path));
        let (tx, rx) = tokio::sync::watch::channel(false);

        let wd_clone = Arc::clone(&wd);
        let handle = tokio::spawn(async move {
            wd_clone.run(rx).await;
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(true).unwrap();

        tokio::time::timeout(Duration::from_secs(3), handle)
            .await
            .expect("CredentialsWatchdog should stop within 3s")
            .expect("task should not panic");
    }

    #[tokio::test]
    async fn test_watchdog_detects_file_change() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();

        // Écriture initiale
        let initial = serde_json::json!({
            "accounts": {},
            "activeAccount": null
        });
        std::fs::write(&path, initial.to_string()).unwrap();

        let cache = CredentialsCache::load(&path).unwrap();
        let wd = Arc::new(CredentialsWatchdog::new(Arc::clone(&cache), path.clone()));
        let (tx, rx) = tokio::sync::watch::channel(false);

        let wd_clone = Arc::clone(&wd);
        let handle = tokio::spawn(async move {
            wd_clone.run(rx).await;
        });

        // Attend que le watcher démarre
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Modifie le fichier
        let updated = serde_json::json!({
            "accounts": {
                "acc1": {
                    "name": "Test",
                    "email": "test@example.com",
                    "deleted": false,
                    "tokens5h": 0,
                    "tokens7d": 0
                }
            },
            "activeAccount": "acc1"
        });
        std::fs::write(&path, updated.to_string()).unwrap();

        // Attend le reload
        tokio::time::sleep(Duration::from_millis(800)).await;

        tx.send(true).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    }

    #[test]
    fn test_is_main_credentials_event_wrong_file() {
        let cache = CredentialsCache::empty();
        let path = PathBuf::from("/home/user/.claude/credentials-multi.json");
        let wd = CredentialsWatchdog::new(cache, path);

        let mut event = Event::new(EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Content,
        )));
        event.paths.push(PathBuf::from("/tmp/some-other-file.json"));

        assert!(!wd.is_main_credentials_event(&event));
    }

    #[test]
    fn test_is_cc_credentials_event() {
        let cache = CredentialsCache::empty();
        let path = PathBuf::from("/home/user/.claude/credentials-multi.json");
        let cc_path = PathBuf::from("/home/user/.claude/.credentials.json");
        let wd = CredentialsWatchdog::new(cache, path).with_cc_credentials(cc_path.clone());

        let mut event = Event::new(EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Content,
        )));
        event.paths.push(cc_path.clone());

        assert!(wd.is_cc_credentials_event(&event));

        // Un autre fichier ne doit pas matcher
        let mut event2 = Event::new(EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Content,
        )));
        event2.paths.push(PathBuf::from("/tmp/other.json"));
        assert!(!wd.is_cc_credentials_event(&event2));
    }

    #[test]
    fn test_is_cc_event_without_cc_path() {
        // Sans chemin CC configuré, is_cc_credentials_event doit toujours retourner false
        let cache = CredentialsCache::empty();
        let path = PathBuf::from("/home/user/.claude/credentials-multi.json");
        let wd = CredentialsWatchdog::new(cache, path);

        let mut event = Event::new(EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Content,
        )));
        event.paths.push(PathBuf::from("/home/user/.claude/.credentials.json"));

        assert!(!wd.is_cc_credentials_event(&event));
    }
}
