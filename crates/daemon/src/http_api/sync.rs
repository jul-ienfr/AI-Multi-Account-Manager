//! Handlers sync P2P — gestion des pairs et de la clé partagée.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use ai_core::config::PeerConfig;
use ai_core::types::Peer;
use tracing::info;

use crate::dto::{AddPeerData, SetKeyData, TestPeerData};
use super::{DaemonState, error_json, ok_json};

// ---------------------------------------------------------------------------
// sync_status
// ---------------------------------------------------------------------------

/// `GET /sync/status` — état général de la synchronisation P2P.
pub async fn sync_status(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let sync = state.config.read().sync.clone();
    let (peer_count, peers_list) = if let Some(bus) = &state.sync_bus {
        let raw = bus.list_peers();
        let count = raw.len();
        let list: Vec<_> = raw.into_iter().map(|(id, host, port)| {
            json!({ "id": id, "host": host, "port": port, "connected": true })
        }).collect();
        (count, list)
    } else {
        let peers = state.peers.read().clone();
        let list: Vec<_> = peers.iter().map(|p| json!({
            "id": p.id, "host": p.host, "port": p.port, "connected": p.connected
        })).collect();
        let count = list.len();
        (count, list)
    };
    ok_json(json!({
        "enabled": sync.enabled,
        "port": sync.port,
        "peer_count": peer_count,
        "peers": peers_list,
        "key_configured": sync.shared_key_hex.is_some(),
    }))
}

// ---------------------------------------------------------------------------
// list_peers
// ---------------------------------------------------------------------------

/// `GET /peers` — liste des pairs (live depuis le bus si actif, sinon config statique).
pub async fn list_peers(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    if let Some(bus) = &state.sync_bus {
        let peers: Vec<Peer> = bus.list_peers().into_iter().map(|(id, host, port)| {
            Peer { id, host, port, connected: true, last_seen: None }
        }).collect();
        ok_json(peers)
    } else {
        let peers = state.peers.read().clone();
        ok_json(peers)
    }
}

// ---------------------------------------------------------------------------
// add_peer
// ---------------------------------------------------------------------------

/// `POST /peers` — ajoute un nouveau pair P2P.
pub async fn add_peer(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<AddPeerData>,
) -> impl IntoResponse {
    let id = body
        .id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()[..8].to_string());

    let peer = Peer {
        id: id.clone(),
        host: body.host.clone(),
        port: body.port,
        connected: false,
        last_seen: None,
    };

    let host = body.host.clone();
    let port = body.port;

    let peer_config = PeerConfig {
        id: id.clone(),
        host: body.host,
        port: body.port,
    };

    state.peers.write().push(peer);
    state.config.write().sync.peers.push(peer_config);
    let _ = state.config.persist();

    // Connecter immédiatement via le SyncBus si disponible
    if let Some(bus) = &state.sync_bus {
        let protocol = ai_sync::compat::PeerProtocol::Unknown;
        bus.connect_peer(&id, &host, port, protocol).await;
        info!("P2P: initiated connection to peer {} @ {}:{}", id, host, port);
    }

    // Broadcast PeerConfigUpdate to P2P peers
    if let Some(bus) = &state.sync_bus {
        let bus = bus.clone();
        let instance_id = bus.instance_id().to_string();
        let peer_id = id.clone();
        let peer_host = host.clone();
        let peer_port = port;
        tokio::spawn(async move {
            let clock = bus.next_clock();
            let msg = ai_sync::messages::SyncMessage::new(
                &instance_id,
                ai_sync::messages::SyncPayload::PeerConfigUpdate {
                    action: "add".into(),
                    peer_id: Some(peer_id),
                    host: Some(peer_host),
                    port: Some(peer_port),
                    shared_key_hex: None,
                    clock,
                },
            );
            let _ = bus.broadcast(msg).await;
        });
    }

    ok_json(json!({"ok": true, "id": id}))
}

// ---------------------------------------------------------------------------
// remove_peer
// ---------------------------------------------------------------------------

/// `DELETE /peers/:id` — supprime un pair par identifiant.
pub async fn remove_peer(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    state.peers.write().retain(|p| p.id != id);
    state.config.write().sync.peers.retain(|p| p.id != id);
    let _ = state.config.persist();

    if let Some(bus) = &state.sync_bus {
        bus.remove_peer(&id);
    }

    // Broadcast PeerConfigUpdate (remove) to P2P peers
    if let Some(bus) = &state.sync_bus {
        let bus = bus.clone();
        let instance_id = bus.instance_id().to_string();
        let peer_id = id.clone();
        tokio::spawn(async move {
            let clock = bus.next_clock();
            let msg = ai_sync::messages::SyncMessage::new(
                &instance_id,
                ai_sync::messages::SyncPayload::PeerConfigUpdate {
                    action: "remove".into(),
                    peer_id: Some(peer_id),
                    host: None,
                    port: None,
                    shared_key_hex: None,
                    clock,
                },
            );
            let _ = bus.broadcast(msg).await;
        });
    }

    ok_json(json!({"ok": true}))
}

// ---------------------------------------------------------------------------
// gen_key
// ---------------------------------------------------------------------------

/// `POST /sync/key/generate` — génère une nouvelle clé partagée 256-bit.
pub async fn gen_key(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let u1 = uuid::Uuid::new_v4();
    let u2 = uuid::Uuid::new_v4();
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(u1.as_bytes());
    bytes[16..].copy_from_slice(u2.as_bytes());
    let hex_key = hex::encode(bytes);

    state.config.write().sync.shared_key_hex = Some(hex_key.clone());
    let _ = state.config.persist();

    // Broadcast PeerConfigUpdate (set_key) to P2P peers
    if let Some(bus) = &state.sync_bus {
        let bus = bus.clone();
        let instance_id = bus.instance_id().to_string();
        let key_clone = hex_key.clone();
        tokio::spawn(async move {
            let clock = bus.next_clock();
            let msg = ai_sync::messages::SyncMessage::new(
                &instance_id,
                ai_sync::messages::SyncPayload::PeerConfigUpdate {
                    action: "set_key".into(),
                    peer_id: None,
                    host: None,
                    port: None,
                    shared_key_hex: Some(key_clone),
                    clock,
                },
            );
            let _ = bus.broadcast(msg).await;
        });
    }

    ok_json(json!({"key": hex_key}))
}

// ---------------------------------------------------------------------------
// set_key
// ---------------------------------------------------------------------------

/// `POST /sync/key/set` — définit manuellement la clé partagée.
pub async fn set_key(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<SetKeyData>,
) -> impl IntoResponse {
    if body.key.len() != 64 || !body.key.chars().all(|c| c.is_ascii_hexdigit()) {
        return error_json(400, "key must be 64 hex chars");
    }

    state.config.write().sync.shared_key_hex = Some(body.key.clone());
    let _ = state.config.persist();

    // Broadcast PeerConfigUpdate (set_key) to P2P peers
    if let Some(bus) = &state.sync_bus {
        let bus = bus.clone();
        let instance_id = bus.instance_id().to_string();
        let key_clone = body.key.clone();
        tokio::spawn(async move {
            let clock = bus.next_clock();
            let msg = ai_sync::messages::SyncMessage::new(
                &instance_id,
                ai_sync::messages::SyncPayload::PeerConfigUpdate {
                    action: "set_key".into(),
                    peer_id: None,
                    host: None,
                    port: None,
                    shared_key_hex: Some(key_clone),
                    clock,
                },
            );
            let _ = bus.broadcast(msg).await;
        });
    }

    ok_json(json!({"ok": true}))
}

// ---------------------------------------------------------------------------
// test_peer
// ---------------------------------------------------------------------------

/// `POST /peers/test` — teste la connectivité TCP vers un pair (stateless).
pub async fn test_peer(Json(body): Json<TestPeerData>) -> impl IntoResponse {
    let addr = format!("{}:{}", body.host, body.port);
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect(addr),
    )
    .await;

    ok_json(json!({"reachable": result.is_ok()}))
}
