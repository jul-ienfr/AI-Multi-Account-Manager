//! Translation layer V2↔V3 pour la compatibilité P2P inter-version.
//!
//! V2 (Python) et V3 (Rust) utilisent des formats de messages différents :
//!
//! | Aspect         | V2                          | V3                           |
//! |---------------|-----------------------------|------------------------------|
//! | Sender field  | `"source"`                  | `"from"`                     |
//! | Type location | top-level `"type"`          | inside `"payload"` (serde)   |
//! | Timestamp     | UNIX float `1709251234.567` | ISO 8601 `"2026-03-01T..."` |
//! | Mesh relay    | `"visited": [...]`          | optionnel (ajouté par A.1)   |
//! | Version       | `"v": 1`                    | optionnel (ajouté par A.1)   |
//!
//! Ce module effectue la traduction bidirectionnelle sans toucher au coordinator.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, warn};

use crate::messages::{SyncMessage, SyncPayload, VectorClock};

// ----------------------------------------------------------------
// Types
// ----------------------------------------------------------------

/// Protocole détecté d'un pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerProtocol {
    V2,
    V3,
    Unknown,
}

/// Message P2P au format V2 (Python).
///
/// Format JSON :
/// ```json
/// {
///   "id": "uuid",
///   "type": "heartbeat",
///   "source": "instance-id",
///   "timestamp": 1709251234.567,
///   "v": 1,
///   "visited": ["peer-a"],
///   "payload": { ... }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2Message {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub source: String,
    pub timestamp: f64,
    #[serde(default)]
    pub v: u32,
    #[serde(default)]
    pub visited: Vec<String>,
    #[serde(default)]
    pub payload: Value,
}

// ----------------------------------------------------------------
// Détection du protocole
// ----------------------------------------------------------------

/// Détecte si un JSON brut est au format V2 ou V3.
///
/// Heuristique :
/// - V2 : présence de `"source"` ET `"type"` au top-level (pas dans payload)
/// - V3 : présence de `"from"` ET `"payload"` avec un objet contenant `"type"`
pub fn detect_protocol(raw_json: &[u8]) -> PeerProtocol {
    let Ok(val) = serde_json::from_slice::<Value>(raw_json) else {
        return PeerProtocol::Unknown;
    };
    let obj = match val.as_object() {
        Some(o) => o,
        None => return PeerProtocol::Unknown,
    };

    // V2 : top-level "source" + top-level "type" (string)
    let has_source = obj.get("source").and_then(|v| v.as_str()).is_some();
    let has_toplevel_type = obj.get("type").and_then(|v| v.as_str()).is_some();

    // V3 : top-level "from" + "payload" object with inner "type"
    let has_from = obj.get("from").and_then(|v| v.as_str()).is_some();
    let has_payload_type = obj
        .get("payload")
        .and_then(|v| v.as_object())
        .and_then(|p| p.get("type"))
        .and_then(|v| v.as_str())
        .is_some();

    if has_source && has_toplevel_type && !has_from {
        PeerProtocol::V2
    } else if has_from && has_payload_type {
        PeerProtocol::V3
    } else {
        PeerProtocol::Unknown
    }
}

// ----------------------------------------------------------------
// Timestamp conversion
// ----------------------------------------------------------------

/// Convertit un timestamp UNIX float (V2) en DateTime<Utc> (V3).
pub fn unix_float_to_datetime(ts: f64) -> DateTime<Utc> {
    let secs = ts.trunc() as i64;
    let nanos = ((ts.fract()) * 1_000_000_000.0) as u32;
    Utc.timestamp_opt(secs, nanos)
        .single()
        .unwrap_or_else(Utc::now)
}

/// Convertit un DateTime<Utc> (V3) en timestamp UNIX float (V2).
pub fn datetime_to_unix_float(dt: &DateTime<Utc>) -> f64 {
    dt.timestamp() as f64 + (dt.timestamp_subsec_nanos() as f64 / 1_000_000_000.0)
}

// ----------------------------------------------------------------
// V2 → V3 translation
// ----------------------------------------------------------------

/// Traduit un message V2 en SyncMessage V3.
///
/// Retourne `None` pour les types V2 inconnus ou non supportés (proxy_state, code_update).
pub fn v2_to_v3(v2: &V2Message, _our_id: &str) -> Option<SyncMessage> {
    let payload = v2_type_to_v3_payload(&v2.msg_type, &v2.payload, &v2.source)?;

    Some(SyncMessage {
        id: v2.id.clone(),
        from: v2.source.clone(),
        payload,
        timestamp: unix_float_to_datetime(v2.timestamp),
        visited: v2.visited.clone(),
        v: if v2.v > 0 { Some(v2.v) } else { None },
    })
}

/// Traduit un type V2 + payload en SyncPayload V3.
fn v2_type_to_v3_payload(msg_type: &str, payload: &Value, source: &str) -> Option<SyncPayload> {
    match msg_type {
        "heartbeat" => Some(SyncPayload::Heartbeat {
            instance_id: source.to_string(),
            timestamp: Utc::now(),
        }),

        "account_switch" | "force_switch" => {
            let new_key = payload
                .get("key")
                .or_else(|| payload.get("new_key"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if new_key.is_empty() {
                warn!("V2 {} missing key field", msg_type);
                return None;
            }
            Some(SyncPayload::AccountSwitch {
                new_key,
                clock: VectorClock::new(),
            })
        }

        "quota_update" => {
            let account_key = payload
                .get("key")
                .or_else(|| payload.get("account_key"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tokens_5h = payload
                .get("tokens_5h")
                .or_else(|| payload.get("usage_5h"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let tokens_7d = payload
                .get("tokens_7d")
                .or_else(|| payload.get("usage_7d"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Some(SyncPayload::QuotaUpdate {
                account_key,
                tokens_5h,
                tokens_7d,
                clock: VectorClock::new(),
            })
        }

        "token_refresh" | "account_add" | "full_sync" => {
            // V2 envoie les credentials sous différents types
            let accounts_json = if let Some(accounts) = payload.get("accounts") {
                serde_json::to_string(accounts).unwrap_or_else(|_| "{}".to_string())
            } else {
                serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string())
            };
            let active_key = payload
                .get("active_key")
                .or_else(|| payload.get("active"))
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(SyncPayload::Credentials {
                accounts_json,
                active_key,
                clock: VectorClock::new(),
            })
        }

        "account_remove" => {
            // Tombstone : on envoie un Credentials avec un JSON vide/marqué
            let accounts_json = serde_json::to_string(payload)
                .unwrap_or_else(|_| "{}".to_string());
            Some(SyncPayload::Credentials {
                accounts_json,
                active_key: None,
                clock: VectorClock::new(),
            })
        }

        "full_sync_request" => Some(SyncPayload::SyncRequest {
            instance_id: source.to_string(),
        }),

        "ack" => Some(SyncPayload::PipelineAck {
            merged_clock: VectorClock::new(),
            accounts_applied: 0,
        }),

        // Types V2 ignorés (log seulement)
        "proxy_state" | "code_update" => {
            debug!("V2 message type '{}' ignored (no V3 equivalent)", msg_type);
            None
        }

        other => {
            warn!("Unknown V2 message type: '{}'", other);
            None
        }
    }
}

// ----------------------------------------------------------------
// V3 → V2 translation
// ----------------------------------------------------------------

/// Traduit un SyncMessage V3 en V2Message.
///
/// Retourne `None` pour les types V3 sans équivalent V2
/// (HandshakeRequest/Response, DiffRequest/Response).
pub fn v3_to_v2(v3: &SyncMessage) -> Option<V2Message> {
    let (msg_type, payload) = v3_payload_to_v2(&v3.payload)?;

    Some(V2Message {
        id: v3.id.clone(),
        msg_type,
        source: v3.from.clone(),
        timestamp: datetime_to_unix_float(&v3.timestamp),
        v: v3.v.unwrap_or(1),
        visited: v3.visited.clone(),
        payload,
    })
}

/// Traduit un SyncPayload V3 en (type_v2, payload_json).
fn v3_payload_to_v2(payload: &SyncPayload) -> Option<(String, Value)> {
    match payload {
        SyncPayload::Heartbeat { .. } => {
            Some(("heartbeat".to_string(), Value::Object(Default::default())))
        }

        SyncPayload::AccountSwitch { new_key, .. } => {
            let mut map = serde_json::Map::new();
            map.insert("key".to_string(), Value::String(new_key.clone()));
            Some(("account_switch".to_string(), Value::Object(map)))
        }

        SyncPayload::QuotaUpdate {
            account_key,
            tokens_5h,
            tokens_7d,
            ..
        } => {
            let mut map = serde_json::Map::new();
            map.insert("key".to_string(), Value::String(account_key.clone()));
            map.insert("tokens_5h".to_string(), Value::Number((*tokens_5h).into()));
            map.insert("tokens_7d".to_string(), Value::Number((*tokens_7d).into()));
            Some(("quota_update".to_string(), Value::Object(map)))
        }

        SyncPayload::Credentials {
            accounts_json,
            active_key,
            ..
        } => {
            let accounts: Value =
                serde_json::from_str(accounts_json).unwrap_or(Value::Object(Default::default()));
            let mut map = serde_json::Map::new();
            map.insert("accounts".to_string(), accounts);
            if let Some(key) = active_key {
                map.insert("active_key".to_string(), Value::String(key.clone()));
            }
            Some(("full_sync".to_string(), Value::Object(map)))
        }

        SyncPayload::SyncRequest { .. } => {
            Some(("full_sync_request".to_string(), Value::Object(Default::default())))
        }

        SyncPayload::SyncResponse {
            credentials_json,
            active_key,
            ..
        } => {
            let accounts: Value =
                serde_json::from_str(credentials_json).unwrap_or(Value::Object(Default::default()));
            let mut map = serde_json::Map::new();
            map.insert("accounts".to_string(), accounts);
            if let Some(key) = active_key {
                map.insert("active_key".to_string(), Value::String(key.clone()));
            }
            Some(("full_sync".to_string(), Value::Object(map)))
        }

        SyncPayload::PipelineAck { .. } => {
            Some(("ack".to_string(), Value::Object(Default::default())))
        }

        // Types V3-only (pipeline avancé) — pas d'équivalent V2
        SyncPayload::HandshakeRequest { .. }
        | SyncPayload::HandshakeResponse { .. }
        | SyncPayload::DiffRequest { .. }
        | SyncPayload::DiffResponse { .. } => {
            debug!("V3 pipeline message has no V2 equivalent, skipping");
            None
        }
    }
}

// ----------------------------------------------------------------
// Helper : parse un JSON brut en SyncMessage (V3 direct ou V2 traduit)
// ----------------------------------------------------------------

/// Parse un JSON brut en SyncMessage, en détectant automatiquement le protocole.
///
/// - V3 : désérialise directement
/// - V2 : désérialise en V2Message puis traduit via `v2_to_v3()`
/// - Unknown : retourne une erreur
///
/// Retourne `(SyncMessage, PeerProtocol)`.
pub fn parse_message(raw_json: &[u8], our_id: &str) -> Result<(SyncMessage, PeerProtocol), String> {
    let protocol = detect_protocol(raw_json);
    match protocol {
        PeerProtocol::V3 => {
            let msg: SyncMessage = serde_json::from_slice(raw_json)
                .map_err(|e| format!("V3 JSON decode: {e}"))?;
            Ok((msg, PeerProtocol::V3))
        }
        PeerProtocol::V2 => {
            let v2: V2Message = serde_json::from_slice(raw_json)
                .map_err(|e| format!("V2 JSON decode: {e}"))?;
            let msg = v2_to_v3(&v2, our_id)
                .ok_or_else(|| format!("V2 type '{}' not translatable", v2.msg_type))?;
            Ok((msg, PeerProtocol::V2))
        }
        PeerProtocol::Unknown => {
            // Fallback : essayer V3 d'abord, puis V2
            if let Ok(msg) = serde_json::from_slice::<SyncMessage>(raw_json) {
                return Ok((msg, PeerProtocol::V3));
            }
            if let Ok(v2) = serde_json::from_slice::<V2Message>(raw_json) {
                if let Some(msg) = v2_to_v3(&v2, our_id) {
                    return Ok((msg, PeerProtocol::V2));
                }
            }
            Err("Cannot parse message: unknown protocol".to_string())
        }
    }
}

/// Sérialise un SyncMessage pour un pair d'un protocole donné.
///
/// - V3 : sérialise directement en JSON
/// - V2 : traduit via `v3_to_v2()` puis sérialise le V2Message
/// - Unknown : sérialise en V3 par défaut
pub fn serialize_for_protocol(msg: &SyncMessage, protocol: PeerProtocol) -> Result<Vec<u8>, String> {
    match protocol {
        PeerProtocol::V2 => {
            let v2 = v3_to_v2(msg)
                .ok_or_else(|| "Cannot translate V3 message to V2".to_string())?;
            serde_json::to_vec(&v2).map_err(|e| format!("V2 JSON encode: {e}"))
        }
        PeerProtocol::V3 | PeerProtocol::Unknown => {
            serde_json::to_vec(msg).map_err(|e| format!("V3 JSON encode: {e}"))
        }
    }
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- detect_protocol ----

    #[test]
    fn test_detect_v2_protocol() {
        let json = br#"{"id":"abc","type":"heartbeat","source":"inst-1","timestamp":1709251234.5,"v":1,"visited":[],"payload":{}}"#;
        assert_eq!(detect_protocol(json), PeerProtocol::V2);
    }

    #[test]
    fn test_detect_v3_protocol() {
        let json = br#"{"id":"abc","from":"inst-1","payload":{"type":"heartbeat","instance_id":"inst-1","timestamp":"2026-03-01T00:00:00Z"},"timestamp":"2026-03-01T00:00:00Z"}"#;
        assert_eq!(detect_protocol(json), PeerProtocol::V3);
    }

    #[test]
    fn test_detect_unknown_protocol() {
        let json = br#"{"random":"data"}"#;
        assert_eq!(detect_protocol(json), PeerProtocol::Unknown);
    }

    #[test]
    fn test_detect_invalid_json() {
        assert_eq!(detect_protocol(b"not json"), PeerProtocol::Unknown);
    }

    // ---- timestamp conversion ----

    #[test]
    fn test_unix_float_to_datetime() {
        let dt = unix_float_to_datetime(1709251234.567);
        assert_eq!(dt.timestamp(), 1709251234);
        assert!((dt.timestamp_subsec_nanos() as f64 - 567_000_000.0).abs() < 1000.0);
    }

    #[test]
    fn test_datetime_to_unix_float() {
        let dt = Utc.timestamp_opt(1709251234, 567_000_000).unwrap();
        let ts = datetime_to_unix_float(&dt);
        assert!((ts - 1709251234.567).abs() < 0.001);
    }

    #[test]
    fn test_timestamp_roundtrip() {
        let original = 1709251234.567;
        let dt = unix_float_to_datetime(original);
        let back = datetime_to_unix_float(&dt);
        assert!((back - original).abs() < 0.001);
    }

    // ---- V2 → V3 ----

    #[test]
    fn test_v2_heartbeat_to_v3() {
        let v2 = V2Message {
            id: "msg-1".to_string(),
            msg_type: "heartbeat".to_string(),
            source: "inst-v2".to_string(),
            timestamp: 1709251234.0,
            v: 1,
            visited: vec!["relay-a".to_string()],
            payload: Value::Object(Default::default()),
        };
        let v3 = v2_to_v3(&v2, "our-id").unwrap();
        assert_eq!(v3.from, "inst-v2");
        assert_eq!(v3.visited, vec!["relay-a"]);
        assert_eq!(v3.v, Some(1));
        assert!(matches!(v3.payload, SyncPayload::Heartbeat { .. }));
    }

    #[test]
    fn test_v2_account_switch_to_v3() {
        let mut payload = serde_json::Map::new();
        payload.insert("key".to_string(), Value::String("acc-1".to_string()));
        let v2 = V2Message {
            id: "msg-2".to_string(),
            msg_type: "account_switch".to_string(),
            source: "inst-v2".to_string(),
            timestamp: 1709251234.0,
            v: 1,
            visited: vec![],
            payload: Value::Object(payload),
        };
        let v3 = v2_to_v3(&v2, "our-id").unwrap();
        if let SyncPayload::AccountSwitch { new_key, .. } = &v3.payload {
            assert_eq!(new_key, "acc-1");
        } else {
            panic!("expected AccountSwitch");
        }
    }

    #[test]
    fn test_v2_force_switch_to_v3() {
        let mut payload = serde_json::Map::new();
        payload.insert("key".to_string(), Value::String("acc-2".to_string()));
        let v2 = V2Message {
            id: "msg-3".to_string(),
            msg_type: "force_switch".to_string(),
            source: "inst-v2".to_string(),
            timestamp: 1709251234.0,
            v: 1,
            visited: vec![],
            payload: Value::Object(payload),
        };
        let v3 = v2_to_v3(&v2, "our-id").unwrap();
        assert!(matches!(v3.payload, SyncPayload::AccountSwitch { .. }));
    }

    #[test]
    fn test_v2_quota_update_to_v3() {
        let mut payload = serde_json::Map::new();
        payload.insert("key".to_string(), Value::String("acc-1".to_string()));
        payload.insert("tokens_5h".to_string(), Value::Number(5000.into()));
        payload.insert("tokens_7d".to_string(), Value::Number(50000.into()));
        let v2 = V2Message {
            id: "msg-4".to_string(),
            msg_type: "quota_update".to_string(),
            source: "inst-v2".to_string(),
            timestamp: 1709251234.0,
            v: 1,
            visited: vec![],
            payload: Value::Object(payload),
        };
        let v3 = v2_to_v3(&v2, "our-id").unwrap();
        if let SyncPayload::QuotaUpdate { tokens_5h, tokens_7d, .. } = &v3.payload {
            assert_eq!(*tokens_5h, 5000);
            assert_eq!(*tokens_7d, 50000);
        } else {
            panic!("expected QuotaUpdate");
        }
    }

    #[test]
    fn test_v2_full_sync_to_v3() {
        let mut payload = serde_json::Map::new();
        payload.insert("accounts".to_string(), Value::Object(Default::default()));
        payload.insert("active_key".to_string(), Value::String("acc-1".to_string()));
        let v2 = V2Message {
            id: "msg-5".to_string(),
            msg_type: "full_sync".to_string(),
            source: "inst-v2".to_string(),
            timestamp: 1709251234.0,
            v: 1,
            visited: vec![],
            payload: Value::Object(payload),
        };
        let v3 = v2_to_v3(&v2, "our-id").unwrap();
        assert!(matches!(v3.payload, SyncPayload::Credentials { .. }));
    }

    #[test]
    fn test_v2_proxy_state_ignored() {
        let v2 = V2Message {
            id: "msg-6".to_string(),
            msg_type: "proxy_state".to_string(),
            source: "inst-v2".to_string(),
            timestamp: 1709251234.0,
            v: 1,
            visited: vec![],
            payload: Value::Object(Default::default()),
        };
        assert!(v2_to_v3(&v2, "our-id").is_none());
    }

    #[test]
    fn test_v2_unknown_type_returns_none() {
        let v2 = V2Message {
            id: "msg-7".to_string(),
            msg_type: "totally_unknown".to_string(),
            source: "inst-v2".to_string(),
            timestamp: 1709251234.0,
            v: 1,
            visited: vec![],
            payload: Value::Object(Default::default()),
        };
        assert!(v2_to_v3(&v2, "our-id").is_none());
    }

    // ---- V3 → V2 ----

    #[test]
    fn test_v3_heartbeat_to_v2() {
        let msg = SyncMessage::heartbeat("inst-v3");
        let v2 = v3_to_v2(&msg).unwrap();
        assert_eq!(v2.msg_type, "heartbeat");
        assert_eq!(v2.source, "inst-v3");
        assert!(v2.timestamp > 0.0);
    }

    #[test]
    fn test_v3_handshake_no_v2_equivalent() {
        let msg = SyncMessage::handshake_request("inst-v3", VectorClock::new(), 0);
        assert!(v3_to_v2(&msg).is_none());
    }

    // ---- parse_message ----

    #[test]
    fn test_parse_v3_message() {
        let msg = SyncMessage::heartbeat("inst-v3");
        let json = serde_json::to_vec(&msg).unwrap();
        let (parsed, proto) = parse_message(&json, "our-id").unwrap();
        assert_eq!(proto, PeerProtocol::V3);
        assert_eq!(parsed.from, "inst-v3");
    }

    #[test]
    fn test_parse_v2_message() {
        let json = br#"{"id":"abc","type":"heartbeat","source":"inst-v2","timestamp":1709251234.0,"v":1,"visited":[],"payload":{}}"#;
        let (parsed, proto) = parse_message(json, "our-id").unwrap();
        assert_eq!(proto, PeerProtocol::V2);
        assert_eq!(parsed.from, "inst-v2");
        assert!(matches!(parsed.payload, SyncPayload::Heartbeat { .. }));
    }

    // ---- serialize_for_protocol ----

    #[test]
    fn test_serialize_for_v2() {
        let msg = SyncMessage::heartbeat("inst-v3");
        let data = serialize_for_protocol(&msg, PeerProtocol::V2).unwrap();
        let v2: V2Message = serde_json::from_slice(&data).unwrap();
        assert_eq!(v2.msg_type, "heartbeat");
        assert_eq!(v2.source, "inst-v3");
    }

    #[test]
    fn test_serialize_for_v3() {
        let msg = SyncMessage::heartbeat("inst-v3");
        let data = serialize_for_protocol(&msg, PeerProtocol::V3).unwrap();
        let parsed: SyncMessage = serde_json::from_slice(&data).unwrap();
        assert_eq!(parsed.from, "inst-v3");
    }
}
