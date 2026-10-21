mod api;
mod tunnel;

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use boringtun::x25519::{PublicKey, StaticSecret};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<()> {
    let server_url = env_or("SHADOWVPN_SERVER", "http://127.0.0.1:8080");
    let group_name = env_or("SHADOWVPN_GROUP", "meu-grupo");
    let group_password = std::env::var("SHADOWVPN_PASSWORD")
        .context("defina SHADOWVPN_PASSWORD (mínimo de 8 caracteres)")?;
    let endpoint = env_or("SHADOWVPN_ENDPOINT", "127.0.0.1:51820");

    let private = StaticSecret::random_from_rng(rand_core::OsRng);
    let public_b64 = BASE64.encode(PublicKey::from(&private).as_bytes());
    let resp = api::register(
        &server_url,
        &group_name,
        &group_password,
        &public_b64,
        &endpoint,
    )
    .await?;
    anyhow::ensure!(
        !resp.virtual_ip.is_empty(),
        "servidor não atribuiu um IP virtual"
    );
    println!("IP virtual atribuído: {}", resp.virtual_ip);

    let peers: Vec<(PublicKey, SocketAddr)> = resp
        .peers
        .iter()
        .filter_map(|p| {
            let key_bytes = BASE64.decode(&p.public_key).ok()?;
            let key = PublicKey::from(<[u8; 32]>::try_from(key_bytes.as_slice()).ok()?);
            Some((key, p.endpoint.parse().ok()?))
        })
        .collect();
    if peers.is_empty() {
        println!("Nenhum peer conectado; aguardando outra conexão.");
        return Ok(());
    }
    tunnel::run_tunnel(private, &resp.virtual_ip, peers).await
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}
