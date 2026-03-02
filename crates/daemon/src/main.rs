//! `ai-daemon` — daemon headless AI Manager v3.
//!
//! Traduit `src/headless_daemon.py` en Rust async avec :
//! - CLI clap (start / stop / status / generate-key / set-key / show-key)
//! - Boucle de refresh OAuth périodique (60s par défaut)
//! - Watcher fichier credentials (notify)
//! - Gestion de signaux Unix (SIGTERM, SIGINT, SIGHUP)
//! - Fichier PID (`/tmp/ai-manager.pid`)
//! - Intégration optionnelle du proxy et de la sync P2P

use std::collections::HashMap;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use parking_lot::RwLock;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use ai_core::config::ConfigCache;
use ai_core::credentials::CredentialsCache;
use ai_core::event_log::EventLog;
use ai_core::types::{ProxyInstanceRuntime, ProxyStatus};
use daemon::http_api::DaemonState;
use daemon::refresh_loop::RefreshLoop;
use daemon::watchdog::CredentialsWatchdog;

/// Chemin du fichier PID.
const PID_FILE: &str = "/tmp/ai-manager.pid";

// ----------------------------------------------------------------
// sdnotify — intégration systemd
// ----------------------------------------------------------------

/// Envoie une notification systemd via le socket NOTIFY_SOCKET.
/// Ne fait rien si NOTIFY_SOCKET n'est pas défini (hors systemd).
/// Sur Windows ce code n'est jamais compilé (cfg unix).
#[cfg(unix)]
fn sd_notify(msg: &str) {
    use std::os::unix::net::UnixDatagram;
    if let Ok(socket_path) = std::env::var("NOTIFY_SOCKET") {
        // Le préfixe '@' indique un socket abstrait Linux (non-fichier).
        // UnixDatagram::send_to ne supporte que les chemins réels, donc
        // on retire le '@' pour obtenir le chemin filesystem.
        let socket_path = socket_path.trim_start_matches('@');
        if let Ok(sock) = UnixDatagram::unbound() {
            let _ = sock.send_to(msg.as_bytes(), socket_path);
        }
    }
}

/// No-op sur Windows (systemd n'existe pas sur cette plateforme).
#[cfg(not(unix))]
fn sd_notify(_msg: &str) {}

/// Chemin par défaut des credentials.
fn default_credentials_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".claude")
        .join("multi-account")
        .join("credentials-multi.json")
}

// ----------------------------------------------------------------
// CLI
// ----------------------------------------------------------------

/// AI Manager v3 — Daemon headless.
///
/// Traduit headless_daemon.py en Rust async. Gère le refresh OAuth périodique,
/// la synchronisation P2P et le proxy optionnel.
#[derive(Parser, Debug)]
#[command(name = "ai-daemon", version, about)]
struct Cli {
    /// Chemin du fichier credentials (défaut: ~/.claude/multi-account/credentials-multi.json)
    #[arg(long, env = "AI_CREDENTIALS_PATH")]
    credentials: Option<PathBuf>,

    /// Intervalle de refresh OAuth en secondes (défaut: 60)
    #[arg(long, default_value = "60", env = "AI_REFRESH_INTERVAL")]
    refresh_interval: u64,

    /// Activer la synchronisation P2P
    #[arg(long, env = "AI_SYNC_ENABLED")]
    sync_enabled: bool,

    /// Port de synchronisation P2P (défaut: 9876)
    #[arg(long, default_value = "9876", env = "AI_SYNC_PORT")]
    sync_port: u16,

    /// Clé symétrique P2P (base64 ou hex 64 chars, 32 bytes)
    #[arg(long, env = "AI_SYNC_KEY")]
    sync_key: Option<String>,

    /// Chemin du fichier settings.json (défaut: ~/.claude/multi-account/settings.json).
    /// Si présent, les valeurs sync/port/key sont lues depuis ce fichier.
    /// Les arguments CLI restent prioritaires (override).
    #[arg(long, env = "AI_SETTINGS_PATH")]
    settings: Option<PathBuf>,

    /// Activer le proxy (défaut: false)
    #[arg(long, env = "AI_PROXY_ENABLED")]
    proxy_enabled: bool,

    /// Port du proxy (défaut: 18080)
    #[arg(long, default_value = "18080", env = "AI_PROXY_PORT")]
    proxy_port: u16,

    /// Activer l'API HTTP REST (défaut: true)
    #[arg(long, default_value = "true", env = "AI_API_ENABLED")]
    api_enabled: bool,

    /// Port de l'API HTTP REST (défaut: 18089)
    #[arg(long, default_value = "18089", env = "AI_API_PORT")]
    api_port: u16,

    /// Token Bearer pour l'API HTTP REST (optionnel)
    #[arg(long, env = "AI_API_TOKEN")]
    api_token: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Démarre le daemon (par défaut si aucune commande)
    Start,
    /// Arrête le daemon via le fichier PID
    Stop,
    /// Affiche le statut du daemon
    Status,
    /// Génère une nouvelle clé de synchronisation P2P
    GenerateKey,
    /// Affiche la clé P2P configurée
    ShowKey,
    /// Définit la clé P2P depuis une valeur base64
    SetKey {
        /// Clé en base64 (32 bytes décodés)
        key: String,
    },
}

// ----------------------------------------------------------------
// Main
// ----------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    // Logging via RUST_LOG (défaut: info)
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        None | Some(Commands::Start) => {
            start_daemon(cli).await?;
        }
        Some(Commands::Stop) => {
            cmd_stop()?;
        }
        Some(Commands::Status) => {
            cmd_status()?;
        }
        Some(Commands::GenerateKey) => {
            cmd_generate_key();
        }
        Some(Commands::ShowKey) => {
            cmd_show_key(&cli.sync_key);
        }
        Some(Commands::SetKey { key }) => {
            cmd_set_key(&key)?;
        }
    }

    Ok(())
}

// ----------------------------------------------------------------
// Commande: start
// ----------------------------------------------------------------

async fn start_daemon(cli: Cli) -> Result<()> {
    // Écrit le PID file
    write_pid_file()?;

    let creds_path = cli
        .credentials
        .unwrap_or_else(default_credentials_path);

    info!("Starting AI Manager daemon");
    info!("Credentials: {:?}", creds_path);
    info!("Refresh interval: {}s", cli.refresh_interval);

    // Crée le répertoire si nécessaire
    if let Some(parent) = creds_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating credentials dir {:?}", parent))?;
    }

    // Charge la configuration depuis settings.json (optionnel)
    let settings_path = cli.settings.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".claude")
            .join("multi-account")
            .join("settings.json")
    });
    let config = ConfigCache::load(&settings_path)
        .with_context(|| format!("loading settings from {:?}", settings_path))?;

    // Résout les paramètres sync : CLI > settings.json > défaut
    let sync_cfg = config.read().sync.clone();
    let sync_enabled = cli.sync_enabled || sync_cfg.enabled;
    let sync_port = if cli.sync_port != 9876 { cli.sync_port } else { sync_cfg.port };
    let sync_key_raw = cli.sync_key.clone().or_else(|| sync_cfg.shared_key_hex.clone());

    if sync_enabled {
        info!("P2P sync config: port={}, key_source={}, options=[account={}, quota={}, split={}, failover={}]",
            sync_port,
            if cli.sync_key.is_some() { "cli" } else if sync_cfg.shared_key_hex.is_some() { "settings.json" } else { "ephemeral" },
            sync_cfg.sync_active_account,
            sync_cfg.sync_quota,
            sync_cfg.split_quota_fetch,
            sync_cfg.proxy_failover,
        );
    }

    // Charge les credentials
    let credentials = CredentialsCache::load(&creds_path)
        .with_context(|| format!("loading credentials from {:?}", creds_path))?;

    info!(
        "Loaded {} account(s)",
        credentials.account_count()
    );

    // Canal de shutdown
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // RefreshLoop
    let refresh_loop = Arc::new(RefreshLoop::new(
        Arc::clone(&credentials),
        cli.refresh_interval,
    ));
    let refresh_rx = shutdown_rx.clone();
    let refresh_handle = {
        let rl = Arc::clone(&refresh_loop);
        tokio::spawn(async move { rl.run(refresh_rx).await })
    };

    // CredentialsWatchdog
    let watchdog = Arc::new(CredentialsWatchdog::new(
        Arc::clone(&credentials),
        creds_path.clone(),
    ));
    let watchdog_rx = shutdown_rx.clone();
    let watchdog_handle = {
        let wd = Arc::clone(&watchdog);
        tokio::spawn(async move { wd.run(watchdog_rx).await })
    };

    // Synchronisation P2P (optionnelle)
    let sync_bus_arc: Option<Arc<ai_sync::bus::SyncBus>> = if sync_enabled {
        let key = parse_sync_key(sync_key_raw.as_deref())?;
        let bus = Arc::new(ai_sync::bus::SyncBus::new(
            generate_instance_id(),
            sync_port,
            key,
        ));
        bus.start().await.with_context(|| "starting sync bus")?;

        // Connecter les pairs configurés dans settings.json
        for peer_cfg in &sync_cfg.peers {
            let protocol = ai_sync::compat::PeerProtocol::Unknown;
            info!("Connecting to configured peer: {}:{} (id={})", peer_cfg.host, peer_cfg.port, peer_cfg.id);
            bus.connect_peer(&peer_cfg.id, &peer_cfg.host, peer_cfg.port, protocol).await;
        }

        Some(bus)
    } else {
        info!("P2P sync disabled");
        None
    };

    let sync_handle = if let Some(ref bus) = sync_bus_arc {
        let instance_id = bus.instance_id().to_string();
        let coordinator = Arc::new(ai_sync::coordinator::SyncCoordinator::new(
            instance_id,
            Arc::clone(bus),
            Arc::clone(&credentials),
        ));
        let coord_rx = shutdown_rx.clone();
        let coord = Arc::clone(&coordinator);
        Some(tokio::spawn(async move {
            if let Err(e) = coord.run(coord_rx).await {
                error!("SyncCoordinator error: {}", e);
            }
        }))
    } else {
        None
    };

    // Proxy (optionnel)
    let proxy_handle = if cli.proxy_enabled {
        let addr: SocketAddr = format!("0.0.0.0:{}", cli.proxy_port)
            .parse()
            .with_context(|| "parsing proxy address")?;
        info!("Starting proxy on {}", addr);
        Some(tokio::spawn(async move {
            if let Err(e) = ai_proxy::server::start(addr).await {
                error!("Proxy server error: {}", e);
            }
        }))
    } else {
        None
    };

    // API HTTP REST (optionnelle)
    let api_handle = if cli.api_enabled {
        let api_addr: SocketAddr = format!("0.0.0.0:{}", cli.api_port)
            .parse()
            .with_context(|| "parsing API address")?;
        if let Some(ref token) = cli.api_token {
            info!(
                "HTTP API listening on {} (Bearer auth enabled, token={}...)",
                api_addr,
                &token[..token.len().min(4)]
            );
        } else {
            info!("HTTP API listening on {} (no auth)", api_addr);
        }
        let event_log = Arc::new(EventLog::new(
            creds_path.parent().unwrap_or(&creds_path),
        ));
        // Pré-peupler la HashMap runtime depuis la config (probe peut alors mettre à jour les statuts)
        let initial_proxy_instances: HashMap<String, Arc<ProxyInstanceRuntime>> = {
            config.read().proxy.instances.iter().map(|inst| {
                let rt = Arc::new(ProxyInstanceRuntime {
                    status: parking_lot::RwLock::new(ProxyStatus {
                        port: inst.port,
                        ..Default::default()
                    }),
                    task_handle: parking_lot::Mutex::new(None),
                    child_process: parking_lot::Mutex::new(None),
                    started_at: parking_lot::Mutex::new(None),
                });
                (inst.id.clone(), rt)
            }).collect()
        };
        let daemon_state = Arc::new(DaemonState {
            credentials: Arc::clone(&credentials),
            config: Arc::clone(&config),
            proxy_instances: Arc::new(RwLock::new(initial_proxy_instances)),
            peers: Arc::new(RwLock::new(Vec::new())),
            velocity_calculators: Arc::new(RwLock::new(HashMap::new())),
            quota_metrics: Arc::new(RwLock::new(HashMap::new())),
            invalid_grant_accounts: Arc::new(RwLock::new(HashSet::new())),
            event_log,
            credentials_path: creds_path.clone(),
            settings_path: settings_path.clone(),
            http_client: reqwest::Client::new(),
            api_token: cli.api_token,
            shutdown_tx: shutdown_tx.clone(),
            sync_bus: sync_bus_arc.clone(),
        });
        let api_rx = shutdown_rx.clone();
        Some(tokio::spawn(async move {
            if let Err(e) = daemon::http_api::serve(daemon_state, api_addr, api_rx).await {
                error!("HTTP API error: {}", e);
            }
        }))
    } else {
        info!("HTTP API disabled");
        None
    };

    // Daemon entièrement démarré — notifier systemd
    sd_notify("READY=1\n");
    info!("Daemon ready — waiting for signals");

    // Boucle de watchdog keepalive : envoie WATCHDOG=1 toutes les 30s.
    // systemd WatchdogSec=60s → marge confortable.
    let (watchdog_cancel_tx, mut watchdog_cancel_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
        ticker.tick().await; // consomme le tick immédiat
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    sd_notify("WATCHDOG=1\n");
                }
                _ = &mut watchdog_cancel_rx => {
                    break;
                }
            }
        }
    });

    wait_for_shutdown(shutdown_tx).await;

    // Signal d'arrêt envoyé à systemd avant le cleanup
    sd_notify("STOPPING=1\n");

    // Annule la boucle watchdog
    let _ = watchdog_cancel_tx.send(());

    // Arrêt propre
    info!("Shutting down...");
    let _ = refresh_handle.await;
    let _ = watchdog_handle.await;
    if let Some(h) = sync_handle {
        let _ = h.await;
    }
    if let Some(h) = proxy_handle {
        h.abort();
    }
    if let Some(h) = api_handle {
        let _ = h.await;
    }

    // Supprime le PID file
    remove_pid_file();
    info!("Daemon stopped cleanly");
    Ok(())
}

// ----------------------------------------------------------------
// Signal handling
// ----------------------------------------------------------------

/// Attend un signal d'arrêt (SIGTERM, SIGINT) ou SIGHUP pour reload.
async fn wait_for_shutdown(shutdown_tx: tokio::sync::watch::Sender<bool>) {
    #[cfg(unix)]
    {
        use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
        use signal_hook_tokio::Signals;
        use futures::StreamExt as _;

        let mut signals = match Signals::new([SIGTERM, SIGINT, SIGHUP]) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to register signal handlers: {}", e);
                // Fallback: attendre Ctrl+C via tokio
                tokio::signal::ctrl_c().await.ok();
                let _ = shutdown_tx.send(true);
                return;
            }
        };

        while let Some(signal) = signals.next().await {
            match signal {
                SIGTERM | SIGINT => {
                    info!("Signal {} received — initiating shutdown", signal);
                    let _ = shutdown_tx.send(true);
                    break;
                }
                SIGHUP => {
                    info!("SIGHUP received — config reload (not yet implemented)");
                }
                _ => {}
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
        info!("Ctrl+C received — shutting down");
        let _ = shutdown_tx.send(true);
    }
}

// ----------------------------------------------------------------
// Commandes utilitaires
// ----------------------------------------------------------------

fn cmd_stop() -> Result<()> {
    let pid = read_pid_file().context("Reading PID file (is the daemon running?)")?;
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        println!("Sent SIGTERM to daemon (PID {})", pid);
    }
    #[cfg(not(unix))]
    {
        eprintln!("stop command not supported on this platform (PID={})", pid);
    }
    Ok(())
}

fn cmd_status() -> Result<()> {
    match read_pid_file() {
        Ok(pid) => {
            println!("AI Manager daemon is running (PID {})", pid);
        }
        Err(_) => {
            println!("AI Manager daemon is not running (no PID file at {})", PID_FILE);
        }
    }
    Ok(())
}

fn cmd_generate_key() {
    let key = ai_sync::bus::SyncBus::generate_key();
    let b64 = base64_encode(&key);
    println!("\nNouvelle clé générée:");
    println!("  {}", b64);
    println!("\nCopiez cette clé sur l'autre instance avec:");
    println!("  ai-daemon --sync-key {}", b64);
    println!("  # ou: ai-daemon set-key {}\n", b64);
}

fn cmd_show_key(configured_key: &Option<String>) {
    match configured_key {
        Some(key) => println!("\nClé configurée (via --sync-key ou AI_SYNC_KEY):\n  {}\n", key),
        None => println!(
            "\nAucune clé configurée. Utilisez 'generate-key' pour en créer une.\n"
        ),
    }
}

fn cmd_set_key(key_b64: &str) -> Result<()> {
    let decoded = base64_decode(key_b64).context("decoding base64 key")?;
    if decoded.len() != 32 {
        anyhow::bail!(
            "Invalid key length: {} bytes (expected 32)",
            decoded.len()
        );
    }
    println!("Clé valide ({} bytes). Utilisez-la avec:", decoded.len());
    println!("  export AI_SYNC_KEY={}", key_b64);
    println!("  ai-daemon --sync-key {} start", key_b64);
    Ok(())
}

// ----------------------------------------------------------------
// Helpers PID file
// ----------------------------------------------------------------

fn write_pid_file() -> Result<()> {
    let pid = std::process::id();
    std::fs::write(PID_FILE, pid.to_string())
        .with_context(|| format!("writing PID file {}", PID_FILE))?;
    info!("PID file written: {} (PID={})", PID_FILE, pid);
    Ok(())
}

fn remove_pid_file() {
    if let Err(e) = std::fs::remove_file(PID_FILE) {
        warn!("Failed to remove PID file {}: {}", PID_FILE, e);
    }
}

fn read_pid_file() -> Result<u32> {
    let content = std::fs::read_to_string(PID_FILE)
        .with_context(|| format!("reading PID file {}", PID_FILE))?;
    content
        .trim()
        .parse::<u32>()
        .with_context(|| format!("parsing PID from {}", PID_FILE))
}

// ----------------------------------------------------------------
// Helpers crypto / instance ID
// ----------------------------------------------------------------

fn parse_sync_key(key_str: Option<&str>) -> Result<[u8; 32]> {
    match key_str {
        Some(s) => {
            // Auto-detect format: 64 hex chars = hex, sinon base64
            let decoded = if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                // Hex format (from settings.json / GUI)
                hex::decode(s).context("decoding hex sync key")?
            } else {
                // Base64 format (from CLI --sync-key)
                base64_decode(s).context("decoding base64 sync key")?
            };
            if decoded.len() != 32 {
                anyhow::bail!(
                    "Invalid sync key length: {} bytes (expected 32)",
                    decoded.len()
                );
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&decoded);
            Ok(key)
        }
        None => {
            // Génère une clé aléatoire si aucune n'est configurée
            warn!("No sync key configured — generating ephemeral key (not shared with peers!)");
            Ok(ai_sync::bus::SyncBus::generate_key())
        }
    }
}

/// Génère un instance_id unique (8 hex chars).
fn generate_instance_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let pid = std::process::id();
    format!("{:04x}{:04x}", t & 0xFFFF, pid & 0xFFFF)
}

/// Encode en base64 (sans padding, RFC 4648 §5).
fn base64_encode(data: &[u8]) -> String {
    // Implémentation simple sans dépendance externe
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let combined = (b0 << 16) | (b1 << 8) | b2;
        result.push(ALPHABET[((combined >> 18) & 0x3F) as usize] as char);
        result.push(ALPHABET[((combined >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((combined >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(combined & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Décode du base64 standard.
fn base64_decode(s: &str) -> Result<Vec<u8>> {
    const DECODE_TABLE: [i8; 256] = {
        let mut t = [-1i8; 256];
        let mut i = 0usize;
        let enc = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        while i < 64 {
            t[enc[i] as usize] = i as i8;
            i += 1;
        }
        t
    };

    let s = s.trim().trim_end_matches('=');
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let b0 = DECODE_TABLE[bytes[i] as usize];
        let b1 = DECODE_TABLE[bytes[i + 1] as usize];
        if b0 < 0 || b1 < 0 {
            anyhow::bail!("invalid base64 character");
        }
        result.push(((b0 as u8) << 2) | ((b1 as u8) >> 4));
        if i + 2 < bytes.len() {
            let b2 = DECODE_TABLE[bytes[i + 2] as usize];
            if b2 < 0 {
                anyhow::bail!("invalid base64 character");
            }
            result.push(((b1 as u8) << 4) | ((b2 as u8) >> 2));
            if i + 3 < bytes.len() {
                let b3 = DECODE_TABLE[bytes[i + 3] as usize];
                if b3 < 0 {
                    anyhow::bail!("invalid base64 character");
                }
                result.push(((b2 as u8) << 6) | (b3 as u8));
            }
        }
        i += 4;
    }
    Ok(result)
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Sans NOTIFY_SOCKET défini, sd_notify ne doit pas paniquer.
    #[test]
    fn test_sd_notify_no_socket() {
        std::env::remove_var("NOTIFY_SOCKET");
        // Doit être silencieux — aucune panique attendue.
        sd_notify("READY=1\n");
        sd_notify("WATCHDOG=1\n");
        sd_notify("STOPPING=1\n");
    }

    /// Avec NOTIFY_SOCKET défini sur un chemin inexistant, sd_notify
    /// doit échouer silencieusement (send_to retourne une erreur ignorée).
    #[test]
    fn test_sd_notify_bad_socket_path() {
        // Chemin inexistant → send_to échoue, on ignore l'erreur
        std::env::set_var("NOTIFY_SOCKET", "/tmp/ai-manager-test-nonexistent-notify.sock");
        sd_notify("READY=1\n");
        std::env::remove_var("NOTIFY_SOCKET");
    }
}
