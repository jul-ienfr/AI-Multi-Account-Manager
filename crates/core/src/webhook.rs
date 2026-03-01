//! Webhooks Discord/Slack/Generic — notifications d'événements système.
//!
//! Supporte l'envoi de messages vers Discord, Slack ou tout endpoint HTTP
//! JSON générique lors d'événements importants (quota critique, auto-switch,
//! token révoqué, transition de phase).

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Types publics
// ---------------------------------------------------------------------------

/// Type de webhook supporté.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WebhookKind {
    Discord,
    Slack,
    /// POST JSON générique : `{"event": "...", "data": {...}}`.
    Generic,
}

/// Cible webhook (URL + kind + filtre d'événements).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookTarget {
    pub url: String,
    pub kind: WebhookKind,
    /// Liste des noms d'événements à envoyer.
    /// Valeurs valides : "quota_warning", "auto_switch", "token_revoked", "phase_transition".
    /// Si vide, tous les événements sont envoyés.
    #[serde(default)]
    pub events: Vec<String>,
}

/// Événement déclencheur d'une notification webhook.
#[derive(Debug, Clone)]
pub enum WebhookEvent {
    QuotaWarning { key: String, pct: f64, phase: String },
    AutoSwitch   { from: String, to: String, reason: String },
    TokenRevoked { key: String },
    PhaseTransition { key: String, from: String, to: String },
}

impl WebhookEvent {
    /// Retourne le nom de l'événement (utilisé pour le filtrage).
    pub fn name(&self) -> &'static str {
        match self {
            WebhookEvent::QuotaWarning { .. }    => "quota_warning",
            WebhookEvent::AutoSwitch { .. }      => "auto_switch",
            WebhookEvent::TokenRevoked { .. }    => "token_revoked",
            WebhookEvent::PhaseTransition { .. } => "phase_transition",
        }
    }

    /// Formate le message texte destiné à Discord/Slack.
    pub fn format_message(&self) -> String {
        match self {
            WebhookEvent::QuotaWarning { key, pct, phase } =>
                format!("⚠️ Compte {} à {:.1}% de quota (phase: {})", key, pct, phase),
            WebhookEvent::AutoSwitch { from, to, reason } =>
                format!("🔄 Switch automatique: {} → {} ({})", from, to, reason),
            WebhookEvent::TokenRevoked { key } =>
                format!("🔴 Token révoqué: {}", key),
            WebhookEvent::PhaseTransition { key, from, to } =>
                format!("📊 {}: {} → {}", key, from, to),
        }
    }

    /// Construit le body JSON pour un webhook Generic.
    pub fn build_generic_payload(&self) -> serde_json::Value {
        match self {
            WebhookEvent::QuotaWarning { key, pct, phase } => serde_json::json!({
                "event": "quota_warning",
                "data": { "key": key, "pct": pct, "phase": phase }
            }),
            WebhookEvent::AutoSwitch { from, to, reason } => serde_json::json!({
                "event": "auto_switch",
                "data": { "from": from, "to": to, "reason": reason }
            }),
            WebhookEvent::TokenRevoked { key } => serde_json::json!({
                "event": "token_revoked",
                "data": { "key": key }
            }),
            WebhookEvent::PhaseTransition { key, from, to } => serde_json::json!({
                "event": "phase_transition",
                "data": { "key": key, "from": from, "to": to }
            }),
        }
    }

    /// Construit le payload Discord : `{"content": "..."}`.
    pub fn build_discord_payload(&self) -> serde_json::Value {
        serde_json::json!({ "content": self.format_message() })
    }

    /// Construit le payload Slack : `{"text": "..."}`.
    pub fn build_slack_payload(&self) -> serde_json::Value {
        serde_json::json!({ "text": self.format_message() })
    }

    /// Construit le payload selon le kind de webhook.
    pub fn build_payload(&self, kind: &WebhookKind) -> serde_json::Value {
        match kind {
            WebhookKind::Discord => self.build_discord_payload(),
            WebhookKind::Slack   => self.build_slack_payload(),
            WebhookKind::Generic => self.build_generic_payload(),
        }
    }
}

// ---------------------------------------------------------------------------
// WebhookSender
// ---------------------------------------------------------------------------

/// Envoie des notifications webhook vers une liste de cibles configurées.
pub struct WebhookSender {
    client: reqwest::Client,
    urls: Vec<WebhookTarget>,
}

impl WebhookSender {
    /// Crée un sender à partir d'une liste de cibles.
    /// Construit un client reqwest partagé (timeout 10s).
    pub fn new(urls: Vec<WebhookTarget>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self { client, urls }
    }

    /// Envoie l'événement à toutes les cibles qui l'ont filtré.
    /// Les erreurs HTTP sont loggées mais n'interrompent pas les envois suivants.
    pub async fn send(&self, event: WebhookEvent) {
        let event_name = event.name();
        for target in &self.urls {
            // Filtrage : si events non vide, vérifier que l'événement est inclus
            if !target.events.is_empty() && !target.events.iter().any(|e| e == event_name) {
                debug!("Webhook {:?} skipped event '{}' (not in filter)", target.kind, event_name);
                continue;
            }

            let payload = event.build_payload(&target.kind);
            debug!("Sending webhook '{}' to {} ({:?})", event_name, target.url, target.kind);

            match self.client
                .post(&target.url)
                .json(&payload)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        warn!(
                            "Webhook '{}' returned HTTP {} for url={}",
                            event_name, status.as_u16(), target.url
                        );
                    } else {
                        debug!("Webhook '{}' sent successfully (HTTP {})", event_name, status.as_u16());
                    }
                }
                Err(e) => {
                    warn!("Webhook '{}' network error for url={}: {}", event_name, target.url, e);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- format_message tests ------------------------------------------------

    #[test]
    fn test_format_quota_warning() {
        let ev = WebhookEvent::QuotaWarning {
            key: "alice@example.com".to_string(),
            pct: 91.5,
            phase: "Critical".to_string(),
        };
        let msg = ev.format_message();
        assert!(msg.contains("alice@example.com"), "should contain account key");
        assert!(msg.contains("91.5"), "should contain percentage");
        assert!(msg.contains("Critical"), "should contain phase");
        assert!(msg.contains("⚠️"), "should contain warning emoji");
    }

    #[test]
    fn test_format_auto_switch() {
        let ev = WebhookEvent::AutoSwitch {
            from: "acc1".to_string(),
            to: "acc2".to_string(),
            reason: "quota_5h".to_string(),
        };
        let msg = ev.format_message();
        assert!(msg.contains("acc1"), "should contain from account");
        assert!(msg.contains("acc2"), "should contain to account");
        assert!(msg.contains("quota_5h"), "should contain reason");
        assert!(msg.contains("🔄"), "should contain switch emoji");
    }

    #[test]
    fn test_format_token_revoked() {
        let ev = WebhookEvent::TokenRevoked {
            key: "revoked@example.com".to_string(),
        };
        let msg = ev.format_message();
        assert!(msg.contains("revoked@example.com"), "should contain key");
        assert!(msg.contains("🔴"), "should contain red emoji");
    }

    #[test]
    fn test_format_phase_transition() {
        let ev = WebhookEvent::PhaseTransition {
            key: "acc1".to_string(),
            from: "Cruise".to_string(),
            to: "Warning".to_string(),
        };
        let msg = ev.format_message();
        assert!(msg.contains("acc1"), "should contain key");
        assert!(msg.contains("Cruise"), "should contain from phase");
        assert!(msg.contains("Warning"), "should contain to phase");
        assert!(msg.contains("📊"), "should contain chart emoji");
    }

    // -- Discord payload tests -----------------------------------------------

    #[test]
    fn test_discord_payload_structure() {
        let ev = WebhookEvent::QuotaWarning {
            key: "acc".to_string(),
            pct: 95.0,
            phase: "Critical".to_string(),
        };
        let payload = ev.build_discord_payload();
        assert!(payload.get("content").is_some(), "Discord payload must have 'content' key");
        let content = payload["content"].as_str().unwrap();
        assert!(content.contains("acc"), "content must reference the account");
        // Must NOT have Slack-specific keys
        assert!(payload.get("text").is_none(), "Discord payload must not have 'text' key");
    }

    // -- Slack payload tests -------------------------------------------------

    #[test]
    fn test_slack_payload_structure() {
        let ev = WebhookEvent::AutoSwitch {
            from: "a".to_string(),
            to: "b".to_string(),
            reason: "test".to_string(),
        };
        let payload = ev.build_slack_payload();
        assert!(payload.get("text").is_some(), "Slack payload must have 'text' key");
        let text = payload["text"].as_str().unwrap();
        assert!(text.contains("a") && text.contains("b"), "text must reference accounts");
        // Must NOT have Discord-specific keys
        assert!(payload.get("content").is_none(), "Slack payload must not have 'content' key");
    }

    // -- Generic payload tests -----------------------------------------------

    #[test]
    fn test_generic_payload_quota_warning() {
        let ev = WebhookEvent::QuotaWarning {
            key: "alice".to_string(),
            pct: 88.0,
            phase: "Warning".to_string(),
        };
        let payload = ev.build_generic_payload();
        assert_eq!(payload["event"].as_str(), Some("quota_warning"));
        let data = &payload["data"];
        assert_eq!(data["key"].as_str(), Some("alice"));
        let pct = data["pct"].as_f64().unwrap();
        assert!((pct - 88.0).abs() < 1e-9, "pct should be 88.0, got {}", pct);
        assert_eq!(data["phase"].as_str(), Some("Warning"));
    }

    #[test]
    fn test_generic_payload_auto_switch() {
        let ev = WebhookEvent::AutoSwitch {
            from: "acc1".to_string(),
            to: "acc2".to_string(),
            reason: "quota_5h".to_string(),
        };
        let payload = ev.build_generic_payload();
        assert_eq!(payload["event"].as_str(), Some("auto_switch"));
        assert_eq!(payload["data"]["from"].as_str(), Some("acc1"));
        assert_eq!(payload["data"]["to"].as_str(), Some("acc2"));
        assert_eq!(payload["data"]["reason"].as_str(), Some("quota_5h"));
    }

    #[test]
    fn test_generic_payload_token_revoked() {
        let ev = WebhookEvent::TokenRevoked {
            key: "revo@example.com".to_string(),
        };
        let payload = ev.build_generic_payload();
        assert_eq!(payload["event"].as_str(), Some("token_revoked"));
        assert_eq!(payload["data"]["key"].as_str(), Some("revo@example.com"));
    }

    #[test]
    fn test_generic_payload_phase_transition() {
        let ev = WebhookEvent::PhaseTransition {
            key: "acc1".to_string(),
            from: "Cruise".to_string(),
            to: "Critical".to_string(),
        };
        let payload = ev.build_generic_payload();
        assert_eq!(payload["event"].as_str(), Some("phase_transition"));
        assert_eq!(payload["data"]["key"].as_str(), Some("acc1"));
        assert_eq!(payload["data"]["from"].as_str(), Some("Cruise"));
        assert_eq!(payload["data"]["to"].as_str(), Some("Critical"));
    }

    // -- build_payload dispatch test -----------------------------------------

    #[test]
    fn test_build_payload_dispatch() {
        let ev = WebhookEvent::TokenRevoked { key: "x".to_string() };
        // Discord
        let d = ev.build_payload(&WebhookKind::Discord);
        assert!(d.get("content").is_some());
        // Slack
        let s = ev.build_payload(&WebhookKind::Slack);
        assert!(s.get("text").is_some());
        // Generic
        let g = ev.build_payload(&WebhookKind::Generic);
        assert_eq!(g["event"].as_str(), Some("token_revoked"));
    }

    // -- event name tests ----------------------------------------------------

    #[test]
    fn test_event_names() {
        assert_eq!(WebhookEvent::QuotaWarning { key: "k".into(), pct: 0.0, phase: "p".into() }.name(), "quota_warning");
        assert_eq!(WebhookEvent::AutoSwitch { from: "a".into(), to: "b".into(), reason: "r".into() }.name(), "auto_switch");
        assert_eq!(WebhookEvent::TokenRevoked { key: "k".into() }.name(), "token_revoked");
        assert_eq!(WebhookEvent::PhaseTransition { key: "k".into(), from: "a".into(), to: "b".into() }.name(), "phase_transition");
    }

    // -- WebhookSender construction test -------------------------------------

    #[test]
    fn test_sender_new_empty() {
        let sender = WebhookSender::new(vec![]);
        assert!(sender.urls.is_empty());
    }

    #[test]
    fn test_sender_new_with_targets() {
        let targets = vec![
            WebhookTarget {
                url: "https://discord.com/api/webhooks/test".to_string(),
                kind: WebhookKind::Discord,
                events: vec!["quota_warning".to_string()],
            },
            WebhookTarget {
                url: "https://hooks.slack.com/services/test".to_string(),
                kind: WebhookKind::Slack,
                events: vec![],
            },
        ];
        let sender = WebhookSender::new(targets);
        assert_eq!(sender.urls.len(), 2);
        assert_eq!(sender.urls[0].kind, WebhookKind::Discord);
        assert_eq!(sender.urls[1].kind, WebhookKind::Slack);
    }

    // -- WebhookKind serialization test -------------------------------------

    #[test]
    fn test_webhook_kind_serde() {
        let json = serde_json::to_string(&WebhookKind::Discord).unwrap();
        assert_eq!(json, "\"discord\"");
        let json = serde_json::to_string(&WebhookKind::Slack).unwrap();
        assert_eq!(json, "\"slack\"");
        let json = serde_json::to_string(&WebhookKind::Generic).unwrap();
        assert_eq!(json, "\"generic\"");

        let kind: WebhookKind = serde_json::from_str("\"discord\"").unwrap();
        assert_eq!(kind, WebhookKind::Discord);
    }

    // -- WebhookTarget default events test -----------------------------------

    #[test]
    fn test_target_default_events_empty() {
        let json = r#"{"url":"https://example.com","kind":"generic"}"#;
        let target: WebhookTarget = serde_json::from_str(json).unwrap();
        assert!(target.events.is_empty(), "events should default to empty vec");
    }

    // -- Pct formatting precision test --------------------------------------

    #[test]
    fn test_quota_warning_pct_format() {
        // Verify that {:.1} rounds correctly
        let ev = WebhookEvent::QuotaWarning {
            key: "k".to_string(),
            pct: 91.456,
            phase: "Critical".to_string(),
        };
        let msg = ev.format_message();
        // {:.1} of 91.456 = "91.5"
        assert!(msg.contains("91.5"), "expected '91.5' in '{}' ", msg);
    }
}
