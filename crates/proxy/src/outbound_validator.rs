//! Validation sortante — s'assure que tout ce qui part vers api.anthropic.com
//! ressemble à une requête Claude Code native (whitelist stricte).

use axum::http::HeaderMap;
use serde_json::Value;
use tracing::warn;

// ---------------------------------------------------------------------------
// Whitelists
// ---------------------------------------------------------------------------

/// Headers autorisés vers api.anthropic.com (lowercase).
/// Calqués exactement sur ce que Claude Code CLI/VS Code envoie.
const ALLOWED_OUTBOUND_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "authorization",
    "anthropic-version",
    "anthropic-beta",
    "x-app",
    "anthropic-dangerous-direct-browser-access",
    "user-agent",
    "accept",
    "accept-encoding",
];

/// Préfixes de headers autorisés (ex: x-stainless-*)
const ALLOWED_HEADER_PREFIXES: &[&str] = &[
    "x-stainless-",
];

/// Champs body requis pour une requête Anthropic /v1/messages valide
const REQUIRED_BODY_KEYS: &[&str] = &["model", "messages"];

/// Champs body interdits (résidus OpenAI/Gemini)
const FORBIDDEN_BODY_KEYS: &[&str] = &[
    "choices",
    "prompt",
    "n",
    "frequency_penalty",
    "presence_penalty",
    "logit_bias",
    "logprobs",
    "contents",
    "generation_config",
    "candidates",
    "usage_metadata",
];

// ---------------------------------------------------------------------------
// Validation + sanitisation
// ---------------------------------------------------------------------------

/// Valide et assainit la requête avant envoi vers api.anthropic.com.
/// - Strip les headers hors whitelist
/// - Vérifie la présence des champs Anthropic requis
/// - Supprime les champs body interdits (résidus format client)
/// - Retourne Err si la requête est invalide (ne doit pas être envoyée)
pub fn validate_and_sanitize(
    headers: &mut HeaderMap,
    body: &mut Value,
) -> Result<(), String> {
    // 1. Strip headers hors whitelist
    let keys_to_remove: Vec<String> = headers
        .keys()
        .filter(|k| {
            let name = k.as_str().to_lowercase();
            let allowed = ALLOWED_OUTBOUND_HEADERS.contains(&name.as_str())
                || ALLOWED_HEADER_PREFIXES.iter().any(|p| name.starts_with(p));
            !allowed
        })
        .map(|k| k.as_str().to_string())
        .collect();

    for key in &keys_to_remove {
        if let Ok(name) = axum::http::header::HeaderName::from_bytes(key.as_bytes()) {
            headers.remove(&name);
            warn!("outbound_validator: stripped header '{}'", key);
        }
    }

    // 2. Vérifier anthropic-version présent
    if !headers.contains_key("anthropic-version") {
        return Err("anthropic-version header manquant".to_string());
    }

    // 3. Vérifier que le body est un objet JSON
    let obj = match body.as_object_mut() {
        Some(o) => o,
        None => return Err("body doit être un objet JSON".to_string()),
    };

    // 4. Supprimer champs interdits
    for key in FORBIDDEN_BODY_KEYS {
        if obj.remove(*key).is_some() {
            warn!("outbound_validator: stripped body field '{}'", key);
        }
    }

    // 5. Vérifier champs requis
    for key in REQUIRED_BODY_KEYS {
        if !obj.contains_key(*key) {
            return Err(format!("champ body requis manquant: '{}'", key));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderName, HeaderValue};
    use serde_json::json;

    fn make_valid_body() -> Value {
        json!({
            "model": "claude-opus-4-6",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 10
        })
    }

    fn make_headers_with_version() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        h.insert("content-type", HeaderValue::from_static("application/json"));
        h.insert("user-agent", HeaderValue::from_static("claude-code/1.0"));
        h
    }

    #[test]
    fn test_valid_cc_request_passes() {
        let mut headers = make_headers_with_version();
        let mut body = make_valid_body();
        assert!(validate_and_sanitize(&mut headers, &mut body).is_ok());
    }

    #[test]
    fn test_openai_residual_body_stripped() {
        let mut headers = make_headers_with_version();
        let mut body = make_valid_body();
        body["choices"] = json!([]);
        body["n"] = json!(1);
        // Should succeed but strip forbidden fields
        validate_and_sanitize(&mut headers, &mut body).unwrap();
        assert!(body.get("choices").is_none());
        assert!(body.get("n").is_none());
        assert!(body.get("model").is_some());
    }

    #[test]
    fn test_unknown_headers_stripped() {
        let mut headers = make_headers_with_version();
        headers.insert(
            HeaderName::from_static("x-evil-header"),
            HeaderValue::from_static("bad"),
        );
        let mut body = make_valid_body();
        validate_and_sanitize(&mut headers, &mut body).unwrap();
        assert!(headers.get("x-evil-header").is_none());
        assert!(headers.get("anthropic-version").is_some());
    }

    #[test]
    fn test_missing_anthropic_version_fails() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        let mut body = make_valid_body();
        let result = validate_and_sanitize(&mut headers, &mut body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("anthropic-version"));
    }

    #[test]
    fn test_missing_required_body_key_fails() {
        let mut headers = make_headers_with_version();
        // messages manquant
        let mut body = json!({"model": "claude-opus-4-6"});
        let result = validate_and_sanitize(&mut headers, &mut body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("messages"));
    }

    #[test]
    fn test_body_without_max_tokens_passes() {
        let mut headers = make_headers_with_version();
        let mut body = json!({"model": "claude-opus-4-6", "messages": [{"role": "user", "content": "hi"}]});
        assert!(validate_and_sanitize(&mut headers, &mut body).is_ok());
    }

    #[test]
    fn test_stainless_headers_allowed() {
        let mut headers = make_headers_with_version();
        headers.insert(
            HeaderName::from_static("x-stainless-arch"),
            HeaderValue::from_static("x86_64"),
        );
        let mut body = make_valid_body();
        validate_and_sanitize(&mut headers, &mut body).unwrap();
        // x-stainless-* doit être conservé
        assert!(headers.get("x-stainless-arch").is_some());
    }
}
