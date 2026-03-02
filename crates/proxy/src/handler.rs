//! Handlers Axum pour le proxy avec impersonation.
//!
//! Copie la logique de claude-translator/src/server.rs et ajoute :
//! - Détection du client via `client_signatures`
//! - Auto-capture via `cc_profile`
//! - Reconstruction headers via `impersonation`
//! - Réécriture body via `body_rewriter`
//! - Résolution modèle via `model_mapping`
//! - Rate limiting par IP
//! - API usage tracking (JSONL)
//! - Session writing pour la GUI
//! - Validation sortante (whitelist)

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use serde_json::{json, Value};
use tracing::{debug, error, info, warn};

use crate::impersonation::{self, ImpersonationState};
use crate::model_mapping;

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AppState {
    /// Client HTTP reqwest (TLS + timeout)
    pub client: reqwest::Client,
    #[allow(dead_code)]
    pub timeout_secs: u64,
    pub upstream_url: String,
    /// Cache des credentials (repris de claude-translator)
    pub credentials: Arc<crate::credentials::CredentialsCache>,
    /// État de l'impersonation
    pub imp: Arc<ImpersonationState>,
    /// Config overrides des modèles
    pub model_config: Arc<model_mapping::ModelMappingConfig>,
    /// Shutdown channel
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// Quota info par provider (extrait des réponses upstream).
    /// Utilise `parking_lot::Mutex` (non empoisonnable) pour éviter les panics
    /// en cascade si un thread panic en tenant ce verrou.
    pub provider_quota: Arc<parking_lot::Mutex<std::collections::HashMap<String, Value>>>,
    /// Verbose logging (headers + body)
    pub verbose: Arc<AtomicBool>,
    /// Rate limiter par IP
    pub rate_limiter: Arc<crate::rate_limiter::RateLimiter>,
    /// API usage tracker (JSONL)
    pub api_usage: Arc<crate::api_usage::ApiUsageTracker>,
    /// Session writer pour la GUI
    pub session_writer: Arc<tokio::sync::Mutex<crate::session_writer::SessionWriter>>,
    /// Dernière IP cliente vue
    pub last_client_ip: Arc<parking_lot::Mutex<String>>,
}

// ---------------------------------------------------------------------------
// Endpoints internes /_proxy/*
// ---------------------------------------------------------------------------

pub async fn proxy_health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "backend": "rust-auto" }))
}

pub async fn proxy_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let account = state.credentials.get_active();

    // Snapshot atomique : un seul verrou pour les 4 champs last_*.
    // Ordre d'acquisition fixe et documenté : meta → last_client_ip → provider_quota.
    let meta_snapshot = state.imp.meta.lock().clone();
    let last_client_ip = state.last_client_ip.lock().clone();
    let quota = state.provider_quota.lock().clone();

    Json(json!({
        "status": "running",
        "type": "router",
        "backend": "rust-auto",
        "version": "0.1.0",
        "active_email": account.as_ref().map(|a| a.email.as_str()),
        "has_token": account.as_ref().map(|a| !a.token.is_empty()).unwrap_or(false),
        "account_type": account.as_ref().map(|a| a.account_type.as_str()),
        "provider": account.as_ref().map(|a| a.provider.as_str()),
        "api_url": account.as_ref().and_then(|a| a.api_url.as_deref()),
        "impersonation_enabled": state.imp.enabled,
        "auto_capture_enabled": state.imp.auto_capture,
        "client_format": meta_snapshot.last_client_format,
        "server_format": meta_snapshot.last_server_format,
        "last_model": meta_snapshot.last_model,
        "last_client": meta_snapshot.last_client,
        "last_client_ip": last_client_ip,
        "provider_quota": quota,
    }))
}

pub async fn proxy_shutdown(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    info!("shutdown requested via /_proxy/shutdown");
    let _ = state.shutdown_tx.send(true);
    Json(json!({ "status": "shutting_down" }))
}

pub async fn proxy_profiles(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let guard = state.imp.profiles.read().unwrap();
    let profiles: Vec<Value> = guard
        .iter()
        .map(|(provider, entry)| {
            let sh = entry.data.static_headers.len();
            let dh = entry.data.dynamic_headers.len();
            json!({
                "provider": provider,
                "request_count": entry.data.request_count,
                "last_capture": entry.data.last_capture,
                "captured_at": entry.data.last_capture,
                "static_headers": sh,
                "dynamic_headers": dh,
                "header_count": sh + dh,
                "has_tools_cache_control": entry.data.has_tools_cache_control,
                "always_streams": entry.data.always_streams,
            })
        })
        .collect();
    Json(json!({ "profiles": profiles }))
}

pub async fn proxy_flush(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let n = crate::cc_profile::flush_cache_count(&state.imp.profiles);
    Json(json!({ "flushed": n }))
}

/// POST /_proxy/verbose — set or toggle verbose logging.
///
/// Body (optional JSON): `{"enabled": true}` → force on/off.
/// Without body (or without "enabled" field) → toggle current state.
pub async fn proxy_verbose_toggle(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
) -> impl IntoResponse {
    let body_bytes = match read_body(req.into_body()).await {
        Ok(b) => b,
        Err(_) => Bytes::new(),
    };
    let explicit_enabled: Option<bool> = if !body_bytes.is_empty() {
        serde_json::from_slice::<Value>(&body_bytes)
            .ok()
            .and_then(|v| v.get("enabled").and_then(|e| e.as_bool()))
    } else {
        None
    };
    let new_val = match explicit_enabled {
        Some(v) => v,
        None => !state.verbose.load(Ordering::Relaxed),
    };
    state.verbose.store(new_val, Ordering::Relaxed);
    info!("Verbose logging: {}", if new_val { "ON" } else { "OFF" });
    Json(json!({ "verbose": new_val }))
}

/// GET /_proxy/api/usage — aggregated API usage stats
///
/// Query params: ?email=...&days=7&by=day|model
pub async fn proxy_api_usage(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
) -> impl IntoResponse {
    let uri = req.uri().clone();
    // Parse query params manually (no url crate dependency)
    let params: std::collections::HashMap<String, String> = uri
        .query()
        .map(|q| {
            q.split('&')
                .filter_map(|pair| {
                    let mut it = pair.splitn(2, '=');
                    let k = it.next()?.to_string();
                    let v = it.next().unwrap_or("").to_string();
                    Some((k, v))
                })
                .collect()
        })
        .unwrap_or_default();

    let email = params.get("email").map(|s| s.as_str());
    let days: u32 = params
        .get("days")
        .and_then(|d| d.parse().ok())
        .unwrap_or(7);
    let group_by = params.get("by").map(|s| s.as_str()).unwrap_or("day");

    let stats = state.api_usage.get_stats(email, days, group_by);
    Json(stats)
}

// ---------------------------------------------------------------------------
// Handler principal : proxy avec impersonation
// ---------------------------------------------------------------------------

const MAX_BODY_SIZE: usize = 50 * 1024 * 1024;

pub async fn handle_proxy(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    req: Request<Body>,
) -> impl IntoResponse {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();

    // --- STEP 1: Extract client_ip ---
    let client_ip: String = addr.ip().to_string();

    // --- STEP 2: Rate limit check ---
    if !state.rate_limiter.check(&client_ip) {
        return error_response(429, &format!(
            "Too many requests from {} (max 100 req/s)", client_ip
        ));
    }

    // --- STEP 3: Detect client_fmt from path ---
    let client_fmt = if path.contains("/v1/chat/completions") {
        "openai"
    } else if path.contains("/v1/messages") {
        "anthropic"
    } else if path.contains("/v1beta/models/") || path.contains(":generateContent") {
        "gemini"
    } else {
        "anthropic"
    };

    // --- STEP 4: Read body bytes and parse JSON ---
    let raw_headers_map: std::collections::HashMap<String, String> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_lowercase(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let body_bytes = match read_body(req.into_body()).await {
        Ok(b) => b,
        Err(e) => {
            error!("handle_proxy: body read error: {e}");
            return error_response(400, "body read error");
        }
    };

    let mut body_json: Value = if !body_bytes.is_empty() && method == Method::POST {
        match serde_json::from_slice(&body_bytes) {
            Ok(v) => v,
            Err(e) => {
                error!("handle_proxy: JSON parse error: {e}");
                return error_response(400, "invalid JSON body");
            }
        }
    } else {
        Value::Null
    };

    // --- STEP 5: Get active account from credentials ---
    let Some(account) = state.credentials.get_active() else {
        return error_response(503, "no active account");
    };

    let provider = account.provider.clone();

    // --- STEP 6: Store original_model before translation ---
    let original_model: String = body_json
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("claude-opus-4-6")
        .to_string();

    // --- STEP 7: Resolve model (client_model_to_anthropic) ---
    let resolved_model = model_mapping::resolve_model(&original_model, &state.model_config);
    if resolved_model != original_model {
        if let Some(obj) = body_json.as_object_mut() {
            obj.insert("model".to_string(), json!(resolved_model));
        }
    }

    // Tracking temps réel : mise à jour atomique des 4 champs en un seul verrou.
    // Ordre d'acquisition fixe : meta (ici) → last_client_ip → provider_quota
    // (dans proxy_status). Aucune inversion possible car chaque site n'acquiert
    // qu'un sous-ensemble dans le même ordre.
    // last_client a déjà été mis à jour dans process_request (ci-dessous).
    // On écrase ici avec le nom du client issu de imp_result après process_request,
    // mais on anticipe : process_request n'a pas encore été appelé à ce stade,
    // donc on passe une chaîne vide pour last_client — elle sera mise à jour
    // dans process_request puis écrasée après l'appel ci-dessous.
    // Pour éviter une double écriture inutile, on regroupe uniquement
    // client_format, server_format et model ici, puis process_request
    // met à jour last_client séparément (verrou court, pas de chevauchement).
    {
        let mut g = state.imp.meta.lock();
        g.last_client_format = client_fmt.to_string();
        // last_server_format est toujours "anthropic" (backend fixe)
        g.last_server_format = "anthropic".to_string();
        g.last_model = resolved_model.clone();
    }

    info!(
        email = %account.email,
        provider = %provider,
        model = %resolved_model,
        client = %client_ip,
        stream = body_json.get("stream").and_then(|v| v.as_bool()).unwrap_or(false),
        "routing request"
    );

    // --- STEP 8: Translate request to Anthropic format ---
    // Always rewrite path to /v1/messages
    if client_fmt != "anthropic" && body_json != Value::Null {
        body_json = crate::body_rewriter::translate_request_to_anthropic(client_fmt, body_json);
    }

    // --- STEP 9: Client detection + impersonation ---
    let imp_result = impersonation::process_request(
        &state.imp,
        &raw_headers_map,
        if body_json == Value::Null { None } else { Some(&body_json) },
        &provider,
    )
    .await;

    // Apply impersonation result body
    let mut final_body = imp_result.body.clone();
    // Re-apply resolved model after impersonation (impersonation may have reset it)
    // Note: we do NOT restore the client's stream value — if the profile forces stream:true
    // (always_streams), we send stream:true to Anthropic for perfect CC impersonation.
    // If the client didn't want streaming, we reassemble the SSE into JSON before returning.
    if final_body != Value::Null {
        if let Some(obj) = final_body.as_object_mut() {
            obj.insert("model".to_string(), json!(resolved_model));
        }
    }

    // --- STEP 10 + 11: Build upstream headers ---
    let upstream_url = account.api_url.as_deref().unwrap_or(
        if account.account_type == "oauth" {
            "https://api.anthropic.com"
        } else {
            &state.upstream_url
        }
    );

    // Always forward to /v1/messages
    let target_url = format!("{upstream_url}/v1/messages{query}");

    let mut hdrs = HeaderMap::new();

    if imp_result.full_replace {
        // IMPERSONATION PARFAITE : headers depuis le profil uniquement
        for (k, v) in &imp_result.headers {
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                hdrs.insert(name, val);
            }
        }
    } else {
        // Comportement normal : forward des headers client sans les headers transport
        let skip = [
            "host", "authorization", "x-api-key", "transfer-encoding",
            "connection", "accept-encoding", "content-length",
        ];
        for (k, v) in &raw_headers_map {
            if skip.contains(&k.as_str()) {
                continue;
            }
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                hdrs.insert(name, val);
            }
        }
    }

    // --- STEP 11: Set auth headers + anthropic-version ---
    // anthropic-version must be present before outbound validation
    if !hdrs.contains_key("anthropic-version") {
        hdrs.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        );
    }

    set_auth_headers(&mut hdrs, &account);

    // Ensure oauth beta flag for OAuth accounts targeting Anthropic.
    // Without this header, the Anthropic API rejects OAuth tokens.
    if account.account_type == "oauth" && provider == "anthropic" {
        ensure_oauth_beta(&raw_headers_map, &mut hdrs);
    }

    hdrs.insert(
        HeaderName::from_static("accept-encoding"),
        HeaderValue::from_static("identity"),
    );

    // --- STEP 10: Validate outbound (whitelist) ---
    // Only validate when we have a proper body (not null/empty GET)
    if method == Method::POST && final_body != Value::Null {
        if let Err(e) = crate::outbound_validator::validate_and_sanitize(&mut hdrs, &mut final_body) {
            warn!("outbound_validator rejected request: {e}");
            return error_response(400, &format!("outbound validation failed: {e}"));
        }
    }

    // --- Body sérialisé ---
    let body_bytes_out = if final_body != Value::Null {
        serde_json::to_vec(&final_body).unwrap_or_else(|_| body_bytes.to_vec())
    } else {
        body_bytes.to_vec()
    };

    if !body_bytes_out.is_empty() {
        hdrs.insert(
            HeaderName::from_static("content-length"),
            HeaderValue::from_str(&body_bytes_out.len().to_string()).unwrap(),
        );
        hdrs.insert(
            HeaderName::from_static("content-type"),
            raw_headers_map
                .get("content-type")
                .and_then(|v| HeaderValue::from_str(v).ok())
                .unwrap_or_else(|| HeaderValue::from_static("application/json")),
        );
    }

    let verbose = state.verbose.load(Ordering::Relaxed);
    if verbose {
        info!(
            "→ {method} {target_url} email={} provider={provider} client={} impersonated={}",
            account.email, imp_result.client_name, imp_result.full_replace
        );
        for (k, v) in hdrs.iter() {
            let val = if matches!(k.as_str(), "authorization" | "x-api-key") {
                format!("{}...(redacted)", &v.to_str().unwrap_or("")[..20.min(v.len())])
            } else {
                v.to_str().unwrap_or("").to_string()
            };
            info!("  > {k}: {val}");
        }
    } else {
        debug!(
            "→ {method} {target_url} (provider={provider}, client={}, impersonated={})",
            imp_result.client_name, imp_result.full_replace
        );
    }

    // =========================================================================
    // Detect streaming: what we SEND to Anthropic vs what the CLIENT expects.
    // If body_rewriter forced stream:true (CC always_streams), Anthropic responds SSE.
    // If the client didn't ask for streaming, we reassemble the SSE into JSON.
    // =========================================================================
    let upstream_streams = final_body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let client_wants_streaming = body_json
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if upstream_streams {
        // --- STREAMING PATH ---
        // Send without buffering the response body.
        let upstream_resp = match state
            .client
            .request(method.clone(), &target_url)
            .headers(hdrs.clone())
            .body(body_bytes_out.clone())
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("upstream streaming error: {e}");
                return error_response(502, &format!("upstream error: {e}"));
            }
        };

        let status = upstream_resp.status();
        let resp_hdr_stream = upstream_resp.headers().clone();

        info!(
            "← {status} from {provider} (streaming) email={}",
            account.email
        );

        // --- Non-success: buffer error body and return (errors are never SSE) ---
        if !status.is_success() {
            if status == StatusCode::UNAUTHORIZED {
                state.credentials.reload();
                if let Some(new_account) = state.credentials.get_active() {
                    if new_account.token != account.token {
                        warn!(
                            "401 streaming → token changed, retrying (buffered) email={}",
                            new_account.email
                        );
                        drop(upstream_resp); // release the failed connection
                        set_auth_headers(&mut hdrs, &new_account);
                        match send_upstream(
                            &state.client,
                            &method,
                            &target_url,
                            &hdrs,
                            &body_bytes_out,
                        )
                        .await
                        {
                            Ok((rs, rh, rb)) => {
                                *state.last_client_ip.lock() = client_ip.clone();
                                info!(
                                    "[{}] {} → anthropic | model={} status={} (streaming-retry-buffered)",
                                    client_ip, client_fmt, resolved_model, rs.as_u16()
                                );
                                let mut resp_builder = Response::builder().status(rs.as_u16());
                                for (k, v) in &rh {
                                    if !matches!(k.as_str(), "transfer-encoding" | "connection") {
                                        if let Some(bh) = resp_builder.headers_mut() {
                                            bh.insert(k, v.clone());
                                        }
                                    }
                                }
                                return resp_builder
                                    .body(Body::from(rb))
                                    .unwrap_or_else(|_| error_response(502, "body error"));
                            }
                            Err(e) => {
                                return error_response(
                                    502,
                                    &format!("upstream retry error: {e}"),
                                )
                            }
                        }
                    }
                }
            }
            // 401 with unchanged token, or other non-success status: buffer and return
            let err_bytes = upstream_resp.bytes().await.unwrap_or_default();
            *state.last_client_ip.lock() = client_ip.clone();
            let mut resp_builder = Response::builder().status(status.as_u16());
            for (k, v) in &resp_hdr_stream {
                if !matches!(k.as_str(), "transfer-encoding" | "connection") {
                    if let Some(bh) = resp_builder.headers_mut() {
                        bh.insert(k, v.clone());
                    }
                }
            }
            return resp_builder
                .body(Body::from(err_bytes))
                .unwrap_or_else(|_| error_response(502, "body error"));
        }

        // --- Success: extract quota ---
        extract_provider_quota(&resp_hdr_stream, &provider, &state.provider_quota);

        if client_wants_streaming {
            // ---------------------------------------------------------------
            // CLIENT WANTS STREAMING: forward SSE as-is (live translation)
            // ---------------------------------------------------------------
            state.api_usage.record(
                &account.email,
                &resolved_model,
                &crate::api_usage::UsageData::default(),
                &client_ip,
                client_fmt,
            );
            {
                let sw = state.session_writer.lock().await;
                sw.record_request(&account.email, &resolved_model, 0, 0, &client_ip);
            }
            *state.last_client_ip.lock() = client_ip.clone();
            info!(
                "[{}] {} → anthropic | model={} status={} (streaming)",
                client_ip, client_fmt, resolved_model, status.as_u16()
            );

            let stream_body =
                build_sse_stream_body(
                    upstream_resp,
                    client_fmt.to_string(),
                    original_model.clone(),
                    state.session_writer.clone(),
                    account.email.clone(),
                    resolved_model.clone(),
                );

            let mut rb = Response::builder().status(status.as_u16());
            for (k, v) in &resp_hdr_stream {
                if !matches!(k.as_str(), "transfer-encoding" | "connection" | "content-length") {
                    if let Some(bh) = rb.headers_mut() {
                        bh.insert(k, v.clone());
                    }
                }
            }
            if let Some(bh) = rb.headers_mut() {
                bh.insert(
                    HeaderName::from_static("content-type"),
                    HeaderValue::from_static("text/event-stream; charset=utf-8"),
                );
            }
            return rb
                .body(stream_body)
                .unwrap_or_else(|_| error_response(502, "streaming body error"));
        } else {
            // ---------------------------------------------------------------
            // CLIENT DOESN'T WANT STREAMING but we forced stream:true for CC
            // impersonation → reassemble SSE into a complete JSON Message.
            // ---------------------------------------------------------------
            info!(
                "reassembling SSE→JSON for non-streaming client (model={})",
                resolved_model
            );

            let mut reassembler = crate::sse_reassemble::SseReassembler::new();
            {
                use futures::StreamExt;
                let byte_stream = upstream_resp.bytes_stream();
                tokio::pin!(byte_stream);
                while let Some(result) = byte_stream.next().await {
                    match result {
                        Ok(chunk) => reassembler.feed(&chunk),
                        Err(e) => {
                            error!("SSE reassemble stream error: {e}");
                            return error_response(502, &format!("upstream stream error: {e}"));
                        }
                    }
                }
            }

            let resp_body = match reassembler.into_message() {
                Some(msg) => msg,
                None => {
                    error!("SSE reassemble: no message_start received");
                    return error_response(502, "upstream returned incomplete SSE stream");
                }
            };

            // Extract usage
            let usage_data = {
                let u = resp_body.get("usage");
                crate::api_usage::UsageData {
                    input_tokens: u.and_then(|u| u.get("input_tokens")).and_then(|v| v.as_u64()).unwrap_or(0),
                    output_tokens: u.and_then(|u| u.get("output_tokens")).and_then(|v| v.as_u64()).unwrap_or(0),
                    cache_read_tokens: u.and_then(|u| u.get("cache_read_input_tokens")).and_then(|v| v.as_u64()).unwrap_or(0),
                    cache_creation_tokens: u.and_then(|u| u.get("cache_creation_input_tokens")).and_then(|v| v.as_u64()).unwrap_or(0),
                }
            };
            state.api_usage.record(
                &account.email,
                &resolved_model,
                &usage_data,
                &client_ip,
                client_fmt,
            );
            {
                let sw = state.session_writer.lock().await;
                sw.record_request(
                    &account.email,
                    &resolved_model,
                    usage_data.input_tokens,
                    usage_data.output_tokens,
                    &client_ip,
                );
            }

            // Translate response if client is not anthropic format
            let final_resp = if client_fmt != "anthropic" {
                let mut r = resp_body;
                if let Some(obj) = r.as_object_mut() {
                    obj.insert("model".to_string(), json!(original_model));
                }
                crate::body_rewriter::translate_response_to_client(client_fmt, r)
            } else {
                resp_body
            };

            let resp_bytes = serde_json::to_vec(&final_resp).unwrap_or_default();

            *state.last_client_ip.lock() = client_ip.clone();
            info!(
                "[{}] {} → anthropic | model={} status={} (sse→json reassembled, {} bytes)",
                client_ip, client_fmt, resolved_model, status.as_u16(), resp_bytes.len()
            );

            let mut rb = Response::builder().status(status.as_u16());
            // Copy relevant response headers (skip streaming-specific ones)
            for (k, v) in &resp_hdr_stream {
                if !matches!(k.as_str(), "transfer-encoding" | "connection" | "content-length" | "content-type") {
                    if let Some(bh) = rb.headers_mut() {
                        bh.insert(k, v.clone());
                    }
                }
            }
            if let Some(bh) = rb.headers_mut() {
                bh.insert(
                    HeaderName::from_static("content-type"),
                    HeaderValue::from_static("application/json"),
                );
                if let Ok(cl) = HeaderValue::from_str(&resp_bytes.len().to_string()) {
                    bh.insert(HeaderName::from_static("content-length"), cl);
                }
            }
            return rb
                .body(Body::from(resp_bytes))
                .unwrap_or_else(|_| error_response(502, "reassemble body error"));
        }
    }
    // END STREAMING PATH
    // =========================================================================

    // --- STEP 12: Forward to upstream ALWAYS at {upstream_url}/v1/messages ---
    let (status, resp_headers, resp_bytes) = match send_upstream(
        &state.client, &method, &target_url, &hdrs, &body_bytes_out,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("upstream error: {e}");
            return error_response(502, &format!("upstream error: {e}"));
        }
    };

    info!(
        "← {status} from {provider} ({} bytes) email={}",
        resp_bytes.len(),
        account.email
    );

    // --- STEP 13: Handle 401 retry (up to 6 × 500ms attempts) ---
    let (status, resp_headers, resp_bytes) = if status == StatusCode::UNAUTHORIZED {
        // Force reload immédiat (pas de debounce sur 401)
        state.credentials.reload();
        if let Some(new_account) = state.credentials.get_active() {
            if new_account.token != account.token {
                warn!(
                    "401 → credentials reloaded (token changed), retrying email={}",
                    new_account.email
                );
                set_auth_headers(&mut hdrs, &new_account);
                match send_upstream(
                    &state.client, &method, &target_url, &hdrs, &body_bytes_out,
                )
                .await
                {
                    Ok(retry) => {
                        info!(
                            "← {} from {provider} ({} bytes) [retry] email={}",
                            retry.0,
                            retry.2.len(),
                            new_account.email
                        );
                        retry
                    }
                    Err(e) => {
                        error!("upstream error on retry: {e}");
                        return error_response(502, &format!("upstream error: {e}"));
                    }
                }
            } else {
                // Token inchangé — attendre que le watchdog Python propage le nouveau token
                warn!(
                    "401 → token unchanged, waiting for credential update (email={})...",
                    account.email
                );
                let mut retried_result = None;
                for i in 0..6u32 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    state.credentials.reload();
                    if let Some(fresh) = state.credentials.get_active() {
                        if fresh.token != account.token {
                            warn!(
                                "401 → token changed after {}ms, retrying email={}",
                                (i + 1) * 500,
                                fresh.email
                            );
                            set_auth_headers(&mut hdrs, &fresh);
                            match send_upstream(
                                &state.client, &method, &target_url, &hdrs, &body_bytes_out,
                            )
                            .await
                            {
                                Ok(retry) => {
                                    info!(
                                        "← {} from {provider} ({} bytes) [retry-wait] email={}",
                                        retry.0,
                                        retry.2.len(),
                                        fresh.email
                                    );
                                    retried_result = Some(retry);
                                }
                                Err(e) => {
                                    error!("upstream error on retry-wait: {e}");
                                    return error_response(
                                        502,
                                        &format!("upstream error: {e}"),
                                    );
                                }
                            }
                            break;
                        }
                    }
                }
                retried_result.unwrap_or_else(|| {
                    warn!(
                        "401 → token still unchanged after 3s wait (email={})",
                        account.email
                    );
                    (status, resp_headers, resp_bytes)
                })
            }
        } else {
            warn!("401 → credentials reloaded but no active account");
            (status, resp_headers, resp_bytes)
        }
    } else {
        (status, resp_headers, resp_bytes)
    };

    // --- STEP 14: Extract quota from response headers ---
    extract_provider_quota(&resp_headers, &provider, &state.provider_quota);

    // --- STEP 15 + 16: Response translation + streaming passthrough ---
    // Detect streaming from response content-type or request body
    let is_streaming = resp_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);

    // Parse response body for non-streaming
    let (final_resp_bytes, input_tokens, output_tokens, cache_read, cache_write) =
        if status.is_success() && !resp_bytes.is_empty() && !is_streaming {
            match serde_json::from_slice::<Value>(&resp_bytes) {
                Ok(mut resp_body) => {
                    // Extract token usage
                    let usage = crate::api_usage::parse_usage_from_chunk(&resp_bytes)
                        .unwrap_or_default();

                    // Re-insert original_model for non-anthropic clients
                    if client_fmt != "anthropic" {
                        if let Some(obj) = resp_body.as_object_mut() {
                            obj.insert("model".to_string(), json!(original_model));
                        }
                        // Translate response to client format
                        resp_body = crate::body_rewriter::translate_response_to_client(
                            client_fmt,
                            resp_body,
                        );
                    }

                    let out_bytes = serde_json::to_vec(&resp_body)
                        .map(Bytes::from)
                        .unwrap_or(resp_bytes);

                    (
                        out_bytes,
                        usage.input_tokens,
                        usage.output_tokens,
                        usage.cache_read_tokens,
                        usage.cache_creation_tokens,
                    )
                }
                Err(_) => (resp_bytes, 0u64, 0u64, 0u64, 0u64),
            }
        } else {
            // Error response or unexpected streaming (stream=false but got text/event-stream):
            // passthrough as-is.
            (resp_bytes, 0u64, 0u64, 0u64, 0u64)
        };

    // --- STEP 17: Log JSONL ---
    {
        let usage = crate::api_usage::UsageData {
            input_tokens,
            output_tokens,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_write,
        };
        state.api_usage.record(
            &account.email,
            &resolved_model,
            &usage,
            &client_ip,
            client_fmt,
        );
    }

    // --- STEP 18: Session write ---
    {
        let sw = state.session_writer.lock().await;
        sw.record_request(
            &account.email,
            &resolved_model,
            input_tokens,
            output_tokens,
            &client_ip,
        );
        // If we have cache tokens too, update them
        if cache_read > 0 || cache_write > 0 {
            sw.update_tokens(
                &account.email,
                &resolved_model,
                0, // already counted above
                0,
                cache_read,
                cache_write,
            );
        }
    }

    // --- STEP 19: Store last_client_ip + info log ---
    *state.last_client_ip.lock() = client_ip.clone();
    info!(
        "[{}] {} → anthropic | model={} status={}",
        client_ip, client_fmt, resolved_model, status.as_u16()
    );

    // Build final response
    let mut response = Response::builder().status(status.as_u16());
    for (k, v) in &resp_headers {
        if !matches!(k.as_str(), "transfer-encoding" | "connection") {
            if let Some(builder) = response.headers_mut() {
                builder.insert(k, v.clone());
            }
        }
    }
    response
        .body(Body::from(final_resp_bytes))
        .unwrap_or_else(|_| error_response(502, "body build error"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

use crate::credentials::ActiveAccount;

/// Injecte le header d'authentification dans la HeaderMap selon le type de compte.
fn set_auth_headers(hdrs: &mut HeaderMap, account: &ActiveAccount) {
    // Retirer l'ancien auth header s'il existe
    hdrs.remove("authorization");
    hdrs.remove("x-api-key");

    let auth_header = account.auth_header.as_deref().unwrap_or(
        if account.account_type == "oauth" {
            "authorization"
        } else {
            "x-api-key"
        },
    );
    if auth_header.to_lowercase() == "authorization" {
        let prefix = if account.account_type == "oauth" { "Bearer " } else { "" };
        if let Ok(val) = HeaderValue::from_str(&format!("{prefix}{}", account.token)) {
            hdrs.insert(HeaderName::from_static("authorization"), val);
        }
    } else if let Ok(name) = HeaderName::from_bytes(auth_header.as_bytes()) {
        if let Ok(val) = HeaderValue::from_str(&account.token) {
            hdrs.insert(name, val);
        }
    }
}

/// Envoie une requête upstream et retourne (status, headers, body).
async fn send_upstream(
    client: &reqwest::Client,
    method: &Method,
    url: &str,
    hdrs: &HeaderMap,
    body: &[u8],
) -> Result<(StatusCode, HeaderMap, Bytes), String> {
    let resp = client
        .request(method.clone(), url)
        .headers(hdrs.clone())
        .body(body.to_vec())
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let resp_bytes = resp.bytes().await.unwrap_or_default();
    Ok((status, resp_headers, resp_bytes))
}

async fn read_body(body: Body) -> Result<Bytes, String> {
    axum::body::to_bytes(body, MAX_BODY_SIZE)
        .await
        .map_err(|e| e.to_string())
}

fn error_response(status: u16, message: &str) -> Response<Body> {
    let body = serde_json::json!({"error": {"message": message}}).to_string();
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

/// Extrait les infos de quota/rate-limit des headers de réponse upstream.
fn extract_provider_quota(
    headers: &HeaderMap,
    provider: &str,
    quota_store: &Arc<parking_lot::Mutex<std::collections::HashMap<String, Value>>>,
) {
    let mut quota = serde_json::Map::new();

    // Headers OpenAI-compat (Gemini, OpenAI, xAI, DeepSeek...)
    let rl_headers = [
        "x-ratelimit-limit-requests",
        "x-ratelimit-remaining-requests",
        "x-ratelimit-limit-tokens",
        "x-ratelimit-remaining-tokens",
        "x-ratelimit-reset-requests",
        "x-ratelimit-reset-tokens",
        "retry-after",
    ];
    for hdr in &rl_headers {
        if let Some(val) = headers.get(*hdr).and_then(|v| v.to_str().ok()) {
            let key = hdr.replace("x-ratelimit-", "").replace('-', "_");
            if let Ok(n) = val.parse::<i64>() {
                quota.insert(key, json!(n));
            } else {
                quota.insert(key, json!(val));
            }
        }
    }

    // Headers Anthropic
    let anthro_headers = [
        "anthropic-ratelimit-requests-limit",
        "anthropic-ratelimit-requests-remaining",
        "anthropic-ratelimit-tokens-limit",
        "anthropic-ratelimit-tokens-remaining",
        "anthropic-ratelimit-requests-reset",
        "anthropic-ratelimit-tokens-reset",
    ];
    for hdr in &anthro_headers {
        if let Some(val) = headers.get(*hdr).and_then(|v| v.to_str().ok()) {
            let key = hdr.replace("anthropic-ratelimit-", "").replace('-', "_");
            if let Ok(n) = val.parse::<i64>() {
                quota.insert(key, json!(n));
            } else {
                quota.insert(key, json!(val));
            }
        }
    }

    if !quota.is_empty() {
        quota.insert("_provider".to_string(), json!(provider));
        // parking_lot::Mutex ne peut pas être empoisonné → pas de Result à gérer.
        quota_store.lock().insert(provider.to_string(), Value::Object(quota));
        debug!("quota extracted for provider={provider}");
    }
}

// ---------------------------------------------------------------------------
// SSE Streaming helpers
// ---------------------------------------------------------------------------

/// Lightweight SSE usage extractor that scans raw Anthropic SSE bytes
/// for `message_start` (input_tokens) and `message_delta` (output_tokens).
struct SseUsageExtractor {
    buf: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
}

impl SseUsageExtractor {
    fn new() -> Self {
        Self {
            buf: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        }
    }

    /// Feed raw bytes from the upstream SSE stream.
    fn feed(&mut self, bytes: &[u8]) {
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        while let Some(pos) = self.buf.find("\n\n") {
            let event_block = self.buf[..pos].to_string();
            self.buf = self.buf[pos + 2..].to_string();
            self.parse_event(&event_block);
        }
    }

    fn parse_event(&mut self, block: &str) {
        let data = match block.lines().find(|l| l.starts_with("data: ")) {
            Some(l) => &l[6..],
            None => return,
        };
        let json: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return,
        };
        match json.get("type").and_then(|v| v.as_str()) {
            Some("message_start") => {
                if let Some(usage) = json.pointer("/message/usage") {
                    self.input_tokens += usage.get("input_tokens")
                        .and_then(|v| v.as_u64()).unwrap_or(0);
                    self.cache_read_tokens += usage.get("cache_read_input_tokens")
                        .and_then(|v| v.as_u64()).unwrap_or(0);
                    self.cache_creation_tokens += usage.get("cache_creation_input_tokens")
                        .and_then(|v| v.as_u64()).unwrap_or(0);
                }
            }
            Some("message_delta") => {
                if let Some(usage) = json.get("usage") {
                    self.output_tokens += usage.get("output_tokens")
                        .and_then(|v| v.as_u64()).unwrap_or(0);
                }
            }
            _ => {}
        }
    }

    fn has_tokens(&self) -> bool {
        self.input_tokens > 0 || self.output_tokens > 0
            || self.cache_read_tokens > 0 || self.cache_creation_tokens > 0
    }
}

/// Build a streaming [`Body`] from an upstream Anthropic SSE response with
/// live translation to the requested client format.
///
/// | `client_fmt`  | Behaviour |
/// |---------------|-----------|
/// | `"openai"`    | Translate via [`crate::sse_translator::SseAnthropicToOpenai`] |
/// | `"gemini"`    | Translate via [`crate::sse_translator::SseAnthropicToGemini`] |
/// | `"anthropic"` | Passthrough (bytes forwarded verbatim) |
/// | anything else | Passthrough |
fn build_sse_stream_body(
    resp: reqwest::Response,
    client_fmt: String,
    model: String,
    session_writer: Arc<tokio::sync::Mutex<crate::session_writer::SessionWriter>>,
    email: String,
    resolved_model: String,
) -> Body {
    match client_fmt.as_str() {
        "openai" => {
            let msg_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
            let stream = async_stream::stream! {
                let mut translator =
                    crate::sse_translator::SseAnthropicToOpenai::new(msg_id, model);
                let mut usage = SseUsageExtractor::new();
                let byte_stream = resp.bytes_stream();
                tokio::pin!(byte_stream);
                use futures::StreamExt;
                while let Some(result) = byte_stream.next().await {
                    match result {
                        Ok(bytes) => {
                            usage.feed(&bytes);
                            let out = translator.process_chunk(&bytes);
                            if !out.is_empty() {
                                yield Ok::<Bytes, std::io::Error>(out);
                            }
                        }
                        Err(e) => {
                            yield Err(std::io::Error::other(e));
                            break;
                        }
                    }
                }
                if usage.has_tokens() {
                    let sw = session_writer.lock().await;
                    sw.update_tokens(
                        &email, &resolved_model,
                        usage.input_tokens, usage.output_tokens,
                        usage.cache_read_tokens, usage.cache_creation_tokens,
                    );
                }
            };
            Body::from_stream(stream)
        }
        "gemini" => {
            let stream = async_stream::stream! {
                let mut translator =
                    crate::sse_translator::SseAnthropicToGemini::new(model);
                let mut remainder = String::new();
                let mut usage = SseUsageExtractor::new();
                let byte_stream = resp.bytes_stream();
                tokio::pin!(byte_stream);
                use futures::StreamExt;
                while let Some(result) = byte_stream.next().await {
                    match result {
                        Ok(bytes) => {
                            usage.feed(&bytes);
                            let out = translator.process_chunk(&bytes, &mut remainder);
                            if !out.is_empty() {
                                yield Ok::<Bytes, std::io::Error>(out);
                            }
                        }
                        Err(e) => {
                            yield Err(std::io::Error::other(e));
                            break;
                        }
                    }
                }
                if usage.has_tokens() {
                    let sw = session_writer.lock().await;
                    sw.update_tokens(
                        &email, &resolved_model,
                        usage.input_tokens, usage.output_tokens,
                        usage.cache_read_tokens, usage.cache_creation_tokens,
                    );
                }
            };
            Body::from_stream(stream)
        }
        _ => {
            // "anthropic" and all other formats: forward bytes verbatim
            let stream = async_stream::stream! {
                let mut usage = SseUsageExtractor::new();
                let byte_stream = resp.bytes_stream();
                tokio::pin!(byte_stream);
                use futures::StreamExt;
                while let Some(result) = byte_stream.next().await {
                    match result {
                        Ok(bytes) => {
                            usage.feed(&bytes);
                            yield Ok::<Bytes, std::io::Error>(bytes);
                        }
                        Err(e) => {
                            yield Err(std::io::Error::other(e));
                            break;
                        }
                    }
                }
                if usage.has_tokens() {
                    let sw = session_writer.lock().await;
                    sw.update_tokens(
                        &email, &resolved_model,
                        usage.input_tokens, usage.output_tokens,
                        usage.cache_read_tokens, usage.cache_creation_tokens,
                    );
                }
            };
            Body::from_stream(stream)
        }
    }
}

// ---------------------------------------------------------------------------
// OAuth beta header — required for OAuth accounts on Anthropic API
// ---------------------------------------------------------------------------

/// Ensures `anthropic-beta` contains an oauth flag for OAuth accounts.
/// Learns the flag from the incoming request, falls back to a default.
fn ensure_oauth_beta(
    raw_headers: &std::collections::HashMap<String, String>,
    hdrs: &mut HeaderMap,
) {
    // Check if outgoing headers already have an oauth flag
    if let Some(existing) = hdrs.get("anthropic-beta") {
        if let Ok(s) = existing.to_str() {
            if s.split(',').any(|f| f.trim().starts_with("oauth-")) {
                return;
            }
        }
    }

    // Try to learn oauth flag from incoming request headers
    let oauth_flag = raw_headers
        .get("anthropic-beta")
        .and_then(|s| {
            s.split(',')
                .find(|f| f.trim().starts_with("oauth-"))
                .map(|f| f.trim().to_string())
        })
        .unwrap_or_else(|| "oauth-2025-04-20".to_string());

    // Merge with existing anthropic-beta or insert new
    if let Some(existing) = hdrs.get("anthropic-beta") {
        if let Ok(s) = existing.to_str() {
            let merged = format!("{},{}", s, oauth_flag);
            if let Ok(val) = HeaderValue::from_str(&merged) {
                hdrs.insert("anthropic-beta", val);
            }
            return;
        }
    }
    if let Ok(val) = HeaderValue::from_str(&oauth_flag) {
        hdrs.insert("anthropic-beta", val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Helper: build a minimal AppState for unit tests
    // -------------------------------------------------------------------------

    fn make_test_state() -> Arc<AppState> {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = tmp.keep(); // prevent deletion, returns PathBuf

        let creds_path = tmp_path.join("credentials-multi.json");
        let _ = std::fs::write(&creds_path, "{}");

        let (shutdown_tx, _) = tokio::sync::watch::channel(false);

        Arc::new(AppState {
            client: reqwest::Client::new(),
            timeout_secs: 300,
            upstream_url: "https://api.anthropic.com".to_string(),
            credentials: crate::credentials::CredentialsCache::load(&creds_path),
            imp: Arc::new(crate::impersonation::ImpersonationState::new(false, false)),
            model_config: Arc::new(std::collections::HashMap::new()),
            shutdown_tx,
            provider_quota: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            verbose: Arc::new(AtomicBool::new(false)),
            rate_limiter: Arc::new(crate::rate_limiter::RateLimiter::new()),
            api_usage: Arc::new(crate::api_usage::ApiUsageTracker::new(&tmp_path)),
            session_writer: Arc::new(tokio::sync::Mutex::new(
                crate::session_writer::SessionWriter::new(&tmp_path),
            )),
            last_client_ip: Arc::new(parking_lot::Mutex::new(String::new())),
        })
    }

    // -------------------------------------------------------------------------
    // Admin endpoint — proxy_health
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn admin_health_returns_ok_json() {
        let resp = proxy_health().await;
        let resp = resp.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(v["backend"], "rust-auto");
    }

    // -------------------------------------------------------------------------
    // Admin endpoint — proxy_shutdown
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn admin_shutdown_returns_shutting_down() {
        let state = make_test_state();
        let resp = proxy_shutdown(State(state)).await;
        let resp = resp.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "shutting_down");
    }

    // -------------------------------------------------------------------------
    // Admin endpoint — proxy_profiles
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn admin_profiles_returns_profiles_array() {
        let state = make_test_state();
        let resp = proxy_profiles(State(state)).await;
        let resp = resp.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        // Must have a "profiles" key that is an array
        assert!(v.get("profiles").is_some(), "missing 'profiles' key");
        assert!(v["profiles"].is_array(), "'profiles' must be an array");
    }

    // -------------------------------------------------------------------------
    // Admin endpoint — proxy_flush
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn admin_flush_returns_flushed_count() {
        let state = make_test_state();
        let resp = proxy_flush(State(state)).await;
        let resp = resp.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        // Must have a "flushed" key with a non-negative integer
        assert!(v.get("flushed").is_some(), "missing 'flushed' key");
        assert!(v["flushed"].is_u64(), "'flushed' must be an unsigned integer");
    }

    // -------------------------------------------------------------------------
    // Admin endpoint — proxy_verbose_toggle (toggle behaviour)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn admin_verbose_toggle_flips_state() {
        let state = make_test_state();
        // Initial state: verbose = false
        assert!(!state.verbose.load(Ordering::Relaxed));

        // Toggle with empty body → should become true
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/_proxy/verbose")
            .body(Body::empty())
            .unwrap();
        let resp = proxy_verbose_toggle(State(Arc::clone(&state)), req).await;
        let resp = resp.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["verbose"], true, "first toggle should set verbose to true");
        assert!(state.verbose.load(Ordering::Relaxed));

        // Toggle again with empty body → should become false
        let req2 = axum::http::Request::builder()
            .method("POST")
            .uri("/_proxy/verbose")
            .body(Body::empty())
            .unwrap();
        let resp2 = proxy_verbose_toggle(State(Arc::clone(&state)), req2).await;
        let resp2 = resp2.into_response();
        let body2 = axum::body::to_bytes(resp2.into_body(), 1024).await.unwrap();
        let v2: Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(v2["verbose"], false, "second toggle should set verbose to false");
        assert!(!state.verbose.load(Ordering::Relaxed));
    }

    // -------------------------------------------------------------------------
    // Admin endpoint — proxy_verbose_toggle with explicit {"enabled": true}
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn admin_verbose_explicit_enabled_true() {
        let state = make_test_state();
        // verbose starts false; send {"enabled": true}
        let body_bytes = serde_json::to_vec(&json!({"enabled": true})).unwrap();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/_proxy/verbose")
            .header("content-type", "application/json")
            .body(Body::from(body_bytes))
            .unwrap();
        let resp = proxy_verbose_toggle(State(Arc::clone(&state)), req).await;
        let resp = resp.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["verbose"], true);
        assert!(state.verbose.load(Ordering::Relaxed));

        // Now send {"enabled": false} while verbose is true — should force to false
        let body_bytes2 = serde_json::to_vec(&json!({"enabled": false})).unwrap();
        let req2 = axum::http::Request::builder()
            .method("POST")
            .uri("/_proxy/verbose")
            .header("content-type", "application/json")
            .body(Body::from(body_bytes2))
            .unwrap();
        let resp2 = proxy_verbose_toggle(State(Arc::clone(&state)), req2).await;
        let resp2 = resp2.into_response();
        let body2 = axum::body::to_bytes(resp2.into_body(), 1024).await.unwrap();
        let v2: Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(v2["verbose"], false);
        assert!(!state.verbose.load(Ordering::Relaxed));
    }

    // -------------------------------------------------------------------------
    // Admin endpoint — proxy_api_usage (empty state → empty stats)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn admin_api_usage_empty_returns_object() {
        let state = make_test_state();
        let req = axum::http::Request::builder()
            .uri("/_proxy/api/usage")
            .body(Body::empty())
            .unwrap();
        let resp = proxy_api_usage(State(state), req).await;
        let resp = resp.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        // Empty stats should be a JSON object ({}), not null or an error
        assert!(v.is_object(), "usage stats should be a JSON object");
    }

    // -------------------------------------------------------------------------
    // SseUsageExtractor — existing unit tests (preserved)
    // -------------------------------------------------------------------------

    #[test]
    fn sse_usage_extractor_message_start() {
        let mut e = SseUsageExtractor::new();
        e.feed(b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1234,\"cache_read_input_tokens\":500,\"cache_creation_input_tokens\":100}}}\n\n");
        assert_eq!(e.input_tokens, 1234);
        assert_eq!(e.cache_read_tokens, 500);
        assert_eq!(e.cache_creation_tokens, 100);
        assert_eq!(e.output_tokens, 0);
        assert!(e.has_tokens());
    }

    #[test]
    fn sse_usage_extractor_message_delta() {
        let mut e = SseUsageExtractor::new();
        e.feed(b"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":567}}\n\n");
        assert_eq!(e.output_tokens, 567);
        assert_eq!(e.input_tokens, 0);
        assert!(e.has_tokens());
    }

    #[test]
    fn sse_usage_extractor_full_stream() {
        let mut e = SseUsageExtractor::new();
        e.feed(b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":50,\"cache_creation_input_tokens\":25}}}\n\n");
        e.feed(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hello\"}}\n\n");
        e.feed(b"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":200}}\n\n");
        assert_eq!(e.input_tokens, 100);
        assert_eq!(e.output_tokens, 200);
        assert_eq!(e.cache_read_tokens, 50);
        assert_eq!(e.cache_creation_tokens, 25);
    }

    #[test]
    fn sse_usage_extractor_chunked_bytes() {
        let mut e = SseUsageExtractor::new();
        e.feed(b"event: message_start\ndata: {\"type\":\"message_");
        assert_eq!(e.input_tokens, 0);
        e.feed(b"start\",\"message\":{\"usage\":{\"input_tokens\":42}}}\n\n");
        assert_eq!(e.input_tokens, 42);
    }

    #[test]
    fn sse_usage_extractor_no_usage_events() {
        let mut e = SseUsageExtractor::new();
        e.feed(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n");
        e.feed(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
        assert!(!e.has_tokens());
    }
}
