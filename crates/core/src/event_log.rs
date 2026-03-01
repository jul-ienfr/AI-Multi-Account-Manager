//! Journal d'événements applicatifs — persisté sur disque en mode append.
//!
//! Chaque ligne du fichier suit le format :
//! ```text
//! [2026-01-15T10:30:00Z] [INFO] event_name | {"key": "value"}
//! ```
//!
//! L'écriture est thread-safe via l'ouverture en mode `append` à chaque appel
//! (le noyau garantit l'atomicité des petites écritures sur la plupart des FS).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::Utc;
use tracing::warn;

// ---------------------------------------------------------------------------
// EventLog
// ---------------------------------------------------------------------------

/// Journal d'événements applicatifs écrits dans un fichier texte (append-only).
///
/// Chaque appel à [`EventLog::log`] ouvre le fichier, écrit une ligne et le
/// referme immédiatement, ce qui garantit qu'aucune donnée n'est perdue même
/// en cas de crash.
pub struct EventLog {
    /// Chemin vers le fichier de log (ex. `~/.claude/multi-account/events.log`).
    path: PathBuf,
}

impl EventLog {
    /// Crée un `EventLog` dont les entrées seront écrites dans `base_dir/events.log`.
    ///
    /// Le répertoire doit exister (ou être créé par l'appelant).
    pub fn new(base_dir: &Path) -> Self {
        Self {
            path: base_dir.join("events.log"),
        }
    }

    /// Crée un `EventLog` pointant directement vers un fichier spécifique.
    ///
    /// Utile pour les tests ou pour un chemin personnalisé.
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    /// Écrit une entrée dans le journal.
    ///
    /// # Paramètres
    /// - `level`   — niveau de log (`"INFO"`, `"WARN"`, `"ERROR"`, `"DEBUG"`, …)
    /// - `event`   — nom court de l'événement (ex. `"quota_update"`, `"auto_switch"`)
    /// - `details` — objet JSON optionnel avec des métadonnées supplémentaires
    ///
    /// # Format de ligne
    /// ```text
    /// [2026-01-15T10:30:00.123456Z] [INFO] quota_update | {"tokens5h":1234}
    /// [2026-01-15T10:30:01.456789Z] [WARN] auto_switch | null
    /// ```
    ///
    /// Les erreurs d'écriture sont loguées via `tracing::warn` mais n'interrompent
    /// pas l'exécution (best-effort logging).
    pub fn log(&self, level: &str, event: &str, details: Option<serde_json::Value>) {
        let timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        let details_str = match &details {
            Some(v) => v.to_string(),
            None => "null".to_string(),
        };
        let line = format!("[{}] [{}] {} | {}\n", timestamp, level, event, details_str);

        match std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(line.as_bytes()) {
                    warn!("EventLog: failed to write to {:?}: {}", self.path, e);
                }
            }
            Err(e) => {
                warn!("EventLog: cannot open {:?}: {}", self.path, e);
            }
        }
    }

    /// Retourne le chemin vers le fichier de log.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_log() -> (EventLog, TempDir) {
        let tmp = TempDir::new().unwrap();
        let log = EventLog::new(tmp.path());
        (log, tmp)
    }

    /// Lit le contenu du fichier de log en tant que `String`.
    fn read_log(log: &EventLog) -> String {
        std::fs::read_to_string(log.path()).unwrap_or_default()
    }

    #[test]
    fn test_log_creates_file() {
        let (log, _tmp) = make_log();
        assert!(!log.path().exists(), "file should not exist before first log");

        log.log("INFO", "startup", None);

        assert!(log.path().exists(), "file should be created after log");
    }

    #[test]
    fn test_log_format_without_details() {
        let (log, _tmp) = make_log();
        log.log("INFO", "test_event", None);

        let content = read_log(&log);
        assert!(content.contains("[INFO]"), "must contain level");
        assert!(content.contains("test_event"), "must contain event name");
        assert!(content.contains("| null"), "no-detail entry must end with 'null'");
    }

    #[test]
    fn test_log_format_with_details() {
        let (log, _tmp) = make_log();
        let details = serde_json::json!({"account": "alice", "tokens": 42000});
        log.log("WARN", "quota_update", Some(details));

        let content = read_log(&log);
        assert!(content.contains("[WARN]"));
        assert!(content.contains("quota_update"));
        assert!(content.contains("alice"));
        assert!(content.contains("42000"));
    }

    #[test]
    fn test_log_appends_multiple_entries() {
        let (log, _tmp) = make_log();

        log.log("INFO", "first", None);
        log.log("INFO", "second", Some(serde_json::json!({"x": 1})));
        log.log("ERROR", "third", None);

        let content = read_log(&log);
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3, "should have 3 lines");
        assert!(lines[0].contains("first"));
        assert!(lines[1].contains("second"));
        assert!(lines[2].contains("third"));
    }

    #[test]
    fn test_log_iso8601_timestamp() {
        let (log, _tmp) = make_log();
        log.log("DEBUG", "ts_check", None);

        let content = read_log(&log);
        // Timestamp must appear inside brackets like [2026-...]
        assert!(content.contains("[202"), "must contain a recent timestamp");
        assert!(content.contains("Z]"), "timestamp must be in UTC (ends with Z)");
    }

    #[test]
    fn test_log_does_not_panic_on_bad_dir() {
        // Point to a file that cannot be created (parent is a file, not a dir)
        let tmp = TempDir::new().unwrap();
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, "I am a file").unwrap();
        // Try to use blocker/events.log — parent "blocker" is a file, not a dir
        let log = EventLog::with_path(blocker.join("events.log"));
        // Should not panic; error is silently absorbed
        log.log("INFO", "should_not_panic", None);
    }

    #[test]
    fn test_log_all_levels() {
        let (log, _tmp) = make_log();
        for level in &["INFO", "WARN", "ERROR", "DEBUG"] {
            log.log(level, "level_test", None);
        }

        let content = read_log(&log);
        assert!(content.contains("[INFO]"));
        assert!(content.contains("[WARN]"));
        assert!(content.contains("[ERROR]"));
        assert!(content.contains("[DEBUG]"));
    }

    #[test]
    fn test_log_complex_details() {
        let (log, _tmp) = make_log();
        let details = serde_json::json!({
            "from": "account_a",
            "to": "account_b",
            "reason": "threshold exceeded",
            "utilization_pct": 87.5
        });
        log.log("INFO", "auto_switch", Some(details));

        let content = read_log(&log);
        assert!(content.contains("auto_switch"));
        assert!(content.contains("account_a"));
        assert!(content.contains("account_b"));
        assert!(content.contains("87.5"));
    }

    #[test]
    fn test_with_path_constructor() {
        let tmp = TempDir::new().unwrap();
        let custom_path = tmp.path().join("custom_events.log");
        let log = EventLog::with_path(custom_path.clone());

        log.log("INFO", "custom_path_test", None);

        assert!(custom_path.exists());
        let content = std::fs::read_to_string(&custom_path).unwrap();
        assert!(content.contains("custom_path_test"));
    }
}
