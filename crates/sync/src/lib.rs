//! Crate `sync` — synchronisation P2P distribuée pour AI Manager v3.
//!
//! Traduit la logique Python de `sync/discovery.py`, `sync_bus.py` et
//! `sync_coordinator.py` en Rust async avec chiffrement NaCl.
//!
//! # Architecture
//!
//! ```text
//! PeerDiscovery (mDNS) ──→ SyncBus (TCP + NaCl) ──→ SyncCoordinator (LWW)
//!      |                         |                          |
//!   découvre                 chiffre /               réconcilie les
//!   les pairs              déchiffre les             credentials via
//!   sur LAN               messages P2P             vector clocks LWW
//! ```

pub mod bus;
pub mod compat;
pub mod coordinator;
pub mod discovery;
pub mod error;
pub mod failure_detector;
pub mod messages;
pub mod outbox;
pub mod ssh;
