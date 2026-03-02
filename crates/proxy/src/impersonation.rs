//! Orchestration de l'impersonation parfaite.
//!
//! Flux :
//! 1. `detect_client()` → ClientMatch
//! 2. Si client natif connu → `auto_capture()` (merge headers + body dans le profil)
//! 3. Si client inconnu + impersonation activée → `apply_impersonation()` :
//!    a. Charger le profil du provider actif
//!    b. Reconstruire les headers DE ZERO (whitelist, pas blacklist)
//!    c. Réécrire le body via `body_rewriter`

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use tracing::{debug, info, warn};

use crate::body_rewriter;
use crate::cc_profile::{self, ProfileCache};
use crate::client_signatures;

// ---------------------------------------------------------------------------
// État partagé de l'impersonation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ImpersonationState {
    pub enabled: bool,
    pub auto_capture: bool,
    /// Cache mémoire des profils par provider
    pub profiles: ProfileCache,
    /// Dernière requête info (pour status endpoint)
    pub last_client: Arc<Mutex<String>>,
    pub last_client_format: Arc<Mutex<String>>,
    pub last_server_format: Arc<Mutex<String>>,
    pub last_model: Arc<Mutex<String>>,
}

impl ImpersonationState {
    pub fn new(enabled: bool, auto_capture: bool) -> Self {
        let profiles = cc_profile::new_cache();

        // Pré-charger les profils existants sur disque
        for provider in cc_profile::list_profiles() {
            if let Some(data) = cc_profile::load_profile(&provider) {
                let mut guard = profiles.write().unwrap();
                guard.insert(
                    provider.clone(),
                    cc_profile::ProfileEntry {
                        data,
                        dirty: false,
                        last_flush: std::time::Instant::now(),
                    },
                );
                debug!("impersonation: loaded profile for {provider}");
            }
        }

        // Charger le profil legacy anthropic si pas déjà chargé
        {
            let has_anthropic = profiles.read().unwrap().contains_key("anthropic");
            if !has_anthropic {
                if let Some(data) = cc_profile::load_profile("anthropic") {
                    let sh = data.static_headers.len();
                    let dh = data.dynamic_headers.len();
                    let mut guard = profiles.write().unwrap();
                    guard.insert(
                        "anthropic".to_string(),
                        cc_profile::ProfileEntry {
                            data,
                            dirty: false,
                            last_flush: std::time::Instant::now(),
                        },
                    );
                    info!("impersonation: loaded legacy profile for anthropic ({sh} static, {dh} dynamic headers)");
                } else {
                    info!("impersonation: legacy profile for anthropic NOT found on disk");
                }
            }
        }

        Self {
            enabled,
            auto_capture,
            profiles,
            last_client: Arc::new(Mutex::new(String::new())),
            last_client_format: Arc::new(Mutex::new(String::new())),
            last_server_format: Arc::new(Mutex::new("anthropic".to_string())),
            last_model: Arc::new(Mutex::new(String::new())),
        }
    }
}

// ---------------------------------------------------------------------------
// Résultat de la décision d'impersonation
// ---------------------------------------------------------------------------

pub struct ImpersonationResult {
    /// Headers à utiliser pour la requête upstream (rebuilt from scratch si impersoné)
    pub headers: IndexMap<String, String>,
    /// Body réécrit (ou original si pas d'impersonation)
    pub body: serde_json::Value,
    /// True si les headers ont été totalement reconstruits (zéro fuite)
    pub full_replace: bool,
    /// Nom du client détecté (ou vide)
    pub client_name: String,
}

// ---------------------------------------------------------------------------
// Fonction principale
// ---------------------------------------------------------------------------

/// Traite une requête entrante pour décider de l'impersonation.
///
/// - Si client natif connu → capture ses headers/body en background
/// - Si client inconnu → reconstruit les headers depuis le profil du provider actif
///
/// `active_provider` : provider actuellement actif (anthropic, gemini, ...)
/// `request_headers` : headers de la requête cliente bruts
/// `request_body` : body JSON parsé (peut être None pour GET)
pub async fn process_request(
    state: &ImpersonationState,
    request_headers: &HashMap<String, String>,
    request_body: Option<&serde_json::Value>,
    active_provider: &str,
) -> ImpersonationResult {
    let client_match = client_signatures::detect_client(request_headers);

    let client_name = client_match
        .as_ref()
        .map(|m| m.client_name.clone())
        .unwrap_or_default();

    // Tracking temps réel
    if let Ok(mut g) = state.last_client.lock() {
        *g = client_name.clone();
    }

    // Auto-capture si client natif connu
    if state.auto_capture {
        if let Some(ref m) = client_match {
            if m.is_known {
                let provider_for_capture = if m.provider.is_empty() {
                    active_provider
                } else {
                    &m.provider
                };
                cc_profile::merge_request(
                    provider_for_capture,
                    request_headers,
                    request_body,
                    &state.profiles,
                );
                debug!("auto_capture: merged request for provider={provider_for_capture}");
            }
        }
    }

    // Impersonation si nécessaire
    if state.enabled {
        let should_impersonate = match client_match {
            // Client tiers connu (Cursor, Windsurf, ...) → toujours impersonater
            Some(ref m) => m.should_impersonate,
            // Client non reconnu → impersonater par défaut (fallback sécuritaire)
            None => true,
        };

        if should_impersonate {
            let label = if client_name.is_empty() { "unknown" } else { &client_name };
            return apply_impersonation(
                state,
                request_headers,
                request_body,
                active_provider,
                label,
            );
        }
    }

    // Client natif connu (Claude Code, Gemini CLI, ...) → headers originaux
    let headers: IndexMap<String, String> = request_headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.clone()))
        .collect();

    ImpersonationResult {
        headers,
        body: request_body.cloned().unwrap_or(serde_json::Value::Null),
        full_replace: false,
        client_name,
    }
}

/// Reconstruit les headers + body depuis le profil du provider actif.
fn apply_impersonation(
    state: &ImpersonationState,
    _request_headers: &HashMap<String, String>,
    request_body: Option<&serde_json::Value>,
    active_provider: &str,
    client_name: &str,
) -> ImpersonationResult {
    info!(
        "impersonation: applying for client={client_name}, provider={active_provider}"
    );

    // Charger le profil du provider actif
    let profile_opt = {
        let guard = state.profiles.read().unwrap();
        guard.get(active_provider).map(|e| e.data.clone())
    };

    // Fallback : charger depuis le disque si pas en cache
    let profile_opt = profile_opt.or_else(|| cc_profile::load_profile(active_provider));

    let (headers, body) = if let Some(profile) = profile_opt {
        // Reconstruction totale des headers depuis le profil
        let mut hdrs = cc_profile::get_ordered_headers(&profile);

        // Retirer authorization (sera ajouté par le handler)
        hdrs.swap_remove("authorization");
        hdrs.swap_remove("x-api-key");

        // Body rewrite
        let body = if let Some(b) = request_body {
            body_rewriter::rewrite_body(b, &profile)
        } else {
            serde_json::Value::Null
        };

        (hdrs, body)
    } else {
        // Fallback : utiliser les fallback headers built-in du provider
        warn!("impersonation: no profile for {active_provider}, using fallback headers");
        let fallback = fallback_headers(active_provider);
        let body = request_body.cloned().unwrap_or(serde_json::Value::Null);
        (fallback, body)
    };

    ImpersonationResult {
        headers,
        body,
        full_replace: true,
        client_name: client_name.to_string(),
    }
}

/// Headers de fallback built-in par provider (si pas de profil capturé).
fn fallback_headers(provider: &str) -> IndexMap<String, String> {
    let mut h = IndexMap::new();
    match provider {
        "anthropic" => {
            h.insert("user-agent".to_string(), "claude-code/1.0.57".to_string());
            h.insert("anthropic-client-type".to_string(), "claude-code".to_string());
            h.insert("accept".to_string(), "application/json".to_string());
        }
        "gemini" => {
            h.insert("user-agent".to_string(), "google-gemini-cli/0.1.0".to_string());
            h.insert(
                "x-goog-api-client".to_string(),
                "gemini-cli/0.1.0".to_string(),
            );
            h.insert("accept".to_string(), "application/json".to_string());
        }
        "openai" => {
            h.insert("user-agent".to_string(), "openai-python/1.0.0".to_string());
            h.insert("accept".to_string(), "application/json".to_string());
        }
        _ => {
            h.insert("user-agent".to_string(), "anthrouter/0.1.0".to_string());
            h.insert("accept".to_string(), "application/json".to_string());
        }
    }
    h
}
