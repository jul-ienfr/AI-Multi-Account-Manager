//! Types de messages P2P pour la synchronisation distribuée.
//!
//! Traduit la logique Python de `sync_coordinator.py` (SyncMessage, SyncPayload)
//! avec support des vector clocks pour la causalité distribuée.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Horloge vectorielle pour causalité distribuée.
///
/// Chaque instance possède une entrée `instance_id -> compteur`.
/// La règle LWW (Last-Write-Wins) : on prend le max de chaque entrée.
pub type VectorClock = HashMap<String, u64>;

/// Types de messages P2P échangés entre instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncPayload {
    /// Synchronise les credentials complets.
    Credentials {
        /// JSON des credentials (potentiellement chiffré côté applicatif)
        accounts_json: String,
        /// Clé du compte actif
        active_key: Option<String>,
        /// Horloge vectorielle de l'émetteur
        clock: VectorClock,
    },

    /// Notifie un changement de compte actif.
    AccountSwitch {
        new_key: String,
        clock: VectorClock,
    },

    /// Met à jour le quota d'un compte.
    QuotaUpdate {
        account_key: String,
        tokens_5h: u64,
        tokens_7d: u64,
        clock: VectorClock,
    },

    /// Heartbeat périodique (maintient la connexion et détecte les pairs morts).
    Heartbeat {
        instance_id: String,
        timestamp: DateTime<Utc>,
    },

    /// Demande une sync complète (envoyée au démarrage ou après reconnexion).
    SyncRequest {
        instance_id: String,
    },

    /// Réponse à une SyncRequest avec les credentials complets.
    SyncResponse {
        credentials_json: String,
        active_key: Option<String>,
        clock: VectorClock,
    },

    // ----------------------------------------------------------------
    // Pipeline 7 étapes (Phase 5.1)
    // ----------------------------------------------------------------

    /// Étape 1 — HANDSHAKE : annonce notre vector clock et le nombre de comptes.
    ///
    /// Permet au pair de déterminer s'il doit envoyer ou recevoir des données.
    HandshakeRequest {
        /// Vector clock de l'émetteur (instance_id → compteur).
        vector_clock: VectorClock,
        /// Nombre de comptes connus localement.
        account_count: usize,
    },

    /// Réponse au handshake : clock du pair + indication si une sync complète est nécessaire.
    HandshakeResponse {
        /// Vector clock du pair répondant.
        vector_clock: VectorClock,
        /// `true` si le pair a besoin de données (son clock est en retard sur au moins un nœud).
        needs_full_sync: bool,
    },

    /// Étape 2 — DIFF : envoie la liste des clés connues avec leur version (clock sum).
    ///
    /// Chaque clé est associée à une valeur de version : la somme des entrées du clock
    /// au moment de la dernière modification. Cela permet de détecter les comptes manquants
    /// ou obsolètes sans transférer les credentials complets.
    DiffRequest {
        /// `{account_key → version}` — version = somme des compteurs du clock au moment
        /// de la dernière écriture sur ce compte.
        keys_and_versions: HashMap<String, u64>,
    },

    /// Réponse au diff : liste des clés que le pair n'a pas ou dont la version est plus ancienne.
    DiffResponse {
        /// Clés que le pair n'a pas du tout.
        missing_keys: Vec<String>,
        /// Clés que le pair a, mais dont la version locale est plus ancienne que celle du demandeur.
        outdated_keys: Vec<String>,
    },

    /// Étape 7 — ACK : confirmation que le pipeline s'est terminé avec succès.
    ///
    /// Porte le vector clock mis à jour après merge, afin que le pair puisse synchroniser le sien.
    PipelineAck {
        /// Vector clock final après merge.
        merged_clock: VectorClock,
        /// Nombre de comptes importés lors du APPLY.
        accounts_applied: usize,
    },
}

// ----------------------------------------------------------------
// Structures nommées pour la lisibilité dans le pipeline
// (utilisées en interne par SyncPipeline — pas de tag serde)
// ----------------------------------------------------------------

/// Résumé de la phase HANDSHAKE, produit par `SyncPipeline::step_handshake`.
#[derive(Debug, Clone)]
pub struct HandshakeSummary {
    /// Vector clock reçu du pair.
    pub peer_clock: VectorClock,
    /// `true` si le pair a besoin de recevoir des données de notre part.
    pub peer_needs_our_data: bool,
    /// `true` si nous avons besoin de recevoir des données du pair.
    pub we_need_peer_data: bool,
}

/// Résumé de la phase DIFF, produit par `SyncPipeline::step_diff`.
#[derive(Debug, Clone, Default)]
pub struct DiffSummary {
    /// Clés que le pair n'a pas et que nous devons lui envoyer.
    pub keys_to_send: Vec<String>,
    /// Clés que nous n'avons pas et que nous devons demander.
    pub keys_to_request: Vec<String>,
}

/// Enveloppe d'un message P2P.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMessage {
    /// UUID unique du message
    pub id: String,
    /// instance_id de l'émetteur (V3: "from", V2: "source" — traduit par compat.rs)
    pub from: String,
    /// Contenu du message
    pub payload: SyncPayload,
    /// Timestamp d'émission (UTC)
    pub timestamp: DateTime<Utc>,
    /// Liste des nœuds déjà traversés (mesh relay V2). Absent en V3 natif.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visited: Vec<String>,
    /// Numéro de version du protocole (V2 envoie `v: 1`). Absent en V3 natif.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v: Option<u32>,
}

impl SyncMessage {
    /// Crée un nouveau message avec un UUID généré automatiquement.
    pub fn new(from: impl Into<String>, payload: SyncPayload) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from: from.into(),
            payload,
            timestamp: Utc::now(),
            visited: Vec::new(),
            v: None,
        }
    }

    /// Crée un message Heartbeat.
    pub fn heartbeat(instance_id: impl Into<String>) -> Self {
        let id = instance_id.into();
        Self::new(
            id.clone(),
            SyncPayload::Heartbeat {
                instance_id: id,
                timestamp: Utc::now(),
            },
        )
    }

    /// Crée une demande de sync complète.
    pub fn sync_request(instance_id: impl Into<String>) -> Self {
        let id = instance_id.into();
        Self::new(
            id.clone(),
            SyncPayload::SyncRequest {
                instance_id: id,
            },
        )
    }

    /// Crée un HandshakeRequest (étape 1 du pipeline).
    pub fn handshake_request(
        from: impl Into<String>,
        vector_clock: VectorClock,
        account_count: usize,
    ) -> Self {
        Self::new(
            from,
            SyncPayload::HandshakeRequest {
                vector_clock,
                account_count,
            },
        )
    }

    /// Crée un HandshakeResponse (réponse à l'étape 1).
    pub fn handshake_response(
        from: impl Into<String>,
        vector_clock: VectorClock,
        needs_full_sync: bool,
    ) -> Self {
        Self::new(
            from,
            SyncPayload::HandshakeResponse {
                vector_clock,
                needs_full_sync,
            },
        )
    }

    /// Crée un DiffRequest (étape 2 du pipeline).
    pub fn diff_request(
        from: impl Into<String>,
        keys_and_versions: HashMap<String, u64>,
    ) -> Self {
        Self::new(from, SyncPayload::DiffRequest { keys_and_versions })
    }

    /// Crée un DiffResponse (réponse à l'étape 2).
    pub fn diff_response(
        from: impl Into<String>,
        missing_keys: Vec<String>,
        outdated_keys: Vec<String>,
    ) -> Self {
        Self::new(
            from,
            SyncPayload::DiffResponse {
                missing_keys,
                outdated_keys,
            },
        )
    }

    /// Crée un PipelineAck (étape 7 du pipeline).
    pub fn pipeline_ack(
        from: impl Into<String>,
        merged_clock: VectorClock,
        accounts_applied: usize,
    ) -> Self {
        Self::new(
            from,
            SyncPayload::PipelineAck {
                merged_clock,
                accounts_applied,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----------------------------------------------------------------
    // Tests de sérialisation / désérialisation SyncMessage
    // ----------------------------------------------------------------

    #[test]
    fn test_serialize_heartbeat() {
        let msg = SyncMessage::heartbeat("instance-abc");
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("heartbeat"));
        assert!(json.contains("instance-abc"));
    }

    #[test]
    fn test_deserialize_heartbeat() {
        let msg = SyncMessage::heartbeat("inst-123");
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: SyncMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.from, "inst-123");
        assert!(matches!(decoded.payload, SyncPayload::Heartbeat { .. }));
    }

    #[test]
    fn test_serialize_credentials() {
        let mut clock = VectorClock::new();
        clock.insert("inst-1".to_string(), 3);
        clock.insert("inst-2".to_string(), 1);

        let payload = SyncPayload::Credentials {
            accounts_json: r#"{"accounts":{}}"#.to_string(),
            active_key: Some("acc1".to_string()),
            clock,
        };
        let msg = SyncMessage::new("inst-1", payload);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: SyncMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.from, "inst-1");
        if let SyncPayload::Credentials { active_key, clock, .. } = decoded.payload {
            assert_eq!(active_key, Some("acc1".to_string()));
            assert_eq!(*clock.get("inst-1").unwrap(), 3);
        } else {
            panic!("wrong payload type");
        }
    }

    #[test]
    fn test_serialize_account_switch() {
        let mut clock = VectorClock::new();
        clock.insert("inst-1".to_string(), 1);
        let payload = SyncPayload::AccountSwitch {
            new_key: "acc2".to_string(),
            clock,
        };
        let msg = SyncMessage::new("inst-1", payload);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: SyncMessage = serde_json::from_str(&json).unwrap();
        if let SyncPayload::AccountSwitch { new_key, .. } = decoded.payload {
            assert_eq!(new_key, "acc2");
        } else {
            panic!("wrong payload type");
        }
    }

    #[test]
    fn test_serialize_quota_update() {
        let mut clock = VectorClock::new();
        clock.insert("inst-1".to_string(), 5);
        let payload = SyncPayload::QuotaUpdate {
            account_key: "acc1".to_string(),
            tokens_5h: 12000,
            tokens_7d: 80000,
            clock,
        };
        let msg = SyncMessage::new("inst-1", payload);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: SyncMessage = serde_json::from_str(&json).unwrap();
        if let SyncPayload::QuotaUpdate { tokens_5h, tokens_7d, .. } = decoded.payload {
            assert_eq!(tokens_5h, 12000);
            assert_eq!(tokens_7d, 80000);
        } else {
            panic!("wrong payload type");
        }
    }

    #[test]
    fn test_serialize_sync_request() {
        let msg = SyncMessage::sync_request("inst-xyz");
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: SyncMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(decoded.payload, SyncPayload::SyncRequest { .. }));
    }

    #[test]
    fn test_serialize_sync_response() {
        let mut clock = VectorClock::new();
        clock.insert("inst-1".to_string(), 2);
        let payload = SyncPayload::SyncResponse {
            credentials_json: r#"{"accounts":{}}"#.to_string(),
            active_key: None,
            clock,
        };
        let msg = SyncMessage::new("inst-1", payload);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("sync_response"));
    }

    #[test]
    fn test_message_has_unique_ids() {
        let msg1 = SyncMessage::heartbeat("inst-1");
        let msg2 = SyncMessage::heartbeat("inst-1");
        assert_ne!(msg1.id, msg2.id, "UUIDs should be unique");
    }

    #[test]
    fn test_message_timestamp_is_recent() {
        let before = Utc::now();
        let msg = SyncMessage::heartbeat("inst-1");
        let after = Utc::now();
        assert!(msg.timestamp >= before);
        assert!(msg.timestamp <= after);
    }

    #[test]
    fn test_vector_clock_empty() {
        let clock = VectorClock::new();
        let payload = SyncPayload::Credentials {
            accounts_json: "{}".to_string(),
            active_key: None,
            clock,
        };
        let msg = SyncMessage::new("inst-1", payload);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: SyncMessage = serde_json::from_str(&json).unwrap();
        if let SyncPayload::Credentials { clock, .. } = decoded.payload {
            assert!(clock.is_empty());
        }
    }

    #[test]
    fn test_roundtrip_all_types() {
        let messages = vec![
            SyncMessage::heartbeat("inst-a"),
            SyncMessage::sync_request("inst-b"),
        ];
        for msg in messages {
            let json = serde_json::to_string(&msg).unwrap();
            let decoded: SyncMessage = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded.from, msg.from);
            assert_eq!(decoded.id, msg.id);
        }
    }

    // ----------------------------------------------------------------
    // Tests V2 compat fields (visited, v)
    // ----------------------------------------------------------------

    #[test]
    fn test_v2_compat_fields_absent_by_default() {
        let msg = SyncMessage::heartbeat("inst-1");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("\"visited\""), "visited should be skipped when empty");
        assert!(!json.contains("\"v\""), "v should be skipped when None");
    }

    #[test]
    fn test_visited_field_roundtrip() {
        let mut msg = SyncMessage::heartbeat("inst-1");
        msg.visited = vec!["peer-a".to_string(), "peer-b".to_string()];
        msg.v = Some(1);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"visited\""));
        assert!(json.contains("\"v\":1"));
        let decoded: SyncMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.visited, vec!["peer-a", "peer-b"]);
        assert_eq!(decoded.v, Some(1));
    }

    #[test]
    fn test_deserialize_v2_message_with_visited() {
        // Simule un JSON V3-like mais avec des champs V2 ajoutés
        let json = r#"{
            "id": "test-uuid",
            "from": "inst-v2",
            "payload": {"type": "heartbeat", "instance_id": "inst-v2", "timestamp": "2026-03-01T00:00:00Z"},
            "timestamp": "2026-03-01T00:00:00Z",
            "visited": ["inst-relay"],
            "v": 1
        }"#;
        let msg: SyncMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.from, "inst-v2");
        assert_eq!(msg.visited, vec!["inst-relay"]);
        assert_eq!(msg.v, Some(1));
    }
}
