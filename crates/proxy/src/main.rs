//! Crate `proxy` — serveur proxy Axum pour AI Manager v3.
//!
//! Binaire principal : `ai-proxy`

fn main() {
    // Parse --port <N> depuis les arguments CLI (fallback 18080)
    let port = std::env::args()
        .skip_while(|a| a != "--port")
        .nth(1)
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(18080);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let addr: std::net::SocketAddr =
            format!("0.0.0.0:{}", port).parse().expect("invalid addr");
        if let Err(e) = proxy::server::start(addr).await {
            eprintln!("proxy error: {e}");
            std::process::exit(1);
        }
    });
}
