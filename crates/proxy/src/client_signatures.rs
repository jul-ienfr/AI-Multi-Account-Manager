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
        // --- Seul client natif : Claude Code (CLI + VS Code) ---
        // Détecté via user-agent "claude-cli/" qui est unique à Claude Code.
        // Tout le reste sera impersonaté par le fallback dans impersonation.rs.
        ClientSignature {
            client_name: "claude-code".to_string(),
            provider: "anthropic".to_string(),
            rules: vec![
                MatchRule::Contains("user-agent".to_string(), "claude-cli/".to_string()),
            ],
            mode: MatchMode::Any,
            is_known: true,
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
}
