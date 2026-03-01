//! Session file writer for the GUI dashboard.
//!
//! Writes JSON files to ~/.claude/multi-account/sessions/ with prefix `rs_`.
//! The GUI polls this directory every 5s and merges sessions from both proxies
//! (Python prefix `py_`, Rust prefix `rs_`).

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionData {
    pub session_id: String,
    pub source: String,
    pub account_email: String,
    pub model: String,
    pub started_at: String,
    pub updated_at: String,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
    pub request_count: u64,
    #[serde(default)]
    pub estimated_cost_usd: f64,
    pub client_ip: String,
}

/// Token pricing per million tokens: (input, output, cache_read, cache_write).
fn estimate_cost(model: &str, input: u64, output: u64, cache_read: u64, cache_write: u64) -> f64 {
    let m = model.to_lowercase();
    let (pi, po, pcr, pcw) = if m.contains("opus") {
        (15.0, 75.0, 1.875, 18.75)
    } else if m.contains("haiku") {
        (0.80, 4.0, 0.08, 1.0)
    } else {
        // sonnet / default
        (3.0, 15.0, 0.30, 3.75)
    };
    (input as f64 * pi + output as f64 * po
        + cache_read as f64 * pcr + cache_write as f64 * pcw)
        / 1_000_000.0
}

pub struct SessionWriter {
    sessions_dir: PathBuf,
    cache: Mutex<HashMap<String, SessionData>>,
}

impl SessionWriter {
    pub fn new(multi_account_dir: &Path) -> Self {
        let sessions_dir = multi_account_dir.join("sessions");
        if let Err(e) = std::fs::create_dir_all(&sessions_dir) {
            tracing::warn!("Failed to create sessions dir: {}", e);
        }

        // Clean old rs_ files on startup (>2h)
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            let cutoff = std::time::SystemTime::now()
                .checked_sub(std::time::Duration::from_secs(7200))
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("rs_") && name.ends_with(".json") {
                    if let Ok(meta) = entry.metadata() {
                        if let Ok(modified) = meta.modified() {
                            if modified < cutoff {
                                let _ = std::fs::remove_file(entry.path());
                            }
                        }
                    }
                }
            }
        }

        Self {
            sessions_dir,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Records a proxied request for session tracking.
    ///
    /// `input_tokens` and `output_tokens` may be 0 if not yet known
    /// (the session file will still be created/updated for request counting).
    /// Computes the session ID for a given email/model at the current time bucket.
    fn current_session_id(email: &str, model: &str) -> (String, u64) {
        let bucket = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / 300) // 5-minute buckets
            .unwrap_or(0);
        let raw_id = format!("{}:{}:{}", email, model, bucket);
        (format!("rs_{}", Self::simple_hash(&raw_id)), bucket)
    }

    pub fn record_request(
        &self,
        email: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        client_ip: &str,
    ) {
        let (session_id, _bucket) = Self::current_session_id(email, model);

        let now = chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let mut cache = self.cache.lock();
        let session = cache.entry(session_id.clone()).or_insert_with(|| SessionData {
            session_id: session_id.clone(),
            source: "rust_router".to_string(),
            account_email: email.to_string(),
            model: model.to_string(),
            started_at: now.clone(),
            updated_at: now.clone(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            request_count: 0,
            estimated_cost_usd: 0.0,
            client_ip: client_ip.to_string(),
        });

        session.updated_at = now;
        session.total_input_tokens += input_tokens;
        session.total_output_tokens += output_tokens;
        session.request_count += 1;
        session.estimated_cost_usd = estimate_cost(
            &session.model,
            session.total_input_tokens,
            session.total_output_tokens,
            session.cache_read_tokens,
            session.cache_creation_tokens,
        );

        // Write to disk (non-critical — don't propagate errors)
        let filepath = self.sessions_dir.join(format!("{}.json", session_id));
        if let Ok(json) = serde_json::to_string(session) {
            let _ = std::fs::write(&filepath, json);
        }
    }

    /// Update token counts for a session (called after response stream completes).
    ///
    /// Checks both the current and previous 5-minute bucket to handle the case
    /// where record_request ran in bucket N but the stream finished in bucket N+1.
    pub fn update_tokens(
        &self,
        email: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
    ) {
        if input_tokens == 0 && output_tokens == 0
            && cache_read_tokens == 0 && cache_creation_tokens == 0
        {
            return;
        }
        let bucket = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / 300)
            .unwrap_or(0);

        let mut cache = self.cache.lock();

        // Try current bucket first, then previous bucket (handles boundary crossing)
        for b in [bucket, bucket.saturating_sub(1)] {
            let raw_id = format!("{}:{}:{}", email, model, b);
            let session_id = format!("rs_{}", Self::simple_hash(&raw_id));

            if let Some(session) = cache.get_mut(&session_id) {
                session.total_input_tokens += input_tokens;
                session.total_output_tokens += output_tokens;
                session.cache_read_tokens += cache_read_tokens;
                session.cache_creation_tokens += cache_creation_tokens;
                session.estimated_cost_usd = estimate_cost(
                    &session.model,
                    session.total_input_tokens,
                    session.total_output_tokens,
                    session.cache_read_tokens,
                    session.cache_creation_tokens,
                );
                let filepath = self.sessions_dir.join(format!("{}.json", session_id));
                if let Ok(json) = serde_json::to_string(session) {
                    let _ = std::fs::write(&filepath, json);
                }
                return;
            }
        }
    }

    fn simple_hash(input: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        input.hash(&mut hasher);
        format!("{:012x}", hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_creates_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let writer = SessionWriter::new(&dir.path().to_path_buf());
        writer.record_request("test@example.com", "claude-sonnet", 0, 0, "127.0.0.1");

        let sessions_dir = dir.path().join("sessions");
        let files: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("rs_"))
            .collect();
        assert_eq!(files.len(), 1);

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        let data: SessionData = serde_json::from_str(&content).unwrap();
        assert_eq!(data.account_email, "test@example.com");
        assert_eq!(data.model, "claude-sonnet");
        assert_eq!(data.request_count, 1);
        assert_eq!(data.client_ip, "127.0.0.1");
    }

    #[test]
    fn record_increments_request_count() {
        let dir = tempfile::tempdir().unwrap();
        let writer = SessionWriter::new(&dir.path().to_path_buf());
        writer.record_request("a@b.com", "m", 0, 0, "10.0.0.1");
        writer.record_request("a@b.com", "m", 0, 0, "10.0.0.1");
        writer.record_request("a@b.com", "m", 0, 0, "10.0.0.1");

        let sessions_dir = dir.path().join("sessions");
        let files: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 1);

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        let data: SessionData = serde_json::from_str(&content).unwrap();
        assert_eq!(data.request_count, 3);
    }

    #[test]
    fn update_tokens_adds_to_existing_session() {
        let dir = tempfile::tempdir().unwrap();
        let writer = SessionWriter::new(&dir.path().to_path_buf());
        writer.record_request("a@b.com", "claude-sonnet", 0, 0, "::1");
        writer.update_tokens("a@b.com", "claude-sonnet", 100, 50, 1000, 200);

        let sessions_dir = dir.path().join("sessions");
        let files: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        let content = std::fs::read_to_string(files[0].path()).unwrap();
        let data: SessionData = serde_json::from_str(&content).unwrap();
        assert_eq!(data.total_input_tokens, 100);
        assert_eq!(data.total_output_tokens, 50);
        assert_eq!(data.cache_read_tokens, 1000);
        assert_eq!(data.cache_creation_tokens, 200);
        assert!(data.estimated_cost_usd > 0.0);
    }

    #[test]
    fn update_tokens_zero_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let writer = SessionWriter::new(&dir.path().to_path_buf());
        writer.record_request("a@b.com", "m", 0, 0, "127.0.0.1");
        writer.update_tokens("a@b.com", "m", 0, 0, 0, 0);  // Should be a no-op

        let cache = writer.cache.lock();
        for (_, session) in cache.iter() {
            assert_eq!(session.total_input_tokens, 0);
            assert_eq!(session.total_output_tokens, 0);
        }
    }

    #[test]
    fn session_id_deterministic() {
        let (id1, _) = SessionWriter::current_session_id("a@b.com", "m");
        let (id2, _) = SessionWriter::current_session_id("a@b.com", "m");
        assert_eq!(id1, id2);
        assert!(id1.starts_with("rs_"));
    }

    #[test]
    fn different_emails_different_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let writer = SessionWriter::new(&dir.path().to_path_buf());
        writer.record_request("a@b.com", "m", 0, 0, "1.2.3.4");
        writer.record_request("c@d.com", "m", 0, 0, "1.2.3.5");

        let sessions_dir = dir.path().join("sessions");
        let files: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn cost_estimation_by_model() {
        let cost = estimate_cost("claude-opus-4-6", 1_000_000, 100_000, 0, 0);
        assert!((cost - 22.5).abs() < 0.01);

        let cost = estimate_cost("claude-sonnet-4-20250514", 1_000_000, 100_000, 0, 0);
        assert!((cost - 4.5).abs() < 0.01);

        let cost = estimate_cost("claude-haiku-4-5-20251001", 1_000_000, 100_000, 0, 0);
        assert!((cost - 1.2).abs() < 0.01);
    }

    #[test]
    fn source_is_rust_router() {
        let dir = tempfile::tempdir().unwrap();
        let writer = SessionWriter::new(&dir.path().to_path_buf());
        writer.record_request("a@b.com", "claude-sonnet", 0, 0, "127.0.0.1");

        let sessions_dir = dir.path().join("sessions");
        let files: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        let content = std::fs::read_to_string(files[0].path()).unwrap();
        let data: SessionData = serde_json::from_str(&content).unwrap();
        assert_eq!(data.source, "rust_router");
    }
}
