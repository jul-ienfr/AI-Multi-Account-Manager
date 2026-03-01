//! Stratégies de routing entre comptes Claude.
//!
//! Traduit `routing/strategy.py` en Rust.
//!
//! # Stratégies disponibles
//!
//! - `Priority` : Compte prioritaire fixe (premier de la liste)
//! - `QuotaAware` : Compte avec le plus de quota restant (5h)
//! - `RoundRobin` : Rotation séquentielle entre comptes
//! - `Latency` : Compte avec la latence mesurée la plus faible
//! - `Usage` : Compte avec le moins d'utilisation totale (7d)

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::debug;

use crate::config::RoutingStrategy;
use crate::credentials::{AccountData, CredentialsCache};

/// Résultat du routing : clé du compte sélectionné.
pub type RoutingResult = Option<String>;

/// Router de comptes — sélectionne le meilleur compte selon la stratégie.
pub struct AccountRouter {
    credentials: Arc<CredentialsCache>,
    /// Index courant pour round-robin.
    rr_index: Arc<AtomicUsize>,
    /// Latences mesurées (ms) par clé de compte.
    latencies: Arc<RwLock<std::collections::HashMap<String, u64>>>,
    /// Ordre de priorité des comptes (clés).
    priority_order: Arc<RwLock<Vec<String>>>,
}

impl AccountRouter {
    /// Crée un nouveau router.
    pub fn new(credentials: Arc<CredentialsCache>) -> Self {
        Self {
            credentials,
            rr_index: Arc::new(AtomicUsize::new(0)),
            latencies: Arc::new(RwLock::new(std::collections::HashMap::new())),
            priority_order: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Sélectionne le compte à utiliser selon la stratégie.
    ///
    /// Retourne `None` si aucun compte n'est disponible.
    pub fn select(&self, strategy: RoutingStrategy) -> RoutingResult {
        let available = self.available_accounts();
        if available.is_empty() {
            debug!("No available accounts for routing");
            return None;
        }

        let selected = match strategy {
            RoutingStrategy::Priority => self.select_priority(&available),
            RoutingStrategy::QuotaAware => self.select_quota_aware(&available),
            RoutingStrategy::RoundRobin => self.select_round_robin(&available),
            RoutingStrategy::Latency => self.select_latency(&available),
            RoutingStrategy::Usage => self.select_usage(&available),
        };

        debug!(
            "Router selected: {:?} (strategy={})",
            selected,
            strategy.as_str()
        );
        selected
    }

    // -----------------------------------------------------------------------
    // Strategies
    // -----------------------------------------------------------------------

    /// Stratégie Priority : retourne le premier compte selon l'ordre de priorité.
    fn select_priority(&self, available: &[(String, AccountData)]) -> RoutingResult {
        let order = self.priority_order.read();
        if order.is_empty() {
            // Pas d'ordre défini → premier disponible
            return available.first().map(|(k, _)| k.clone());
        }

        // Suit l'ordre de priorité
        for key in order.iter() {
            if available.iter().any(|(k, _)| k == key) {
                return Some(key.clone());
            }
        }

        // Fallback si aucun compte prioritaire n'est disponible
        available.first().map(|(k, _)| k.clone())
    }

    /// Stratégie QuotaAware : retourne le compte avec le plus de quota restant.
    fn select_quota_aware(&self, available: &[(String, AccountData)]) -> RoutingResult {
        available
            .iter()
            .max_by(|(_, a), (_, b)| {
                let ra = quota_remaining_5h(a);
                let rb = quota_remaining_5h(b);
                ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(k, _)| k.clone())
    }

    /// Stratégie RoundRobin : rotation séquentielle.
    fn select_round_robin(&self, available: &[(String, AccountData)]) -> RoutingResult {
        let n = available.len();
        if n == 0 {
            return None;
        }
        let idx = self.rr_index.fetch_add(1, Ordering::Relaxed) % n;
        available.get(idx).map(|(k, _)| k.clone())
    }

    /// Stratégie Latency : retourne le compte avec la latence la plus faible.
    fn select_latency(&self, available: &[(String, AccountData)]) -> RoutingResult {
        let latencies = self.latencies.read();
        available
            .iter()
            .min_by_key(|(k, _)| latencies.get(k).copied().unwrap_or(u64::MAX))
            .map(|(k, _)| k.clone())
    }

    /// Stratégie Usage : retourne le compte le moins utilisé (tokens_7d).
    fn select_usage(&self, available: &[(String, AccountData)]) -> RoutingResult {
        available
            .iter()
            .min_by_key(|(_, a)| a.tokens_7d)
            .map(|(k, _)| k.clone())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Liste les comptes disponibles (non supprimés, token valide).
    fn available_accounts(&self) -> Vec<(String, AccountData)> {
        let data = self.credentials.read();
        data.accounts
            .iter()
            .filter(|(_, a)| !a.deleted && a.has_valid_token())
            .map(|(k, a)| (k.clone(), a.clone()))
            .collect()
    }

    /// Met à jour la latence mesurée pour un compte.
    pub fn record_latency(&self, key: &str, latency_ms: u64) {
        let mut latencies = self.latencies.write();
        // EMA simple pour la latence
        let current = latencies.entry(key.to_string()).or_insert(latency_ms);
        *current = ((*current * 7) + latency_ms) / 8;
    }

    /// Définit l'ordre de priorité des comptes.
    pub fn set_priority_order(&self, order: Vec<String>) {
        *self.priority_order.write() = order;
    }

    /// Nombre de comptes disponibles.
    pub fn available_count(&self) -> usize {
        let data = self.credentials.read();
        data.accounts
            .values()
            .filter(|a| !a.deleted && a.has_valid_token())
            .count()
    }
}

/// Quota restant sur 5h (fraction 0.0–1.0).
fn quota_remaining_5h(account: &AccountData) -> f64 {
    let limit = account.quota_5h.unwrap_or(90_000);
    if limit == 0 {
        return 1.0;
    }
    (1.0 - account.tokens_5h as f64 / limit as f64).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crate::credentials::{CredentialsCache, AccountData, OAuthData};

    fn make_account(tokens_5h: u64, tokens_7d: u64) -> AccountData {
        AccountData {
            name: None,
            email: None,
            oauth: Some(OAuthData {
                access_token: "tok".to_string(),
                refresh_token: "ref".to_string(),
                expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
                token_type: None,
                scope: None,
                scopes: None,
                refresh_token_expires_at: None,
                organization_uuid: None,
            }),
            tokens_5h,
            tokens_7d,
            quota_5h: Some(90_000),
            ..Default::default()
        }
    }

    fn make_router_with_accounts(accounts: Vec<(&str, u64, u64)>) -> AccountRouter {
        let cache = CredentialsCache::empty();
        {
            let mut data = cache.write();
            for (key, t5h, t7d) in &accounts {
                data.accounts.insert(key.to_string(), make_account(*t5h, *t7d));
            }
        }
        AccountRouter::new(Arc::clone(&cache))
    }

    #[test]
    fn test_no_accounts_returns_none() {
        let cache = CredentialsCache::empty();
        let router = AccountRouter::new(Arc::clone(&cache));
        assert!(router.select(RoutingStrategy::Priority).is_none());
    }

    #[test]
    fn test_quota_aware_selects_least_used() {
        let router = make_router_with_accounts(vec![
            ("heavy", 80_000, 0),
            ("light", 10_000, 0),
        ]);
        // "light" has more quota remaining
        let selected = router.select(RoutingStrategy::QuotaAware).unwrap();
        assert_eq!(selected, "light");
    }

    #[test]
    fn test_round_robin_cycles() {
        let router = make_router_with_accounts(vec![
            ("a", 0, 0),
            ("b", 0, 0),
        ]);
        let first = router.select(RoutingStrategy::RoundRobin).unwrap();
        let second = router.select(RoutingStrategy::RoundRobin).unwrap();
        assert_ne!(first, second, "RoundRobin should cycle");
    }

    #[test]
    fn test_latency_selects_fastest() {
        let router = make_router_with_accounts(vec![
            ("fast", 0, 0),
            ("slow", 0, 0),
        ]);
        router.record_latency("fast", 50);
        router.record_latency("slow", 500);
        let selected = router.select(RoutingStrategy::Latency).unwrap();
        assert_eq!(selected, "fast");
    }

    #[test]
    fn test_usage_selects_least_used_7d() {
        let router = make_router_with_accounts(vec![
            ("heavy7d", 0, 100_000),
            ("light7d", 0, 5_000),
        ]);
        let selected = router.select(RoutingStrategy::Usage).unwrap();
        assert_eq!(selected, "light7d");
    }

    #[test]
    fn test_priority_with_order() {
        let router = make_router_with_accounts(vec![
            ("a", 0, 0),
            ("b", 0, 0),
            ("c", 0, 0),
        ]);
        router.set_priority_order(vec!["c".to_string(), "a".to_string(), "b".to_string()]);
        let selected = router.select(RoutingStrategy::Priority).unwrap();
        assert_eq!(selected, "c");
    }

    #[test]
    fn test_available_count() {
        let router = make_router_with_accounts(vec![("a", 0, 0), ("b", 0, 0)]);
        assert_eq!(router.available_count(), 2);
    }

    #[test]
    fn test_deleted_account_not_routed() {
        let cache = CredentialsCache::empty();
        {
            let mut data = cache.write();
            let mut acc = make_account(0, 0);
            acc.deleted = true;
            data.accounts.insert("deleted".to_string(), acc);
            data.accounts.insert("active".to_string(), make_account(0, 0));
        }
        let router = AccountRouter::new(Arc::clone(&cache));
        let selected = router.select(RoutingStrategy::Priority).unwrap();
        assert_eq!(selected, "active");
    }
}
