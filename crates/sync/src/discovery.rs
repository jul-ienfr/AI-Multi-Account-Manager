//! Découverte mDNS des pairs AI Manager sur le réseau local.
//!
//! Traduit la logique Python de `sync/discovery.py` (PeerDiscovery/Zeroconf)
//! en Rust avec la crate `mdns-sd`.
//!
//! # Comportement
//!
//! - Annonce `_ai-manager._tcp.local.` avec l'instance_id dans les propriétés TXT
//! - Découvre les pairs du même service sur le LAN
//! - Ignore notre propre annonce
//! - Envoie les pairs découverts via un `mpsc::Receiver<PeerEvent>`

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::compat::PeerProtocol;
use crate::error::{Result, SyncError};

/// Type du service mDNS V3.
const SERVICE_TYPE_V3: &str = "_ai-manager._tcp.local.";
/// Type du service mDNS V2 (Python legacy).
const SERVICE_TYPE_V2: &str = "_ai-mgr._tcp.local.";

/// Événement de découverte retourné par `PeerDiscovery::discover()`.
#[derive(Debug, Clone)]
pub enum PeerEvent {
    /// Un pair a été découvert (ou mis à jour).
    Found {
        peer_id: String,
        host: String,
        port: u16,
        protocol: PeerProtocol,
    },
    /// Un pair a disparu.
    Lost { peer_id: String },
}

/// Découverte mDNS des pairs AI Manager sur le LAN.
pub struct PeerDiscovery {
    instance_id: String,
    port: u16,
    daemon: ServiceDaemon,
    /// Pairs connus : {peer_id → (host, port)}
    known_peers: Arc<Mutex<HashMap<String, (String, u16)>>>,
}

impl PeerDiscovery {
    /// Crée une nouvelle instance de `PeerDiscovery`.
    pub fn new(instance_id: String, port: u16) -> Result<Self> {
        let daemon =
            ServiceDaemon::new().map_err(|e| SyncError::Mdns(format!("ServiceDaemon::new: {e}")))?;
        Ok(Self {
            instance_id,
            port,
            daemon,
            known_peers: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Annonce notre service sur le réseau local.
    ///
    /// Publie sur les DEUX service types (_ai-manager pour V3, _ai-mgr pour V2)
    /// afin que les pairs V2 et V3 nous découvrent.
    pub fn advertise(&self) -> Result<()> {
        let local_ip = get_local_ip();
        let host_name = format!("{}.local.", self.instance_id);
        let properties = [
            ("version", "1"),
            ("instance_id", self.instance_id.as_str()),
        ];

        // Annonce V3
        let info_v3 = ServiceInfo::new(
            SERVICE_TYPE_V3,
            &self.instance_id,
            &host_name,
            &local_ip,
            self.port,
            &properties[..],
        )
        .map_err(|e| SyncError::Mdns(format!("ServiceInfo::new V3: {e}")))?;

        self.daemon
            .register(info_v3)
            .map_err(|e| SyncError::Mdns(format!("register V3: {e}")))?;

        // Annonce V2 (pour que les pairs Python nous voient)
        let info_v2 = ServiceInfo::new(
            SERVICE_TYPE_V2,
            &self.instance_id,
            &host_name,
            &local_ip,
            self.port,
            &properties[..],
        )
        .map_err(|e| SyncError::Mdns(format!("ServiceInfo::new V2: {e}")))?;

        self.daemon
            .register(info_v2)
            .map_err(|e| SyncError::Mdns(format!("register V2: {e}")))?;

        info!(
            "PeerDiscovery: advertising instance={} port={} ip={} (V2+V3)",
            self.instance_id, self.port, local_ip
        );
        Ok(())
    }

    /// Découvre les pairs sur le réseau local.
    ///
    /// Browse les DEUX service types (V3 + V2) et retourne un `Receiver<PeerEvent>`
    /// qui reçoit les événements de découverte avec le protocole détecté.
    pub fn discover(&self) -> mpsc::Receiver<PeerEvent> {
        let (tx, rx) = mpsc::channel(64);
        let instance_id = self.instance_id.clone();
        let known_peers = Arc::clone(&self.known_peers);

        // Browse V3
        let receiver_v3 = match self.daemon.browse(SERVICE_TYPE_V3) {
            Ok(r) => Some(r),
            Err(e) => {
                error!("Failed to start mDNS browser (V3): {}", e);
                None
            }
        };

        // Browse V2
        let receiver_v2 = match self.daemon.browse(SERVICE_TYPE_V2) {
            Ok(r) => Some(r),
            Err(e) => {
                error!("Failed to start mDNS browser (V2): {}", e);
                None
            }
        };

        if let Some(rx_v3) = receiver_v3 {
            let tx_clone = tx.clone();
            let iid = instance_id.clone();
            let kp = Arc::clone(&known_peers);
            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    Self::browse_loop(rx_v3, tx_clone, iid, kp, PeerProtocol::V3)
                })
                .await;
                if let Err(e) = result {
                    error!("mDNS V3 browse task panicked: {}", e);
                }
            });
        }

        if let Some(rx_v2) = receiver_v2 {
            let tx_clone = tx;
            let iid = instance_id;
            let kp = known_peers;
            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    Self::browse_loop(rx_v2, tx_clone, iid, kp, PeerProtocol::V2)
                })
                .await;
                if let Err(e) = result {
                    error!("mDNS V2 browse task panicked: {}", e);
                }
            });
        }

        rx
    }

    /// Boucle de navigation mDNS (thread bloquant).
    fn browse_loop(
        receiver: mdns_sd::Receiver<ServiceEvent>,
        tx: mpsc::Sender<PeerEvent>,
        instance_id: String,
        known_peers: Arc<Mutex<HashMap<String, (String, u16)>>>,
        protocol: PeerProtocol,
    ) {
        loop {
            match receiver.recv_timeout(Duration::from_secs(5)) {
                Ok(event) => {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            Self::handle_resolved(&info, &tx, &instance_id, &known_peers, protocol);
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            Self::handle_removed(&fullname, &tx, &instance_id, &known_peers);
                        }
                        _ => {}
                    }
                }
                Err(flume::RecvTimeoutError::Timeout) => {}
                Err(flume::RecvTimeoutError::Disconnected) => {
                    info!("mDNS browser channel disconnected");
                    break;
                }
            }
        }
    }

    /// Traite un service résolu.
    fn handle_resolved(
        info: &ServiceInfo,
        tx: &mpsc::Sender<PeerEvent>,
        my_instance_id: &str,
        known_peers: &Arc<Mutex<HashMap<String, (String, u16)>>>,
        protocol: PeerProtocol,
    ) {
        // Extraire l'instance_id depuis les propriétés TXT
        let peer_id = info
            .get_properties()
            .get("instance_id")
            .map(|v| v.val_str().to_string())
            .unwrap_or_default();

        if peer_id.is_empty() || peer_id == my_instance_id {
            return; // Ignore notre propre service
        }

        // Récupère la première adresse IPv4
        let addresses: Vec<IpAddr> = info.get_addresses().iter().cloned().collect();
        let host = addresses
            .iter()
            .find(|a| a.is_ipv4())
            .or_else(|| addresses.first())
            .map(|a| a.to_string())
            .unwrap_or_default();

        if host.is_empty() {
            warn!("mDNS: no address for peer {}", peer_id);
            return;
        }

        let port = info.get_port();

        {
            let mut peers = known_peers.lock();
            let existing = peers.get(&peer_id);
            if existing == Some(&(host.clone(), port)) {
                return; // Déjà connu, rien à faire
            }
            peers.insert(peer_id.clone(), (host.clone(), port));
        }

        info!("mDNS: discovered peer {} @ {}:{} (proto={:?})", peer_id, host, port, protocol);
        let event = PeerEvent::Found { peer_id, host, port, protocol };
        if tx.blocking_send(event).is_err() {
            debug!("mDNS: receiver dropped, stopping browse");
        }
    }

    /// Traite la disparition d'un service.
    fn handle_removed(
        fullname: &str,
        tx: &mpsc::Sender<PeerEvent>,
        my_instance_id: &str,
        known_peers: &Arc<Mutex<HashMap<String, (String, u16)>>>,
    ) {
        // Format : "{instance_id}.{SERVICE_TYPE}"
        // Ex V3 : "my-inst._ai-manager._tcp.local."
        // Ex V2 : "my-inst._ai-mgr._tcp.local."
        let stripped = fullname.trim_end_matches('.');
        let peer_id = stripped
            .trim_end_matches(SERVICE_TYPE_V3.trim_end_matches('.'))
            .trim_end_matches(SERVICE_TYPE_V2.trim_end_matches('.'))
            .trim_end_matches('.')
            .to_string();

        if peer_id.is_empty() || peer_id == my_instance_id {
            return;
        }

        {
            let mut peers = known_peers.lock();
            if peers.remove(&peer_id).is_none() {
                return; // Inconnu
            }
        }

        info!("mDNS: peer lost {}", peer_id);
        let event = PeerEvent::Lost {
            peer_id: peer_id.clone(),
        };
        if tx.blocking_send(event).is_err() {
            debug!("mDNS: receiver dropped");
        }
    }

    /// Arrête le daemon mDNS (dé-publie le service).
    pub fn stop(&self) {
        if let Err(e) = self.daemon.shutdown() {
            warn!("mDNS daemon shutdown error: {}", e);
        }
        info!("PeerDiscovery stopped");
    }

    /// Retourne les pairs actuellement connus.
    pub fn known_peers(&self) -> HashMap<String, (String, u16)> {
        self.known_peers.lock().clone()
    }

    /// Retourne l'instance_id de cette découverte.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
}

/// Détecte l'adresse IP locale de la machine.
///
/// Traduit le `_get_local_ip()` Python (socket UDP vers 8.8.8.8).
fn get_local_ip() -> String {
    use std::net::UdpSocket;
    match UdpSocket::bind("0.0.0.0:0") {
        Ok(socket) => {
            if socket.connect("8.8.8.8:80").is_ok() {
                if let Ok(addr) = socket.local_addr() {
                    return addr.ip().to_string();
                }
            }
            "127.0.0.1".to_string()
        }
        Err(_) => "127.0.0.1".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_type_constants() {
        assert_eq!(SERVICE_TYPE_V3, "_ai-manager._tcp.local.");
        assert_eq!(SERVICE_TYPE_V2, "_ai-mgr._tcp.local.");
        // Les deux doivent être différents
        assert_ne!(SERVICE_TYPE_V3, SERVICE_TYPE_V2);
    }

    #[test]
    fn test_peer_event_found_has_protocol() {
        let event = PeerEvent::Found {
            peer_id: "inst-1".to_string(),
            host: "192.168.1.10".to_string(),
            port: 5555,
            protocol: PeerProtocol::V2,
        };
        if let PeerEvent::Found { protocol, .. } = event {
            assert_eq!(protocol, PeerProtocol::V2);
        }
    }

    #[test]
    fn test_handle_removed_v3_fullname() {
        // Simule le parsing d'un fullname V3
        let fullname = "my-inst._ai-manager._tcp.local.";
        let stripped = fullname.trim_end_matches('.');
        let peer_id = stripped
            .trim_end_matches(SERVICE_TYPE_V3.trim_end_matches('.'))
            .trim_end_matches(SERVICE_TYPE_V2.trim_end_matches('.'))
            .trim_end_matches('.')
            .to_string();
        assert_eq!(peer_id, "my-inst");
    }

    #[test]
    fn test_handle_removed_v2_fullname() {
        // Simule le parsing d'un fullname V2
        let fullname = "my-inst._ai-mgr._tcp.local.";
        let stripped = fullname.trim_end_matches('.');
        let peer_id = stripped
            .trim_end_matches(SERVICE_TYPE_V3.trim_end_matches('.'))
            .trim_end_matches(SERVICE_TYPE_V2.trim_end_matches('.'))
            .trim_end_matches('.')
            .to_string();
        assert_eq!(peer_id, "my-inst");
    }
}
