//! Auto-switch intelligent entre comptes.
//!
//! Porte la logique Python de `src/gui/controllers/switch_controller.py` en Rust.
//!
//! # Algorithmes
//!
//! - [`SwitchController::try_auto_switch`] : détecte la dégradation quota sur le compte
//!   actif et sélectionne le meilleur candidat de remplacement.
//! - [`SwitchController::check_rotation`] : rotation temporelle circulaire entre comptes.
//! - [`SwitchController::record_switch`] : met à jour les timestamps internes après switch.
//!
//! # Thresholds
//!
//! Les seuils configurés dans `AppConfig.proxy` sont des **fractions** (0.85 = 85 %).
//! La comparaison se fait en fraction (pas en pourcentage) pour éviter toute confusion.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use tracing::{debug, info};

use crate::config::AppConfig;
use crate::credentials::CredentialsCache;
use crate::quota::DEFAULT_QUOTA_5H;

// ---------------------------------------------------------------------------
// Limite 7j par défaut (si non configurée dans le compte)
// ---------------------------------------------------------------------------

/// Limite de tokens sur 7 jours (si non stockée sur le compte).
/// Correspond à ~7 × 24 × DEFAULT_QUOTA_5H / 5 ≈ 1 008 000, arrondi.
const DEFAULT_QUOTA_7D: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// Types publics
// ---------------------------------------------------------------------------

/// Raison du switch de compte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchReason {
    /// Le compte actif a dépassé un seuil de quota.
    QuotaDegradation,
    /// Rotation temporelle automatique (round-robin).
    Rotation,
    /// Switch d'urgence (ex: erreur HTTP 429/529).
    Emergency,
}

impl SwitchReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            SwitchReason::QuotaDegradation => "degradation",
            SwitchReason::Rotation => "rotation",
            SwitchReason::Emergency => "emergency",
        }
    }
}

impl std::fmt::Display for SwitchReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Décision de switch produite par le contrôleur.
#[derive(Debug, Clone)]
pub struct SwitchDecision {
    /// Clé du compte vers lequel switcher.
    pub to_key: String,
    /// Raison du switch.
    pub reason: SwitchReason,
    /// Pourcentage quota 5h du compte actif au moment du déclenchement (fraction 0..1).
    pub active_pct_5h: f64,
    /// Pourcentage quota 7j du compte actif au moment du déclenchement (fraction 0..1).
    pub active_pct_7d: f64,
}

// ---------------------------------------------------------------------------
// Contrôleur
// ---------------------------------------------------------------------------

/// Contrôleur d'auto-switch entre comptes.
///
/// Maintient les timestamps du dernier switch et de la dernière rotation
/// pour respecter la grace period et l'intervalle de rotation.
///
/// Maintient également la liste des comptes marqués `invalid_grant` (token
/// révoqué côté serveur) afin de les exclure des candidats à la rotation et
/// à l'auto-switch. Appelez [`SwitchController::set_invalid_accounts`] à
/// chaque cycle pour tenir cette liste à jour.
///
/// # Thread-safety
///
/// Ce struct n'est **pas** `Sync` par lui-même : il est prévu pour être utilisé
/// depuis un seul thread ou protégé par un mutex externe.
pub struct SwitchController {
    /// Instant du dernier switch (auto ou rotation).
    last_switch_at: Instant,
    /// Instant de la dernière rotation.
    last_rotation_at: Instant,
    /// Comptes dont le refresh OAuth a retourné `invalid_grant`.
    /// Ces comptes sont exclus de `try_auto_switch` et `check_rotation`.
    invalid_accounts: HashSet<String>,
}

impl SwitchController {
    /// Crée un nouveau contrôleur.
    ///
    /// Les deux timestamps sont initialisés à `Instant::now()` afin que
    /// la grace period et l'intervalle de rotation soient respectés dès le départ.
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            last_switch_at: now,
            last_rotation_at: now,
            invalid_accounts: HashSet::new(),
        }
    }

    /// Met à jour la liste des comptes exclus (marqués `invalid_grant`).
    ///
    /// À appeler depuis la boucle de refresh à chaque cycle, en passant
    /// un snapshot du `Arc<RwLock<HashSet<String>>>` partagé avec `AppState`.
    /// Le `SwitchController` conserve une copie locale pour ses filtrages.
    pub fn set_invalid_accounts(&mut self, accounts: &HashSet<String>) {
        self.invalid_accounts = accounts.clone();
    }

    /// Enregistre qu'un switch vient d'avoir lieu (met à jour les deux timestamps).
    ///
    /// À appeler après avoir effectivement changé de compte actif.
    pub fn record_switch(&mut self) {
        let now = Instant::now();
        self.last_switch_at = now;
        self.last_rotation_at = now;
    }

    // -----------------------------------------------------------------------
    // try_auto_switch
    // -----------------------------------------------------------------------

    /// Vérifie si le compte actif nécessite un switch et retourne le meilleur candidat.
    ///
    /// # Algorithme
    ///
    /// 1. Récupère le compte actif et calcule ses taux d'utilisation quota.
    /// 2. Si `pct_5h >= threshold_5h OR pct_7d >= threshold_7d` → dégradation détectée.
    /// 3. Applique la grace period : si le dernier switch est trop récent, retourne `None`.
    /// 4. Parcourt tous les comptes éligibles (non deleted, non api, non autoSwitchDisabled).
    /// 5. Retourne `Some(SwitchDecision)` pointant vers le compte avec le score le plus bas
    ///    (score = max(pct_5h, pct_7d)), à condition que ses deux métriques soient sous seuil.
    ///
    /// # Retourne
    ///
    /// - `None` si aucun switch n'est nécessaire ou si aucun candidat valide n'existe.
    /// - `Some(SwitchDecision)` avec le compte cible et la raison.
    pub fn try_auto_switch(
        &mut self,
        credentials: &CredentialsCache,
        config: &AppConfig,
    ) -> Option<SwitchDecision> {
        let threshold_5h = config.proxy.auto_switch_threshold_5h;
        let threshold_7d = config.proxy.auto_switch_threshold_7d;
        let grace_secs = config.proxy.auto_switch_grace_secs;

        let creds = credentials.read();

        // Compte actif
        let active_key = creds.active_account.as_deref()?;
        let active_account = creds.accounts.get(active_key)?;

        // Les comptes API ne participent pas à l'auto-switch
        if active_account.account_type.as_deref() == Some("api") {
            debug!("try_auto_switch: compte actif est de type api, ignoré");
            return None;
        }

        // Si le compte actif est marqué invalid_grant, on ne peut pas déduire
        // la dégradation quota de son état → on ne déclenche pas de switch depuis lui.
        // (Le daemon aura déjà loggé le warning, ce compte sera simplement ignoré.)
        if self.invalid_accounts.contains(active_key) {
            debug!("try_auto_switch: compte actif {} marqué invalid_grant, skip", active_key);
            return None;
        }

        // Calcul des taux d'utilisation du compte actif
        let (pct_5h, pct_7d) = compute_usage_fractions(active_account);

        debug!(
            "try_auto_switch: active={} pct_5h={:.1}% pct_7d={:.1}% threshold_5h={:.1}% threshold_7d={:.1}%",
            active_key,
            pct_5h * 100.0,
            pct_7d * 100.0,
            threshold_5h * 100.0,
            threshold_7d * 100.0
        );

        // Déclenchement dégradation ?
        let needs_degradation = pct_5h >= threshold_5h || pct_7d >= threshold_7d;

        if !needs_degradation {
            debug!("try_auto_switch: pas de dégradation détectée");
            return None;
        }

        // Grace period : ne pas switcher trop vite après un switch récent
        let elapsed = self.last_switch_at.elapsed();
        if elapsed < Duration::from_secs(grace_secs) {
            debug!(
                "try_auto_switch: grace period active ({:.0}s / {}s)",
                elapsed.as_secs_f64(),
                grace_secs
            );
            return None;
        }

        // Chercher le meilleur candidat
        let active_key_owned = active_key.to_string();
        let mut best_key: Option<String> = None;
        let mut best_score = f64::INFINITY;

        // Collecter et trier les candidats par priorité (ordre décroissant : plus faible
        // priorité numérique = plus haute priorité)
        let mut candidates: Vec<(&str, _)> = creds
            .accounts
            .iter()
            .filter(|(key, acc)| {
                // Exclure le compte actif
                key.as_str() != active_key_owned.as_str()
                    // Exclure les comptes supprimés
                    && !acc.deleted
                    // Exclure les comptes API
                    && acc.account_type.as_deref() != Some("api")
                    // Exclure les comptes avec auto-switch désactivé
                    && !acc.auto_switch_disabled.unwrap_or(false)
                    // Exclure les comptes marqués invalid_grant
                    && !self.invalid_accounts.contains(key.as_str())
            })
            .map(|(key, acc)| (key.as_str(), acc))
            .collect();

        // Trier par priorité croissante (valeur basse = haute priorité)
        candidates.sort_by_key(|(_, acc)| acc.priority.unwrap_or(50));

        for (candidate_key, candidate_account) in candidates {
            let (c_pct_5h, c_pct_7d) = compute_usage_fractions(candidate_account);
            debug!(
                "try_auto_switch: candidat={} pct_5h={:.1}% pct_7d={:.1}%",
                candidate_key,
                c_pct_5h * 100.0,
                c_pct_7d * 100.0
            );

            // Les deux métriques doivent être sous seuil
            if c_pct_5h < threshold_5h && c_pct_7d < threshold_7d {
                let score = f64::max(c_pct_5h, c_pct_7d);
                if score < best_score {
                    best_score = score;
                    best_key = Some(candidate_key.to_string());
                }
            }
        }

        if let Some(to_key) = best_key {
            info!(
                "try_auto_switch: switch vers {} (score={:.1}% 5h:{:.1}% 7d:{:.1}%)",
                to_key,
                best_score * 100.0,
                pct_5h * 100.0,
                pct_7d * 100.0
            );
            Some(SwitchDecision {
                to_key,
                reason: SwitchReason::QuotaDegradation,
                active_pct_5h: pct_5h,
                active_pct_7d: pct_7d,
            })
        } else {
            info!(
                "try_auto_switch: dégradation détectée (5h:{:.1}% 7d:{:.1}%) mais aucun candidat éligible",
                pct_5h * 100.0,
                pct_7d * 100.0
            );
            None
        }
    }

    // -----------------------------------------------------------------------
    // check_rotation
    // -----------------------------------------------------------------------

    /// Vérifie si une rotation temporelle est due et retourne le prochain compte.
    ///
    /// # Algorithme
    ///
    /// 1. Si `rotation_enabled` est faux ou si l'intervalle n'est pas écoulé → `None`.
    /// 2. Collecte les comptes actifs (non deleted, non api, non autoSwitchDisabled).
    /// 3. Trie par clé pour garantir un ordre déterministe.
    /// 4. Trouve le compte courant dans la liste et retourne le suivant (circulaire).
    ///
    /// # Retourne
    ///
    /// - `None` si la rotation n'est pas due ou si un seul compte est disponible.
    /// - `Some(SwitchDecision)` vers le prochain compte dans la rotation.
    pub fn check_rotation(
        &mut self,
        credentials: &CredentialsCache,
        config: &AppConfig,
    ) -> Option<SwitchDecision> {
        if !config.proxy.rotation_enabled {
            return None;
        }

        let rotation_secs = config.proxy.rotation_interval_secs;
        let elapsed = self.last_rotation_at.elapsed();

        if elapsed < Duration::from_secs(rotation_secs) {
            debug!(
                "check_rotation: intervalle non atteint ({:.0}s / {}s)",
                elapsed.as_secs_f64(),
                rotation_secs
            );
            return None;
        }

        let creds = credentials.read();
        let active_key = creds.active_account.as_deref()?;

        // Comptes éligibles à la rotation, triés pour ordre déterministe
        let mut eligible: Vec<&str> = creds
            .accounts
            .iter()
            .filter(|(key, acc)| {
                !acc.deleted
                    && acc.account_type.as_deref() != Some("api")
                    && !acc.auto_switch_disabled.unwrap_or(false)
                    // Exclure les comptes marqués invalid_grant
                    && !self.invalid_accounts.contains(key.as_str())
            })
            .map(|(key, _)| key.as_str())
            .collect();

        eligible.sort_unstable();

        if eligible.len() < 2 {
            debug!("check_rotation: moins de 2 comptes éligibles, rotation ignorée");
            return None;
        }

        // Trouver la position du compte actif dans la liste
        let current_idx = eligible.iter().position(|&k| k == active_key);
        let start_idx = current_idx.map(|i| i + 1).unwrap_or(0);

        // Sélectionner le prochain compte dans l'ordre circulaire
        for i in 0..eligible.len() {
            let candidate_key = eligible[(start_idx + i) % eligible.len()];
            if candidate_key != active_key {
                info!(
                    "check_rotation: rotation {} -> {} (après {}s)",
                    active_key,
                    candidate_key,
                    elapsed.as_secs()
                );
                return Some(SwitchDecision {
                    to_key: candidate_key.to_string(),
                    reason: SwitchReason::Rotation,
                    active_pct_5h: 0.0,
                    active_pct_7d: 0.0,
                });
            }
        }

        None
    }
}

impl Default for SwitchController {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers internes
// ---------------------------------------------------------------------------

/// Calcule les taux d'utilisation 5h et 7j d'un compte sous forme de fractions (0.0..1.0).
///
/// Utilise `quota_5h` si disponible, sinon `DEFAULT_QUOTA_5H`.
/// Utilise `DEFAULT_QUOTA_7D` pour la fenêtre 7j (non stockée dans `AccountData`).
fn compute_usage_fractions(account: &crate::credentials::AccountData) -> (f64, f64) {
    let limit_5h = account.quota_5h.unwrap_or(DEFAULT_QUOTA_5H).max(1);
    let limit_7d = DEFAULT_QUOTA_7D.max(1);

    let pct_5h = account.tokens_5h as f64 / limit_5h as f64;
    let pct_7d = account.tokens_7d as f64 / limit_7d as f64;

    // Borner à [0, 1] pour éviter des fractions > 1 due à des dépassements
    (pct_5h.min(1.0).max(0.0), pct_7d.min(1.0).max(0.0))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::credentials::{AccountData, CredentialsFile};
    use crate::config::AppConfig;

    // -----------------------------------------------------------------------
    // Helpers de test
    // -----------------------------------------------------------------------

    fn make_account(
        tokens_5h: u64,
        tokens_7d: u64,
        quota_5h: Option<u64>,
        priority: Option<u32>,
        account_type: Option<&str>,
        auto_switch_disabled: Option<bool>,
        deleted: bool,
    ) -> AccountData {
        AccountData {
            tokens_5h,
            tokens_7d,
            quota_5h,
            priority,
            account_type: account_type.map(str::to_string),
            auto_switch_disabled,
            deleted,
            ..Default::default()
        }
    }

    fn make_credentials(
        accounts: HashMap<String, AccountData>,
        active: Option<&str>,
    ) -> std::sync::Arc<CredentialsCache> {
        let cf = CredentialsFile {
            accounts,
            active_account: active.map(str::to_string),
            version: None,
            last_updated: None,
        };
        let cache = CredentialsCache::empty();
        {
            let mut guard = cache.write();
            *guard = cf;
        }
        cache
    }

    fn default_config() -> AppConfig {
        AppConfig::default()
        // Par défaut: threshold_5h=0.85, threshold_7d=0.90, grace_secs=30
        // rotation_enabled=false, rotation_interval_secs=3600
    }

    fn config_with_thresholds(t5h: f64, t7d: f64, grace: u64) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.proxy.auto_switch_threshold_5h = t5h;
        cfg.proxy.auto_switch_threshold_7d = t7d;
        cfg.proxy.auto_switch_grace_secs = grace;
        cfg
    }

    // -----------------------------------------------------------------------
    // Tests try_auto_switch
    // -----------------------------------------------------------------------

    #[test]
    fn test_no_active_account_returns_none() {
        let mut ctrl = SwitchController::new();
        let creds = make_credentials(HashMap::new(), None);
        let cfg = default_config();
        assert!(ctrl.try_auto_switch(&creds, &cfg).is_none());
    }

    #[test]
    fn test_active_api_account_ignored() {
        let mut ctrl = SwitchController::new();
        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(80_000, 900_000, None, None, Some("api"), None, false),
        );
        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = default_config();
        assert!(ctrl.try_auto_switch(&creds, &cfg).is_none());
    }

    #[test]
    fn test_no_degradation_returns_none() {
        let mut ctrl = SwitchController::new();
        // 50% utilisation — sous les deux seuils (85% / 90%)
        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(45_000, 500_000, None, None, None, None, false),
        );
        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = default_config();
        assert!(ctrl.try_auto_switch(&creds, &cfg).is_none());
    }

    #[test]
    fn test_degradation_with_no_candidates_returns_none() {
        let mut ctrl = SwitchController::new();
        // 90% sur 5h → dégradation, mais pas d'autre compte
        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, None, None, None, false),
        );
        let creds = make_credentials(accounts, Some("acc1"));
        // grace_secs=0 pour ne pas bloquer
        let cfg = config_with_thresholds(0.85, 0.90, 0);
        assert!(ctrl.try_auto_switch(&creds, &cfg).is_none());
    }

    #[test]
    fn test_degradation_with_eligible_candidate() {
        // Initialise avec last_switch_at très ancien (> grace period)
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(400),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        // acc1 = actif, 90% 5h → dégradation
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, Some(1), None, None, false),
        );
        // acc2 = candidat, 20% 5h
        accounts.insert(
            "acc2".to_string(),
            make_account(18_000, 100_000, None, Some(2), None, None, false),
        );

        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        let decision = ctrl.try_auto_switch(&creds, &cfg);
        assert!(decision.is_some());
        let d = decision.unwrap();
        assert_eq!(d.to_key, "acc2");
        assert_eq!(d.reason, SwitchReason::QuotaDegradation);
        assert!(d.active_pct_5h >= 0.85);
    }

    #[test]
    fn test_grace_period_blocks_switch() {
        // Dernier switch il y a seulement 10s, grace period = 300s
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(10),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, None, None, None, false),
        );
        accounts.insert(
            "acc2".to_string(),
            make_account(10_000, 50_000, None, None, None, None, false),
        );

        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        // Doit être bloqué par la grace period
        assert!(ctrl.try_auto_switch(&creds, &cfg).is_none());
    }

    #[test]
    fn test_candidate_with_auto_switch_disabled_excluded() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(400),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, None, None, None, false),
        );
        // acc2 a autoSwitchDisabled=true
        accounts.insert(
            "acc2".to_string(),
            make_account(10_000, 50_000, None, None, None, Some(true), false),
        );

        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        // acc2 exclu → aucun candidat
        assert!(ctrl.try_auto_switch(&creds, &cfg).is_none());
    }

    #[test]
    fn test_deleted_candidate_excluded() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(400),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, None, None, None, false),
        );
        // acc2 est deleted
        accounts.insert(
            "acc2".to_string(),
            make_account(10_000, 50_000, None, None, None, None, true),
        );

        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        assert!(ctrl.try_auto_switch(&creds, &cfg).is_none());
    }

    #[test]
    fn test_candidate_api_excluded() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(400),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, None, None, None, false),
        );
        // acc2 est un compte api
        accounts.insert(
            "acc2".to_string(),
            make_account(10_000, 50_000, None, None, Some("api"), None, false),
        );

        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        assert!(ctrl.try_auto_switch(&creds, &cfg).is_none());
    }

    #[test]
    fn test_best_candidate_selected_by_score() {
        // Deux candidats : acc2 (40%) et acc3 (20%) → acc3 doit être choisi
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(400),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, None, None, None, false),
        );
        accounts.insert(
            "acc2".to_string(),
            make_account(36_000, 200_000, None, None, None, None, false), // ~40% 5h
        );
        accounts.insert(
            "acc3".to_string(),
            make_account(18_000, 100_000, None, None, None, None, false), // ~20% 5h
        );

        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        let decision = ctrl.try_auto_switch(&creds, &cfg).unwrap();
        assert_eq!(decision.to_key, "acc3");
    }

    #[test]
    fn test_candidate_over_threshold_excluded_from_selection() {
        // acc2 est aussi saturé → acc3 seulement
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(400),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, None, None, None, false), // ~90% 5h
        );
        // acc2 dépasse aussi le seuil 5h
        accounts.insert(
            "acc2".to_string(),
            make_account(77_000, 100_000, None, None, None, None, false), // ~86% 5h
        );
        // acc3 est ok
        accounts.insert(
            "acc3".to_string(),
            make_account(18_000, 100_000, None, None, None, None, false), // ~20% 5h
        );

        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        let decision = ctrl.try_auto_switch(&creds, &cfg).unwrap();
        assert_eq!(decision.to_key, "acc3");
    }

    #[test]
    fn test_degradation_triggered_by_7d_quota() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(400),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        // acc1 : 5h ok mais 7d saturé (91%)
        accounts.insert(
            "acc1".to_string(),
            make_account(40_000, 910_000, None, None, None, None, false),
        );
        accounts.insert(
            "acc2".to_string(),
            make_account(10_000, 100_000, None, None, None, None, false),
        );

        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        let decision = ctrl.try_auto_switch(&creds, &cfg).unwrap();
        assert_eq!(decision.to_key, "acc2");
        assert_eq!(decision.reason, SwitchReason::QuotaDegradation);
    }

    #[test]
    fn test_custom_quota_5h_used_for_percentage() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(400),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        // acc1 : quota_5h = 200_000, tokens_5h = 170_000 → 85% exactement
        // Threshold = 0.85 → pas déclenché (strict <, seuil exact non dépassé)
        accounts.insert(
            "acc1".to_string(),
            make_account(170_000, 100_000, Some(200_000), None, None, None, false),
        );
        accounts.insert(
            "acc2".to_string(),
            make_account(10_000, 50_000, None, None, None, None, false),
        );

        let creds = make_credentials(accounts, Some("acc1"));
        // threshold exactement 0.85 → 85% == 0.85 → >= donc switch déclenché
        let cfg = config_with_thresholds(0.85, 0.90, 0);

        let decision = ctrl.try_auto_switch(&creds, &cfg).unwrap();
        assert_eq!(decision.to_key, "acc2");
    }

    // -----------------------------------------------------------------------
    // Tests check_rotation
    // -----------------------------------------------------------------------

    #[test]
    fn test_rotation_disabled_returns_none() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(7200),
            last_rotation_at: Instant::now() - Duration::from_secs(7200),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert("acc1".to_string(), make_account(0, 0, None, None, None, None, false));
        accounts.insert("acc2".to_string(), make_account(0, 0, None, None, None, None, false));

        let creds = make_credentials(accounts, Some("acc1"));
        let mut cfg = default_config();
        cfg.proxy.rotation_enabled = false;

        assert!(ctrl.check_rotation(&creds, &cfg).is_none());
    }

    #[test]
    fn test_rotation_interval_not_elapsed_returns_none() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now(),
            last_rotation_at: Instant::now() - Duration::from_secs(100),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert("acc1".to_string(), make_account(0, 0, None, None, None, None, false));
        accounts.insert("acc2".to_string(), make_account(0, 0, None, None, None, None, false));

        let creds = make_credentials(accounts, Some("acc1"));
        let mut cfg = default_config();
        cfg.proxy.rotation_enabled = true;
        cfg.proxy.rotation_interval_secs = 3600;

        assert!(ctrl.check_rotation(&creds, &cfg).is_none());
    }

    #[test]
    fn test_rotation_single_account_returns_none() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now(),
            last_rotation_at: Instant::now() - Duration::from_secs(7200),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert("acc1".to_string(), make_account(0, 0, None, None, None, None, false));

        let creds = make_credentials(accounts, Some("acc1"));
        let mut cfg = default_config();
        cfg.proxy.rotation_enabled = true;
        cfg.proxy.rotation_interval_secs = 3600;

        assert!(ctrl.check_rotation(&creds, &cfg).is_none());
    }

    #[test]
    fn test_rotation_selects_next_account() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now(),
            last_rotation_at: Instant::now() - Duration::from_secs(7200),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert("acc1".to_string(), make_account(0, 0, None, None, None, None, false));
        accounts.insert("acc2".to_string(), make_account(0, 0, None, None, None, None, false));
        accounts.insert("acc3".to_string(), make_account(0, 0, None, None, None, None, false));

        let creds = make_credentials(accounts, Some("acc1"));
        let mut cfg = default_config();
        cfg.proxy.rotation_enabled = true;
        cfg.proxy.rotation_interval_secs = 3600;

        let decision = ctrl.check_rotation(&creds, &cfg).unwrap();
        assert_eq!(decision.reason, SwitchReason::Rotation);
        // Comptes triés: acc1, acc2, acc3 → suivant de acc1 = acc2
        assert_eq!(decision.to_key, "acc2");
    }

    #[test]
    fn test_rotation_wraps_around() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now(),
            last_rotation_at: Instant::now() - Duration::from_secs(7200),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert("acc1".to_string(), make_account(0, 0, None, None, None, None, false));
        accounts.insert("acc2".to_string(), make_account(0, 0, None, None, None, None, false));

        // acc2 est le compte actif → suivant = acc1 (wrap)
        let creds = make_credentials(accounts, Some("acc2"));
        let mut cfg = default_config();
        cfg.proxy.rotation_enabled = true;
        cfg.proxy.rotation_interval_secs = 3600;

        let decision = ctrl.check_rotation(&creds, &cfg).unwrap();
        assert_eq!(decision.reason, SwitchReason::Rotation);
        assert_eq!(decision.to_key, "acc1");
    }

    #[test]
    fn test_rotation_skips_disabled_accounts() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now(),
            last_rotation_at: Instant::now() - Duration::from_secs(7200),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert("acc1".to_string(), make_account(0, 0, None, None, None, None, false));
        // acc2 est exclu de la rotation
        accounts.insert(
            "acc2".to_string(),
            make_account(0, 0, None, None, None, Some(true), false),
        );
        accounts.insert("acc3".to_string(), make_account(0, 0, None, None, None, None, false));

        let creds = make_credentials(accounts, Some("acc1"));
        let mut cfg = default_config();
        cfg.proxy.rotation_enabled = true;
        cfg.proxy.rotation_interval_secs = 3600;

        let decision = ctrl.check_rotation(&creds, &cfg).unwrap();
        // acc2 exclu → acc3
        assert_eq!(decision.to_key, "acc3");
    }

    // -----------------------------------------------------------------------
    // Tests record_switch
    // -----------------------------------------------------------------------

    #[test]
    fn test_record_switch_resets_grace_period() {
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(400),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, None, None, None, false),
        );
        accounts.insert(
            "acc2".to_string(),
            make_account(10_000, 50_000, None, None, None, None, false),
        );

        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        // Avant record_switch → switch possible
        let d = ctrl.try_auto_switch(&creds, &cfg);
        assert!(d.is_some());

        // Après record_switch → grace period bloque
        ctrl.record_switch();
        let d2 = ctrl.try_auto_switch(&creds, &cfg);
        assert!(d2.is_none());
    }

    // -----------------------------------------------------------------------
    // Tests compute_usage_fractions
    // -----------------------------------------------------------------------

    #[test]
    fn test_compute_usage_fractions_default_quota() {
        let acc = make_account(45_000, 500_000, None, None, None, None, false);
        let (pct_5h, pct_7d) = compute_usage_fractions(&acc);
        // DEFAULT_QUOTA_5H = 90_000
        assert!((pct_5h - 0.5).abs() < 0.01, "pct_5h={pct_5h}");
        // DEFAULT_QUOTA_7D = 1_000_000
        assert!((pct_7d - 0.5).abs() < 0.01, "pct_7d={pct_7d}");
    }

    #[test]
    fn test_compute_usage_fractions_custom_quota() {
        let acc = make_account(80_000, 0, Some(100_000), None, None, None, false);
        let (pct_5h, pct_7d) = compute_usage_fractions(&acc);
        assert!((pct_5h - 0.8).abs() < 0.01, "pct_5h={pct_5h}");
        assert!((pct_7d - 0.0).abs() < 0.01, "pct_7d={pct_7d}");
    }

    #[test]
    fn test_compute_usage_fractions_capped_at_one() {
        // tokens > quota → cap à 1.0
        let acc = make_account(200_000, 2_000_000, Some(100_000), None, None, None, false);
        let (pct_5h, pct_7d) = compute_usage_fractions(&acc);
        assert_eq!(pct_5h, 1.0);
        assert_eq!(pct_7d, 1.0);
    }

    // -----------------------------------------------------------------------
    // Tests SwitchReason display
    // -----------------------------------------------------------------------

    #[test]
    fn test_switch_reason_display() {
        assert_eq!(SwitchReason::QuotaDegradation.to_string(), "degradation");
        assert_eq!(SwitchReason::Rotation.to_string(), "rotation");
        assert_eq!(SwitchReason::Emergency.to_string(), "emergency");
    }

    #[test]
    fn test_switch_controller_default() {
        let ctrl = SwitchController::default();
        // Les timestamps sont initialisés récemment → grace period active
        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, None, None, None, false),
        );
        accounts.insert(
            "acc2".to_string(),
            make_account(10_000, 50_000, None, None, None, None, false),
        );
        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        // Contrôleur fraîchement créé → grace period active → None
        let mut ctrl = ctrl;
        assert!(ctrl.try_auto_switch(&creds, &cfg).is_none());
    }

    // -----------------------------------------------------------------------
    // Tests invalid_grant exclusion
    // -----------------------------------------------------------------------

    #[test]
    fn test_invalid_grant_candidate_excluded_from_auto_switch() {
        // acc2 est marqué invalid_grant → ne doit pas être sélectionné comme candidat
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now() - Duration::from_secs(400),
            last_rotation_at: Instant::now(),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert(
            "acc1".to_string(),
            make_account(81_000, 100_000, None, None, None, None, false), // dégradé
        );
        accounts.insert(
            "acc2".to_string(),
            make_account(10_000, 50_000, None, None, None, None, false), // ok mais invalid_grant
        );
        accounts.insert(
            "acc3".to_string(),
            make_account(20_000, 100_000, None, None, None, None, false), // ok
        );

        let creds = make_credentials(accounts, Some("acc1"));
        let cfg = config_with_thresholds(0.85, 0.90, 300);

        // Marquer acc2 comme invalid_grant
        ctrl.set_invalid_accounts(&HashSet::from(["acc2".to_string()]));

        // acc2 exclu → acc3 doit être choisi
        let decision = ctrl.try_auto_switch(&creds, &cfg).unwrap();
        assert_eq!(decision.to_key, "acc3");
    }

    #[test]
    fn test_invalid_grant_candidate_excluded_from_rotation() {
        // acc2 est marqué invalid_grant → skippé en rotation
        let mut ctrl = SwitchController {
            last_switch_at: Instant::now(),
            last_rotation_at: Instant::now() - Duration::from_secs(7200),
            invalid_accounts: HashSet::new(),
        };

        let mut accounts = HashMap::new();
        accounts.insert("acc1".to_string(), make_account(0, 0, None, None, None, None, false));
        accounts.insert("acc2".to_string(), make_account(0, 0, None, None, None, None, false));
        accounts.insert("acc3".to_string(), make_account(0, 0, None, None, None, None, false));

        let creds = make_credentials(accounts, Some("acc1"));
        let mut cfg = default_config();
        cfg.proxy.rotation_enabled = true;
        cfg.proxy.rotation_interval_secs = 3600;

        // Marquer acc2 comme invalid_grant
        ctrl.set_invalid_accounts(&HashSet::from(["acc2".to_string()]));

        // Comptes éligibles triés: acc1, acc3 → suivant de acc1 = acc3 (acc2 exclu)
        let decision = ctrl.check_rotation(&creds, &cfg).unwrap();
        assert_eq!(decision.to_key, "acc3");
    }

    #[test]
    fn test_set_invalid_accounts_clears_previous() {
        // Vérifier que set_invalid_accounts remplace (pas accumule)
        let mut ctrl = SwitchController::new();
        ctrl.set_invalid_accounts(&HashSet::from(["acc1".to_string(), "acc2".to_string()]));
        assert_eq!(ctrl.invalid_accounts.len(), 2);

        // Nouvelle liste vide → l'ancienne est effacée
        ctrl.set_invalid_accounts(&HashSet::new());
        assert_eq!(ctrl.invalid_accounts.len(), 0);
    }
}
