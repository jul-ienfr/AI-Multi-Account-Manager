//! Phi Accrual Failure Detector et déduplication de messages.
//!
//! Traduit la logique Python de `sync_bus.py` (failure detector Phi Accrual,
//! fenêtre glissante des heartbeats, états ALIVE / SUSPECT / DEAD) en Rust.
//!
//! # Phi Accrual
//!
//! Le score phi mesure la probabilité qu'un nœud soit tombé :
//!
//! ```text
//! phi = -log10(1 - CDF_normal(elapsed, mean, stddev))
//! ```
//!
//! - `elapsed`  = temps écoulé depuis le dernier heartbeat (secondes)
//! - `mean`     = moyenne des intervalles entre heartbeats
//! - `stddev`   = écart-type des intervalles (minimum 1s si pas assez de données)
//! - `phi < 1`  → Alive
//! - `phi >= 1` → Suspect
//! - `phi >= 3` → Dead
//!
//! # MessageDedup
//!
//! Fenêtre glissante de 1000 message IDs pour éviter de traiter deux fois
//! le même message (rebond dans un réseau P2P).

use std::collections::VecDeque;
use std::time::Instant;

// ────────────────────────────────────────────────────────────────────────────
// NodeStatus
// ────────────────────────────────────────────────────────────────────────────

/// État d'un nœud déterminé par le failure detector.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeStatus {
    /// Le nœud répond normalement (phi < 1.0).
    Alive,
    /// Le nœud est suspecté d'être tombé (1.0 ≤ phi < 3.0).
    Suspect,
    /// Le nœud est considéré mort (phi ≥ 3.0).
    Dead,
}

// ────────────────────────────────────────────────────────────────────────────
// PhiAccrualDetector
// ────────────────────────────────────────────────────────────────────────────

/// Taille maximale de la fenêtre glissante des intervalles heartbeat.
const MAX_INTERVALS: usize = 1000;

/// Phi au-delà duquel le nœud passe en SUSPECT.
const THRESHOLD_SUSPECT: f64 = 1.0;

/// Phi au-delà duquel le nœud passe en DEAD.
const THRESHOLD_DEAD: f64 = 3.0;

/// Écart-type minimal utilisé quand on manque de données (évite division par 0).
const MIN_STDDEV_SECS: f64 = 1.0;

/// Failure detector Phi Accrual.
///
/// Maintient une fenêtre glissante des intervalles entre heartbeats et calcule
/// un score phi accrual indiquant la probabilité qu'un nœud soit tombé.
pub struct PhiAccrualDetector {
    /// Fenêtre glissante des intervalles heartbeat (en secondes, max 1000).
    intervals: VecDeque<f64>,
    /// Dernier heartbeat enregistré.
    last_heartbeat: Option<Instant>,
    /// Seuil phi → SUSPECT.
    threshold_suspect: f64,
    /// Seuil phi → DEAD.
    threshold_dead: f64,
}

impl PhiAccrualDetector {
    /// Crée un nouveau détecteur avec les seuils par défaut (1.0 / 3.0).
    pub fn new() -> Self {
        Self {
            intervals: VecDeque::with_capacity(MAX_INTERVALS),
            last_heartbeat: None,
            threshold_suspect: THRESHOLD_SUSPECT,
            threshold_dead: THRESHOLD_DEAD,
        }
    }

    /// Crée un détecteur avec des seuils personnalisés (utile pour les tests).
    pub fn with_thresholds(threshold_suspect: f64, threshold_dead: f64) -> Self {
        Self {
            intervals: VecDeque::with_capacity(MAX_INTERVALS),
            last_heartbeat: None,
            threshold_suspect,
            threshold_dead,
        }
    }

    /// Enregistre un heartbeat.
    ///
    /// Si un heartbeat précédent existe, l'intervalle est ajouté à la fenêtre.
    /// La fenêtre est bornée à `MAX_INTERVALS` entrées (FIFO).
    pub fn heartbeat(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_heartbeat {
            let interval = now.duration_since(last).as_secs_f64();
            if self.intervals.len() >= MAX_INTERVALS {
                self.intervals.pop_front();
            }
            self.intervals.push_back(interval);
        }
        self.last_heartbeat = Some(now);
    }

    /// Calcule le score phi accrual à l'instant présent.
    ///
    /// Retourne `0.0` si aucun heartbeat n'a été reçu ou si un seul heartbeat
    /// a été reçu (pas encore d'intervalle enregistré).
    ///
    /// Formule : `phi = -log10(1 - CDF_normal(elapsed, mean, stddev))`
    pub fn phi(&self) -> f64 {
        let last = match self.last_heartbeat {
            Some(t) => t,
            None => return 0.0,
        };

        if self.intervals.is_empty() {
            // Un seul heartbeat reçu, pas encore d'intervalle → nœud considéré vivant.
            return 0.0;
        }

        let elapsed = last.elapsed().as_secs_f64();
        let mean = self.mean();
        let stddev = self.stddev(mean).max(MIN_STDDEV_SECS);

        // CDF gaussienne approchée par la fonction erf.
        // CDF(x, μ, σ) = 0.5 * (1 + erf((x - μ) / (σ * √2)))
        let z = (elapsed - mean) / (stddev * std::f64::consts::SQRT_2);
        let cdf = 0.5 * (1.0 + erf(z));

        // Borne inférieure de (1 - cdf) pour éviter log10(0) = -∞ → phi = +∞.
        let one_minus_cdf = (1.0 - cdf).max(1e-300);

        // phi = -log10(1 - CDF)
        -one_minus_cdf.log10()
    }

    /// Retourne le statut du nœud basé sur le score phi courant.
    pub fn status(&self) -> NodeStatus {
        let phi = self.phi();
        if phi >= self.threshold_dead {
            NodeStatus::Dead
        } else if phi >= self.threshold_suspect {
            NodeStatus::Suspect
        } else {
            NodeStatus::Alive
        }
    }

    /// Retourne `true` si le détecteur a reçu au moins un heartbeat.
    pub fn has_heartbeat(&self) -> bool {
        self.last_heartbeat.is_some()
    }

    /// Nombre d'intervalles dans la fenêtre.
    pub fn interval_count(&self) -> usize {
        self.intervals.len()
    }

    // ── Helpers statistiques ──────────────────────────────────────────────

    fn mean(&self) -> f64 {
        if self.intervals.is_empty() {
            return 0.0;
        }
        self.intervals.iter().sum::<f64>() / self.intervals.len() as f64
    }

    fn stddev(&self, mean: f64) -> f64 {
        if self.intervals.len() < 2 {
            return 0.0;
        }
        let variance = self
            .intervals
            .iter()
            .map(|&x| {
                let diff = x - mean;
                diff * diff
            })
            .sum::<f64>()
            / self.intervals.len() as f64;
        variance.sqrt()
    }
}

impl Default for PhiAccrualDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Constructeurs de test uniquement — permettent d'injecter des intervalles
/// et de fixer le `last_heartbeat` sans dépendre de la vitesse système.
#[cfg(test)]
impl PhiAccrualDetector {
    /// Injecte une liste d'intervalles pré-calculés dans la fenêtre.
    fn inject_intervals(&mut self, intervals: &[f64]) {
        for &i in intervals {
            if self.intervals.len() >= MAX_INTERVALS {
                self.intervals.pop_front();
            }
            self.intervals.push_back(i);
        }
    }

    /// Fixe le dernier heartbeat à maintenant.
    fn set_last_heartbeat_now(&mut self) {
        self.last_heartbeat = Some(Instant::now());
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Fonction erf (approximation numérique — Abramowitz & Stegun 7.1.26)
// ────────────────────────────────────────────────────────────────────────────

/// Approximation de la fonction d'erreur (erf) précise à ~1.5×10⁻⁷.
///
/// Algorithme : Abramowitz & Stegun, formule 7.1.26.
fn erf(x: f64) -> f64 {
    // erf est impaire : erf(-x) = -erf(x)
    let sign = if x < 0.0 { -1.0_f64 } else { 1.0_f64 };
    let x = x.abs();

    // Coefficients polynomiaux A&S 7.1.26
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));

    sign * (1.0 - poly * (-x * x).exp())
}

// ────────────────────────────────────────────────────────────────────────────
// MessageDedup — déduplication fenêtre glissante
// ────────────────────────────────────────────────────────────────────────────

/// Taille par défaut de la fenêtre de déduplication.
const DEDUP_DEFAULT_SIZE: usize = 1000;

/// Déduplication de messages par fenêtre glissante d'IDs.
///
/// Conserve les `max_size` derniers IDs vus. Quand la fenêtre est pleine,
/// le message le plus ancien est oublié (le réseau peut le réintroduire sans
/// qu'il soit bloqué indéfiniment).
pub struct MessageDedup {
    /// File FIFO des IDs vus (ordre d'insertion).
    seen: VecDeque<String>,
    /// Taille maximale de la fenêtre.
    max_size: usize,
}

impl MessageDedup {
    /// Crée un déduplicateur avec la taille par défaut (1000).
    pub fn new() -> Self {
        Self {
            seen: VecDeque::with_capacity(DEDUP_DEFAULT_SIZE),
            max_size: DEDUP_DEFAULT_SIZE,
        }
    }

    /// Crée un déduplicateur avec une taille de fenêtre personnalisée.
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            seen: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    /// Retourne `true` si l'ID a déjà été vu dans la fenêtre courante.
    pub fn is_duplicate(&self, id: &str) -> bool {
        self.seen.iter().any(|s| s == id)
    }

    /// Marque un ID comme vu.
    ///
    /// Si la fenêtre est pleine, l'ID le plus ancien est supprimé avant
    /// d'insérer le nouveau.
    pub fn mark_seen(&mut self, id: &str) {
        if self.seen.len() >= self.max_size {
            self.seen.pop_front();
        }
        self.seen.push_back(id.to_string());
    }

    /// Retourne le nombre d'IDs dans la fenêtre.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Retourne `true` si la fenêtre est vide.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

impl Default for MessageDedup {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    // ── Tests PhiAccrualDetector ──────────────────────────────────────────

    #[test]
    fn test_phi_no_heartbeat_is_zero() {
        let detector = PhiAccrualDetector::new();
        assert_eq!(detector.phi(), 0.0);
        assert_eq!(detector.status(), NodeStatus::Alive);
    }

    #[test]
    fn test_phi_single_heartbeat_is_zero() {
        let mut detector = PhiAccrualDetector::new();
        detector.heartbeat();
        // Un seul heartbeat → pas d'intervalle → phi = 0
        assert_eq!(detector.phi(), 0.0);
        assert_eq!(detector.status(), NodeStatus::Alive);
    }

    #[test]
    fn test_phi_alive_after_recent_heartbeat() {
        // Simule des heartbeats réguliers puis un heartbeat récent
        let mut detector = PhiAccrualDetector::new();

        // Pompe plusieurs intervalles à ~100ms
        for _ in 0..5 {
            detector.heartbeat();
            thread::sleep(Duration::from_millis(50));
        }
        // Dernier heartbeat très récent → phi très faible → Alive
        detector.heartbeat();

        let phi = detector.phi();
        assert!(
            phi < THRESHOLD_SUSPECT,
            "phi should be < {} after recent heartbeat, got {}",
            THRESHOLD_SUSPECT,
            phi
        );
        assert_eq!(detector.status(), NodeStatus::Alive);
    }

    /// Test déterministe : injecte des intervalles de 0.1s directement,
    /// puis attend 5s → phi très élevé (SUSPECT ou DEAD).
    #[test]
    fn test_phi_suspect_after_silence() {
        let mut detector = PhiAccrualDetector::new();

        // Injecte 20 intervalles de 0.1s → mean=0.1s, stddev=0 → clamped à 1.0s
        // Avec mean=0.1, stddev_eff=1.0 :
        //   CDF(2.0, 0.1, 1.0) = 0.5*(1+erf((2.0-0.1)/(1.0*√2))) ≈ 0.9713
        //   phi = -log10(1-0.9713) ≈ 1.54 → SUSPECT
        let intervals: Vec<f64> = vec![0.1; 20];
        detector.inject_intervals(&intervals);
        detector.set_last_heartbeat_now();
        thread::sleep(Duration::from_secs(2));

        let phi = detector.phi();
        assert!(
            phi >= THRESHOLD_SUSPECT,
            "phi should be >= {} after long silence with short mean intervals, got {}",
            THRESHOLD_SUSPECT,
            phi
        );
        assert_ne!(detector.status(), NodeStatus::Alive);
    }

    #[test]
    fn test_phi_dead_after_very_long_silence() {
        // Intervalles de 0.1s, attend 4s :
        //   CDF(4.0, 0.1, 1.0) = 0.5*(1+erf((4.0-0.1)/(1.0*√2))) ≈ 0.9997
        //   phi = -log10(1-0.9997) ≈ 3.52 → DEAD
        let mut detector = PhiAccrualDetector::new();
        let intervals: Vec<f64> = vec![0.1; 20];
        detector.inject_intervals(&intervals);
        detector.set_last_heartbeat_now();
        thread::sleep(Duration::from_secs(4));

        let phi = detector.phi();
        assert!(
            phi >= THRESHOLD_DEAD,
            "phi should be >= {} after very long silence, got {}",
            THRESHOLD_DEAD,
            phi
        );
        assert_eq!(detector.status(), NodeStatus::Dead);
    }

    #[test]
    fn test_window_bounded_at_1000() {
        let mut detector = PhiAccrualDetector::new();
        // Pompe 1100 heartbeats sans attente → ~1100 intervalles quasi-nuls
        for _ in 0..1100 {
            detector.heartbeat();
        }
        assert_eq!(
            detector.interval_count(),
            MAX_INTERVALS,
            "window should be bounded at {}",
            MAX_INTERVALS
        );
    }

    #[test]
    fn test_erf_known_values() {
        // erf(0) = 0
        assert!((erf(0.0)).abs() < 1e-6);
        // erf(∞) ≈ 1
        assert!((erf(10.0) - 1.0).abs() < 1e-6);
        // erf(-∞) ≈ -1
        assert!((erf(-10.0) + 1.0).abs() < 1e-6);
        // erf est impaire
        assert!((erf(1.0) + erf(-1.0)).abs() < 1e-10);
    }

    // ── Tests MessageDedup ────────────────────────────────────────────────

    #[test]
    fn test_dedup_basic() {
        let mut dedup = MessageDedup::new();

        // Message inconnu → pas duplicate
        assert!(!dedup.is_duplicate("msg-1"));
        dedup.mark_seen("msg-1");

        // Même message → duplicate
        assert!(dedup.is_duplicate("msg-1"));

        // Autre message → pas duplicate
        assert!(!dedup.is_duplicate("msg-2"));
    }

    #[test]
    fn test_dedup_mark_then_check() {
        let mut dedup = MessageDedup::new();
        dedup.mark_seen("abc");
        dedup.mark_seen("def");
        dedup.mark_seen("ghi");

        assert!(dedup.is_duplicate("abc"));
        assert!(dedup.is_duplicate("def"));
        assert!(dedup.is_duplicate("ghi"));
        assert!(!dedup.is_duplicate("xyz"));
        assert_eq!(dedup.len(), 3);
    }

    #[test]
    fn test_dedup_window_evicts_oldest() {
        // Fenêtre de taille 10 : après 11 insertions, le premier est oublié.
        let mut dedup = MessageDedup::with_max_size(10);

        for i in 0..10 {
            dedup.mark_seen(&format!("msg-{}", i));
        }
        assert_eq!(dedup.len(), 10);

        // msg-0 est encore dans la fenêtre
        assert!(dedup.is_duplicate("msg-0"));

        // Insère un 11ème → msg-0 est évincé
        dedup.mark_seen("msg-10");
        assert_eq!(dedup.len(), 10);
        assert!(!dedup.is_duplicate("msg-0"), "msg-0 should have been evicted");
        assert!(dedup.is_duplicate("msg-10"));
    }

    #[test]
    fn test_dedup_window_size_1001_evicts_oldest() {
        // Teste la borne de 1000 du déduplicateur par défaut.
        let mut dedup = MessageDedup::new(); // max_size = 1000

        for i in 0..1001 {
            dedup.mark_seen(&format!("id-{}", i));
        }

        // La fenêtre est bornée à 1000
        assert_eq!(dedup.len(), 1000);

        // id-0 a été évincé après 1001 insertions
        assert!(
            !dedup.is_duplicate("id-0"),
            "id-0 should have been evicted after 1001 inserts"
        );

        // id-1 est encore présent (premier survivant)
        assert!(
            dedup.is_duplicate("id-1"),
            "id-1 should still be in the window"
        );

        // id-1000 (le dernier inséré) est présent
        assert!(dedup.is_duplicate("id-1000"));
    }

    #[test]
    fn test_dedup_empty_initially() {
        let dedup = MessageDedup::new();
        assert!(dedup.is_empty());
        assert_eq!(dedup.len(), 0);
    }
}
