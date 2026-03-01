//! Crate `daemon` — daemon headless AI Manager v3.
//!
//! Traduit `src/headless_daemon.py` en Rust async avec :
//! - CLI clap (start / stop / status / generate-key)
//! - Boucle de refresh OAuth périodique (60s)
//! - Watcher fichier credentials (notify)
//! - Gestion de signaux (SIGTERM, SIGINT, SIGHUP sur Unix)
//! - Fichier PID (`/tmp/ai-manager.pid`)
//! - Intégration optionnelle du proxy et de la sync P2P
//! - API HTTP REST (`/admin/api/...`) via Axum 0.8

pub mod dto;
pub mod http_api;
pub mod refresh_loop;
pub mod watchdog;
