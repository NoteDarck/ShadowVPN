mod relay;
mod routes;
mod state;

use anyhow::{Context, Result};
use axum::{
    routing::{get, post},
    Router,
};
use shadowvpn_core::{DEFAULT_HTTP_PORT, DEFAULT_PEER_TTL_SECS, DEFAULT_RELAY_PORT};
use state::AppState;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let http_listen = env_address("SHADOWVPN_LISTEN", DEFAULT_HTTP_PORT)?;
    let relay_listen = env_address("SHADOWVPN_RELAY_LISTEN", DEFAULT_RELAY_PORT)?;
    let peer_ttl = env_peer_ttl();

    let relay_socket = relay::bind(relay_listen).await?;
    let relay_port = relay_socket.local_addr()?.port();
    let relay_advertise = advertised_relay(relay_port);

    let networks_file = env_networks_file();
    let state =
        AppState::with_networks_file(peer_ttl, relay_port, relay_advertise, networks_file.clone())
            .with_context(|| {
                format!(
                    "não foi possível carregar as redes de {}",
                    networks_file.display()
                )
            })?;
    tokio::spawn(relay::run(state.clone(), relay_socket));

    let cleanup_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(15));
        loop {
            ticker.tick().await;
            let removed = cleanup_state.cleanup();
            if removed > 0 {
                tracing::debug!(removed, "peers expirados removidos");
            }
        }
    });

    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(routes::health))
        .route("/networks", post(routes::create_network))
        .route("/register", post(routes::register))
        .route("/peers", post(routes::peers))
        .route("/unregister", post(routes::unregister))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(http_listen)
        .await
        .with_context(|| format!("não foi possível escutar em {http_listen}"))?;

    let http_bound = listener.local_addr()?;
    tracing::info!(address = %http_bound, "HTTP de coordenação ativo");
    tracing::info!(
        path = %networks_file.display(),
        "redes persistidas neste arquivo"
    );
    tracing::info!(ttl_secs = peer_ttl.as_secs(), "ShadowVPN iniciado");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn root() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "name": "ShadowVPN",
        "version": env!("CARGO_PKG_VERSION"),
        "api": ["/networks", "/register", "/peers", "/unregister", "/health"]
    }))
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env("SHADOWVPN_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn env_address(name: &str, default_port: u16) -> Result<SocketAddr> {
    let value = std::env::var(name).unwrap_or_else(|_| format!("0.0.0.0:{default_port}"));
    value
        .parse()
        .with_context(|| format!("{name} deve ser um endereço IP:porta válido"))
}

fn env_networks_file() -> PathBuf {
    std::env::var_os("SHADOWVPN_NETWORKS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/networks.json"))
}

fn env_peer_ttl() -> Duration {
    let seconds = std::env::var("SHADOWVPN_PEER_TTL")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_PEER_TTL_SECS);
    Duration::from_secs(seconds.max(30))
}

fn advertised_relay(relay_port: u16) -> Option<String> {
    let value = std::env::var("SHADOWVPN_RELAY_ADVERTISE").ok()?;
    normalize_advertised(&value, relay_port)
}

fn normalize_advertised(value: &str, relay_port: u16) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if value.starts_with('[') {
        if value.contains("]:") {
            return Some(value.to_string());
        }
        return Some(format!("{value}:{relay_port}"));
    }

    if let Ok(address) = value.parse::<std::net::IpAddr>() {
        return Some(if address.is_ipv6() {
            format!("[{address}]:{relay_port}")
        } else {
            format!("{address}:{relay_port}")
        });
    }

    if value.contains(':') {
        Some(value.to_string())
    } else {
        Some(format!("{value}:{relay_port}"))
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let terminate = signal(SignalKind::terminate());
        match terminate {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }

    tracing::info!("encerrando ShadowVPN");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_advertised_relay() {
        assert_eq!(normalize_advertised("", 8081), None);
        assert_eq!(
            normalize_advertised("vpn.example.com", 8081),
            Some("vpn.example.com:8081".to_string())
        );
        assert_eq!(
            normalize_advertised("2001:db8::10", 8081),
            Some("[2001:db8::10]:8081".to_string())
        );
        assert_eq!(
            normalize_advertised("[2001:db8::10]", 8081),
            Some("[2001:db8::10]:8081".to_string())
        );
        assert_eq!(
            normalize_advertised("vpn.example.com:9000", 8081),
            Some("vpn.example.com:9000".to_string())
        );
    }
}
