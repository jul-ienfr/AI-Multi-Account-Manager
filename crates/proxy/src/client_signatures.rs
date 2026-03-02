//! Détection du client HTTP à partir des headers.
//!
//! Port de `src/agents/client_signatures.py`.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchMode {
    Any,
    #[allow(dead_code)]
    All,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum MatchRule {
    /// Header dont la valeur contient exactement ce texte
    Equals(String, String),
    /// Header dont la valeur contient ce texte (case-insensitive)
    Contains(String, String),
    /// Header dont le nom existe (quelle que soit la valeur)
    Exists(String),
}

#[derive(Debug, Clone)]
pub struct ClientSignature {
    pub client_name: String,
    /// Provider associé (anthropic, gemini, openai, ...)
    pub provider: String,
    pub rules: Vec<MatchRule>,
    pub mode: MatchMode,
    /// Si vrai, ce client est "natif" : ses headers sont capturés pour mettre à jour le profil.
    /// Si faux (Cursor, Windsurf...), on applique l'impersonation.
    pub is_known: bool,
}

#[derive(Debug, Clone)]
pub struct ClientMatch {
    pub client_name: String,
    pub provider: String,
    /// Client natif connu (on capture) ou inconnu (on impersonate)
    pub is_known: bool,
    /// Doit-on appliquer l'impersonation pour cette requête ?
    pub should_impersonate: bool,
}

// ---------------------------------------------------------------------------
// Registry de signatures
// ---------------------------------------------------------------------------

fn build_registry() -> Vec<ClientSignature> {
    vec![
        // --- Claude Code (CLI + VS Code) — client natif, pas d'impersonation ---
        // Détecté via :
        //   - user-agent "claude-cli/" (CLI v3+, extension VSCode)
        //   - user-agent "claude-code" (V2 compatibility)
        //   - anthropic-client-type: claude-code (header explicite)
        // (port de V2 signatures.py + règle claude-cli/ native V3)
        ClientSignature {
            client_name: "claude-code".to_string(),
            provider: "anthropic".to_string(),
            rules: vec![
                MatchRule::Contains("user-agent".to_string(), "claude-cli/".to_string()),
                MatchRule::Contains("user-agent".to_string(), "claude-code".to_string()),
                MatchRule::Equals("anthropic-client-type".to_string(), "claude-code".to_string()),
            ],
            mode: MatchMode::Any,
            is_known: true,
        },
        // --- Claude Web ---
        ClientSignature {
            client_name: "claude-web".to_string(),
            provider: "anthropic".to_string(),
            rules: vec![
                MatchRule::Contains("user-agent".to_string(), "claude.ai".to_string()),
            ],
            mode: MatchMode::Any,
            is_known: false,
        },
        // --- Gemini CLI ---
        ClientSignature {
            client_name: "gemini-cli".to_string(),
            provider: "gemini".to_string(),
            rules: vec![
                MatchRule::Contains("user-agent".to_string(), "google-gemini-cli".to_string()),
                MatchRule::Contains("x-goog-api-client".to_string(), "gemini-cli".to_string()),
            ],
            mode: MatchMode::Any,
            is_known: false,
        },
        // --- Gemini Code Assist ---
        ClientSignature {
            client_name: "gemini-code-assist".to_string(),
            provider: "gemini".to_string(),
            rules: vec![
                MatchRule::Contains("user-agent".to_string(), "gemini-code-assist".to_string()),
                MatchRule::Contains("x-goog-api-client".to_string(), "code-assist".to_string()),
            ],
            mode: MatchMode::Any,
            is_known: false,
        },
        // --- OpenAI Python SDK ---
        ClientSignature {
            client_name: "openai-python".to_string(),
            provider: "openai".to_string(),
            rules: vec![
                MatchRule::Contains("user-agent".to_string(), "OpenAI/Python".to_string()),
            ],
            mode: MatchMode::Any,
            is_known: false,
        },
        // --- OpenAI Node SDK ---
        ClientSignature {
            client_name: "openai-node".to_string(),
            provider: "openai".to_string(),
            rules: vec![
                MatchRule::Contains("user-agent".to_string(), "OpenAI/JS".to_string()),
            ],
            mode: MatchMode::Any,
            is_known: false,
        },
        // --- ChatGPT ---
        ClientSignature {
            client_name: "chatgpt".to_string(),
            provider: "openai".to_string(),
            rules: vec![
                MatchRule::Contains("user-agent".to_string(), "ChatGPT".to_string()),
            ],
            mode: MatchMode::Any,
            is_known: false,
        },
        // --- Grok / xAI ---
        ClientSignature {
            client_name: "grok".to_string(),
            provider: "xai".to_string(),
            rules: vec![
                MatchRule::Contains("user-agent".to_string(), "grok".to_string()),
                MatchRule::Exists("x-grok-client".to_string()),
            ],
            mode: MatchMode::Any,
            is_known: false,
        },
        // --- DeepSeek ---
        ClientSignature {
            client_name: "deepseek".to_string(),
            provider: "deepseek".to_string(),
            rules: vec![
                MatchRule::Contains("user-agent".to_string(), "deepseek".to_string()),
            ],
            mode: MatchMode::Any,
            is_known: false,
        },
    ]
}

// ---------------------------------------------------------------------------
// Fonction principale de détection
// ---------------------------------------------------------------------------

/// Détecte le client à partir des headers bruts.
///
/// Retourne `None` si aucune signature ne correspond.
pub fn detect_client(headers: &HashMap<String, String>) -> Option<ClientMatch> {
    let registry = build_registry();

    // Normalise les headers en lowercase pour la comparaison
    let lower: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.to_lowercase()))
        .collect();

    for sig in &registry {
        let matched = match sig.mode {
            MatchMode::Any => sig.rules.iter().any(|r| rule_matches(r, &lower)),
            MatchMode::All => sig.rules.iter().all(|r| rule_matches(r, &lower)),
        };

        if matched {
            // Les clients inconnus doivent être impersonés si l'impersonation est activée.
            // Les clients natifs connus sont auto-capturés uniquement.
            let should_impersonate = !sig.is_known;
            return Some(ClientMatch {
                client_name: sig.client_name.clone(),
                provider: sig.provider.clone(),
                is_known: sig.is_known,
                should_impersonate,
            });
        }
    }
    None
}

fn rule_matches(rule: &MatchRule, headers: &HashMap<String, String>) -> bool {
    match rule {
        MatchRule::Equals(header, value) => {
            headers.get(&header.to_lowercase()).map(|v| v == &value.to_lowercase()).unwrap_or(false)
        }
        MatchRule::Contains(header, value) => {
            headers
                .get(&header.to_lowercase())
                .map(|v| v.contains(&value.to_lowercase()))
                .unwrap_or(false)
        }
        MatchRule::Exists(header) => headers.contains_key(&header.to_lowercase()),
    }
}

/// Retourne la liste de tous les providers connus.
#[allow(dead_code)]
pub fn known_providers() -> Vec<&'static str> {
    vec!["anthropic", "gemini", "openai", "xai", "deepseek", "mistral", "groq"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_claude_code_cli() {
        let mut h = HashMap::new();
        h.insert("user-agent".to_string(), "claude-cli/2.1.61 (external, claude-vscode, agent-sdk/0.2.61)".to_string());
        h.insert("anthropic-version".to_string(), "2023-06-01".to_string());
        let m = detect_client(&h).unwrap();
        assert_eq!(m.client_name, "claude-code");
        assert_eq!(m.provider, "anthropic");
        assert!(m.is_known);
        assert!(!m.should_impersonate);
    }

    #[test]
    fn test_cursor_not_detected() {
        // Cursor n'est plus dans le registre → None → sera impersonaté par le fallback
        let mut h = HashMap::new();
        h.insert("user-agent".to_string(), "Cursor/0.44.0 (darwin arm64)".to_string());
        assert!(detect_client(&h).is_none());
    }

    #[test]
    fn test_kilo_code_not_detected() {
        // KiloCode → None → sera impersonaté par le fallback
        let mut h = HashMap::new();
        h.insert("user-agent".to_string(), "kilo-code/1.0.0".to_string());
        assert!(detect_client(&h).is_none());
    }

    #[test]
    fn test_sdk_anthropic_not_detected_as_claude_code() {
        // Un client utilisant le SDK Anthropic mais pas Claude Code → None
        let mut h = HashMap::new();
        h.insert("user-agent".to_string(), "my-custom-app/1.0".to_string());
        h.insert("anthropic-version".to_string(), "2023-06-01".to_string());
        h.insert("x-stainless-lang".to_string(), "js".to_string());
        assert!(detect_client(&h).is_none());
    }

    #[test]
    fn test_unknown_client() {
        let mut h = HashMap::new();
        h.insert("user-agent".to_string(), "python-httpx/0.27.0".to_string());
        assert!(detect_client(&h).is_none());
    }

    #[test]
    fn test_gemini_cli_detected() {
        let mut h = HashMap::new();
        h.insert("user-agent".to_string(), "google-gemini-cli/1.0.0".to_string());
        let m = detect_client(&h).unwrap();
        assert_eq!(m.client_name, "gemini-cli");
        assert_eq!(m.provider, "gemini");
        assert!(!m.is_known);
        assert!(m.should_impersonate);
    }

    #[test]
    fn test_openai_python_detected() {
        let mut h = HashMap::new();
        h.insert("user-agent".to_string(), "OpenAI/Python 1.23.0".to_string());
        let m = detect_client(&h).unwrap();
        assert_eq!(m.client_name, "openai-python");
        assert_eq!(m.provider, "openai");
        assert!(!m.is_known);
        assert!(m.should_impersonate);
    }

    #[test]
    fn test_claude_code_via_anthropic_client_type() {
        // Claude Code peut aussi s'identifier via ce header (V2 compat)
        let mut h = HashMap::new();
        h.insert("user-agent".to_string(), "my-wrapper/1.0".to_string());
        h.insert("anthropic-client-type".to_string(), "claude-code".to_string());
        let m = detect_client(&h).unwrap();
        assert_eq!(m.client_name, "claude-code");
        assert!(m.is_known);
        assert!(!m.should_impersonate);
    }
}
