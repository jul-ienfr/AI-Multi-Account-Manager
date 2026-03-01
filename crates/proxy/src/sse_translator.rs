//! SSE stream translation layer for anthroute.
//!
//! This module provides two translators that convert Anthropic SSE events
//! to other provider formats, for use when the router needs to forward
//! Anthropic-format streams to clients expecting a different format.
//!
//! # Translators
//!
//! - [`SseAnthropicToOpenai`] — Anthropic SSE → OpenAI chat.completion.chunk SSE
//!   Copied and renamed from `claude-impersonator/src/openai_compat.rs::SseTranslator`.
//!
//! - [`SseAnthropicToGemini`] — Anthropic SSE → Gemini GenerateContentResponse SSE
//!   New translator following the Gemini streaming JSON format.

use bytes::Bytes;
use serde_json::json;

// ============================================================================
// SseAnthropicToOpenai
// ============================================================================

/// Translates Anthropic streaming SSE events to OpenAI chat.completion.chunk format.
///
/// Anthropic sends:
///   `event: message_start / content_block_delta / message_delta / message_stop`
///
/// OpenAI expects:
///   `data: {"choices":[{"delta":{"content":"..."}}]}`
///   `data: [DONE]`
///
/// # Usage
/// ```no_run
/// use anthroute::sse_translator::SseAnthropicToOpenai;
/// let mut t = SseAnthropicToOpenai::new("msg_abc".into(), "claude-sonnet-4-6".into());
/// let out: bytes::Bytes = t.process_chunk(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
/// ```
pub struct SseAnthropicToOpenai {
    buffer: String,
    msg_id: String,
    model: String,
    created: i64,
}

impl SseAnthropicToOpenai {
    /// Create a new translator.
    ///
    /// - `msg_id`: the message ID to embed in each chunk (e.g. `"msg_01abc"`).
    /// - `model`: the model name to embed in each chunk.
    pub fn new(msg_id: String, model: String) -> Self {
        Self {
            buffer: String::new(),
            msg_id,
            model,
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        }
    }

    /// Process a raw bytes chunk from the Anthropic SSE stream.
    ///
    /// Buffers incomplete events and returns translated OpenAI SSE bytes
    /// for every complete Anthropic event block found.  May return empty
    /// `Bytes` if no complete event is available yet.
    pub fn process_chunk(&mut self, chunk: &[u8]) -> Bytes {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        let mut output = String::new();

        // Process complete SSE event blocks (each terminated by \n\n).
        while let Some(pos) = self.buffer.find("\n\n") {
            let event_block = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + 2..].to_string();

            if let Some(translated) = self.translate_event(&event_block) {
                output.push_str(&translated);
            }
        }

        Bytes::from(output)
    }

    // -----------------------------------------------------------------------
    // Private: translate one Anthropic SSE event block → OpenAI SSE string
    // -----------------------------------------------------------------------

    fn translate_event(&self, event_block: &str) -> Option<String> {
        let mut event_type = None;
        let mut data_str = None;

        for line in event_block.lines() {
            if let Some(et) = line.strip_prefix("event: ") {
                event_type = Some(et.trim());
            } else if let Some(d) = line.strip_prefix("data: ") {
                data_str = Some(d);
            }
        }

        let data_str = data_str?;

        match event_type? {
            // message_start → emit role-introduction chunk
            "message_start" => {
                let chunk = json!({
                    "id": self.msg_id,
                    "object": "chat.completion.chunk",
                    "created": self.created,
                    "model": self.model,
                    "choices": [{
                        "index": 0,
                        "delta": { "role": "assistant", "content": "" },
                        "finish_reason": serde_json::Value::Null
                    }]
                });
                Some(format!("data: {}\n\n", chunk))
            }

            // content_block_delta → emit content delta chunk
            "content_block_delta" => {
                let parsed: serde_json::Value = serde_json::from_str(data_str).ok()?;
                // Text delta → content field
                if let Some(text) = parsed
                    .get("delta")
                    .and_then(|d| d.get("text"))
                    .and_then(|t| t.as_str())
                {
                    let chunk = json!({
                        "id": self.msg_id,
                        "object": "chat.completion.chunk",
                        "created": self.created,
                        "model": self.model,
                        "choices": [{
                            "index": 0,
                            "delta": { "content": text },
                            "finish_reason": serde_json::Value::Null
                        }]
                    });
                    return Some(format!("data: {}\n\n", chunk));
                }
                // Thinking delta → skip (OpenAI has no equivalent)
                None
            }

            // message_delta → emit finish_reason chunk
            "message_delta" => {
                let parsed: serde_json::Value = serde_json::from_str(data_str).ok()?;
                if let Some(stop_reason) = parsed
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|s| s.as_str())
                {
                    let finish_reason = map_stop_reason_to_openai(stop_reason);
                    let chunk = json!({
                        "id": self.msg_id,
                        "object": "chat.completion.chunk",
                        "created": self.created,
                        "model": self.model,
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": finish_reason
                        }]
                    });
                    return Some(format!("data: {}\n\n", chunk));
                }
                None
            }

            // message_stop → [DONE] sentinel
            "message_stop" => Some("data: [DONE]\n\n".to_string()),

            // ping, content_block_start, content_block_stop → skip
            _ => None,
        }
    }
}

// ============================================================================
// SseAnthropicToGemini
// ============================================================================

/// Translates Anthropic streaming SSE events to Gemini `GenerateContentResponse`
/// newline-delimited JSON format (wrapped in `data: …\n\n` SSE envelopes).
///
/// # Anthropic events handled
///
/// | Anthropic event        | Action                                                          |
/// |------------------------|-----------------------------------------------------------------|
/// | `message_start`        | Extract `usage.input_tokens`; no output yet                     |
/// | `content_block_delta`  | Emit Gemini content chunk with `parts[].text`                   |
/// | `message_delta`        | Emit final Gemini chunk with `finishReason` and `usageMetadata` |
/// | `message_stop`         | Mark stream as done; no output (Gemini uses the final chunk)    |
/// | everything else        | Ignored                                                         |
///
/// # Gemini chunk format
///
/// Content chunk:
/// ```json
/// data: {"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"index":0}]}
/// ```
///
/// Final chunk (with stop reason + usage):
/// ```json
/// data: {"candidates":[{"content":{"parts":[],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5,"totalTokenCount":15}}
/// ```
pub struct SseAnthropicToGemini {
    #[allow(dead_code)]
    model: String,
    /// Accumulated text fragments (kept for potential future use / introspection).
    buffer: Vec<String>,
    /// Input token count extracted from `message_start`.
    input_tokens: u64,
    /// Output token count extracted from `message_delta`.
    output_tokens: u64,
    /// Set to true once `message_stop` is seen.
    done: bool,
}

impl SseAnthropicToGemini {
    /// Create a new translator.
    ///
    /// - `model`: Gemini model name to embed in log / debug context (not
    ///   included in Gemini streaming chunks directly, but stored for future use).
    pub fn new(model: String) -> Self {
        Self {
            model,
            buffer: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            done: false,
        }
    }

    /// Returns true if `message_stop` has been observed.
    #[allow(dead_code)]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The model name this translator was created with.
    #[allow(dead_code)]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Process one raw Anthropic SSE line (or multi-line event block).
    ///
    /// Accepts either a single `data: …` line **or** a complete `event: …\ndata: …`
    /// block.  Returns `Some(gemini_sse_string)` when a Gemini chunk should be
    /// forwarded, `None` otherwise.
    ///
    /// The returned string is already formatted as an SSE event:
    /// `data: {json}\n\n`
    pub fn process_line(&mut self, line: &str) -> Option<String> {
        // ── parse the event block ──────────────────────────────────────────
        let mut event_type: Option<&str> = None;
        let mut data_str: Option<&str> = None;

        for l in line.lines() {
            if let Some(et) = l.strip_prefix("event: ") {
                event_type = Some(et.trim());
            } else if let Some(d) = l.strip_prefix("data: ") {
                data_str = Some(d);
            }
        }

        let data_str = data_str?;

        match event_type? {
            // ── message_start ─────────────────────────────────────────────
            // Extract input token count from the embedded message.usage field.
            "message_start" => {
                let parsed: serde_json::Value = serde_json::from_str(data_str).ok()?;
                if let Some(tokens) = parsed
                    .get("message")
                    .and_then(|m| m.get("usage"))
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|v| v.as_u64())
                {
                    self.input_tokens = tokens;
                }
                None
            }

            // ── content_block_delta ───────────────────────────────────────
            // Emit a Gemini content chunk for every text delta.
            "content_block_delta" => {
                let parsed: serde_json::Value = serde_json::from_str(data_str).ok()?;
                let text = parsed
                    .get("delta")
                    .and_then(|d| d.get("text"))
                    .and_then(|t| t.as_str())?;

                if text.is_empty() {
                    return None;
                }

                self.buffer.push(text.to_string());

                let chunk = json!({
                    "candidates": [{
                        "content": {
                            "parts": [{ "text": text }],
                            "role": "model"
                        },
                        "index": 0
                    }]
                });
                Some(format!("data: {}\n\n", chunk))
            }

            // ── message_delta ─────────────────────────────────────────────
            // Emit the final Gemini chunk: finishReason + usageMetadata.
            "message_delta" => {
                let parsed: serde_json::Value = serde_json::from_str(data_str).ok()?;

                // Extract stop_reason → map to Gemini finishReason
                let stop_reason = parsed
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("end_turn");
                let finish_reason = map_stop_reason_to_gemini(stop_reason);

                // Extract output_tokens if available
                if let Some(tokens) = parsed
                    .get("usage")
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|v| v.as_u64())
                {
                    self.output_tokens = tokens;
                }

                let total = self.input_tokens + self.output_tokens;

                let chunk = json!({
                    "candidates": [{
                        "content": {
                            "parts": [],
                            "role": "model"
                        },
                        "finishReason": finish_reason,
                        "index": 0
                    }],
                    "usageMetadata": {
                        "promptTokenCount": self.input_tokens,
                        "candidatesTokenCount": self.output_tokens,
                        "totalTokenCount": total
                    }
                });
                Some(format!("data: {}\n\n", chunk))
            }

            // ── message_stop ──────────────────────────────────────────────
            // Mark done; no extra output (the final chunk was in message_delta).
            "message_stop" => {
                self.done = true;
                None
            }

            // ── everything else ───────────────────────────────────────────
            // ping, content_block_start, content_block_stop → ignore
            _ => None,
        }
    }

    /// Process a raw byte chunk that may contain multiple SSE event blocks.
    ///
    /// Splits on `\n\n`, delegates each block to [`process_line`], and
    /// concatenates all produced Gemini SSE strings.  The internal remainder
    /// (incomplete event) is accumulated in `remainder` and should be passed
    /// back on the next call if needed.
    ///
    /// For simple callers that feed complete lines, prefer [`process_line`]
    /// directly.
    pub fn process_chunk(&mut self, chunk: &[u8], remainder: &mut String) -> Bytes {
        remainder.push_str(&String::from_utf8_lossy(chunk));
        let mut output = String::new();

        while let Some(pos) = remainder.find("\n\n") {
            let event_block = remainder[..pos].to_string();
            *remainder = remainder[pos + 2..].to_string();

            if let Some(translated) = self.process_line(&event_block) {
                output.push_str(&translated);
            }
        }

        Bytes::from(output)
    }
}

// ============================================================================
// Shared helpers
// ============================================================================

/// Map an Anthropic `stop_reason` to an OpenAI `finish_reason`.
fn map_stop_reason_to_openai(stop_reason: &str) -> &str {
    match stop_reason {
        "end_turn" | "stop_sequence" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        _ => "stop",
    }
}

/// Map an Anthropic `stop_reason` to a Gemini `finishReason`.
fn map_stop_reason_to_gemini(stop_reason: &str) -> &str {
    match stop_reason {
        "end_turn" | "stop_sequence" => "STOP",
        "max_tokens" => "MAX_TOKENS",
        "tool_use" => "STOP", // Gemini uses STOP for tool-call completion in streaming
        _ => "STOP",
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn anthropic_event(event: &str, data: &serde_json::Value) -> String {
        format!("event: {}\ndata: {}\n\n", event, data)
    }

    // ── SseAnthropicToOpenai tests ────────────────────────────────────────────

    #[test]
    fn openai_message_start_emits_role_chunk() {
        let mut t = SseAnthropicToOpenai::new("msg_01".into(), "claude-sonnet-4-6".into());
        let event = anthropic_event(
            "message_start",
            &json!({
                "type": "message_start",
                "message": {
                    "id": "msg_01",
                    "type": "message",
                    "role": "assistant",
                    "model": "claude-sonnet-4-6",
                    "content": [],
                    "usage": { "input_tokens": 10, "output_tokens": 0 }
                }
            }),
        );
        let out = t.process_chunk(event.as_bytes());
        let s = std::str::from_utf8(&out).unwrap();

        assert!(s.starts_with("data: "), "should start with 'data: '");
        let json: serde_json::Value = serde_json::from_str(s.trim_start_matches("data: ").trim_end_matches("\n\n")).unwrap();
        assert_eq!(json["id"], "msg_01");
        assert_eq!(json["object"], "chat.completion.chunk");
        assert_eq!(json["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(json["choices"][0]["delta"]["content"], "");
    }

    #[test]
    fn openai_content_block_delta_emits_text() {
        let mut t = SseAnthropicToOpenai::new("msg_02".into(), "claude-sonnet-4-6".into());
        let event = anthropic_event(
            "content_block_delta",
            &json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": "Hello world" }
            }),
        );
        let out = t.process_chunk(event.as_bytes());
        let s = std::str::from_utf8(&out).unwrap();

        assert!(s.contains("Hello world"), "output should contain the text delta");
        let json_part = s.trim_start_matches("data: ").trim_end_matches("\n\n");
        let json: serde_json::Value = serde_json::from_str(json_part).unwrap();
        assert_eq!(json["choices"][0]["delta"]["content"], "Hello world");
        assert!(json["choices"][0]["finish_reason"].is_null());
    }

    #[test]
    fn openai_thinking_delta_is_skipped() {
        let mut t = SseAnthropicToOpenai::new("msg_03".into(), "claude-sonnet-4-6".into());
        let event = anthropic_event(
            "content_block_delta",
            &json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "thinking_delta", "thinking": "Let me think..." }
            }),
        );
        let out = t.process_chunk(event.as_bytes());
        assert!(out.is_empty(), "thinking deltas should produce no OpenAI output");
    }

    #[test]
    fn openai_message_delta_emits_finish_reason() {
        let mut t = SseAnthropicToOpenai::new("msg_04".into(), "claude-sonnet-4-6".into());
        let event = anthropic_event(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn", "stop_sequence": null },
                "usage": { "output_tokens": 20 }
            }),
        );
        let out = t.process_chunk(event.as_bytes());
        let s = std::str::from_utf8(&out).unwrap();

        let json_part = s.trim_start_matches("data: ").trim_end_matches("\n\n");
        let json: serde_json::Value = serde_json::from_str(json_part).unwrap();
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn openai_message_delta_max_tokens_maps_to_length() {
        let mut t = SseAnthropicToOpenai::new("msg_05".into(), "claude-sonnet-4-6".into());
        let event = anthropic_event(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": { "stop_reason": "max_tokens" },
                "usage": { "output_tokens": 4096 }
            }),
        );
        let out = t.process_chunk(event.as_bytes());
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("\"length\""), "max_tokens should map to 'length'");
    }

    #[test]
    fn openai_message_stop_emits_done() {
        let mut t = SseAnthropicToOpenai::new("msg_06".into(), "claude-sonnet-4-6".into());
        let event = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let out = t.process_chunk(event.as_bytes());
        let s = std::str::from_utf8(&out).unwrap();
        assert_eq!(s, "data: [DONE]\n\n");
    }

    #[test]
    fn openai_ping_is_skipped() {
        let mut t = SseAnthropicToOpenai::new("msg_07".into(), "claude-sonnet-4-6".into());
        let event = "event: ping\ndata: {\"type\":\"ping\"}\n\n";
        let out = t.process_chunk(event.as_bytes());
        assert!(out.is_empty(), "ping events should produce no output");
    }

    #[test]
    fn openai_buffers_incomplete_events() {
        let mut t = SseAnthropicToOpenai::new("msg_08".into(), "claude-sonnet-4-6".into());
        // Send only half of the event (no \n\n terminator yet)
        let partial = b"event: message_stop\ndata: {\"type\":\"message_stop\"}";
        let out1 = t.process_chunk(partial);
        assert!(out1.is_empty(), "incomplete event should produce no output yet");

        // Now send the terminator
        let out2 = t.process_chunk(b"\n\n");
        let s = std::str::from_utf8(&out2).unwrap();
        assert_eq!(s, "data: [DONE]\n\n");
    }

    #[test]
    fn openai_full_stream_sequence() {
        let mut t = SseAnthropicToOpenai::new("msg_full".into(), "claude-3-opus".into());
        let mut full_output = String::new();

        let events = [
            anthropic_event("message_start", &json!({
                "type": "message_start",
                "message": { "id": "msg_full", "usage": { "input_tokens": 5, "output_tokens": 0 } }
            })),
            anthropic_event("content_block_delta", &json!({
                "type": "content_block_delta", "index": 0,
                "delta": { "type": "text_delta", "text": "Hello" }
            })),
            anthropic_event("content_block_delta", &json!({
                "type": "content_block_delta", "index": 0,
                "delta": { "type": "text_delta", "text": " world" }
            })),
            anthropic_event("message_delta", &json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn" },
                "usage": { "output_tokens": 2 }
            })),
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string(),
        ];

        for event in &events {
            let out = t.process_chunk(event.as_bytes());
            full_output.push_str(std::str::from_utf8(&out).unwrap());
        }

        // Should have: role chunk, 2 content chunks, finish chunk, [DONE]
        assert!(full_output.contains("\"assistant\""), "should have role introduction");
        assert!(full_output.contains("Hello"), "should have first text chunk");
        assert!(full_output.contains("world"), "should have second text chunk");
        assert!(full_output.contains("\"stop\""), "should have finish reason");
        assert!(full_output.ends_with("data: [DONE]\n\n"), "should end with [DONE]");
    }

    // ── SseAnthropicToGemini tests ─────────────────────────────────────────────

    #[test]
    fn gemini_message_start_extracts_input_tokens() {
        let mut t = SseAnthropicToGemini::new("gemini-2.0-flash".into());
        let event = anthropic_event(
            "message_start",
            &json!({
                "type": "message_start",
                "message": {
                    "id": "msg_g1",
                    "usage": { "input_tokens": 42, "output_tokens": 0 }
                }
            }),
        );
        // Trim trailing \n\n since process_line expects a block without it,
        // but it tolerates the full form too.
        let result = t.process_line(event.trim_end_matches('\n'));
        assert!(result.is_none(), "message_start should not produce output");
        assert_eq!(t.input_tokens, 42);
    }

    #[test]
    fn gemini_content_block_delta_emits_chunk() {
        let mut t = SseAnthropicToGemini::new("gemini-2.0-flash".into());
        let event = anthropic_event(
            "content_block_delta",
            &json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": "Bonjour" }
            }),
        );
        let result = t.process_line(event.trim_end_matches('\n')).unwrap();
        assert!(result.starts_with("data: "), "should be an SSE data line");
        assert!(result.ends_with("\n\n"), "should end with \\n\\n");

        let json_part = result.trim_start_matches("data: ").trim_end_matches("\n\n");
        let json: serde_json::Value = serde_json::from_str(json_part).unwrap();

        assert_eq!(json["candidates"][0]["content"]["parts"][0]["text"], "Bonjour");
        assert_eq!(json["candidates"][0]["content"]["role"], "model");
        assert_eq!(json["candidates"][0]["index"], 0);
        assert!(json.get("usageMetadata").is_none(), "content chunks should not have usage");
    }

    #[test]
    fn gemini_empty_text_delta_is_skipped() {
        let mut t = SseAnthropicToGemini::new("gemini-2.0-flash".into());
        let event = anthropic_event(
            "content_block_delta",
            &json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": "" }
            }),
        );
        let result = t.process_line(event.trim_end_matches('\n'));
        assert!(result.is_none(), "empty text delta should produce no output");
    }

    #[test]
    fn gemini_message_delta_emits_final_chunk() {
        let mut t = SseAnthropicToGemini::new("gemini-2.0-flash".into());
        t.input_tokens = 10;

        let event = anthropic_event(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn", "stop_sequence": null },
                "usage": { "output_tokens": 5 }
            }),
        );
        let result = t.process_line(event.trim_end_matches('\n')).unwrap();
        let json_part = result.trim_start_matches("data: ").trim_end_matches("\n\n");
        let json: serde_json::Value = serde_json::from_str(json_part).unwrap();

        assert_eq!(json["candidates"][0]["finishReason"], "STOP");
        assert_eq!(json["candidates"][0]["content"]["parts"], json!([]));
        assert_eq!(json["usageMetadata"]["promptTokenCount"], 10);
        assert_eq!(json["usageMetadata"]["candidatesTokenCount"], 5);
        assert_eq!(json["usageMetadata"]["totalTokenCount"], 15);
    }

    #[test]
    fn gemini_message_delta_max_tokens_maps_correctly() {
        let mut t = SseAnthropicToGemini::new("gemini-2.0-flash".into());
        let event = anthropic_event(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": { "stop_reason": "max_tokens" },
                "usage": { "output_tokens": 4096 }
            }),
        );
        let result = t.process_line(event.trim_end_matches('\n')).unwrap();
        assert!(result.contains("\"MAX_TOKENS\""), "max_tokens should map to MAX_TOKENS");
    }

    #[test]
    fn gemini_message_stop_marks_done() {
        let mut t = SseAnthropicToGemini::new("gemini-2.0-flash".into());
        assert!(!t.is_done());
        let event = "event: message_stop\ndata: {\"type\":\"message_stop\"}";
        let result = t.process_line(event);
        assert!(result.is_none(), "message_stop should produce no output");
        assert!(t.is_done(), "translator should be marked done after message_stop");
    }

    #[test]
    fn gemini_ping_is_skipped() {
        let mut t = SseAnthropicToGemini::new("gemini-2.0-flash".into());
        let result = t.process_line("event: ping\ndata: {\"type\":\"ping\"}");
        assert!(result.is_none(), "ping should be ignored");
    }

    #[test]
    fn gemini_process_chunk_handles_multiple_events() {
        let mut t = SseAnthropicToGemini::new("gemini-2.0-flash".into());
        let mut remainder = String::new();

        let combined = format!(
            "{}{}",
            anthropic_event(
                "content_block_delta",
                &json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": { "type": "text_delta", "text": "Hi" }
                })
            ),
            anthropic_event(
                "content_block_delta",
                &json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": { "type": "text_delta", "text": " there" }
                })
            ),
        );

        let out = t.process_chunk(combined.as_bytes(), &mut remainder);
        let s = std::str::from_utf8(&out).unwrap();

        assert!(s.contains("Hi"), "first chunk text should be present");
        assert!(s.contains("there"), "second chunk text should be present");
        // Two separate data: lines
        assert_eq!(s.matches("data: ").count(), 2);
    }

    #[test]
    fn gemini_process_chunk_buffers_incomplete_events() {
        let mut t = SseAnthropicToGemini::new("gemini-2.0-flash".into());
        let mut remainder = String::new();

        // Send partial event (no \n\n)
        let partial = b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"test\"}}";
        let out1 = t.process_chunk(partial, &mut remainder);
        assert!(out1.is_empty(), "incomplete block should produce no output");
        assert!(!remainder.is_empty(), "remainder should hold the partial event");

        // Complete the event
        let out2 = t.process_chunk(b"\n\n", &mut remainder);
        let s = std::str::from_utf8(&out2).unwrap();
        assert!(s.contains("test"), "completed event should produce output");
    }

    #[test]
    fn gemini_full_stream_sequence() {
        let mut t = SseAnthropicToGemini::new("gemini-2.0-flash".into());
        let mut remainder = String::new();
        let mut full_output = String::new();

        let events = [
            anthropic_event("message_start", &json!({
                "type": "message_start",
                "message": { "id": "msg_g_full", "usage": { "input_tokens": 15, "output_tokens": 0 } }
            })),
            anthropic_event("content_block_delta", &json!({
                "type": "content_block_delta", "index": 0,
                "delta": { "type": "text_delta", "text": "Hello " }
            })),
            anthropic_event("content_block_delta", &json!({
                "type": "content_block_delta", "index": 0,
                "delta": { "type": "text_delta", "text": "Gemini" }
            })),
            anthropic_event("message_delta", &json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn" },
                "usage": { "output_tokens": 3 }
            })),
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string(),
        ];

        for event in &events {
            let out = t.process_chunk(event.as_bytes(), &mut remainder);
            full_output.push_str(std::str::from_utf8(&out).unwrap());
        }

        assert!(t.is_done(), "translator should be done after message_stop");
        assert_eq!(t.input_tokens, 15);
        assert_eq!(t.output_tokens, 3);

        // 2 content chunks + 1 final chunk (message_start and message_stop produce nothing)
        assert_eq!(full_output.matches("data: ").count(), 3, "expected 3 SSE chunks");
        assert!(full_output.contains("Hello "));
        assert!(full_output.contains("Gemini"));
        assert!(full_output.contains("STOP"));
        assert!(full_output.contains("promptTokenCount"));
    }

    // ── Helper function tests ─────────────────────────────────────────────────

    #[test]
    fn stop_reason_openai_mappings() {
        assert_eq!(map_stop_reason_to_openai("end_turn"), "stop");
        assert_eq!(map_stop_reason_to_openai("stop_sequence"), "stop");
        assert_eq!(map_stop_reason_to_openai("max_tokens"), "length");
        assert_eq!(map_stop_reason_to_openai("tool_use"), "tool_calls");
        assert_eq!(map_stop_reason_to_openai("unknown_reason"), "stop");
    }

    #[test]
    fn stop_reason_gemini_mappings() {
        assert_eq!(map_stop_reason_to_gemini("end_turn"), "STOP");
        assert_eq!(map_stop_reason_to_gemini("stop_sequence"), "STOP");
        assert_eq!(map_stop_reason_to_gemini("max_tokens"), "MAX_TOKENS");
        assert_eq!(map_stop_reason_to_gemini("tool_use"), "STOP");
        assert_eq!(map_stop_reason_to_gemini("unknown"), "STOP");
    }
}
