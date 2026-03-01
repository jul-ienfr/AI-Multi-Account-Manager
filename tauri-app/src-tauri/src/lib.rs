//! Bibliothèque Tauri — point d'entrée principal.

pub mod commands;
pub mod events;
pub mod state;

use std::sync::Arc;
use tauri::Manager;
use state::AppState;
use commands::*;

/// Construit et configure l'application Tauri.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let state = AppState::init().expect("Failed to initialize AppState");

            // Spawn background quota refresh task
            let credentials = state.credentials.clone();
            let config = state.config.clone();
            let velocity_calcs = state.velocity_calculators.clone();
            let quota_metrics = state.quota_metrics.clone();
            let invalid_grant = state.invalid_grant_accounts.clone();
            let event_log = state.event_log.clone();
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(quota_refresh_loop(
                credentials, config, app_handle, velocity_calcs, quota_metrics, invalid_grant,
                event_log,
            ));

            app.manage(state);

            // Ouvrir les devtools uniquement en mode debug
            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                window.open_devtools();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Comptes
            get_accounts,
            get_active_account,
            switch_account,
            refresh_account,
            add_account,
            update_account,
            delete_account,
            revoke_account,
            // Config
            get_config,
            set_config,
            // Proxy (legacy)
            get_proxy_status,
            start_proxy,
            stop_proxy,
            restart_proxy,
            // Proxy instances (dynamic)
            get_proxy_instances,
            add_proxy_instance,
            update_proxy_instance,
            delete_proxy_instance,
            start_proxy_instance,
            stop_proxy_instance,
            restart_proxy_instance,
            detect_proxy_binaries,
            probe_proxy_instances,
            // Setup injection
            setup_claude_code,
            remove_claude_code_setup,
            setup_vscode_proxy,
            remove_vscode_proxy,
            // Sync
            get_sync_status,
            get_peers,
            add_peer,
            remove_peer,
            generate_sync_key,
            set_sync_key,
            test_peer_connection,
            // SSH Sync
            get_hostname,
            add_ssh_host,
            remove_ssh_host,
            test_ssh_connection,
            // Import local credentials
            scan_local_credentials,
            import_scanned_credentials,
            // OAuth capture via Claude CLI
            find_claude_binary,
            capture_oauth_token,
            // Monitoring
            get_quota_history,
            get_switch_history,
            get_impersonation_profiles,
            get_sessions,
            get_logs,
            // Systemd
            get_systemd_status,
            install_systemd_service,
            uninstall_systemd_service,
            // Webhooks
            test_webhook,
            // Profiles (Phase 6.2)
            list_profiles,
            save_profile,
            load_profile,
            delete_profile,
            // Stats (Phase 6.6)
            get_stats,
            // Phase 3.4a — Capture token roté avant switch
            capture_before_switch,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Background task: fetches quota from Anthropic API every 60s for all accounts.
async fn quota_refresh_loop(
    credentials: Arc<ai_manager_core::credentials::CredentialsCache>,
    config: Arc<ai_manager_core::config::ConfigCache>,
    app_handle: tauri::AppHandle,
    velocity_calculators: Arc<parking_lot::RwLock<std::collections::HashMap<String, ai_manager_core::quota::VelocityCalculator>>>,
    quota_metrics_cache: Arc<parking_lot::RwLock<std::collections::HashMap<String, state::QuotaMetricsCache>>>,
    invalid_grant_accounts: Arc<parking_lot::RwLock<std::collections::HashSet<String>>>,
    event_log: Arc<ai_manager_core::event_log::EventLog>,
) {
    use ai_manager_core::oauth::{self, RefreshResult};
    use ai_manager_core::quota::{VelocityCalculator, load_velocity_states, save_velocity_states};
    use ai_manager_core::stats::StatsManager;
    use ai_manager_core::switch_controller::SwitchController;
    use ai_manager_core::webhook::{WebhookSender, WebhookEvent};
    use events::{QuotaUpdateEvent, QuotaUpdatePayload, PhaseTransitionEvent};
    use tauri::Emitter;

    // Wait 5s before first fetch to let the UI load
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("claude-cli/1.0")
        .build()
        .unwrap_or_default();

    // Restore velocity states from disk
    let multi_account_dir = {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        home.join(".claude").join("multi-account")
    };

    // Stats manager (Phase 6.6)
    let stats_mgr = StatsManager::new(&multi_account_dir);

    {
        let saved_states = load_velocity_states(&multi_account_dir);
        let mut calcs = velocity_calculators.write();
        for (key, state) in &saved_states {
            let calc = calcs.entry(key.clone()).or_insert_with(|| VelocityCalculator::new(state.quota_limit_5h));
            calc.restore_from_state(state);
        }
        if !saved_states.is_empty() {
            tracing::info!("Restored velocity states for {} accounts from disk", saved_states.len());
        }
    }

    // Track last known phases for transition detection
    let mut last_phases: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    // Auto-switch controller
    let mut switch_ctrl = SwitchController::new();

    // Build webhook sender from config
    let webhook_sender = {
        let cfg = config.read();
        WebhookSender::new(cfg.webhooks.clone())
    };

    loop {
        let interval_secs = config.read().refresh_interval_secs.max(30);

        // Phase 3.3 — Migrate Google OAuth slots silently at each cycle
        match credentials.migrate_google_oauth_slots() {
            Ok(0) => {} // nothing to do
            Ok(n) => tracing::debug!("quota_refresh_loop: {} Google OAuth slot(s) migrated", n),
            Err(e) => tracing::debug!("quota_refresh_loop: migrate_google_oauth_slots failed: {}", e),
        }

        let keys = credentials.account_keys();

        for key in &keys {
            let account = match credentials.get_account(key) {
                Some(a) => a,
                None => continue,
            };

            // Skip API key accounts (no OAuth quota)
            if account.account_type.as_deref() == Some("api") {
                continue;
            }

            let oauth_data = match account.get_best_oauth() {
                Some(o) => o.clone(),
                None => continue,
            };

            // Try to fetch quota
            let mut access_token = oauth_data.access_token.clone();
            let quota_result = match oauth::fetch_quota(&client, &access_token).await {
                Ok(q) => {
                    // Quota OK → ce compte est sain, retirer de invalid_grant si présent
                    {
                        let mut ig = invalid_grant_accounts.write();
                        if ig.remove(key) {
                            tracing::info!(
                                "Compte {} retiré de invalid_grant_accounts (quota fetch réussi)",
                                key
                            );
                        }
                    }
                    Some(q)
                }
                Err(e) => {
                    let err_str = e.to_string();
                    // If token expired, try refreshing first
                    if err_str.contains("token_expired") {
                        tracing::debug!("Token expired for {}, attempting refresh", key);
                        match oauth::refresh_oauth_token(&client, &oauth_data.refresh_token).await {
                            RefreshResult::Ok(new_oauth) => {
                                // Refresh réussi → token valide, retirer de invalid_grant
                                {
                                    let mut ig = invalid_grant_accounts.write();
                                    if ig.remove(key) {
                                        tracing::info!(
                                            "Compte {} retiré de invalid_grant_accounts (refresh réussi)",
                                            key
                                        );
                                    }
                                }
                                access_token = new_oauth.access_token.clone();
                                let _ = credentials.update_oauth(key, new_oauth);
                                // Retry quota fetch with new token
                                oauth::fetch_quota(&client, &access_token).await.ok()
                            }
                            RefreshResult::InvalidGrant => {
                                // Token révoqué → webhook + exclure ce compte
                                tracing::warn!(
                                    "invalid_grant pour {} — compte exclu de la rotation",
                                    key
                                );
                                invalid_grant_accounts.write().insert(key.clone());
                                webhook_sender.send(WebhookEvent::TokenRevoked {
                                    key: key.clone(),
                                }).await;
                                None
                            }
                            RefreshResult::Expired => {
                                tracing::warn!("Token expiré (non révoqué) pour {}, réessai plus tard", key);
                                None
                            }
                            RefreshResult::NetworkError(msg) => {
                                tracing::warn!("Token refresh network error pour {}: {}", key, msg);
                                None
                            }
                        }
                    } else {
                        tracing::debug!("Quota fetch failed for {}: {}", key, err_str);
                        None
                    }
                }
            };

            if let Some(quota) = quota_result {
                // Calculate tokens from utilization and limits
                let limit5h = quota.five_hour.as_ref()
                    .and_then(|q| q.limit)
                    .or(account.quota_5h)
                    .unwrap_or(45_000_000);
                let limit7d = quota.seven_day.as_ref()
                    .and_then(|q| q.limit)
                    .unwrap_or(limit5h * 4);

                let util_5h = quota.five_hour.as_ref().map(|q| q.utilization).unwrap_or(0.0);
                let util_7d = quota.seven_day.as_ref().map(|q| q.utilization).unwrap_or(0.0);

                let tokens5h = ((util_5h / 100.0) * limit5h as f64) as u64;
                let tokens7d = ((util_7d / 100.0) * limit7d as f64) as u64;

                // Extract resets_at
                let resets_at_5h = quota.five_hour.as_ref()
                    .and_then(|q| q.resets_at.clone());
                let resets_at_7d = quota.seven_day.as_ref()
                    .and_then(|q| q.resets_at.clone());

                // Update velocity calculator
                let (ema_velocity, ttt, phase_str) = {
                    let mut calcs = velocity_calculators.write();
                    let calc = calcs.entry(key.clone())
                        .or_insert_with(|| VelocityCalculator::new(limit5h));
                    calc.update(tokens5h);
                    let vel = calc.ema_velocity();
                    let ttt = calc.time_to_threshold(tokens5h);
                    let phase = calc.phase(tokens5h);
                    let phase_str = format!("{:?}", phase);
                    (vel, ttt, phase_str)
                };

                // Detect phase transition and emit event + webhook
                let prev_phase = last_phases.get(key).cloned().unwrap_or_else(|| "Cruise".to_string());
                if prev_phase != phase_str {
                    let usage_pct = tokens5h as f64 / limit5h.max(1) as f64 * 100.0;
                    events::emit_phase_transition(&app_handle, PhaseTransitionEvent {
                        key: key.clone(),
                        previous_phase: prev_phase.clone(),
                        new_phase: phase_str.clone(),
                        time_to_threshold: ttt,
                        usage_pct,
                    });
                    tracing::info!("Phase transition for {}: {} → {}", key, prev_phase, phase_str);

                    // Log phase transition (Phase 6.5)
                    event_log.log("INFO", "phase_transition", Some(serde_json::json!({
                        "key": key,
                        "from": prev_phase,
                        "to": phase_str,
                        "time_to_threshold": ttt,
                    })));

                    // Webhook: phase transition
                    webhook_sender.send(WebhookEvent::PhaseTransition {
                        key: key.clone(),
                        from: prev_phase.clone(),
                        to: phase_str.clone(),
                    }).await;

                    // Webhook: quota warning when entering Critical phase (>90%)
                    if phase_str == "Critical" {
                        let pct = tokens5h as f64 / limit5h.max(1) as f64 * 100.0;
                        webhook_sender.send(WebhookEvent::QuotaWarning {
                            key: key.clone(),
                            pct,
                            phase: phase_str.clone(),
                        }).await;
                    }

                    last_phases.insert(key.clone(), phase_str.clone());
                }

                // Cache metrics for get_accounts
                {
                    let mut metrics = quota_metrics_cache.write();
                    metrics.insert(key.clone(), state::QuotaMetricsCache {
                        ema_velocity,
                        time_to_threshold: ttt,
                        resets_at_5h: resets_at_5h.clone(),
                        resets_at_7d: resets_at_7d.clone(),
                    });
                }

                // Update credentials cache
                let _ = credentials.update_quota(key, tokens5h, tokens7d);

                // Store full quota JSON
                if let Ok(quota_json) = serde_json::to_value(&quota) {
                    let mut data = credentials.write();
                    if let Some(acct) = data.accounts.get_mut(key) {
                        acct.quota = Some(quota_json);
                        acct.last_refresh = Some(chrono::Utc::now());
                    }
                }
                let _ = credentials.persist();

                // Convert velocity tokens/min to %/min
                let velocity_pct_min = if limit5h > 0 {
                    (ema_velocity / limit5h as f64) * 100.0
                } else {
                    0.0
                };

                // Emit event to frontend
                let event = QuotaUpdateEvent {
                    key: key.clone(),
                    quota: QuotaUpdatePayload {
                        tokens5h,
                        limit5h,
                        tokens7d,
                        limit7d,
                        phase: Some(phase_str.clone()),
                        ema_velocity: velocity_pct_min,
                        time_to_threshold: ttt,
                        last_updated: Some(chrono::Utc::now().to_rfc3339()),
                        resets_at_5h,
                        resets_at_7d,
                    },
                };
                let _ = app_handle.emit("quota_update", &event);
                tracing::debug!("Quota updated for {}: 5h={:.1}% 7d={:.1}% vel={:.2}%/min", key, util_5h, util_7d, velocity_pct_min);

                // Log quota update (Phase 6.5)
                event_log.log("DEBUG", "quota_update", Some(serde_json::json!({
                    "key": key,
                    "tokens5h": tokens5h,
                    "limit5h": limit5h,
                    "util_5h_pct": util_5h,
                    "phase": phase_str,
                })));

                // Persist quota history snapshot
                let history_path = multi_account_dir.join("quota_history.jsonl");
                let hist_entry = serde_json::json!({
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "key": key,
                    "tokens5h": tokens5h,
                    "tokens7d": tokens7d,
                    "limit5h": limit5h,
                    "limit7d": limit7d,
                    "phase": &phase_str,
                });
                if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(&history_path) {
                    use std::io::Write;
                    let _ = writeln!(f, "{}", hist_entry);
                }
            }

            // Small delay between accounts to avoid rate limits
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        // Persist velocity states after each full cycle
        {
            let calcs = velocity_calculators.read();
            let states: std::collections::HashMap<String, _> = calcs.iter()
                .map(|(k, c)| (k.clone(), c.to_state()))
                .collect();
            save_velocity_states(&multi_account_dir, &states);
        }

        // Auto-switch check
        {
            let cfg = config.read().clone();
            // Mettre à jour la liste des comptes invalid_grant dans le contrôleur
            {
                let ig_snapshot = invalid_grant_accounts.read().clone();
                switch_ctrl.set_invalid_accounts(&ig_snapshot);
            }
            if let Some(decision) = switch_ctrl.try_auto_switch(&credentials, &cfg) {
                let from_key = credentials.active_key().unwrap_or_else(|| "none".to_string());
                tracing::info!("Auto-switch triggered: {} → {} ({})",
                    &from_key,
                    decision.to_key,
                    decision.reason);
                let _ = credentials.write().active_account = Some(decision.to_key.clone());
                let _ = credentials.persist();
                // Record switch in stats + event log (Phase 6.5/6.6)
                stats_mgr.record_switch(&from_key, &decision.to_key);
                event_log.log("INFO", "auto_switch", Some(serde_json::json!({
                    "from": from_key,
                    "to": decision.to_key,
                    "reason": decision.reason.to_string(),
                })));
                switch_ctrl.record_switch();
                events::emit_toast(&app_handle,
                    format!("Switch automatique → {}", decision.to_key),
                    events::ToastKind::Switch);
                let _ = app_handle.emit("account_switch", &decision.to_key);
                // Webhook: auto-switch
                webhook_sender.send(WebhookEvent::AutoSwitch {
                    from: credentials.active_key().unwrap_or_else(|| "none".to_string()),
                    to: decision.to_key.clone(),
                    reason: decision.reason.to_string(),
                }).await;
            } else if let Some(rotation) = switch_ctrl.check_rotation(&credentials, &cfg) {
                let from_key = credentials.active_key().unwrap_or_else(|| "none".to_string());
                tracing::info!("Rotation: → {} ({})", rotation.to_key, rotation.reason);
                let _ = credentials.write().active_account = Some(rotation.to_key.clone());
                let _ = credentials.persist();
                // Record switch in stats + event log (Phase 6.5/6.6)
                stats_mgr.record_switch(&from_key, &rotation.to_key);
                event_log.log("INFO", "account_switch", Some(serde_json::json!({
                    "from": from_key,
                    "to": rotation.to_key,
                    "reason": rotation.reason.to_string(),
                })));
                switch_ctrl.record_switch();
                events::emit_toast(&app_handle,
                    format!("Rotation → {}", rotation.to_key),
                    events::ToastKind::Switch);
                let _ = app_handle.emit("account_switch", &rotation.to_key);
                // Webhook: rotation treated as auto-switch
                webhook_sender.send(WebhookEvent::AutoSwitch {
                    from: credentials.active_key().unwrap_or_else(|| "none".to_string()),
                    to: rotation.to_key.clone(),
                    reason: rotation.reason.to_string(),
                }).await;
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(interval_secs as u64)).await;
    }
}
