mod relay;
mod routes;
mod state;

use anyhow::{bail, Context, Result};
use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};
use rand_core::{OsRng, RngCore};
use shadowvpn_core::{DEFAULT_HTTP_PORT, DEFAULT_PEER_TTL_SECS, DEFAULT_RELAY_PORT};
use state::AppState;
use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let http_listen = env_address("SHADOWVPN_LISTEN", DEFAULT_HTTP_PORT)?;
    let relay_listen = env_address("SHADOWVPN_RELAY_LISTEN", DEFAULT_RELAY_PORT)?;
    let peer_ttl = env_peer_ttl();
    let admin_token = env_admin_token()?;
    let max_networks = env_max_networks();

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
            })?
            .with_admin_token(admin_token.clone())
            .with_max_networks(max_networks);
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
        .route("/admin/networks", get(routes::admin_list_networks))
        .route(
            "/admin/networks/:name",
            delete(routes::admin_delete_network),
        )
        .route(
            "/admin/networks/:name/password",
            post(routes::admin_rotate_password),
        )
        .route(
            "/admin/networks/:name/members",
            get(routes::admin_list_members).delete(routes::admin_revoke_member),
        )
        .route(
            "/admin/networks/:name/members/restore",
            post(routes::admin_restore_member),
        )
        .layer(DefaultBodyLimit::max(16 * 1024))
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
    tracing::info!(
        ttl_secs = peer_ttl.as_secs(),
        max_networks,
        admin_auth = admin_token.is_some(),
        "ShadowVPN iniciado"
    );
    if admin_token.is_none() {
        tracing::warn!(
            "criação de redes sem autenticação; defina SHADOWVPN_ADMIN_TOKEN ou SHADOWVPN_ADMIN_TOKEN_FILE"
        );
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn root() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "name": "ShadowVPN",
        "version": env!("CARGO_PKG_VERSION"),
        "api": [
            "/networks",
            "/register",
            "/peers",
            "/unregister",
            "/admin/networks",
            "/admin/networks/{name}/members",
            "/health"
        ]
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

fn env_max_networks() -> usize {
    std::env::var("SHADOWVPN_MAX_NETWORKS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1_000)
        .max(1)
}

fn env_admin_token() -> Result<Option<String>> {
    if let Ok(value) = std::env::var("SHADOWVPN_ADMIN_TOKEN") {
        let value = value.trim();
        if !value.is_empty() {
            if value.len() < 16 {
                bail!("SHADOWVPN_ADMIN_TOKEN deve ter pelo menos 16 caracteres");
            }
            return Ok(Some(value.to_string()));
        }
    }

    let Some(path) = std::env::var_os("SHADOWVPN_ADMIN_TOKEN_FILE") else {
        return Ok(None);
    };
    let token = load_or_create_admin_token(Path::new(&path))?;
    Ok(Some(token))
}

fn load_or_create_admin_token(path: &Path) -> Result<String> {
    if path.exists() {
        return read_admin_token(path);
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("não foi possível criar {}", parent.display()))?;
    }

    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    match options.open(path) {
        Ok(mut file) => {
            file.write_all(token.as_bytes())
                .with_context(|| format!("não foi possível escrever {}", path.display()))?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            tracing::info!(path = %path.display(), "token de administração criado");
            Ok(token)
        }
        Err(error) if path.exists() => {
            tracing::debug!(%error, path = %path.display(), "token criado por outra instância");
            read_admin_token(path)
        }
        Err(error) => {
            Err(error).with_context(|| format!("não foi possível criar {}", path.display()))
        }
    }
}

fn read_admin_token(path: &Path) -> Result<String> {
    let token = fs::read_to_string(path)
        .with_context(|| format!("não foi possível ler {}", path.display()))?
        .trim()
        .to_string();
    if token.len() < 16 {
        bail!(
            "token de administração inválido em {}; use pelo menos 16 caracteres",
            path.display()
        );
    }
    Ok(token)
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
    fn creates_and_reuses_admin_token_file() {
        let directory = tempfile::tempdir().expect("diretório temporário");
        let path = directory.path().join("admin-token");
        let first = load_or_create_admin_token(&path).expect("criar token");
        assert!(first.len() >= 64);
        assert_eq!(load_or_create_admin_token(&path).unwrap(), first);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn rejects_short_admin_token_file() {
        let directory = tempfile::tempdir().expect("diretório temporário");
        let path = directory.path().join("admin-token");
        fs::write(&path, "curto\n").unwrap();
        assert!(load_or_create_admin_token(&path).is_err());
    }

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
