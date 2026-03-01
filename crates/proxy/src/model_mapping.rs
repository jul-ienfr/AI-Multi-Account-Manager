//! Résolution de modèles : client-model → Anthropic canonical model.
//!
//! Le backend est toujours Anthropic. On traduit les noms de modèles
//! OpenAI/Gemini/alias Claude vers les vrais noms Anthropic.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Table de traduction client → Anthropic
// ---------------------------------------------------------------------------

/// Mappe un nom de modèle client (OpenAI, Gemini, alias Claude)
/// vers le nom de modèle Anthropic canonique.
///
/// Retourne `None` si le modèle n'est pas dans la table (passthrough).
pub fn client_model_to_anthropic(model: &str) -> Option<String> {
    let mapped = match model {
        // ── OpenAI → Anthropic ──────────────────────────────────────────────
        "gpt-4o"                  => "claude-opus-4-6",
        "gpt-4o-mini"             => "claude-haiku-4-5-20251001",
        "gpt-4-turbo"             => "claude-sonnet-4-6",
        "gpt-4-turbo-preview"     => "claude-sonnet-4-6",
        "gpt-4"                   => "claude-sonnet-4-6",
        "gpt-3.5-turbo"           => "claude-haiku-4-5-20251001",
        "gpt-3.5-turbo-instruct"  => "claude-haiku-4-5-20251001",

        // ── Gemini → Anthropic ──────────────────────────────────────────────
        "gemini-2.0-flash"        => "claude-haiku-4-5-20251001",
        "gemini-2.0-flash-exp"    => "claude-haiku-4-5-20251001",
        "gemini-1.5-pro"          => "claude-sonnet-4-6",
        "gemini-1.5-pro-latest"   => "claude-sonnet-4-6",
        "gemini-1.5-flash"        => "claude-haiku-4-5-20251001",
        "gemini-1.5-flash-latest" => "claude-haiku-4-5-20251001",
        "gemini-2.0-pro"          => "claude-opus-4-6",
        "gemini-pro"              => "claude-sonnet-4-6",

        // ── Alias Claude → canonique ────────────────────────────────────────
        "claude-3-opus"           => "claude-opus-4-6",
        "claude-3-5-sonnet"       => "claude-sonnet-4-6",
        "claude-3-haiku"          => "claude-haiku-4-5-20251001",
        "claude-3-sonnet"         => "claude-sonnet-4-6",

        _ => return None,
    };
    Some(mapped.to_string())
}

// ---------------------------------------------------------------------------
// Config override
// ---------------------------------------------------------------------------

/// Config override par modèle chargée depuis settings.json.
/// Format : { "modelOverrides": { "gpt-4o": "claude-opus-4-6", ... } }
pub type ModelMappingConfig = HashMap<String, String>;

/// Résout le modèle Anthropic cible pour un nom de modèle client.
///
/// Priorité :
/// 1. Config override depuis settings.json (`config_mappings` → model exact)
/// 2. `client_model_to_anthropic` (table built-in)
/// 3. Modèle original (pass-through — supposé déjà un nom Anthropic valide)
pub fn resolve_model(
    orig_model: &str,
    config_mappings: &ModelMappingConfig,
) -> String {
    // 1. Override depuis la config (exact match)
    if let Some(overridden) = config_mappings.get(orig_model) {
        if !overridden.is_empty() {
            return overridden.clone();
        }
    }

    // 2. Table built-in client → Anthropic
    if let Some(mapped) = client_model_to_anthropic(orig_model) {
        return mapped;
    }

    // 3. Pass-through (déjà un modèle Anthropic ou nom inconnu)
    orig_model.to_string()
}

/// Charge les overrides de modèles depuis settings.json.
///
/// Lit `proxy.router.modelOverrides` (map directe model→model).
pub fn load_config_mappings(settings_path: &std::path::Path) -> ModelMappingConfig {
    let Ok(content) = std::fs::read_to_string(settings_path) else {
        return HashMap::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return HashMap::new();
    };

    let mut result = ModelMappingConfig::new();

    // Format : { "proxy": { "router": { "modelOverrides": { "gpt-4o": "claude-opus-4-6" } } } }
    if let Some(overrides) = json
        .get("proxy")
        .and_then(|p| p.get("router"))
        .and_then(|r| r.get("modelOverrides"))
        .and_then(|m| m.as_object())
    {
        for (src_model, target) in overrides {
            if let Some(t) = target.as_str() {
                result.insert(src_model.clone(), t.to_string());
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpt4o_maps_to_opus() {
        let cfg = ModelMappingConfig::new();
        let result = resolve_model("gpt-4o", &cfg);
        assert_eq!(result, "claude-opus-4-6");
    }

    #[test]
    fn test_gemini_pro_maps_to_sonnet() {
        let cfg = ModelMappingConfig::new();
        let result = resolve_model("gemini-1.5-pro", &cfg);
        assert_eq!(result, "claude-sonnet-4-6");
        let result2 = resolve_model("gemini-pro", &cfg);
        assert_eq!(result2, "claude-sonnet-4-6");
    }

    #[test]
    fn test_anthropic_model_passthrough() {
        let cfg = ModelMappingConfig::new();
        // Un vrai nom Anthropic → pass-through intact
        let result = resolve_model("claude-opus-4-6", &cfg);
        assert_eq!(result, "claude-opus-4-6");
        let result2 = resolve_model("claude-haiku-4-5-20251001", &cfg);
        assert_eq!(result2, "claude-haiku-4-5-20251001");
        // Nom inconnu → pass-through
        let result3 = resolve_model("some-unknown-model", &cfg);
        assert_eq!(result3, "some-unknown-model");
    }

    #[test]
    fn test_config_override_takes_priority() {
        let mut cfg = ModelMappingConfig::new();
        // Override : gpt-4o → haiku (au lieu de opus)
        cfg.insert("gpt-4o".to_string(), "claude-haiku-4-5-20251001".to_string());
        let result = resolve_model("gpt-4o", &cfg);
        assert_eq!(result, "claude-haiku-4-5-20251001");

        // Override vide → ignoré, revient à la table built-in
        cfg.insert("gpt-4o-mini".to_string(), "".to_string());
        let result2 = resolve_model("gpt-4o-mini", &cfg);
        assert_eq!(result2, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn test_gemini_flash_maps_to_haiku() {
        let cfg = ModelMappingConfig::new();
        assert_eq!(resolve_model("gemini-2.0-flash", &cfg), "claude-haiku-4-5-20251001");
        assert_eq!(resolve_model("gemini-1.5-flash", &cfg), "claude-haiku-4-5-20251001");
        assert_eq!(resolve_model("gemini-2.0-pro", &cfg), "claude-opus-4-6");
    }

    #[test]
    fn test_claude_aliases_resolve() {
        let cfg = ModelMappingConfig::new();
        assert_eq!(resolve_model("claude-3-opus", &cfg), "claude-opus-4-6");
        assert_eq!(resolve_model("claude-3-5-sonnet", &cfg), "claude-sonnet-4-6");
        assert_eq!(resolve_model("claude-3-haiku", &cfg), "claude-haiku-4-5-20251001");
    }

    #[test]
    fn test_gpt35_maps_to_haiku() {
        let cfg = ModelMappingConfig::new();
        assert_eq!(resolve_model("gpt-3.5-turbo", &cfg), "claude-haiku-4-5-20251001");
        assert_eq!(resolve_model("gpt-3.5-turbo-instruct", &cfg), "claude-haiku-4-5-20251001");
    }
}
