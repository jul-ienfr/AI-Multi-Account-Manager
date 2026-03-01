//! Crate `proxy` — serveur proxy Axum pour AI Manager v3.
//!
//! Binaire principal : `ai-proxy`

fn main() {
    // Déléguer à server::run() une fois implémenté
    // Pour l'instant, utilise le runtime Tokio minimal
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let addr: std::net::SocketAddr = "0.0.0.0:18080".parse().expect("invalid addr");
        if let Err(e) = proxy::server::start(addr).await {
            eprintln!("proxy error: {e}");
            std::process::exit(1);
        }
    });
}
