//! Calcul EMA de la vélocité des tokens et phases quota.
//!
//! Traduit `velocity_controller.py` (VelocityController, EMA, TTT, phases)
//! en Rust pur (pas de dépendances externes).
//!
//! # Algorithme
//!
//! 1. À chaque mesure, on calcule la vélocité instantanée (tokens/min).
//! 2. On applique un EMA (Exponential Moving Average) pour lisser.
//! 3. On calcule le TTT (Time To Threshold) = quota_restant / ema_velocity.
//! 4. La phase est déterminée par le TTT.

use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::types::QuotaPhase;

/// Paramètre EMA (alpha) — lissage de la vélocité.
/// Plus alpha est élevé, plus l'EMA réagit vite aux variations.
const EMA_ALPHA: f64 = 0.3;

/// Seuil TTT (minutes) pour chaque phase.
const TTT_CRUISE_MIN: f64 = 20.0;
const TTT_WATCH_MIN: f64 = 5.0;
const TTT_ALERT_MIN: f64 = 2.0;

/// Limite max de tokens consommés sur 5h (si non configuré).
pub const DEFAULT_QUOTA_5H: u64 = 90_000;

/// Calculateur de vélocité EMA et phase quota.
///
/// Maintient l'EMA de la vélocité en tokens/min et détermine la phase
/// actuelle (Cruise/Watch/Alert/Critical) selon le TTT.
#[derive(Debug, Clone)]
pub struct VelocityCalculator {
    /// Vélocité EMA actuelle (tokens/min).
    ema_velocity: f64,
    /// Dernier timestamp de mesure.
    last_measurement: Option<Instant>,
    /// Dernier nombre de tokens observé.
    last_tokens: u64,
    /// Quota limite 5h.
    quota_limit_5h: u64,
}

impl VelocityCalculator {
    /// Crée un nouveau calculateur.
    pub fn new(quota_limit_5h: u64) -> Self {
        Self {
            ema_velocity: 0.0,
            last_measurement: None,
            last_tokens: 0,
            quota_limit_5h,
        }
    }

    /// Met à jour la vélocité avec une nouvelle mesure de tokens.
    ///
    /// Retourne la nouvelle vélocité EMA.
    pub fn update(&mut self, tokens_consumed: u64) -> f64 {
        let now = Instant::now();

        if let Some(last_time) = self.last_measurement {
            let elapsed_mins = now.duration_since(last_time).as_secs_f64() / 60.0;
            if elapsed_mins > 0.001 {
                // Tokens ajoutés depuis la dernière mesure
                let new_tokens = tokens_consumed.saturating_sub(self.last_tokens);
                let instant_velocity = new_tokens as f64 / elapsed_mins;

                // EMA
                if self.ema_velocity == 0.0 {
                    self.ema_velocity = instant_velocity;
                } else {
                    self.ema_velocity =
                        EMA_ALPHA * instant_velocity + (1.0 - EMA_ALPHA) * self.ema_velocity;
                }

                debug!(
                    "VelocityCalc: tokens={} elapsed_min={:.2} instant={:.1} ema={:.1}",
                    tokens_consumed, elapsed_mins, instant_velocity, self.ema_velocity
                );
            }
        }

        self.last_measurement = Some(now);
        self.last_tokens = tokens_consumed;
        self.ema_velocity
    }

    /// Calcule le TTT (Time To Threshold) en minutes.
    ///
    /// Retourne `None` si la vélocité est nulle (pas de TTT calculable).
    pub fn time_to_threshold(&self, tokens_consumed: u64) -> Option<f64> {
        if self.ema_velocity <= 0.0 {
            return None;
        }
        let remaining = self.quota_limit_5h.saturating_sub(tokens_consumed) as f64;
        Some(remaining / self.ema_velocity)
    }

    /// Détermine la phase quota selon le TTT et le quota consommé.
    pub fn phase(&self, tokens_consumed: u64) -> QuotaPhase {
        // Critical si quota >= 95% ou TTT < ALERT threshold
        let usage_pct = tokens_consumed as f64 / self.quota_limit_5h.max(1) as f64;
        if usage_pct >= 0.95 {
            return QuotaPhase::Critical;
        }

        let ttt = match self.time_to_threshold(tokens_consumed) {
            None => return QuotaPhase::Cruise, // Pas de vélocité → OK
            Some(t) => t,
        };

        if ttt < TTT_ALERT_MIN {
            QuotaPhase::Critical
        } else if ttt < TTT_WATCH_MIN {
            QuotaPhase::Alert
        } else if ttt < TTT_CRUISE_MIN {
            QuotaPhase::Watch
        } else {
            QuotaPhase::Cruise
        }
    }

    /// Vélocité EMA actuelle (tokens/min).
    pub fn ema_velocity(&self) -> f64 {
        self.ema_velocity
    }

    /// Réinitialise le calculateur (ex: nouveau cycle 5h).
    pub fn reset(&mut self) {
        self.ema_velocity = 0.0;
        self.last_measurement = None;
        self.last_tokens = 0;
    }

    /// Restaure l'EMA depuis un VelocityState sauvegardé.
    pub fn restore_from_state(&mut self, state: &VelocityState) {
        self.ema_velocity = state.ema_velocity;
        self.last_tokens = state.last_tokens;
        // last_measurement reste None (Instant non sérialisable)
    }

    /// Exporte l'état courant pour persistance.
    pub fn to_state(&self) -> VelocityState {
        VelocityState {
            ema_velocity: self.ema_velocity,
            last_tokens: self.last_tokens,
            quota_limit_5h: self.quota_limit_5h,
            saved_at: Utc::now().to_rfc3339(),
        }
    }
}

/// État sérialisable d'un VelocityCalculator (sans Instant — non sérialisable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VelocityState {
    pub ema_velocity: f64,
    pub last_tokens: u64,
    pub quota_limit_5h: u64,
    pub saved_at: String, // ISO8601
}

/// Charge les états de vélocité depuis `{multi_account_dir}/velocity-state.json`.
///
/// Les états de plus de 1800 s (30 min) sont ignorés (trop vieux).
/// Retourne une `HashMap` vide si le fichier est absent ou illisible.
pub fn load_velocity_states(
    multi_account_dir: &std::path::Path,
) -> HashMap<String, VelocityState> {
    let path = multi_account_dir.join("velocity-state.json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return HashMap::new(),
    };
    let all: HashMap<String, VelocityState> = match serde_json::from_slice(&bytes) {
        Ok(m) => m,
        Err(e) => {
            warn!("velocity-state.json parse error: {e}");
            return HashMap::new();
        }
    };

    let now = Utc::now();
    all.into_iter()
        .filter(|(_, state)| {
            match chrono::DateTime::parse_from_rfc3339(&state.saved_at) {
                Ok(saved) => {
                    let age = now.signed_duration_since(saved.with_timezone(&Utc));
                    age.num_seconds() <= 1800
                }
                Err(_) => false,
            }
        })
        .collect()
}

/// Sauvegarde les états de vélocité dans `{multi_account_dir}/velocity-state.json`.
///
/// Écriture atomique via fichier `.tmp` + rename.
/// Les erreurs sont loggées via `tracing::warn!` sans paniquer.
pub fn save_velocity_states(
    multi_account_dir: &std::path::Path,
    states: &HashMap<String, VelocityState>,
) {
    let json = match serde_json::to_string_pretty(states) {
        Ok(j) => j,
        Err(e) => {
            warn!("velocity-state.json serialize error: {e}");
            return;
        }
    };

    let tmp_path = multi_account_dir.join("velocity-state.json.tmp");
    let final_path = multi_account_dir.join("velocity-state.json");

    if let Err(e) = std::fs::write(&tmp_path, &json) {
        warn!("velocity-state.json write tmp error: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, &final_path) {
        warn!("velocity-state.json rename error: {e}");
    }
}

/// Résumé des métriques quota pour un compte.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaMetrics {
    pub tokens_5h: u64,
    pub limit_5h: u64,
    pub ema_velocity: f64,
    pub time_to_threshold_mins: Option<f64>,
    pub phase: QuotaPhase,
    pub usage_pct: f64,
}

impl QuotaMetrics {
    /// Calcule les métriques depuis un calculateur.
    pub fn compute(calc: &VelocityCalculator, tokens_5h: u64) -> Self {
        let limit_5h = calc.quota_limit_5h;
        let ema_velocity = calc.ema_velocity();
        let ttt = calc.time_to_threshold(tokens_5h);
        let phase = calc.phase(tokens_5h);
        let usage_pct = tokens_5h as f64 / limit_5h.max(1) as f64 * 100.0;

        QuotaMetrics {
            tokens_5h,
            limit_5h,
            ema_velocity,
            time_to_threshold_mins: ttt,
            phase,
            usage_pct,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_calculator() {
        let calc = VelocityCalculator::new(90_000);
        assert_eq!(calc.ema_velocity(), 0.0);
    }

    #[test]
    fn test_first_update_no_velocity() {
        let mut calc = VelocityCalculator::new(90_000);
        let v = calc.update(1000);
        // Premier appel → pas de delta → velocity = 0
        assert_eq!(v, 0.0);
    }

    #[test]
    fn test_phase_cruise_no_velocity() {
        let calc = VelocityCalculator::new(90_000);
        assert_eq!(calc.phase(1000), QuotaPhase::Cruise);
    }

    #[test]
    fn test_phase_critical_quota_full() {
        let calc = VelocityCalculator::new(90_000);
        // 96% utilisé → Critical
        assert_eq!(calc.phase(86_400), QuotaPhase::Critical);
    }

    #[test]
    fn test_ttt_none_when_velocity_zero() {
        let calc = VelocityCalculator::new(90_000);
        assert!(calc.time_to_threshold(1000).is_none());
    }

    #[test]
    fn test_ttt_calculation() {
        let mut calc = VelocityCalculator::new(90_000);
        // Simuler une vélocité de 1000 tokens/min
        calc.ema_velocity = 1000.0;
        // 80_000 tokens consommés, 10_000 restants → TTT = 10 min
        let ttt = calc.time_to_threshold(80_000).unwrap();
        assert!((ttt - 10.0).abs() < 0.1);
    }

    #[test]
    fn test_phase_from_ttt() {
        let mut calc = VelocityCalculator::new(90_000);
        // Vélocité élevée : TTT < 2 min → Critical
        calc.ema_velocity = 60_000.0;
        assert_eq!(calc.phase(0), QuotaPhase::Critical);

        // TTT modéré : Watch
        calc.ema_velocity = 5_000.0; // 90_000 / 5_000 = 18 min → Watch
        assert_eq!(calc.phase(0), QuotaPhase::Watch);

        // TTT long : Cruise
        calc.ema_velocity = 100.0; // 90_000 / 100 = 900 min → Cruise
        assert_eq!(calc.phase(0), QuotaPhase::Cruise);
    }

    #[test]
    fn test_reset() {
        let mut calc = VelocityCalculator::new(90_000);
        calc.ema_velocity = 500.0;
        calc.reset();
        assert_eq!(calc.ema_velocity(), 0.0);
    }

    #[test]
    fn test_quota_metrics_compute() {
        let mut calc = VelocityCalculator::new(90_000);
        calc.ema_velocity = 1000.0;
        let metrics = QuotaMetrics::compute(&calc, 45_000);
        assert_eq!(metrics.tokens_5h, 45_000);
        assert_eq!(metrics.limit_5h, 90_000);
        assert!((metrics.usage_pct - 50.0).abs() < 0.01);
        assert!(metrics.time_to_threshold_mins.is_some());
    }

    // ── Tests persistance ──────────────────────────────────────────────────────

    fn make_state(ema: f64, tokens: u64, saved_at: &str) -> VelocityState {
        VelocityState {
            ema_velocity: ema,
            last_tokens: tokens,
            quota_limit_5h: 90_000,
            saved_at: saved_at.to_string(),
        }
    }

    #[test]
    fn test_to_state_round_trip() {
        let mut calc = VelocityCalculator::new(90_000);
        calc.ema_velocity = 42.5;
        calc.last_tokens = 12_000;

        let state = calc.to_state();
        assert!((state.ema_velocity - 42.5).abs() < f64::EPSILON);
        assert_eq!(state.last_tokens, 12_000);
        assert_eq!(state.quota_limit_5h, 90_000);
        // saved_at doit être parsable en RFC3339
        assert!(chrono::DateTime::parse_from_rfc3339(&state.saved_at).is_ok());
    }

    #[test]
    fn test_restore_from_state() {
        let mut calc = VelocityCalculator::new(90_000);
        let state = make_state(77.0, 5_000, &Utc::now().to_rfc3339());
        calc.restore_from_state(&state);
        assert!((calc.ema_velocity() - 77.0).abs() < f64::EPSILON);
        assert_eq!(calc.last_tokens, 5_000);
        // last_measurement doit rester None (Instant non sérialisable)
        assert!(calc.last_measurement.is_none());
    }

    #[test]
    fn test_save_and_load_velocity_states() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        let mut states: HashMap<String, VelocityState> = HashMap::new();
        states.insert(
            "acc1".to_string(),
            make_state(10.0, 1_000, &Utc::now().to_rfc3339()),
        );
        states.insert(
            "acc2".to_string(),
            make_state(20.0, 2_000, &Utc::now().to_rfc3339()),
        );

        save_velocity_states(path, &states);

        // Le fichier doit exister
        assert!(path.join("velocity-state.json").exists());

        let loaded = load_velocity_states(path);
        assert_eq!(loaded.len(), 2);
        let s1 = loaded.get("acc1").unwrap();
        assert!((s1.ema_velocity - 10.0).abs() < f64::EPSILON);
        assert_eq!(s1.last_tokens, 1_000);
    }

    #[test]
    fn test_load_missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_velocity_states(dir.path());
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_load_filters_stale_states() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        // saved_at il y a 2 h → trop vieux (> 1800 s)
        let old_ts = (Utc::now() - chrono::Duration::seconds(7200)).to_rfc3339();
        let fresh_ts = Utc::now().to_rfc3339();

        let mut states: HashMap<String, VelocityState> = HashMap::new();
        states.insert("old".to_string(), make_state(5.0, 100, &old_ts));
        states.insert("fresh".to_string(), make_state(8.0, 200, &fresh_ts));

        save_velocity_states(path, &states);

        let loaded = load_velocity_states(path);
        // "old" doit être filtré, "fresh" doit être présent
        assert!(!loaded.contains_key("old"), "stale entry should be filtered");
        assert!(loaded.contains_key("fresh"), "fresh entry should be kept");
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn test_load_invalid_json_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join("velocity-state.json"), b"not valid json").unwrap();
        let loaded = load_velocity_states(path);
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_save_is_atomic_tmp_then_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        let mut states: HashMap<String, VelocityState> = HashMap::new();
        states.insert("k".to_string(), make_state(1.0, 0, &Utc::now().to_rfc3339()));

        save_velocity_states(path, &states);

        // Le fichier .tmp ne doit plus exister (rename effectué)
        assert!(!path.join("velocity-state.json.tmp").exists());
        assert!(path.join("velocity-state.json").exists());
    }
}
