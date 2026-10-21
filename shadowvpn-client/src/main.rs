mod api;
mod tunnel;

use anyhow::Result;
use boringtun::x25519::{PublicKey, StaticSecret};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<()> {
    // Configurações via argumentos ou hardcoded para teste
    let server_url = "http://SEU_SERVIDOR:8080";
    let group_name = "meu-grupo";
    let group_password = "senha123";

    // 1. Gera par de chaves WireGuard
    let private = StaticSecret::random_from_rng(rand_core::OsRng);
    let public = PublicKey::from(&private);
    let public_b64 = BASE64.encode(public.as_bytes());

    // Endpoint público deste cliente (em produção, descubra via STUN ou config)
    let my_endpoint = "203.0.113.5:51820";

    // 2. Registra no servidor de coordenação
    let resp = api::register(server_url, group_name, group_password, &public_b64, my_endpoint).await?;
    println!("IP virtual atribuído: {}", resp.virtual_ip);

    // 3. Monta lista de peers (chave pública + endpoint)
    let peers: Vec<(PublicKey, SocketAddr)> = resp.peers
        .iter()
        .filter_map(|p| {
            let key_bytes = BASE64.decode(&p.public_key).ok()?;
            let key = PublicKey::from(<[u8; 32]>::try_from(key_bytes.as_slice()).ok()?);
            let addr: SocketAddr = p.endpoint.parse().ok()?;
            Some((key, addr))
        })
        .collect();

    if peers.is_empty() {
        println!("Aguardando outros peers entrarem no grupo...");
        // Em produção: loop de polling para buscar novos peers
        return Ok(());
    }

    // 4. Inicia o túnel
    tunnel::run_tunnel(private, &resp.virtual_ip, peers).await
}