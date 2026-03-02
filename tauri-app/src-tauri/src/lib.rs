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

            // Spawn SyncBus si activé
            if let Some(bus) = app.state::<AppState>().sync_bus.read().clone() {
                // Enregistre les credentials pour répondre aux SyncRequest entrants
                bus.set_credentials(app.state::<AppState>().credentials.clone());

                // 1. Démarrer l'écoute TCP entrante
                let bus_for_start = bus.clone();
                tauri::async_runtime::spawn(async move {
                    tracing::info!("[sync] Starting SyncBus listener...");
                    let result = bus_for_start.start().await;
                    let msg = match &result {
                        Ok(()) => "OK: SyncBus listener started".to_string(),
                        Err(e) => format!("FAIL: SyncBus start error: {}", e),
                    };
                    tracing::info!("[sync] {}", msg);
                    // Diagnostic file (visible sans console)
                    if let Some(data_dir) = dirs::data_dir() {
                        let diag = data_dir.join("ai-manager").join("sync-diag.txt");
                        let _ = std::fs::write(&diag, &msg);
                    }
                });

                // 2. Connecter les peers configurés
                let bus_for_peers = bus.clone();
                let peers_to_connect: Vec<(String, String, u16)> = {
                    let app_state_ref = app.state::<AppState>();
                    let app_state = &*app_state_ref;
                    let cfg = app_state.config.read();
                    let v: Vec<(String, String, u16)> = cfg.sync.peers.iter()
                        .map(|p| (p.id.clone(), p.host.clone(), p.port))
                        .collect();
                    v
                };
                tauri::async_runtime::spawn(async move {
                    // Petit délai pour laisser le bus démarrer
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    for (id, host, port) in peers_to_connect {
                        bus_for_peers.connect_peer(&id, &host, port, ai_sync::compat::PeerProtocol::V3).await;
                        tracing::info!("P2P: connecting to peer {}@{}:{}", id, host, port);
                    }
                });

                // 3. Spawn mDNS discovery
                let instance_id = bus.instance_id().to_string();
                let sync_port = {
                    let app_state_ref = app.state::<AppState>();
                    let app_state = &*app_state_ref;
                    let port = app_state.config.read().sync.port;
                    port
                };
                tauri::async_runtime::spawn(async move {
                    match ai_sync::discovery::PeerDiscovery::new(instance_id, sync_port) {
                        Ok(discovery) => {
                            if let Err(e) = discovery.advertise() {
                                tracing::warn!("mDNS advertise failed: {}", e);
                            }
                            let mut rx = discovery.discover();
                            while let Some(event) = rx.recv().await {
                                tracing::debug!("mDNS event: {:?}", event);
                            }
                            tracing::info!("mDNS discovery channel closed");
                        }
                        Err(e) => tracing::warn!("mDNS discovery init failed: {}", e),
                    }
                });

                // 4. Spawn SyncCoordinator — traite les messages entrants et reconcilie les credentials
                let credentials_for_coord = app.state::<AppState>().credentials.clone();
                let instance_id_coord = bus.instance_id().to_string();
                let coordinator = std::sync::Arc::new(ai_sync::coordinator::SyncCoordinator::new(
                    instance_id_coord,
                    bus.clone(),
                    credentials_for_coord,
                ));
                let (coord_shutdown_tx, coord_shutdown_rx) = tokio::sync::watch::channel(false);
                *app.state::<AppState>().sync_coordinator_shutdown.lock() = Some(coord_shutdown_tx);
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = coordinator.run(coord_shutdown_rx).await {
                        tracing::error!("SyncCoordinator error: {}", e);
                    }
                });
                tracing::info!("SyncCoordinator started");

                // 5. Spawn P2P → Tauri event bridge
                //    Écoute les messages sync entrants et émet des événements Tauri
                //    pour que le frontend Svelte se rafraîchisse.
                let bus_for_events = bus.clone();
                let app_for_events = app.handle().clone();
                let creds_for_events = app.state::<AppState>().credentials.clone();
                let local_instance_id = bus.instance_id().to_string();
                tauri::async_runtime::spawn(async move {
                    use ai_sync::messages::SyncPayload;
                    use tauri::Emitter;
                    let mut rx = bus_for_events.subscribe();
                    loop {
                        match rx.recv().await {
                            Ok(msg) => {
                                // Ignorer nos propres messages
                                if msg.from == local_instance_id { continue; }

                                match &msg.payload {
                                    SyncPayload::Credentials { .. } | SyncPayload::SyncResponse { .. } => {
                                        tracing::debug!("[sync-event-bridge] Credentials from {} → sync_refresh", msg.from);
                                        let _ = app_for_events.emit("sync_refresh", ());
                                    }
                                    SyncPayload::AccountSwitch { new_key, .. } => {
                                        tracing::debug!("[sync-event-bridge] AccountSwitch → {}", new_key);
                                        let _ = app_for_events.emit("account_switch", new_key);
                                    }
                                    SyncPayload::QuotaUpdate { account_key, tokens_5h, tokens_7d, .. } => {
                                        let limit5h = creds_for_events.get_account(account_key)
                                            .and_then(|a| a.quota_5h)
                                            .unwrap_or(45_000_000);
                                        let event = events::QuotaUpdateEvent {
                                            key: account_key.clone(),
                                            quota: events::QuotaUpdatePayload {
                                                tokens5h: *tokens_5h,
                                                limit5h: limit5h,
                                                tokens7d: *tokens_7d,
                                                limit7d: limit5h * 4,
                                                phase: None,
                                                ema_velocity: 0.0,
                                                time_to_threshold: None,
                                                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                                                resets_at_5h: None,
                                                resets_at_7d: None,
                                            },
                                        };
                                        let _ = app_for_events.emit("quota_update", &event);
                                    }
                                    SyncPayload::ConfigUpdate { .. }
                                    | SyncPayload::InvalidGrantUpdate { .. }
                                    | SyncPayload::ProfileUpdate { .. }
                                    | SyncPayload::ProxyConfigUpdate { .. }
                                    | SyncPayload::SshHostUpdate { .. }
                                    | SyncPayload::PeerConfigUpdate { .. } => {
                                        tracing::debug!("[sync-event-bridge] {} from {} → sync_refresh", msg.payload.variant_name(), msg.from);
                                        let _ = app_for_events.emit("sync_refresh", ());
                                    }
                                    _ => {} // Heartbeat, Handshake, etc.
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!("[sync-event-bridge] lagged {} messages", n);
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                tracing::info!("[sync-event-bridge] bus closed, stopping");
                                break;
                            }
                        }
                    }
                });
                tracing::info!("[sync-event-bridge] started");
            }

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
            toggle_sync,
            set_sync_port,
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
            // Gemini OAuth 2.0 PKCE
            gemini_oauth_flow,
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

            // Skip non-Anthropic accounts (Google/Gemini don't have Claude quota)
            if account.effective_provider() != "anthropic" {
                continue;
            }

            // Split quota fetch : si ce compte a été rafraîchi récemment par un pair
            // (last_refresh mis à jour par handle_quota_update), on évite de re-fetch.
            if config.read().sync.split_quota_fetch {
                if let Some(lr) = account.last_refresh {
                    let age_secs = chrono::Utc::now()
                        .signed_duration_since(lr)
                        .num_seconds();
                    let threshold = interval_secs.saturating_sub(5) as i64;
                    if age_secs < threshold {
                        tracing::debug!(
                            "Split quota fetch: skipping {} (refreshed {}s ago by peer)",
                            key, age_secs
                        );
                        continue;
                    }
                }
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
                                // Broadcast le nouveau token aux pairs P2P
                                if let Some(bus) = app_handle.try_state::<AppState>()
                                    .and_then(|s| s.sync_bus.read().clone())
                                {
                                    let instance_id = bus.instance_id().to_string();
                                    let creds_clone = credentials.clone();
                                    tauri::async_runtime::spawn(async move {
                                        if let Ok(accounts_json) = creds_clone.export_json() {
                                            let active_key = creds_clone.active_key();
                                            let clock = bus.next_clock();
                                            let msg = ai_sync::messages::SyncMessage::new(
                                                &instance_id,
                                                ai_sync::messages::SyncPayload::Credentials {
                                                    accounts_json,
                                                    active_key,
                                                    clock,
                                                },
                                            );
                                            if let Err(e) = bus.broadcast(msg).await {
                                                tracing::debug!("Credentials broadcast after token refresh failed: {}", e);
                                            }
                                        }
                                    });
                                }
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
                                // Broadcast invalid_grant update to P2P peers
                                if let Some(bus) = app_handle.try_state::<crate::state::AppState>().and_then(|s| s.sync_bus.read().clone()) {
                                    let ig_snapshot: Vec<String> = invalid_grant_accounts.read().iter().cloned().collect();
                                    tokio::spawn(async move {
                                        let instance_id = bus.instance_id().to_string();
                                        let clock = bus.next_clock();
                                        let msg = ai_sync::messages::SyncMessage::new(
                                            &instance_id,
                                            ai_sync::messages::SyncPayload::InvalidGrantUpdate { invalid_keys: ig_snapshot, clock },
                                        );
                                        let _ = bus.broadcast(msg).await;
                                    });
                                }
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

                // Broadcast QuotaUpdate aux pairs P2P
                if let Some(bus) = app_handle.try_state::<AppState>()
                    .and_then(|s| s.sync_bus.read().clone())
                {
                    let instance_id = bus.instance_id().to_string();
                    let msg = ai_sync::messages::SyncMessage::new(
                        &instance_id,
                        ai_sync::messages::SyncPayload::QuotaUpdate {
                            account_key: key.clone(),
                            tokens_5h: tokens5h,
                            tokens_7d: tokens7d,
                            clock: bus.next_clock(),
                        },
                    );
                    let bus_clone = bus.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = bus_clone.broadcast(msg).await {
                            tracing::debug!("QuotaUpdate broadcast failed: {}", e);
                        }
                    });
                }

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
                // Broadcast AccountSwitch to P2P peers
                if let Some(bus) = app_handle.try_state::<AppState>()
                    .and_then(|s| s.sync_bus.read().clone())
                {
                    let key_clone = decision.to_key.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = bus.broadcast_account_switch(key_clone).await;
                    });
                }
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
                // Broadcast AccountSwitch to P2P peers
                if let Some(bus) = app_handle.try_state::<AppState>()
                    .and_then(|s| s.sync_bus.read().clone())
                {
                    let key_clone = rotation.to_key.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = bus.broadcast_account_switch(key_clone).await;
                    });
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(interval_secs as u64)).await;
    }
}
