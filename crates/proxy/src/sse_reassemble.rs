//! Reassembles an Anthropic SSE stream into a single JSON Message.
//!
//! When the proxy forces stream=true for CC impersonation but the client
//! expected a non-streaming response (stream=false or absent), this module
//! buffers all SSE events and reconstructs the complete Message object.

use serde_json::Value;

/// Accumulator that buffers SSE events and rebuilds a complete Anthropic Message.
pub struct SseReassembler {
    /// Message skeleton from message_start
    message: Option<Value>,
    /// Completed content blocks
    content_blocks: Vec<Value>,
    /// Current content block being built
    current_block: Option<Value>,
    /// Text accumulator for text_delta / thinking_delta
    current_text: String,
    /// JSON accumulator for input_json_delta (tool_use)
    current_tool_input: String,
    /// Stop reason from message_delta
    stop_reason: Option<String>,
    /// Final usage from message_delta
    final_usage: Option<Value>,
    /// Buffer for incomplete SSE lines across chunks
    buffer: String,
    /// Whether we've seen message_stop
    done: bool,
}

impl SseReassembler {
    pub fn new() -> Self {
        Self {
            message: None,
            content_blocks: Vec::new(),
            current_block: None,
            current_text: String::new(),
            current_tool_input: String::new(),
            stop_reason: None,
            final_usage: None,
            buffer: String::new(),
            done: false,
        }
    }

    /// Feed a chunk of SSE data. Call repeatedly as chunks arrive from upstream.
    pub fn feed(&mut self, chunk: &[u8]) {
        let text = match std::str::from_utf8(chunk) {
            Ok(t) => t,
            Err(_) => return,
        };
        self.buffer.push_str(text);

        // Process complete events (separated by double newline)
        while let Some(pos) = self.buffer.find("\n\n") {
            let event_block = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + 2..].to_string();
            self.process_event_block(&event_block);
        }
    }

    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.done
    }

    fn process_event_block(&mut self, block: &str) {
        let mut event_type = String::new();
        let mut data = String::new();

        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("event: ") {
                event_type = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("data: ") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest);
            } else if let Some(rest) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest);
            }
        }

        if data.is_empty() {
            return;
        }

        let json: Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => return,
        };

        match event_type.as_str() {
            "message_start" => {
                if let Some(msg) = json.get("message") {
                    self.message = Some(msg.clone());
                }
            }
            "content_block_start" => {
                if let Some(cb) = json.get("content_block") {
                    self.current_block = Some(cb.clone());
                    self.current_text.clear();
                    self.current_tool_input.clear();
                }
            }
            "content_block_delta" => {
                if let Some(delta) = json.get("delta") {
                    match delta.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                        "text_delta" => {
                            if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                self.current_text.push_str(text);
                            }
                        }
                        "input_json_delta" => {
                            if let Some(s) = delta.get("partial_json").and_then(|t| t.as_str()) {
                                self.current_tool_input.push_str(s);
                            }
                        }
                        "thinking_delta" => {
                            if let Some(t) = delta.get("thinking").and_then(|t| t.as_str()) {
                                self.current_text.push_str(t);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                if let Some(mut block) = self.current_block.take() {
                    if let Some(obj) = block.as_object_mut() {
                        match obj.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                            "text" => {
                                obj.insert(
                                    "text".to_string(),
                                    Value::String(self.current_text.clone()),
                                );
                            }
                            "thinking" => {
                                obj.insert(
                                    "thinking".to_string(),
                                    Value::String(self.current_text.clone()),
                                );
                            }
                            "tool_use" => {
                                let input: Value =
                                    serde_json::from_str(&self.current_tool_input)
                                        .unwrap_or(Value::Object(serde_json::Map::new()));
                                obj.insert("input".to_string(), input);
                            }
                            _ => {}
                        }
                    }
                    self.content_blocks.push(block);
                    self.current_text.clear();
                    self.current_tool_input.clear();
                }
            }
            "message_delta" => {
                if let Some(delta) = json.get("delta") {
                    if let Some(reason) = delta.get("stop_reason").and_then(|r| r.as_str()) {
                        self.stop_reason = Some(reason.to_string());
                    }
                }
                if let Some(usage) = json.get("usage") {
                    self.final_usage = Some(usage.clone());
                }
            }
            "message_stop" => {
                self.done = true;
            }
            _ => {}
        }
    }

    /// Reassemble the buffered SSE events into a complete Anthropic Message JSON.
    /// Returns None if message_start was never received.
    pub fn into_message(self) -> Option<Value> {
        let mut msg = self.message?;

        if let Some(obj) = msg.as_object_mut() {
            obj.insert("content".to_string(), Value::Array(self.content_blocks));

            if let Some(reason) = self.stop_reason {
                obj.insert("stop_reason".to_string(), Value::String(reason));
            }

            // Merge final usage (output_tokens) into initial usage (input_tokens)
            if let Some(final_usage) = self.final_usage {
                if let Some(existing) = obj.get_mut("usage") {
                    if let (Some(existing_obj), Some(final_obj)) =
                        (existing.as_object_mut(), final_usage.as_object())
                    {
                        for (k, v) in final_obj {
                            existing_obj.insert(k.clone(), v.clone());
                        }
                    }
                } else {
                    obj.insert("usage".to_string(), final_usage);
                }
            }
        }

        Some(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reassemble_simple_text_message() {
        let mut r = SseReassembler::new();

        r.feed(b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-6\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":100,\"output_tokens\":0}}}\n\n");
        r.feed(b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n");
        r.feed(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n");
        r.feed(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\n");
        r.feed(b"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n");
        r.feed(b"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":10}}\n\n");
        r.feed(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");

        assert!(r.is_done());
        let msg = r.into_message().expect("should produce message");

        assert_eq!(msg["id"], "msg_01");
        assert_eq!(msg["type"], "message");
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"][0]["type"], "text");
        assert_eq!(msg["content"][0]["text"], "Hello world");
        assert_eq!(msg["stop_reason"], "end_turn");
        assert_eq!(msg["usage"]["input_tokens"], 100);
        assert_eq!(msg["usage"]["output_tokens"], 10);
    }

    #[test]
    fn reassemble_tool_use() {
        let mut r = SseReassembler::new();

        r.feed(b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_02\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-6\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":50,\"output_tokens\":0}}}\n\n");
        r.feed(b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"get_weather\",\"input\":{}}}\n\n");
        r.feed(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\": \\\"\"}}\n\n");
        r.feed(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"Paris\\\"}\"}}\n\n");
        r.feed(b"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n");
        r.feed(b"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":20}}\n\n");
        r.feed(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");

        let msg = r.into_message().expect("should produce message");
        assert_eq!(msg["content"][0]["type"], "tool_use");
        assert_eq!(msg["content"][0]["name"], "get_weather");
        assert_eq!(msg["content"][0]["input"]["city"], "Paris");
        assert_eq!(msg["stop_reason"], "tool_use");
    }

    #[test]
    fn handles_chunked_delivery() {
        let mut r = SseReassembler::new();

        // Split an event across two chunks
        r.feed(b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_03\",\"type\":\"message\",\"role\":\"assistant\"");
        r.feed(b",\"model\":\"claude-sonnet-4-6\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");

        assert!(r.is_done());
        let msg = r.into_message().expect("should produce message");
        assert_eq!(msg["id"], "msg_03");
    }

    #[test]
    fn handles_multiline_data_event() {
        let mut r = SseReassembler::new();
        // Split JSON across multiple data lines
        let block = b"event: message_start\ndata: {\"type\":\ndata: \"message_start\",\ndata: \"message\": {\"id\": \"msg_multi\", \"type\": \"message\", \"role\": \"assistant\", \"content\": [], \"model\": \"test\", \"stop_reason\": null, \"usage\": {\"input_tokens\": 0, \"output_tokens\": 0}}}\n\n";
        r.feed(block);

        // Finish it
        r.feed(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");

        assert!(r.is_done());
        let msg = r.into_message().expect("should parse multiline data");
        assert_eq!(msg["id"], "msg_multi");
    }

    #[test]
    fn no_message_start_returns_none() {
        let r = SseReassembler::new();
        assert!(r.into_message().is_none());
    }
}
