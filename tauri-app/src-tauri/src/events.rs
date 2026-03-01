//! Events push — émis depuis le backend vers le frontend Svelte.
//!
//! Utilise `tauri::AppHandle::emit()` pour les événements globaux.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tracing::debug;

/// Événement de mise à jour de quota.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QuotaUpdateEvent {
    pub key: String,
    pub quota: QuotaUpdatePayload,
}

/// Payload quota envoyé au frontend (matches QuotaInfo in types.ts).
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QuotaUpdatePayload {
    pub tokens5h: u64,
    pub limit5h: u64,
    pub tokens7d: u64,
    pub limit7d: u64,
    pub phase: Option<String>,
    pub ema_velocity: f64,
    pub time_to_threshold: Option<f64>,
    pub last_updated: Option<String>,
    pub resets_at_5h: Option<String>,
    pub resets_at_7d: Option<String>,
}

/// Événement toast (notification UI).
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ToastEvent {
    pub message: String,
    pub kind: ToastKind,
}

/// Type de toast.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum ToastKind {
    Info,
    Switch,
    Error,
}

/// Événement de changement de statut proxy.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatusEvent {
    pub proxy_type: String, // "router" | "impersonator"
    pub running: bool,
    pub port: u16,
}

/// Émet un événement de mise à jour quota vers le frontend.
pub fn emit_quota_update(app: &AppHandle, event: QuotaUpdateEvent) {
    debug!("Emitting quota_update for {}", event.key);
    let _ = app.emit("quota_update", event);
}

/// Émet un toast vers le frontend.
pub fn emit_toast(app: &AppHandle, message: impl Into<String>, kind: ToastKind) {
    let event = ToastEvent {
        message: message.into(),
        kind,
    };
    debug!("Emitting toast: {:?}", event.message);
    let _ = app.emit("toast", event);
}

/// Émet un changement de statut proxy.
pub fn emit_proxy_status(app: &AppHandle, event: ProxyStatusEvent) {
    debug!("Emitting proxy_status: {} running={}", event.proxy_type, event.running);
    let _ = app.emit("proxy_status", event);
}

/// Événement émis quand la phase quota d'un compte change.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PhaseTransitionEvent {
    pub key: String,
    pub previous_phase: String,
    pub new_phase: String,
    pub time_to_threshold: Option<f64>,
    pub usage_pct: f64,
}

/// Émet un événement de transition de phase.
pub fn emit_phase_transition(app: &AppHandle, event: PhaseTransitionEvent) {
    debug!("Emitting phase_transition: {} {} -> {}", event.key, event.previous_phase, event.new_phase);
    let _ = app.emit("phase_transition", event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toast_kind_serialize() {
        let kind = ToastKind::Switch;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"switch\"");
    }

    #[test]
    fn test_quota_update_serialize() {
        let event = QuotaUpdateEvent {
            key: "acc1".to_string(),
            quota: QuotaUpdatePayload {
                tokens5h: 5000,
                limit5h: 45_000_000,
                tokens7d: 20000,
                limit7d: 180_000_000,
                phase: Some("Cruise".to_string()),
                ema_velocity: 10.5,
                time_to_threshold: None,
                last_updated: None,
                resets_at_5h: None,
                resets_at_7d: None,
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("acc1"));
        assert!(json.contains("Cruise"));
    }
}
