//! CRUD comptes Claude — sélection active, validation, priorités.
//!
//! Couche métier au-dessus de `CredentialsCache`.
//! Toutes les opérations passent par cette couche pour garantir
//! la cohérence (pas de doublons, validation, persistance).

use std::sync::Arc;

use tracing::info;

use crate::credentials::{AccountData, CredentialsCache, OAuthData};
use crate::error::{CoreError, Result};
use crate::types::QuotaInfo;

/// Service de gestion des comptes.
pub struct AccountService {
    credentials: Arc<CredentialsCache>,
}

impl AccountService {
    /// Crée un nouveau service de comptes.
    pub fn new(credentials: Arc<CredentialsCache>) -> Self {
        Self { credentials }
    }

    // -----------------------------------------------------------------------
    // CRUD
    // -----------------------------------------------------------------------

    /// Retourne tous les comptes actifs (non supprimés).
    pub fn list_accounts(&self) -> Vec<(String, AccountData)> {
        let data = self.credentials.read();
        data.accounts
            .iter()
            .filter(|(_, a)| !a.deleted)
            .map(|(k, a)| (k.clone(), a.clone()))
            .collect()
    }

    /// Retourne un compte par clé.
    pub fn get_account(&self, key: &str) -> Option<AccountData> {
        self.credentials.get_account(key)
    }

    /// Retourne la clé du compte actif.
    pub fn active_key(&self) -> Option<String> {
        self.credentials.active_key()
    }

    /// Retourne le compte actif.
    pub fn active_account(&self) -> Option<AccountData> {
        self.credentials.active_account()
    }

    /// Ajoute ou met à jour un compte.
    ///
    /// Si la clé existe déjà, merge les données (LWW pour OAuth).
    /// Retourne `Err(NotFound)` si la clé est vide.
    pub fn upsert_account(&self, key: &str, data: AccountData) -> Result<()> {
        if key.is_empty() {
            return Err(CoreError::Config("Account key cannot be empty".to_string()));
        }
        validate_account_data(&data)?;
        {
            let mut creds = self.credentials.write();
            creds.accounts.insert(key.to_string(), data);
        }
        self.credentials.persist()?;
        info!("Account upserted: {}", key);
        Ok(())
    }

    /// Supprime un compte (soft-delete).
    ///
    /// Si le compte était actif, le compte actif est mis à None.
    pub fn delete_account(&self, key: &str) -> Result<()> {
        {
            let mut creds = self.credentials.write();
            let account = creds.accounts.get_mut(key).ok_or_else(|| {
                CoreError::NotFound(format!("Account not found: {}", key))
            })?;
            account.deleted = true;
            // Si c'était le compte actif, le désactiver
            if creds.active_account.as_deref() == Some(key) {
                creds.active_account = None;
            }
        }
        self.credentials.persist()?;
        info!("Account soft-deleted: {}", key);
        Ok(())
    }

    /// Change le compte actif.
    ///
    /// Retourne `Err(NotFound)` si le compte n'existe pas ou est supprimé.
    pub fn switch_account(&self, key: &str) -> Result<()> {
        {
            let mut creds = self.credentials.write();
            let account = creds.accounts.get(key).ok_or_else(|| {
                CoreError::NotFound(format!("Account not found: {}", key))
            })?;
            if account.deleted {
                return Err(CoreError::NotFound(format!(
                    "Account {} is deleted",
                    key
                )));
            }
            creds.active_account = Some(key.to_string());
        }
        self.credentials.persist()?;
        info!("Switched to account: {}", key);
        Ok(())
    }

    /// Met à jour les données OAuth d'un compte.
    pub fn update_oauth(&self, key: &str, oauth: OAuthData) -> Result<()> {
        self.credentials.update_oauth(key, oauth)
    }

    /// Met à jour les quotas d'un compte.
    pub fn update_quota(&self, key: &str, tokens_5h: u64, tokens_7d: u64) -> Result<()> {
        self.credentials.update_quota(key, tokens_5h, tokens_7d)
    }

    // -----------------------------------------------------------------------
    // Routing helpers
    // -----------------------------------------------------------------------

    /// Sélectionne le meilleur compte selon la stratégie "quota-aware".
    ///
    /// Retourne la clé du compte avec le plus de quota restant (5h).
    pub fn best_by_quota(&self) -> Option<String> {
        let data = self.credentials.read();
        data.accounts
            .iter()
            .filter(|(_, a)| !a.deleted && a.has_valid_token())
            .max_by(|(_, a), (_, b)| {
                let ra = quota_remaining_5h(a);
                let rb = quota_remaining_5h(b);
                ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(k, _)| k.clone())
    }

    /// Retourne les comptes ordonnés par quota restant décroissant.
    pub fn accounts_by_quota(&self) -> Vec<String> {
        let mut accounts: Vec<(String, f64)> = {
            let data = self.credentials.read();
            data.accounts
                .iter()
                .filter(|(_, a)| !a.deleted && a.has_valid_token())
                .map(|(k, a)| (k.clone(), quota_remaining_5h(a)))
                .collect()
        };
        accounts.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        accounts.into_iter().map(|(k, _)| k).collect()
    }

    /// Nombre de comptes actifs (non supprimés, token valide).
    pub fn active_count(&self) -> usize {
        let data = self.credentials.read();
        data.accounts
            .values()
            .filter(|a| !a.deleted && a.has_valid_token())
            .count()
    }

    /// Construit un `QuotaInfo` pour un compte donné.
    pub fn quota_info(&self, key: &str) -> Option<QuotaInfo> {
        let account = self.credentials.get_account(key)?;
        Some(QuotaInfo {
            tokens_5h: account.tokens_5h,
            limit_5h: account.quota_5h.unwrap_or(90_000),
            tokens_7d: account.tokens_7d,
            limit_7d: 0, // Pas de limite 7d connue statiquement
            phase: None,
            ema_velocity: 0.0,
            time_to_threshold: None,
            last_updated: account.last_refresh,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Quota 5h restant comme fraction (0.0 = épuisé, 1.0 = plein).
fn quota_remaining_5h(account: &AccountData) -> f64 {
    let limit = account.quota_5h.unwrap_or(90_000);
    if limit == 0 {
        return 1.0;
    }
    (1.0 - account.tokens_5h as f64 / limit as f64).max(0.0)
}

/// Valide les données d'un compte avant insertion.
fn validate_account_data(data: &AccountData) -> Result<()> {
    if let Some(oauth) = &data.oauth {
        if oauth.access_token.is_empty() {
            return Err(CoreError::Config(
                "OAuth access_token cannot be empty".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crate::credentials::{AccountData, CredentialsCache, OAuthData};

    fn make_service() -> AccountService {
        AccountService::new(CredentialsCache::empty())
    }

    fn make_account(name: &str, tokens_5h: u64) -> AccountData {
        AccountData {
            name: Some(name.to_string()),
            email: Some(format!("{}@example.com", name)),
            oauth: Some(OAuthData {
                access_token: format!("tok_{}", name),
                refresh_token: format!("ref_{}", name),
                expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
                token_type: Some("Bearer".to_string()),
                scope: None,
                scopes: None,
                refresh_token_expires_at: None,
                organization_uuid: None,
            }),
            tokens_5h,
            tokens_7d: tokens_5h * 7,
            quota_5h: Some(90_000),
            ..Default::default()
        }
    }

    #[test]
    fn test_upsert_and_get() {
        let svc = make_service();
        svc.upsert_account("acc1", make_account("Alice", 1000)).unwrap();
        let acc = svc.get_account("acc1").unwrap();
        assert_eq!(acc.name, Some("Alice".to_string()));
    }

    #[test]
    fn test_upsert_empty_key_fails() {
        let svc = make_service();
        let result = svc.upsert_account("", make_account("Alice", 0));
        assert!(result.is_err());
    }

    #[test]
    fn test_switch_account() {
        let svc = make_service();
        svc.upsert_account("acc1", make_account("Alice", 1000)).unwrap();
        svc.switch_account("acc1").unwrap();
        assert_eq!(svc.active_key(), Some("acc1".to_string()));
    }

    #[test]
    fn test_switch_nonexistent_account_fails() {
        let svc = make_service();
        let result = svc.switch_account("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_account() {
        let svc = make_service();
        svc.upsert_account("acc1", make_account("Alice", 0)).unwrap();
        svc.switch_account("acc1").unwrap();
        svc.delete_account("acc1").unwrap();
        // After delete, account is marked deleted and active is cleared
        assert!(svc.active_key().is_none());
        let keys: Vec<_> = svc.list_accounts().into_iter().map(|(k,_)| k).collect();
        assert!(!keys.contains(&"acc1".to_string()));
    }

    #[test]
    fn test_best_by_quota() {
        let svc = make_service();
        svc.upsert_account("low", make_account("Low", 80_000)).unwrap();
        svc.upsert_account("high", make_account("High", 10_000)).unwrap();
        // "high" has more quota remaining (10k used vs 80k used out of 90k)
        assert_eq!(svc.best_by_quota(), Some("high".to_string()));
    }

    #[test]
    fn test_accounts_by_quota_ordering() {
        let svc = make_service();
        svc.upsert_account("a", make_account("A", 80_000)).unwrap();
        svc.upsert_account("b", make_account("B", 50_000)).unwrap();
        svc.upsert_account("c", make_account("C", 10_000)).unwrap();
        let ordered = svc.accounts_by_quota();
        // c has least tokens used → most remaining → comes first
        assert_eq!(ordered[0], "c");
    }

    #[test]
    fn test_active_count() {
        let svc = make_service();
        assert_eq!(svc.active_count(), 0);
        svc.upsert_account("acc1", make_account("Alice", 0)).unwrap();
        svc.upsert_account("acc2", make_account("Bob", 0)).unwrap();
        assert_eq!(svc.active_count(), 2);
    }

    #[test]
    fn test_list_accounts_excludes_deleted() {
        let svc = make_service();
        svc.upsert_account("acc1", make_account("Alice", 0)).unwrap();
        svc.upsert_account("acc2", make_account("Bob", 0)).unwrap();
        svc.delete_account("acc1").unwrap();
        let accounts = svc.list_accounts();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].0, "acc2");
    }

    #[test]
    fn test_quota_info() {
        let svc = make_service();
        svc.upsert_account("acc1", make_account("Alice", 45_000)).unwrap();
        let qi = svc.quota_info("acc1").unwrap();
        assert_eq!(qi.tokens_5h, 45_000);
        assert_eq!(qi.limit_5h, 90_000);
        assert!((qi.usage_pct_5h() - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_validate_empty_access_token_fails() {
        let svc = make_service();
        let bad_account = AccountData {
            oauth: Some(OAuthData {
                access_token: "".to_string(),
                refresh_token: "ref".to_string(),
                expires_at: None,
                token_type: None,
                scope: None,
                scopes: None,
                refresh_token_expires_at: None,
                organization_uuid: None,
            }),
            ..Default::default()
        };
        let result = svc.upsert_account("bad", bad_account);
        assert!(result.is_err());
    }
}
