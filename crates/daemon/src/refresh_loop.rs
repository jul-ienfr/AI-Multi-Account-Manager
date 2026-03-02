//! Boucle périodique de refresh OAuth.
//!
//! Traduit la logique Python `_do_periodic_refresh_locked` de `headless_daemon.py`
//! en Rust async. Rafraîchit tous les tokens proches de l'expiration toutes les
//! `interval_secs` secondes.
//!
//! ## Phase 3.4f — Nettoyage des cooldowns après rotation
//!
//! Quand un refresh retourne un nouveau refresh_token (rotation), l'ancien RT
//! est supprimé du `CooldownMap` (HashMap<sha256(RT) → Instant>).
//! Cela évite que l'ancien RT reste en cooldown alors qu'il ne sera plus
//! jamais réutilisé.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, error, info, warn};

use ai_core::credentials::{AccountData, CredentialsCache};
use ai_core::oauth::{needs_refresh, refresh_oauth_token, RefreshResult};

/// Clé de cooldown : sha256(refresh_token)[:16] en hexadécimal.
///
/// Reproduit la logique Python `sha256(rt)[:32]` (32 nibbles = 16 bytes).
fn cooldown_key(refresh_token: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(refresh_token.as_bytes());
    hex::encode(&hash[..16])
}

/// Map cooldown : clé = sha256(RT)[:16], valeur = instant de début de cooldown.
type CooldownMap = HashMap<String, std::time::Instant>;

/// Boucle périodique de refresh OAuth.
///
/// Tourne indéfiniment jusqu'à ce que `shutdown` reçoive `true`.
pub struct RefreshLoop {
    credentials: Arc<CredentialsCache>,
    http_client: reqwest::Client,
    interval_secs: u64,
}

impl RefreshLoop {
    /// Crée une nouvelle boucle de refresh.
    pub fn new(credentials: Arc<CredentialsCache>, interval_secs: u64) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("claude-cli/1.0")
            .build()
            .unwrap_or_default();
        Self {
            credentials,
            http_client,
            interval_secs,
        }
    }

    /// Lance la boucle de refresh.
    ///
    /// S'arrête quand `shutdown.changed()` reçoit `true`.
    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(Duration::from_secs(self.interval_secs));
        // La première tick est immédiate — on la consume pour éviter un refresh au démarrage
        interval.tick().await;

        // Phase 3.4f — map de cooldown locale à la boucle.
        // Clé : sha256(RT)[:16].  Valeur : instant de mise en cooldown.
        // Durée de cooldown : 5 minutes (aligné sur la logique Python).
        let mut cooldowns: CooldownMap = HashMap::new();
        const COOLDOWN_DURATION: Duration = Duration::from_secs(5 * 60);

        info!(
            "RefreshLoop started (interval={}s)",
            self.interval_secs
        );

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Purge les cooldowns expirés avant chaque cycle
                    let now = std::time::Instant::now();
                    cooldowns.retain(|_k, t| now.duration_since(*t) < COOLDOWN_DURATION);

                    let refreshed = self.refresh_all(&mut cooldowns).await;
                    if refreshed > 0 {
                        info!("RefreshLoop: {} token(s) refreshed", refreshed);
                    } else {
                        debug!("RefreshLoop: no tokens needed refresh");
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("RefreshLoop: shutdown signal received");
                        break;
                    }
                }
            }
        }

        info!("RefreshLoop stopped");
    }

    /// Rafraîchit tous les tokens expirés ou proches de l'expiration.
    ///
    /// Retourne le nombre de tokens effectivement rafraîchis.
    async fn refresh_all(&self, cooldowns: &mut CooldownMap) -> usize {
        let keys = self.credentials.account_keys();
        let mut refreshed_count = 0;

        for key in keys {
            let account = match self.credentials.get_account(&key) {
                Some(a) => a,
                None => continue,
            };

            if account.deleted {
                continue;
            }

            if self.refresh_account(&key, &account, cooldowns).await {
                refreshed_count += 1;
            }
        }

        refreshed_count
    }

    /// Rafraîchit le token d'un compte si nécessaire.
    ///
    /// Retourne `true` si le token a été rafraîchi.
    async fn refresh_account(
        &self,
        key: &str,
        account: &AccountData,
        cooldowns: &mut CooldownMap,
    ) -> bool {
        let oauth = match &account.oauth {
            Some(o) => o,
            None => {
                debug!("Account {} has no OAuth data, skipping", key);
                return false;
            }
        };

        // Phase 3.4f — vérifie le cooldown sur l'ancien RT avant de tenter le refresh
        let ck = cooldown_key(&oauth.refresh_token);
        if cooldowns.contains_key(&ck) {
            debug!("Account {} RT is in cooldown, skipping refresh", key);
            return false;
        }

        // Vérifie si un refresh est nécessaire
        if !needs_refresh(oauth) {
            debug!("Account {} token does not need refresh", key);
            return false;
        }

        debug!("Refreshing token for account {}", key);

        // Conserve le RT actuel pour nettoyage cooldown en cas d'invalidation
        let old_rt = oauth.refresh_token.clone();

        match refresh_oauth_token(&self.http_client, &oauth.refresh_token).await {
            RefreshResult::Ok(new_oauth) => {
                // Phase 3.4f — si le RT a tourné (nouveau RT différent de l'ancien),
                // on supprime l'entrée de cooldown de l'ancien RT.
                // Cela évite qu'un ancien RT révoqué génère un faux cooldown persistant.
                if new_oauth.refresh_token != old_rt {
                    let old_ck = cooldown_key(&old_rt);
                    if cooldowns.remove(&old_ck).is_some() {
                        debug!(
                            "RefreshLoop: removed cooldown for rotated RT (account {})",
                            key
                        );
                    }
                    debug!(
                        "RefreshLoop: RT rotation detected for account {} — old RT cooldown cleared",
                        key
                    );
                }

                match self.credentials.update_oauth(key, new_oauth) {
                    Ok(_) => {
                        info!("Token refreshed for account {}", key);
                        true
                    }
                    Err(e) => {
                        error!("Failed to persist refreshed token for {}: {}", key, e);
                        false
                    }
                }
            }
            RefreshResult::InvalidGrant => {
                // invalid_grant — token révoqué.
                // Phase 3.4f : met l'ancien RT en cooldown pour éviter des boucles de retry.
                warn!(
                    "Token refresh for {} returned invalid_grant (token may be revoked) — adding to cooldown",
                    key
                );
                cooldowns.insert(ck, std::time::Instant::now());
                false
            }
            RefreshResult::Expired => {
                warn!("Token refresh for {}: token expiré (non révoqué), réessai plus tard", key);
                false
            }
            RefreshResult::NetworkError(msg) => {
                warn!("Token refresh failed for {}: {}", key, msg);
                false
            }
        }
    }

    /// Retourne l'intervalle de refresh en secondes.
    pub fn interval_secs(&self) -> u64 {
        self.interval_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_core::credentials::{AccountData, CredentialsCache, OAuthData};
    use chrono::Utc;

    fn make_credentials_with_token(expires_minutes: i64) -> Arc<CredentialsCache> {
        let cache = CredentialsCache::empty();
        {
            let mut data = cache.write();
            data.accounts.insert(
                "acc1".to_string(),
                AccountData {
                    name: Some("Test".to_string()),
                    email: Some("test@example.com".to_string()),
                    oauth: Some(OAuthData {
                        access_token: "tok".to_string(),
                        refresh_token: "ref_tok".to_string(),
                        expires_at: Some(
                            Utc::now() + chrono::Duration::minutes(expires_minutes),
                        ),
                        token_type: Some("Bearer".to_string()),
                        scope: None,
                        scopes: None,
                        refresh_token_expires_at: None,
                        organization_uuid: None,
                    }),
                    ..Default::default()
                },
            );
        }
        cache
    }

    #[test]
    fn test_refresh_loop_new() {
        let cache = CredentialsCache::empty();
        let rl = RefreshLoop::new(cache, 60);
        assert_eq!(rl.interval_secs(), 60);
    }

    #[tokio::test]
    async fn test_refresh_all_no_accounts() {
        let cache = CredentialsCache::empty();
        let rl = RefreshLoop::new(cache, 60);
        let mut cooldowns = CooldownMap::new();
        // Aucun compte → 0 refresh
        let count = rl.refresh_all(&mut cooldowns).await;
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_refresh_account_no_oauth() {
        let cache = CredentialsCache::empty();
        {
            let mut data = cache.write();
            data.accounts.insert(
                "acc-notoken".to_string(),
                AccountData {
                    name: Some("NoToken".to_string()),
                    ..Default::default()
                },
            );
        }
        let rl = RefreshLoop::new(Arc::clone(&cache), 60);
        // Pas de token → false
        let account = cache.get_account("acc-notoken").unwrap();
        let mut cooldowns = CooldownMap::new();
        let refreshed = rl.refresh_account("acc-notoken", &account, &mut cooldowns).await;
        assert!(!refreshed);
    }

    #[tokio::test]
    async fn test_refresh_account_not_needed() {
        // Token expire dans 2 heures → pas de refresh nécessaire
        let cache = make_credentials_with_token(120);
        let rl = RefreshLoop::new(Arc::clone(&cache), 60);
        let account = cache.get_account("acc1").unwrap();
        let mut cooldowns = CooldownMap::new();
        let refreshed = rl.refresh_account("acc1", &account, &mut cooldowns).await;
        // Pas de refresh (pas expiré dans 30 min)
        assert!(!refreshed);
    }

    #[tokio::test]
    async fn test_refresh_all_skips_deleted() {
        let cache = CredentialsCache::empty();
        {
            let mut data = cache.write();
            data.accounts.insert(
                "acc-deleted".to_string(),
                AccountData {
                    deleted: true,
                    oauth: Some(OAuthData {
                        access_token: "tok".to_string(),
                        refresh_token: "ref".to_string(),
                        expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
                        token_type: None,
                        scope: None,
                        scopes: None,
                        refresh_token_expires_at: None,
                        organization_uuid: None,
                    }),
                    ..Default::default()
                },
            );
        }
        let rl = RefreshLoop::new(Arc::clone(&cache), 60);
        let mut cooldowns = CooldownMap::new();
        let count = rl.refresh_all(&mut cooldowns).await;
        // Compte supprimé → ignoré
        assert_eq!(count, 0);
    }

    #[test]
    fn test_cooldown_key_stable() {
        // La même clé doit toujours produire le même hash
        let rt = "sk-ant-ort01-test-refresh-token";
        let k1 = cooldown_key(rt);
        let k2 = cooldown_key(rt);
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 32, "sha256[:16] = 32 hex chars");
    }

    #[test]
    fn test_cooldown_key_different_for_different_tokens() {
        let k1 = cooldown_key("refresh_token_A");
        let k2 = cooldown_key("refresh_token_B");
        assert_ne!(k1, k2);
    }

    #[tokio::test]
    async fn test_refresh_account_in_cooldown_skipped() {
        // Un compte dont le RT est en cooldown ne doit pas être refreshé
        let cache = make_credentials_with_token(5); // expire dans 5 min → needs_refresh
        let rl = RefreshLoop::new(Arc::clone(&cache), 60);
        let account = cache.get_account("acc1").unwrap();
        let mut cooldowns = CooldownMap::new();

        // Met le RT en cooldown
        let ck = cooldown_key("ref_tok");
        cooldowns.insert(ck, std::time::Instant::now());

        let refreshed = rl.refresh_account("acc1", &account, &mut cooldowns).await;
        assert!(!refreshed, "Token en cooldown ne doit pas être refreshé");
    }

    #[tokio::test]
    async fn test_run_shutdown_immediately() {
        let cache = CredentialsCache::empty();
        let rl = Arc::new(RefreshLoop::new(cache, 3600)); // Intervalle très long

        let (tx, rx) = tokio::sync::watch::channel(false);

        let rl_clone = Arc::clone(&rl);
        let handle = tokio::spawn(async move {
            rl_clone.run(rx).await;
        });

        // Envoie le signal d'arrêt immédiatement
        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send(true).unwrap();

        // La boucle doit s'arrêter rapidement
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("RefreshLoop should stop within 2s")
            .expect("task should not panic");
    }
}
