//! API usage tracker — JSONL logging of token consumption.
//!
//! Equivalent Python: api_usage_tracker.py
//!
//! Records each proxied /v1/messages request with token counts to a JSONL file.
//! Provides aggregated stats via an HTTP endpoint.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct UsageEntry {
    pub timestamp: String,
    pub account_email: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_tokens: u64,
    pub client_ip: String,
    pub client_fmt: String,
}

pub struct ApiUsageTracker {
    usage_file: PathBuf,
    pending: Mutex<Vec<UsageEntry>>,
    last_flush: Mutex<std::time::Instant>,
}

impl ApiUsageTracker {
    pub fn new(multi_account_dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(multi_account_dir);
        Self {
            usage_file: multi_account_dir.join("api_usage.jsonl"),
            pending: Mutex::new(Vec::new()),
            last_flush: Mutex::new(std::time::Instant::now()),
        }
    }

    /// Records a request with token usage data.
    pub fn record(
        &self,
        email: &str,
        model: &str,
        usage: &UsageData,
        client_ip: &str,
        client_fmt: &str,
    ) {
        let entry = UsageEntry {
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            account_email: email.to_string(),
            model: model.to_string(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            total_tokens: usage.input_tokens
                + usage.output_tokens
                + usage.cache_read_tokens
                + usage.cache_creation_tokens,
            client_ip: client_ip.to_string(),
            client_fmt: client_fmt.to_string(),
        };

        let mut pending = self.pending.lock();
        pending.push(entry);

        // Flush if 10+ entries or 5+ seconds since last flush
        let should_flush = pending.len() >= 10
            || self.last_flush.lock().elapsed().as_secs_f64() >= 5.0;
        if should_flush {
            self.flush_locked(&mut pending);
        }
    }

    /// Force flush all pending entries to disk.
    pub fn flush(&self) {
        let mut pending = self.pending.lock();
        self.flush_locked(&mut pending);
    }

    fn flush_locked(&self, pending: &mut Vec<UsageEntry>) {
        if pending.is_empty() {
            return;
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.usage_file)
        {
            use std::io::Write;
            for entry in pending.iter() {
                if let Ok(line) = serde_json::to_string(entry) {
                    let _ = writeln!(file, "{}", line);
                }
            }
        }
        pending.clear();
        *self.last_flush.lock() = std::time::Instant::now();
    }

    /// Returns aggregated stats as JSON (for the /_proxy/api/usage endpoint).
    pub fn get_stats(
        &self,
        email: Option<&str>,
        days: u32,
        group_by: &str,
    ) -> serde_json::Value {
        self.flush();

        let cutoff = chrono::Utc::now() - chrono::TimeDelta::days(days as i64);
        let mut daily: std::collections::HashMap<String, AggStats> =
            std::collections::HashMap::new();
        let mut by_model: std::collections::HashMap<String, AggStats> =
            std::collections::HashMap::new();

        if let Ok(content) = std::fs::read_to_string(&self.usage_file) {
            for line in content.lines() {
                if let Ok(entry) = serde_json::from_str::<UsageEntry>(line) {
                    if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&entry.timestamp) {
                        let ts_utc = ts.with_timezone(&chrono::Utc);
                        if ts_utc < cutoff {
                            continue;
                        }
                        if let Some(filter_email) = email {
                            if entry.account_email != filter_email {
                                continue;
                            }
                        }

                        let date_key = ts_utc.format("%Y-%m-%d").to_string();
                        let d = daily.entry(date_key).or_default();
                        d.request_count += 1;
                        d.input_tokens += entry.input_tokens;
                        d.output_tokens += entry.output_tokens;
                        d.total_tokens += entry.total_tokens;

                        let m = by_model.entry(entry.model.clone()).or_default();
                        m.request_count += 1;
                        m.input_tokens += entry.input_tokens;
                        m.output_tokens += entry.output_tokens;
                        m.total_tokens += entry.total_tokens;
                    }
                }
            }
        }

        match group_by {
            "model" => serde_json::to_value(&by_model).unwrap_or_default(),
            _ => serde_json::to_value(&daily).unwrap_or_default(),
        }
    }
}

/// Parsed usage data from an API response.
#[derive(Default)]
pub struct UsageData {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

/// Extract usage from a response body chunk (streaming SSE or non-streaming JSON).
pub fn parse_usage_from_chunk(chunk: &[u8]) -> Option<UsageData> {
    let text = std::str::from_utf8(chunk).ok()?;

    // Non-streaming: single JSON object with "usage"
    if let Ok(data) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(usage) = data.get("usage") {
            return Some(extract_usage(usage));
        }
    }

    // Streaming: look for message_delta event with usage
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("data: ") && line.contains("\"message_delta\"") {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&line[6..]) {
                if let Some(usage) = data.get("usage") {
                    return Some(extract_usage(usage));
                }
            }
        }
    }

    None
}

fn extract_usage(usage: &serde_json::Value) -> UsageData {
    UsageData {
        input_tokens: usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output_tokens: usage
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_read_tokens: usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_creation_tokens: usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    }
}

#[derive(Default, Serialize, Deserialize)]
struct AggStats {
    request_count: u64,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_non_streaming_json() {
        let body = br#"{"type":"message","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":10,"cache_creation_input_tokens":5}}"#;
        let usage = parse_usage_from_chunk(body).expect("should parse usage");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_read_tokens, 10);
        assert_eq!(usage.cache_creation_tokens, 5);
    }

    #[test]
    fn parse_streaming_message_delta() {
        let body = b"event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":200,\"output_tokens\":80}}\n\n";
        let usage = parse_usage_from_chunk(body).expect("should parse SSE delta");
        assert_eq!(usage.input_tokens, 200);
        assert_eq!(usage.output_tokens, 80);
    }

    #[test]
    fn parse_no_usage_returns_none() {
        let body = b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hello\"}}\n\n";
        assert!(parse_usage_from_chunk(body).is_none());
    }

    #[test]
    fn parse_empty_body_returns_none() {
        assert!(parse_usage_from_chunk(b"").is_none());
    }

    #[test]
    fn parse_invalid_utf8_returns_none() {
        assert!(parse_usage_from_chunk(&[0xFF, 0xFE]).is_none());
    }

    #[test]
    fn tracker_record_and_flush() {
        let dir = tempfile::tempdir().unwrap();
        let tracker = ApiUsageTracker::new(&dir.path().to_path_buf());
        let usage = UsageData {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 10,
            cache_creation_tokens: 5,
        };
        tracker.record("test@example.com", "claude-sonnet", &usage, "127.0.0.1", "Claude Code");
        tracker.flush();

        let content = std::fs::read_to_string(dir.path().join("api_usage.jsonl")).unwrap();
        let entry: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(entry["account_email"], "test@example.com");
        assert_eq!(entry["model"], "claude-sonnet");
        assert_eq!(entry["input_tokens"], 100);
        assert_eq!(entry["total_tokens"], 165);
        assert_eq!(entry["client_ip"], "127.0.0.1");
        assert_eq!(entry["client_fmt"], "Claude Code");
    }

    #[test]
    fn tracker_get_stats_empty() {
        let dir = tempfile::tempdir().unwrap();
        let tracker = ApiUsageTracker::new(&dir.path().to_path_buf());
        let stats = tracker.get_stats(None, 7, "day");
        assert_eq!(stats, serde_json::json!({}));
    }
}
