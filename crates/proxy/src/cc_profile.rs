//! Chargement et sauvegarde des profils d'impersonation par provider.
//!
//! Port de `src/agents/cc_profile.py` — version multi-provider.
//!
//! Chaque profil est stocké dans `~/.claude/multi-account/profiles/{provider}.json`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

pub const MAX_SAMPLES: usize = 5;
const FLUSH_INTERVAL_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// Types JSON (profil sur disque)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DynamicHeader {
    /// Dernière valeur observée
    pub latest: String,
    /// Pattern détecté : "uuid" | "iso8601" | "counter" | "variable" | "static"
    pub pattern: String,
    /// Derniers échantillons (FIFO, max MAX_SAMPLES)
    pub samples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileData {
    /// Provider associé (anthropic, gemini, ...)
    pub provider: String,
    /// Headers statiques (nom → valeur fixe)
    pub static_headers: IndexMap<String, String>,
    /// Headers dynamiques (nom → DynamicHeader)
    pub dynamic_headers: HashMap<String, DynamicHeader>,
    /// Ordre d'envoi des headers (noms en minuscules)
    pub header_order: Vec<String>,
    /// Champs body que le vrai client envoie toujours
    pub body_field_whitelist: Vec<String>,
    /// Format du system prompt capturé ("string" | "array")
    pub system_format: String,
    /// Le vrai client envoie cache_control sur les tools
    pub has_tools_cache_control: bool,
    /// Le vrai client stream toujours
    pub always_streams: bool,
    /// Nombre de requêtes capturées
    pub request_count: u64,
    /// Date de la dernière capture ISO8601
    pub last_capture: String,
    /// Version du profil
    pub version: u32,
}

// ---------------------------------------------------------------------------
// Cache mémoire
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ProfileEntry {
    pub data: ProfileData,
    pub dirty: bool,
    pub last_flush: Instant,
}

pub type ProfileCache = Arc<RwLock<HashMap<String, ProfileEntry>>>;

pub fn new_cache() -> ProfileCache {
    Arc::new(RwLock::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Chemins
// ---------------------------------------------------------------------------

pub fn profiles_dir() -> Option<PathBuf> {
    // Chercher ~/.claude/multi-account/profiles/
    let candidates: Vec<PathBuf> = vec![
        dirs_home().map(|h| h.join(".claude").join("multi-account").join("profiles")),
        // WSL : répertoire Windows
        std::env::var("USERPROFILE")
            .ok()
            .map(|p| PathBuf::from(p).join(".claude").join("multi-account").join("profiles")),
    ]
    .into_iter()
    .flatten()
    .collect();

    candidates
        .into_iter()
        .find(|path| path.parent().map(|p| p.exists()).unwrap_or(false) || path.exists())
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
}

pub fn profile_path(provider: &str) -> Option<PathBuf> {
    profiles_dir().map(|d| d.join(format!("{provider}.json")))
}

// ---------------------------------------------------------------------------
// Chargement / sauvegarde
// ---------------------------------------------------------------------------

pub fn load_profile(provider: &str) -> Option<ProfileData> {
    let path = profile_path(provider)?;
    if !path.exists() {
        // Fallback legacy : ~/.claude/multi-account/cc-profile.json pour anthropic
        if provider == "anthropic" {
            return load_legacy_profile();
        }
        return None;
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| warn!("Cannot read profile {}: {e}", path.display()))
        .ok()?;
    serde_json::from_str::<ProfileData>(&content)
        .map_err(|e| warn!("Cannot parse profile {provider}: {e}"))
        .ok()
}

fn load_legacy_profile() -> Option<ProfileData> {
    let home = dirs_home()?;
    let path = home.join(".claude").join("multi-account").join("cc-profile.json");
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;

    // Essayer d'abord le format Rust natif
    if let Ok(mut data) = serde_json::from_str::<ProfileData>(&content) {
        data.provider = "anthropic".to_string();
        return Some(data);
    }

    // Sinon, parser le format Python legacy
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    let headers = v.get("headers")?;

    let mut static_headers = IndexMap::new();
    if let Some(obj) = headers.get("static").and_then(|s| s.as_object()) {
        for (k, val) in obj {
            if let Some(s) = val.as_str() {
                static_headers.insert(k.to_lowercase(), s.to_string());
            }
        }
    }

    let mut dynamic_headers = HashMap::new();
    if let Some(obj) = headers.get("dynamic").and_then(|d| d.as_object()) {
        for (k, val) in obj {
            let samples: Vec<String> = val.get("samples")
                .and_then(|s| s.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let latest = val.get("latest")
                .and_then(|l| l.as_str())
                .unwrap_or("")
                .to_string();
            let pattern = if samples.len() >= 2 { detect_pattern(&samples) } else { "static".to_string() };
            dynamic_headers.insert(k.to_lowercase(), DynamicHeader { latest, pattern, samples });
        }
    }

    let header_order = headers.get("order")
        .and_then(|o| o.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_lowercase())).collect())
        .unwrap_or_default();

    let body_field_whitelist = v.get("body_field_whitelist")
        .and_then(|b| b.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let body_info = v.get("body").and_then(|b| b.as_object());
    let system_format = body_info
        .and_then(|b| b.get("system_format"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let always_streams = body_info
        .and_then(|b| b.get("always_streams"))
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    let has_tools_cache_control = body_info
        .and_then(|b| b.get("has_tools_cache_control"))
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    Some(ProfileData {
        provider: "anthropic".to_string(),
        static_headers,
        dynamic_headers,
        header_order,
        body_field_whitelist,
        system_format,
        has_tools_cache_control,
        always_streams,
        request_count: v.get("request_count").and_then(|r| r.as_u64()).unwrap_or(0),
        last_capture: v.get("captured_at").and_then(|c| c.as_str()).unwrap_or("").to_string(),
        version: v.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
    })
}

pub fn save_profile(provider: &str, data: &ProfileData) -> std::io::Result<()> {
    let Some(path) = profile_path(provider) else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "profiles dir not found"));
    };
    // Créer le dossier si nécessaire
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, json)
}

/// Liste tous les providers qui ont un profil sur disque.
pub fn list_profiles() -> Vec<String> {
    let Some(dir) = profiles_dir() else { return vec![] };
    let Ok(entries) = std::fs::read_dir(&dir) else { return vec![] };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json").map(|p| p.to_string())
        })
        .collect()
}

/// Retourne les headers dans l'ordre du profil, toujours avec la dernière valeur capturée.
pub fn get_ordered_headers(data: &ProfileData) -> IndexMap<String, String> {
    let mut result = IndexMap::new();

    // Utiliser l'ordre capturé si disponible
    let order: Vec<String> = if data.header_order.is_empty() {
        data.static_headers.keys().cloned().collect()
    } else {
        data.header_order.clone()
    };

    for name in &order {
        let name_lower = name.to_lowercase();
        // Toujours prioriser dynamic_headers (contient latest = dernière valeur capturée)
        if let Some(dyn_h) = data.dynamic_headers.get(&name_lower) {
            result.insert(name_lower.clone(), dyn_h.latest.clone());
        } else if let Some(val) = data.static_headers.get(&name_lower) {
            result.insert(name_lower.clone(), val.clone());
        }
    }

    // Ajouter les headers dynamiques non encore insérés
    for (k, dyn_h) in &data.dynamic_headers {
        result.entry(k.clone()).or_insert_with(|| dyn_h.latest.clone());
    }

    result
}

// ---------------------------------------------------------------------------
// Auto-capture : merge d'une requête entrante dans le profil
// ---------------------------------------------------------------------------

/// Merge les headers et infos body d'une requête native dans le profil en mémoire.
pub fn merge_request(
    provider: &str,
    headers: &HashMap<String, String>,
    body: Option<&serde_json::Value>,
    cache: &ProfileCache,
) {
    let mut guard = cache.write().unwrap();
    let entry = guard.entry(provider.to_string()).or_insert_with(|| ProfileEntry {
        data: ProfileData {
            provider: provider.to_string(),
            ..Default::default()
        },
        dirty: false,
        last_flush: Instant::now(),
    });

    let data = &mut entry.data;
    data.request_count += 1;
    data.last_capture = Utc::now().to_rfc3339();
    data.version = 1;

    // Inférer l'ordre des headers si pas encore capturé
    if data.header_order.is_empty() {
        data.header_order = headers.keys().map(|k| k.to_lowercase()).collect();
    }

    // Mettre à jour les headers (static vs dynamic)
    for (k, v) in headers {
        let key = k.to_lowercase();
        // Ignorer les headers transport/auth
        if matches!(
            key.as_str(),
            "host" | "authorization" | "x-api-key" | "content-length" | "transfer-encoding"
                | "connection" | "accept-encoding"
        ) {
            continue;
        }

        let dyn_entry = data.dynamic_headers.entry(key.clone()).or_insert_with(|| DynamicHeader {
            latest: v.clone(),
            pattern: "static".to_string(),
            samples: vec![],
        });

        // FIFO rotation
        if !dyn_entry.samples.contains(v) {
            dyn_entry.samples.push(v.clone());
            if dyn_entry.samples.len() > MAX_SAMPLES {
                dyn_entry.samples.remove(0);
            }
        }
        dyn_entry.latest = v.clone();

        // Détection de pattern si assez d'échantillons
        if dyn_entry.samples.len() >= 2 {
            dyn_entry.pattern = detect_pattern(&dyn_entry.samples);
        }

        // Si le pattern est "static", déplacer vers static_headers
        if dyn_entry.pattern == "static" {
            data.static_headers.insert(key.clone(), v.clone());
        } else {
            // Pattern devenu variable → retirer l'ancienne valeur statique périmée
            data.static_headers.swap_remove(&key);
        }
    }

    // Infos body
    if let Some(body_val) = body {
        // Whitelist des champs effectivement présents dans le body
        if let Some(obj) = body_val.as_object() {
            for k in obj.keys() {
                if !data.body_field_whitelist.contains(k) {
                    data.body_field_whitelist.push(k.clone());
                }
            }
            // Détecter le format du system
            if let Some(sys) = obj.get("system") {
                data.system_format = if sys.is_array() {
                    "array".to_string()
                } else {
                    "string".to_string()
                };
            }
            // Détecter stream
            if let Some(stream) = obj.get("stream") {
                if stream.as_bool() == Some(true) {
                    data.always_streams = true;
                }
            }
        }
    }

    entry.dirty = true;
}

fn detect_pattern(samples: &[String]) -> String {
    // UUID
    let uuid_re = regex::Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap();
    if samples.iter().all(|s| uuid_re.is_match(s)) {
        return "uuid".to_string();
    }
    // ISO8601
    let iso_re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}").unwrap();
    if samples.iter().all(|s| iso_re.is_match(s)) {
        return "iso8601".to_string();
    }
    // Counter (numériques croissants)
    let nums: Vec<u64> = samples.iter().filter_map(|s| s.parse().ok()).collect();
    if nums.len() == samples.len() && nums.windows(2).all(|w| w[1] >= w[0]) {
        return "counter".to_string();
    }
    // Variable mais pas classifiable
    if samples.windows(2).any(|w| w[0] != w[1]) {
        return "variable".to_string();
    }
    "static".to_string()
}

/// Flush périodique vers le disque (appeler depuis un background task).
pub fn flush_cache(cache: &ProfileCache) {
    flush_cache_count(cache);
}

/// Flush périodique vers le disque — retourne le nombre de profils effectivement sauvegardés.
pub fn flush_cache_count(cache: &ProfileCache) -> usize {
    let mut guard = cache.write().unwrap();
    let mut flushed = 0usize;
    for (provider, entry) in guard.iter_mut() {
        if entry.dirty
            && entry.last_flush.elapsed() >= Duration::from_secs(FLUSH_INTERVAL_SECS)
        {
            if let Err(e) = save_profile(provider, &entry.data) {
                warn!("flush_cache: cannot save profile for {provider}: {e}");
            } else {
                debug!("flush_cache: profile {provider} saved");
                entry.dirty = false;
                entry.last_flush = Instant::now();
                flushed += 1;
            }
        }
    }
    flushed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_pattern_uuid() {
        let samples = vec![
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
            "6ba7b810-9dad-11d1-80b4-00c04fd430c8".to_string(),
        ];
        assert_eq!(detect_pattern(&samples), "uuid");
    }

    #[test]
    fn test_detect_pattern_static() {
        let samples = vec!["application/json".to_string(), "application/json".to_string()];
        assert_eq!(detect_pattern(&samples), "static");
    }

}
