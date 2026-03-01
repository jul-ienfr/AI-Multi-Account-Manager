//! Réécriture complète du body de requête pour l'impersonation parfaite.
//!
//! Port de `rewrite_body_full()` de cc_profile.py.
//!
//! Applique :
//! 1. metadata → sample capturé du profil
//! 2. system prompt → format array+cache_control si le vrai client le fait
//! 3. tools → ajouter cache_control si le vrai client le fait
//! 4. stream → forcer si le vrai client stream toujours
//! 5. Supprimer les champs que le vrai client n'envoie JAMAIS (whitelist)

use serde_json::{json, Map, Value};

use crate::cc_profile::ProfileData;

/// Réécrit le body JSON d'une requête Anthropic conformément au profil capturé.
///
/// Retourne le body réécrit (même si aucune modification n'est nécessaire).
pub fn rewrite_body(body: &Value, profile: &ProfileData) -> Value {
    let Some(obj) = body.as_object() else {
        return body.clone();
    };

    let mut result = obj.clone();

    // 1. Whitelist des champs : supprimer les champs que le vrai client n'envoie jamais
    if !profile.body_field_whitelist.is_empty() {
        result.retain(|k, _| {
            // Garder les champs essentiels même si pas dans la whitelist
            let essential = matches!(
                k.as_str(),
                "model" | "messages" | "max_tokens" | "stream" | "system" | "tools"
                    | "tool_choice" | "metadata"
            );
            essential || profile.body_field_whitelist.contains(k)
        });
    }

    // 2. metadata → utiliser un sample du profil (user_id généré)
    if profile.request_count > 0 {
        result.insert(
            "metadata".to_string(),
            json!({ "user_id": uuid::Uuid::new_v4().to_string() }),
        );
    }

    // 3. system prompt → convertir en format array avec cache_control si le profil l'indique
    if profile.system_format == "array" {
        if let Some(system) = result.get("system") {
            if system.is_string() {
                let sys_text = system.as_str().unwrap_or("").to_string();
                result.insert(
                    "system".to_string(),
                    json!([{
                        "type": "text",
                        "text": sys_text,
                        "cache_control": { "type": "ephemeral" }
                    }]),
                );
            }
        }
    }

    // 4. tools → ajouter cache_control sur le dernier tool si le profil l'indique
    if profile.has_tools_cache_control {
        if let Some(tools) = result.get_mut("tools") {
            if let Some(arr) = tools.as_array_mut() {
                if let Some(last) = arr.last_mut() {
                    if let Some(obj) = last.as_object_mut() {
                        obj.entry("cache_control")
                            .or_insert_with(|| json!({ "type": "ephemeral" }));
                    }
                }
            }
        }
    }

    // 5. stream → forcer si le vrai client stream toujours
    if profile.always_streams {
        result.insert("stream".to_string(), json!(true));
    }

    Value::Object(result)
}

/// Flatten les content blocks Anthropic (array de {type, text}) en string plain text.
#[allow(dead_code)]
fn flatten_content_blocks(content: &Value) -> Value {
    match content {
        Value::String(_) => content.clone(),
        Value::Array(blocks) => {
            let text: String = blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                        b.get("text").and_then(|t| t.as_str()).map(str::to_string)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");
            json!(text)
        }
        _ => content.clone(),
    }
}

/// Traduit un body OpenAI Chat Completions en body Anthropic Messages.
///
/// Utilisé quand un client OpenAI-format (Cursor, Windsurf, Gemini clients)
/// envoie au proxy, et que le backend cible est toujours Anthropic.
pub fn openai_request_to_anthropic(body: Value) -> Value {
    let Some(obj) = body.as_object() else {
        return body;
    };

    let mut result = Map::new();

    // Model passthrough (sera résolu séparément)
    if let Some(model) = obj.get("model") {
        result.insert("model".to_string(), model.clone());
    }

    // max_tokens
    result.insert(
        "max_tokens".to_string(),
        obj.get("max_tokens").cloned().unwrap_or(json!(8096)),
    );

    // stream
    if let Some(stream) = obj.get("stream") {
        result.insert("stream".to_string(), stream.clone());
    }

    // temperature / top_p passthrough
    if let Some(v) = obj.get("temperature") {
        result.insert("temperature".to_string(), v.clone());
    }
    if let Some(v) = obj.get("top_p") {
        result.insert("top_p".to_string(), v.clone());
    }

    // messages : séparer system des autres
    if let Some(messages) = obj.get("messages").and_then(|m| m.as_array()) {
        let mut anthro_messages: Vec<Value> = Vec::new();
        let mut system_parts: Vec<Value> = Vec::new();

        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let content = msg.get("content").cloned().unwrap_or(json!(""));

            if role == "system" {
                let text = content.as_str().unwrap_or("").to_string();
                system_parts.push(json!({ "type": "text", "text": text }));
            } else {
                let anthro_role = if role == "assistant" { "assistant" } else { "user" };
                anthro_messages.push(json!({
                    "role": anthro_role,
                    "content": content
                }));
            }
        }

        if !system_parts.is_empty() {
            result.insert("system".to_string(), Value::Array(system_parts));
        }
        result.insert("messages".to_string(), Value::Array(anthro_messages));
    }

    Value::Object(result)
}

/// Traduit un body Gemini generateContent en body Anthropic Messages.
///
/// Mapping des champs :
/// - `contents[]` (role "user"/"model") → `messages[]` (role "user"/"assistant")
/// - `systemInstruction.parts[0].text` → `system`
/// - `generationConfig.maxOutputTokens` → `max_tokens`
/// - `generationConfig.temperature` → `temperature`
/// - `generationConfig.topP` → `top_p`
/// - Le model est géré séparément par model_mapping.
pub fn gemini_request_to_anthropic(body: Value) -> Value {
    let Some(obj) = body.as_object() else {
        return body;
    };

    let mut result = Map::new();

    // Model passthrough (résolution séparée via model_mapping)
    if let Some(model) = obj.get("model") {
        result.insert("model".to_string(), model.clone());
    }

    // systemInstruction → system
    if let Some(sys_instr) = obj.get("systemInstruction") {
        let text = sys_instr
            .get("parts")
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())
            .and_then(|part| part.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        if !text.is_empty() {
            result.insert("system".to_string(), json!(text));
        }
    }

    // generationConfig → max_tokens, temperature, top_p
    let gen_cfg = obj.get("generationConfig");
    let max_tokens = gen_cfg
        .and_then(|g| g.get("maxOutputTokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(8096);
    result.insert("max_tokens".to_string(), json!(max_tokens));

    if let Some(temp) = gen_cfg.and_then(|g| g.get("temperature")) {
        result.insert("temperature".to_string(), temp.clone());
    }
    if let Some(top_p) = gen_cfg.and_then(|g| g.get("topP")) {
        result.insert("top_p".to_string(), top_p.clone());
    }

    // contents[] → messages[]
    if let Some(contents) = obj.get("contents").and_then(|c| c.as_array()) {
        let mut messages: Vec<Value> = Vec::new();

        for item in contents {
            let gemini_role = item
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("user");
            let anthro_role = if gemini_role == "model" { "assistant" } else { "user" };

            // parts[].text → join en string
            let text = item
                .get("parts")
                .and_then(|p| p.as_array())
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();

            messages.push(json!({
                "role": anthro_role,
                "content": text
            }));
        }

        result.insert("messages".to_string(), Value::Array(messages));
    }

    Value::Object(result)
}

/// Traduit une réponse Anthropic Message en format OpenAI chat.completion.
///
/// Utilisé quand un client OpenAI-format (Cursor, Windsurf) connecte au proxy
/// mais que le provider actif est Anthropic.
pub fn anthropic_response_to_openai(body: &Value) -> Value {
    let content_text = body
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");

    let stop_reason = body
        .get("stop_reason")
        .and_then(|s| s.as_str())
        .unwrap_or("end_turn");
    let finish_reason = match stop_reason {
        "end_turn" | "stop_sequence" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        _ => "stop",
    };

    let usage = body.get("usage");
    let prompt_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let completion_tokens = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    json!({
        "id": body.get("id").and_then(|v| v.as_str()).unwrap_or("chatcmpl-unknown"),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": body.get("model").and_then(|v| v.as_str()).unwrap_or(""),
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content_text
            },
            "finish_reason": finish_reason
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        }
    })
}

/// Traduit une réponse Anthropic Message en format Gemini generateContent response.
///
/// Mapping des champs :
/// - `content[]` texte → `candidates[0].content.parts[].text`
/// - `stop_reason` → `candidates[0].finishReason` ("end_turn"→"STOP", "max_tokens"→"MAX_TOKENS")
/// - `usage.input_tokens` → `usageMetadata.promptTokenCount`
/// - `usage.output_tokens` → `usageMetadata.candidatesTokenCount`
/// - `usage.input_tokens + output_tokens` → `usageMetadata.totalTokenCount`
pub fn anthropic_response_to_gemini(body: &Value) -> Value {
    // Extraire les blocs texte du contenu Anthropic
    let parts: Vec<Value> = body
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|block| {
                    if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                        block.get("text").and_then(|t| t.as_str()).map(|text| {
                            json!({ "text": text })
                        })
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let stop_reason = body
        .get("stop_reason")
        .and_then(|s| s.as_str())
        .unwrap_or("end_turn");
    let finish_reason = match stop_reason {
        "max_tokens" => "MAX_TOKENS",
        _ => "STOP",
    };

    let usage = body.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": parts
            },
            "finishReason": finish_reason
        }],
        "usageMetadata": {
            "promptTokenCount": input_tokens,
            "candidatesTokenCount": output_tokens,
            "totalTokenCount": input_tokens + output_tokens
        },
        "model": model
    })
}

/// Dispatche la traduction de requête selon le format du client.
///
/// - "openai" → `openai_request_to_anthropic`
/// - "gemini" → `gemini_request_to_anthropic`
/// - autre    → passthrough
pub fn translate_request_to_anthropic(client_fmt: &str, body: Value) -> Value {
    match client_fmt {
        "openai" => openai_request_to_anthropic(body),
        "gemini" => gemini_request_to_anthropic(body),
        _ => body,
    }
}

/// Dispatche la traduction de réponse Anthropic vers le format du client.
///
/// - "openai" → `anthropic_response_to_openai`
/// - "gemini" → `anthropic_response_to_gemini`
/// - autre    → passthrough
pub fn translate_response_to_client(client_fmt: &str, body: Value) -> Value {
    match client_fmt {
        "openai" => anthropic_response_to_openai(&body),
        "gemini" => anthropic_response_to_gemini(&body),
        _ => body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_adds_metadata() {
        let body = json!({ "model": "claude-opus-4-5", "messages": [], "max_tokens": 1024 });
        let profile = ProfileData {
            request_count: 5,
            system_format: "string".to_string(),
            ..Default::default()
        };
        let result = rewrite_body(&body, &profile);
        assert!(result.get("metadata").is_some());
    }

    #[test]
    fn test_rewrite_system_to_array() {
        let body = json!({
            "model": "claude-opus-4-5",
            "messages": [],
            "max_tokens": 1024,
            "system": "You are helpful."
        });
        let profile = ProfileData {
            request_count: 3,
            system_format: "array".to_string(),
            ..Default::default()
        };
        let result = rewrite_body(&body, &profile);
        let system = result.get("system").unwrap();
        assert!(system.is_array());
    }

    #[test]
    fn test_openai_request_to_anthropic() {
        let body = json!({
            "model": "gpt-4o",
            "messages": [
                { "role": "system", "content": "You are helpful." },
                { "role": "user", "content": "Hello" }
            ],
            "max_tokens": 512,
            "temperature": 0.7
        });
        let result = openai_request_to_anthropic(body);
        // system message extracted
        let system = result.get("system").unwrap();
        assert!(system.is_array());
        let sys_arr = system.as_array().unwrap();
        assert_eq!(sys_arr[0].get("text").unwrap().as_str().unwrap(), "You are helpful.");
        // only user message remains in messages
        let msgs = result.get("messages").unwrap().as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get("role").unwrap().as_str().unwrap(), "user");
        assert_eq!(msgs[0].get("content").unwrap().as_str().unwrap(), "Hello");
        // max_tokens and temperature preserved
        assert_eq!(result.get("max_tokens").unwrap().as_i64().unwrap(), 512);
        assert!((result.get("temperature").unwrap().as_f64().unwrap() - 0.7).abs() < 1e-9);
    }

    #[test]
    fn test_gemini_request_to_anthropic() {
        let body = json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [{ "text": "Hello Gemini" }]
                },
                {
                    "role": "model",
                    "parts": [{ "text": "Hello user!" }]
                }
            ],
            "systemInstruction": {
                "parts": [{ "text": "Be concise." }]
            },
            "generationConfig": {
                "maxOutputTokens": 1024,
                "temperature": 0.5,
                "topP": 0.9
            }
        });
        let result = gemini_request_to_anthropic(body);
        // system
        assert_eq!(result.get("system").unwrap().as_str().unwrap(), "Be concise.");
        // max_tokens
        assert_eq!(result.get("max_tokens").unwrap().as_i64().unwrap(), 1024);
        // temperature
        assert!((result.get("temperature").unwrap().as_f64().unwrap() - 0.5).abs() < 1e-9);
        // top_p
        assert!((result.get("top_p").unwrap().as_f64().unwrap() - 0.9).abs() < 1e-9);
        // messages : "model" → "assistant"
        let msgs = result.get("messages").unwrap().as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].get("role").unwrap().as_str().unwrap(), "user");
        assert_eq!(msgs[0].get("content").unwrap().as_str().unwrap(), "Hello Gemini");
        assert_eq!(msgs[1].get("role").unwrap().as_str().unwrap(), "assistant");
        assert_eq!(msgs[1].get("content").unwrap().as_str().unwrap(), "Hello user!");
    }

    #[test]
    fn test_translate_request_passthrough_anthropic() {
        let body = json!({
            "model": "claude-opus-4-6",
            "messages": [{ "role": "user", "content": "Hi" }],
            "max_tokens": 100
        });
        let result = translate_request_to_anthropic("anthropic", body.clone());
        assert_eq!(result, body);
    }

    #[test]
    fn test_anthropic_response_to_gemini() {
        let body = json!({
            "id": "msg_abc",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-6",
            "content": [{ "type": "text", "text": "Hello from Anthropic!" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 20, "output_tokens": 10 }
        });
        let result = anthropic_response_to_gemini(&body);
        // candidates
        let candidates = result.get("candidates").unwrap().as_array().unwrap();
        assert_eq!(candidates.len(), 1);
        let cand = &candidates[0];
        assert_eq!(cand.get("finishReason").unwrap().as_str().unwrap(), "STOP");
        let parts = cand
            .get("content").unwrap()
            .get("parts").unwrap()
            .as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].get("text").unwrap().as_str().unwrap(), "Hello from Anthropic!");
        // usageMetadata
        let meta = result.get("usageMetadata").unwrap();
        assert_eq!(meta.get("promptTokenCount").unwrap().as_i64().unwrap(), 20);
        assert_eq!(meta.get("candidatesTokenCount").unwrap().as_i64().unwrap(), 10);
        assert_eq!(meta.get("totalTokenCount").unwrap().as_i64().unwrap(), 30);
        // max_tokens stop reason
        let body2 = json!({
            "content": [{ "type": "text", "text": "Partial." }],
            "stop_reason": "max_tokens",
            "model": "claude-haiku-4-5-20251001",
            "usage": { "input_tokens": 5, "output_tokens": 50 }
        });
        let result2 = anthropic_response_to_gemini(&body2);
        let cand2 = &result2.get("candidates").unwrap().as_array().unwrap()[0];
        assert_eq!(cand2.get("finishReason").unwrap().as_str().unwrap(), "MAX_TOKENS");
    }

    #[test]
    fn test_translate_response_passthrough_anthropic() {
        let body = json!({
            "id": "msg_xyz",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-6",
            "content": [{ "type": "text", "text": "Hi" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 5, "output_tokens": 3 }
        });
        let result = translate_response_to_client("anthropic", body.clone());
        assert_eq!(result, body);
    }
}
