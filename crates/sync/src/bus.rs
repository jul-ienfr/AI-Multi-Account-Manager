//! Transport TCP chiffré avec NaCl SecretBox.
//!
//! Traduit la logique Python de `sync_bus.py` (SyncBus, chiffrement NaCl,
//! reconnexion automatique) en Rust async tokio.
//!
//! # Protocole de framing
//!
//! ```text
//! [u32 BE: longueur payload] [payload chiffré NaCl SecretBox]
//! ```
//!
//! # Chiffrement
//!
//! Utilise `sodiumoxide::crypto::secretbox` (XSalsa20-Poly1305).
//! Chaque message est chiffré avec un nonce aléatoire préfixé au payload :
//! ```text
//! [24 bytes nonce] [ciphertext]
//! ```

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use sodiumoxide::crypto::secretbox;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::compat::{self, PeerProtocol};
use crate::error::{Result, SyncError};
use crate::failure_detector::{MessageDedup, NodeStatus, PhiAccrualDetector};
use crate::messages::{SyncMessage, SyncPayload};

/// Capacité du canal broadcast (nombre de messages en attente).
const BROADCAST_CAPACITY: usize = 256;

/// Délai initial de reconnexion (ms).
const RECONNECT_INITIAL_MS: u64 = 500;
/// Délai maximum de reconnexion (ms).
const RECONNECT_MAX_MS: u64 = 30_000;
/// Multiplicateur de backoff.
const RECONNECT_BACKOFF: u64 = 2;

/// Taille maximale d'un frame (16 MiB) — protection anti-DoS.
const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

/// Intervalle de vérification du statut des pairs (secondes).
const PEER_CHECK_INTERVAL_SECS: u64 = 5;

/// État d'une connexion pair, avec failure detector associé.
struct PeerEntry {
    host: String,
    port: u16,
    /// Failure detector Phi Accrual pour ce pair.
    detector: PhiAccrualDetector,
    /// Protocole détecté pour ce pair (V2 ou V3).
    protocol: PeerProtocol,
}

/// Clé symétrique NaCl SecretBox.
#[derive(Clone)]
pub struct SharedKey(pub [u8; 32]);

impl std::fmt::Debug for SharedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SharedKey([REDACTED])")
    }
}

/// Bus de communication P2P TCP chiffré.
///
/// Chaque instance écoute sur un port TCP et se connecte aux pairs configurés.
/// Tous les messages sont chiffrés avec NaCl SecretBox (clé symétrique partagée).
pub struct SyncBus {
    instance_id: String,
    port: u16,
    /// Clé symétrique NaCl 32 bytes
    shared_key: SharedKey,
    /// Pairs connus {peer_id -> PeerEntry}
    peers: Arc<RwLock<HashMap<String, PeerEntry>>>,
    /// Canal broadcast des messages reçus
    message_tx: broadcast::Sender<SyncMessage>,
    /// Déduplication rolling window des messages traités
    dedup: Arc<RwLock<MessageDedup>>,
    /// Credentials pour répondre directement aux SyncRequest entrants (sans peer config symétrique)
    direct_creds: Arc<RwLock<Option<Arc<ai_core::credentials::CredentialsCache>>>>,
}

impl SyncBus {
    /// Crée un nouveau SyncBus.
    ///
    /// Initialise sodiumoxide si ce n'est pas déjà fait.
    pub fn new(instance_id: String, port: u16, shared_key: [u8; 32]) -> Self {
        // sodiumoxide::init() est idempotent
        let _ = sodiumoxide::init();
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            instance_id,
            port,
            shared_key: SharedKey(shared_key),
            peers: Arc::new(RwLock::new(HashMap::new())),
            message_tx: tx,
            dedup: Arc::new(RwLock::new(MessageDedup::new())),
            direct_creds: Arc::new(RwLock::new(None)),
        }
    }

    /// Enregistre les credentials pour répondre directement aux SyncRequest entrants.
    ///
    /// Permet à un pair qui ne nous a pas configuré comme peer de recevoir nos
    /// credentials en réponse directe sur la connexion entrante.
    pub fn set_credentials(&self, creds: Arc<ai_core::credentials::CredentialsCache>) {
        *self.direct_creds.write() = Some(creds);
    }

    /// Génère une clé symétrique aléatoire (pour le CLI --generate-key).
    pub fn generate_key() -> [u8; 32] {
        let _ = sodiumoxide::init();
        let key = secretbox::gen_key();
        key.0
    }

    /// Démarre le serveur TCP en écoute et la surveillance des pairs.
    ///
    /// Les connexions entrantes sont traitées dans des tâches tokio séparées.
    /// Une tâche de surveillance vérifie périodiquement le statut (phi accrual)
    /// de chaque pair enregistré.
    pub async fn start(self: &Arc<Self>) -> Result<()> {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| SyncError::Connection(format!("bind {addr}: {e}")))?;
        info!("SyncBus listening on {}", addr);

        // Tâche d'acceptation des connexions entrantes
        let bus = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer_addr)) => {
                        debug!("Incoming connection from {}", peer_addr);
                        let bus_clone = Arc::clone(&bus);
                        tokio::spawn(async move {
                            if let Err(e) = bus_clone.handle_incoming(stream, peer_addr).await {
                                warn!("Incoming connection error from {}: {}", peer_addr, e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Accept error: {}", e);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });

        // Tâche de surveillance des pairs (phi accrual)
        let bus_watch = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(PEER_CHECK_INTERVAL_SECS));
            loop {
                interval.tick().await;
                let statuses: Vec<(String, NodeStatus)> = {
                    let peers = bus_watch.peers.read();
                    peers
                        .iter()
                        .map(|(id, entry)| (id.clone(), entry.detector.status()))
                        .collect()
                };
                for (peer_id, status) in &statuses {
                    match status {
                        NodeStatus::Alive => {}
                        NodeStatus::Suspect => {
                            warn!("Peer {} is SUSPECT (phi accrual threshold reached)", peer_id);
                        }
                        NodeStatus::Dead => {
                            warn!("Peer {} is DEAD (phi accrual — no heartbeat)", peer_id);
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// Traite une connexion TCP entrante.
    async fn handle_incoming(&self, mut stream: TcpStream, peer_addr: SocketAddr) -> Result<()> {
        loop {
            match recv_frame(&mut stream).await {
                Ok(encrypted) => {
                    let data = decrypt(&encrypted, &self.shared_key.0)?;
                    let (msg, detected_proto) = compat::parse_message(&data, &self.instance_id)
                        .map_err(|e| SyncError::Encode(e))?;
                    debug!("Received message from {} (proto={:?}): id={}", peer_addr, detected_proto, msg.id);

                    // Ignore nos propres messages (rebond)
                    if msg.from == self.instance_id {
                        continue;
                    }

                    // Met à jour le protocole du pair si connu
                    if detected_proto != PeerProtocol::Unknown {
                        let mut peers = self.peers.write();
                        if let Some(entry) = peers.get_mut(&msg.from) {
                            if entry.protocol == PeerProtocol::Unknown {
                                entry.protocol = detected_proto;
                                debug!("Updated protocol for peer {} to {:?}", msg.from, detected_proto);
                            }
                        }
                    }

                    // Déduplication rolling window
                    {
                        let mut dedup = self.dedup.write();
                        if dedup.is_duplicate(&msg.id) {
                            debug!("Duplicate message {} from {} — dropped", msg.id, peer_addr);
                            continue;
                        }
                        dedup.mark_seen(&msg.id);
                    }

                    // Enregistre le heartbeat phi accrual pour ce pair (si connu)
                    if let SyncPayload::Heartbeat { ref instance_id, .. } = msg.payload {
                        let mut peers = self.peers.write();
                        if let Some(entry) = peers.get_mut(instance_id) {
                            entry.detector.heartbeat();
                            debug!("Heartbeat from peer {} registered (phi accrual)", instance_id);
                        }
                    }

                    // Réponse directe aux SyncRequest entrants (sans peer config symétrique)
                    // Si le pair qui nous envoie un SyncRequest n'est pas dans notre liste
                    // de peers configurés, son broadcast() ne pourra pas nous répondre.
                    // On répond donc directement sur la même connexion TCP.
                    if matches!(&msg.payload, SyncPayload::SyncRequest { .. }) {
                        // Cloner les données AVANT tout await (RwLockReadGuard n'est pas Send)
                        let maybe_frame: Option<Vec<u8>> = {
                            let creds_lock = self.direct_creds.read();
                            if let Some(creds) = creds_lock.as_ref() {
                                match creds.export_json() {
                                    Ok(accounts_json) => {
                                        let active_key = creds.active_key();
                                        let mut clock = HashMap::new();
                                        clock.insert(self.instance_id.clone(), 1u64);
                                        let response = SyncMessage::new(
                                            &self.instance_id,
                                            SyncPayload::SyncResponse {
                                                credentials_json: accounts_json,
                                                active_key,
                                                clock,
                                            },
                                        );
                                        match serde_json::to_vec(&response) {
                                            Ok(data) => Some(encrypt(&data, &self.shared_key.0)),
                                            Err(e) => {
                                                warn!("Failed to serialize SyncResponse: {}", e);
                                                None
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to export credentials for SyncResponse: {}", e);
                                        None
                                    }
                                }
                            } else {
                                None
                            }
                        }; // drop creds_lock ici
                        if let Some(frame) = maybe_frame {
                            if let Err(e) = send_frame(&mut stream, &frame).await {
                                warn!("Direct SyncResponse to {} failed: {}", peer_addr, e);
                            } else {
                                info!("Direct SyncResponse sent to {}", peer_addr);
                            }
                        }
                    }

                    // Broadcast aux abonnés locaux
                    if let Err(e) = self.message_tx.send(msg) {
                        debug!("No subscribers for message: {}", e);
                    }
                }
                Err(SyncError::Io(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof
                        || e.kind() == std::io::ErrorKind::ConnectionReset =>
                {
                    debug!("Peer {} disconnected", peer_addr);
                    break;
                }
                Err(e) => {
                    warn!("Error receiving frame from {}: {}", peer_addr, e);
                    break;
                }
            }
        }
        Ok(())
    }

    /// Connecte ce bus à un pair distant avec reconnexion automatique.
    ///
    /// La tâche de reconnexion tourne en arrière-plan avec backoff exponentiel.
    pub async fn connect_peer(self: &Arc<Self>, peer_id: &str, host: &str, port: u16, protocol: PeerProtocol) {
        // Enregistre le pair avec un failure detector fraîchement initialisé
        {
            let mut peers = self.peers.write();
            peers.insert(
                peer_id.to_string(),
                PeerEntry {
                    host: host.to_string(),
                    port,
                    detector: PhiAccrualDetector::new(),
                    protocol,
                },
            );
        }

        let bus = Arc::clone(self);
        let peer_id = peer_id.to_string();
        let host = host.to_string();
        tokio::spawn(async move {
            bus.reconnect_loop(&peer_id, &host, port).await;
        });
    }

    /// Boucle de reconnexion avec backoff exponentiel.
    async fn reconnect_loop(&self, peer_id: &str, host: &str, port: u16) {
        let mut delay_ms = RECONNECT_INITIAL_MS;
        loop {
            let addr = format!("{host}:{port}");
            match TcpStream::connect(&addr).await {
                Ok(stream) => {
                    info!("Connected to peer {} @ {}", peer_id, addr);
                    delay_ms = RECONNECT_INITIAL_MS; // reset backoff

                    // Envoie une SyncRequest pour demander l'état courant
                    let request = crate::messages::SyncMessage::sync_request(&self.instance_id);
                    if let Ok(data) = serde_json::to_vec(&request) {
                        let encrypted = encrypt(&data, &self.shared_key.0);
                        let mut s = stream;
                        if let Err(e) = send_frame(&mut s, &encrypted).await {
                            warn!("Failed to send sync request to {}: {}", peer_id, e);
                            // Reconnexion sera déclenchée en dessous
                        }
                        // Écoute les réponses
                        if let Err(e) = self.read_from_peer(&mut s, peer_id).await {
                            warn!("Peer {} disconnected: {}", peer_id, e);
                        }
                    }
                }
                Err(e) => {
                    debug!("Cannot connect to {} @ {}: {} — retry in {}ms", peer_id, addr, e, delay_ms);
                }
            }

            // Vérifie si le pair est toujours enregistré
            if !self.peers.read().contains_key(peer_id) {
                info!("Peer {} removed, stopping reconnect loop", peer_id);
                break;
            }

            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            // Backoff exponentiel plafonné
            delay_ms = (delay_ms * RECONNECT_BACKOFF).min(RECONNECT_MAX_MS);
        }
    }

    /// Lit les messages d'un pair connecté.
    async fn read_from_peer(&self, stream: &mut TcpStream, peer_id: &str) -> Result<()> {
        loop {
            let encrypted = recv_frame(stream).await?;
            let data = decrypt(&encrypted, &self.shared_key.0)?;
            let (msg, detected_proto) = compat::parse_message(&data, &self.instance_id)
                .map_err(|e| SyncError::Encode(e))?;

            if msg.from == self.instance_id {
                continue;
            }

            // Met à jour le protocole du pair si détecté
            if detected_proto != PeerProtocol::Unknown {
                let mut peers = self.peers.write();
                if let Some(entry) = peers.get_mut(peer_id) {
                    if entry.protocol == PeerProtocol::Unknown {
                        entry.protocol = detected_proto;
                    }
                }
            }

            // Déduplication rolling window
            {
                let mut dedup = self.dedup.write();
                if dedup.is_duplicate(&msg.id) {
                    debug!("Duplicate message {} from peer {} — dropped", msg.id, peer_id);
                    continue;
                }
                dedup.mark_seen(&msg.id);
            }

            // Heartbeat phi accrual : enregistre si c'est un heartbeat de ce pair
            if let SyncPayload::Heartbeat { ref instance_id, .. } = msg.payload {
                if instance_id == peer_id {
                    let mut peers = self.peers.write();
                    if let Some(entry) = peers.get_mut(peer_id) {
                        entry.detector.heartbeat();
                        debug!("Heartbeat from peer {} registered (phi accrual)", peer_id);
                    }
                }
            }

            debug!("Message from peer {}: id={}", peer_id, msg.id);
            let _ = self.message_tx.send(msg);
        }
    }

    /// Supprime un pair et arrête la reconnexion.
    pub fn remove_peer(&self, peer_id: &str) {
        self.peers.write().remove(peer_id);
        debug!("Peer {} removed from SyncBus", peer_id);
    }

    /// Broadcast un message à tous les pairs connectés.
    ///
    /// Chaque envoi est tenté de manière best-effort (un échec n'arrête pas les autres).
    /// Le message est sérialisé en V2 ou V3 selon le protocole du pair.
    pub async fn broadcast(&self, msg: SyncMessage) -> Result<()> {
        // Pré-sérialise en V3 (cas le plus courant)
        let v3_data = serde_json::to_vec(&msg)
            .map_err(|e| SyncError::Encode(format!("JSON encode: {e}")))?;
        let v3_encrypted = encrypt(&v3_data, &self.shared_key.0);

        // Pré-sérialise en V2 (lazy, seulement si un pair V2 existe)
        let v2_encrypted = {
            let peers = self.peers.read();
            let has_v2 = peers.values().any(|e| e.protocol == PeerProtocol::V2);
            if has_v2 {
                match compat::serialize_for_protocol(&msg, PeerProtocol::V2) {
                    Ok(v2_data) => Some(encrypt(&v2_data, &self.shared_key.0)),
                    Err(e) => {
                        debug!("Cannot serialize to V2: {} — will use V3 for all", e);
                        None
                    }
                }
            } else {
                None
            }
        };

        // Snapshot de (peer_id, host, port, protocol) uniquement
        let peers_snapshot: Vec<(String, String, u16, PeerProtocol)> = self
            .peers
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), v.host.clone(), v.port, v.protocol))
            .collect();

        for (peer_id, host, port, protocol) in &peers_snapshot {
            let encrypted = match protocol {
                PeerProtocol::V2 => v2_encrypted.as_ref().unwrap_or(&v3_encrypted),
                _ => &v3_encrypted,
            };
            let addr = format!("{}:{}", host, port);
            match TcpStream::connect(&addr).await {
                Ok(mut stream) => {
                    if let Err(e) = send_frame(&mut stream, encrypted).await {
                        warn!("Failed to send to peer {}: {}", peer_id, e);
                    } else {
                        debug!("Sent msg {} to peer {} (proto={:?})", msg.id, peer_id, protocol);
                    }
                }
                Err(e) => {
                    warn!("Cannot connect to peer {} @ {}: {}", peer_id, addr, e);
                }
            }
        }
        Ok(())
    }

    /// Retourne un Receiver pour s'abonner aux messages entrants.
    pub fn subscribe(&self) -> broadcast::Receiver<SyncMessage> {
        self.message_tx.subscribe()
    }

    /// Retourne l'instance_id de ce bus.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Retourne le nombre de pairs enregistrés.
    pub fn peer_count(&self) -> usize {
        self.peers.read().len()
    }

    /// Met à jour le protocole d'un pair.
    pub fn set_peer_protocol(&self, peer_id: &str, protocol: PeerProtocol) {
        if let Some(entry) = self.peers.write().get_mut(peer_id) {
            entry.protocol = protocol;
        }
    }

    /// Retourne le protocole d'un pair, ou `Unknown` s'il n'est pas enregistré.
    pub fn peer_protocol(&self, peer_id: &str) -> PeerProtocol {
        self.peers
            .read()
            .get(peer_id)
            .map(|e| e.protocol)
            .unwrap_or(PeerProtocol::Unknown)
    }

    /// Liste les pairs enregistrés (peer_id -> (host, port)).
    pub fn list_peers(&self) -> Vec<(String, String, u16)> {
        self.peers
            .read()
            .iter()
            .map(|(id, e)| (id.clone(), e.host.clone(), e.port))
            .collect()
    }
}

// ----------------------------------------------------------------
// Chiffrement NaCl SecretBox
// ----------------------------------------------------------------

/// Chiffre `data` avec NaCl SecretBox (XSalsa20-Poly1305).
///
/// Format du résultat : `[24 bytes nonce] [ciphertext]`
pub fn encrypt(data: &[u8], key: &[u8; 32]) -> Vec<u8> {
    let key = secretbox::Key(*key);
    let nonce = secretbox::gen_nonce();
    let ciphertext = secretbox::seal(data, &nonce, &key);
    let mut result = Vec::with_capacity(secretbox::NONCEBYTES + ciphertext.len());
    result.extend_from_slice(&nonce.0);
    result.extend_from_slice(&ciphertext);
    result
}

/// Déchiffre un payload NaCl SecretBox.
///
/// Attend le format : `[24 bytes nonce] [ciphertext]`
pub fn decrypt(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    if data.len() < secretbox::NONCEBYTES {
        return Err(SyncError::Decrypt);
    }
    let (nonce_bytes, ciphertext) = data.split_at(secretbox::NONCEBYTES);
    let nonce = secretbox::Nonce::from_slice(nonce_bytes).ok_or(SyncError::Decrypt)?;
    let key = secretbox::Key(*key);
    secretbox::open(ciphertext, &nonce, &key).map_err(|_| SyncError::Decrypt)
}

// ----------------------------------------------------------------
// Framing length-prefix
// ----------------------------------------------------------------

/// Envoie un frame avec préfixe de longueur (u32 BE).
pub async fn send_frame(stream: &mut TcpStream, data: &[u8]) -> Result<()> {
    let len = data.len() as u32;
    if len > MAX_FRAME_SIZE {
        return Err(SyncError::Encode(format!(
            "Frame too large: {} > {}",
            len, MAX_FRAME_SIZE
        )));
    }
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(data).await?;
    stream.flush().await?;
    Ok(())
}

/// Reçoit un frame avec préfixe de longueur (u32 BE).
pub async fn recv_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_SIZE {
        return Err(SyncError::Encode(format!(
            "Frame too large: {} > {}",
            len, MAX_FRAME_SIZE
        )));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        let _ = sodiumoxide::init();
        SyncBus::generate_key()
    }

    // ---- Tests chiffrement / déchiffrement NaCl ----

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = test_key();
        let plaintext = b"Hello, P2P world!";
        let encrypted = encrypt(plaintext, &key);
        let decrypted = decrypt(&encrypted, &key).expect("decrypt");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_produces_different_output_each_time() {
        let key = test_key();
        let plaintext = b"same message";
        let enc1 = encrypt(plaintext, &key);
        let enc2 = encrypt(plaintext, &key);
        // Nonces aléatoires → ciphertext différent à chaque fois
        assert_ne!(enc1, enc2);
    }

    #[test]
    fn test_decrypt_wrong_key_fails() {
        let key1 = test_key();
        let key2 = test_key();
        let plaintext = b"secret data";
        let encrypted = encrypt(plaintext, &key1);
        let result = decrypt(&encrypted, &key2);
        assert!(result.is_err(), "Decryption with wrong key should fail");
        assert!(matches!(result.unwrap_err(), SyncError::Decrypt));
    }

    #[test]
    fn test_decrypt_tampered_ciphertext_fails() {
        let key = test_key();
        let plaintext = b"integrity check";
        let mut encrypted = encrypt(plaintext, &key);
        // Modifie un byte du ciphertext
        if let Some(byte) = encrypted.last_mut() {
            *byte ^= 0xFF;
        }
        let result = decrypt(&encrypted, &key);
        assert!(result.is_err());
    }

    #[test]
    fn test_decrypt_too_short_fails() {
        let key = test_key();
        let result = decrypt(&[0u8; 10], &key); // < NONCEBYTES (24)
        assert!(result.is_err());
    }

    #[test]
    fn test_encrypt_decrypt_empty_payload() {
        let key = test_key();
        let encrypted = encrypt(b"", &key);
        let decrypted = decrypt(&encrypted, &key).expect("decrypt empty");
        assert!(decrypted.is_empty());
    }

    #[test]
    fn test_encrypt_decrypt_large_payload() {
        let key = test_key();
        let plaintext = vec![0xABu8; 1024 * 64]; // 64 KiB
        let encrypted = encrypt(&plaintext, &key);
        let decrypted = decrypt(&encrypted, &key).expect("decrypt large");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_includes_nonce_prefix() {
        let key = test_key();
        let plaintext = b"test";
        let encrypted = encrypt(plaintext, &key);
        // Doit contenir au moins le nonce (24 bytes) + overhead poly1305 (16 bytes)
        assert!(encrypted.len() >= secretbox::NONCEBYTES + 16);
    }

    // ---- Tests framing ----

    #[tokio::test]
    async fn test_send_recv_frame() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let send_data = b"frame test payload";
        let send_data_clone = send_data.to_vec();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            recv_frame(&mut stream).await.unwrap()
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        send_frame(&mut client, send_data).await.unwrap();

        let received = server.await.unwrap();
        assert_eq!(received, send_data_clone);
    }

    #[tokio::test]
    async fn test_frame_roundtrip_multiple() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let messages = vec![
            b"first".to_vec(),
            b"second message longer".to_vec(),
            b"third".to_vec(),
        ];
        let messages_clone = messages.clone();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            for _ in 0..3 {
                received.push(recv_frame(&mut stream).await.unwrap());
            }
            received
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        for msg in &messages {
            send_frame(&mut client, msg).await.unwrap();
        }

        let received = server.await.unwrap();
        assert_eq!(received, messages_clone);
    }

    // ---- Tests SyncBus lifecycle ----

    #[test]
    fn test_syncbus_new() {
        let key = test_key();
        let bus = SyncBus::new("inst-1".to_string(), 9876, key);
        assert_eq!(bus.instance_id(), "inst-1");
        assert_eq!(bus.peer_count(), 0);
    }

    #[test]
    fn test_generate_key_unique() {
        let k1 = SyncBus::generate_key();
        let k2 = SyncBus::generate_key();
        assert_ne!(k1, k2, "Keys should be unique");
    }

    #[test]
    fn test_remove_peer() {
        let key = test_key();
        let bus = SyncBus::new("inst-1".to_string(), 0, key);
        bus.peers.write().insert(
            "peer-1".to_string(),
            PeerEntry {
                host: "127.0.0.1".to_string(),
                port: 9877,
                detector: PhiAccrualDetector::new(),
                protocol: PeerProtocol::V3,
            },
        );
        assert_eq!(bus.peer_count(), 1);
        bus.remove_peer("peer-1");
        assert_eq!(bus.peer_count(), 0);
    }

    #[test]
    fn test_subscribe_returns_receiver() {
        let key = test_key();
        let bus = SyncBus::new("inst-1".to_string(), 0, key);
        let _rx = bus.subscribe();
        // Juste vérifie qu'on peut créer un subscriber
    }

    // ---- Tests protocol-aware (V2/V3 compat) ----

    #[test]
    fn test_peer_entry_has_protocol() {
        let key = test_key();
        let bus = SyncBus::new("inst-1".to_string(), 0, key);
        bus.peers.write().insert(
            "peer-v2".to_string(),
            PeerEntry {
                host: "192.168.1.10".to_string(),
                port: 5555,
                detector: PhiAccrualDetector::new(),
                protocol: PeerProtocol::V2,
            },
        );
        assert_eq!(bus.peer_protocol("peer-v2"), PeerProtocol::V2);
    }

    #[test]
    fn test_set_peer_protocol() {
        let key = test_key();
        let bus = SyncBus::new("inst-1".to_string(), 0, key);
        bus.peers.write().insert(
            "peer-1".to_string(),
            PeerEntry {
                host: "127.0.0.1".to_string(),
                port: 5556,
                detector: PhiAccrualDetector::new(),
                protocol: PeerProtocol::Unknown,
            },
        );
        assert_eq!(bus.peer_protocol("peer-1"), PeerProtocol::Unknown);
        bus.set_peer_protocol("peer-1", PeerProtocol::V3);
        assert_eq!(bus.peer_protocol("peer-1"), PeerProtocol::V3);
    }

    #[test]
    fn test_peer_protocol_unknown_for_missing_peer() {
        let key = test_key();
        let bus = SyncBus::new("inst-1".to_string(), 0, key);
        assert_eq!(bus.peer_protocol("nonexistent"), PeerProtocol::Unknown);
    }

    #[test]
    fn test_broadcast_v2_serialization() {
        // Vérifie que le code de pré-sérialisation V2 fonctionne
        let msg = SyncMessage::heartbeat("inst-1");
        let v2_data = compat::serialize_for_protocol(&msg, PeerProtocol::V2).unwrap();
        let v2: compat::V2Message = serde_json::from_slice(&v2_data).unwrap();
        assert_eq!(v2.msg_type, "heartbeat");
        assert_eq!(v2.source, "inst-1");
    }

    #[test]
    fn test_broadcast_v3_serialization() {
        let msg = SyncMessage::heartbeat("inst-1");
        let v3_data = compat::serialize_for_protocol(&msg, PeerProtocol::V3).unwrap();
        let parsed: SyncMessage = serde_json::from_slice(&v3_data).unwrap();
        assert_eq!(parsed.from, "inst-1");
        assert!(matches!(parsed.payload, SyncPayload::Heartbeat { .. }));
    }
}
