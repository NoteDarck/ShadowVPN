use anyhow::{Context, Result};
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use bytes::BytesMut;
use futures::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tun::Configuration;

/// Inicia o loop do túnel WireGuard.
/// `peers` contém as chaves públicas e endpoints dos outros computadores.
pub async fn run_tunnel(
    private_key: StaticSecret,
    virtual_ip: &str,
    peers: Vec<(PublicKey, SocketAddr)>,
) -> Result<()> {
    // 1. Cria interface TUN com o IP virtual
    let mut config = Configuration::default();
    let ip_parts: Vec<u8> = virtual_ip.split('.').map(|s| s.parse().unwrap()).collect();
    config
        .address((ip_parts[0], ip_parts[1], ip_parts[2], ip_parts[3]))
        .netmask((255, 255, 255, 0))
        .destination((10, 0, 0, 1))
        .up();

    #[cfg(target_os = "linux")]
    config.platform_config(|cfg| {
        cfg.ensure_root_privileges(true);
    });

    let mut dev = tun::create_as_async(&config)
        .context("Falha ao criar TUN (execute como root)")?;
    let mut framed = dev.into_framed();

    // 2. Socket UDP para WireGuard
    let udp = UdpSocket::bind("0.0.0.0:0").await?;

    // 3. Cria um túnel para cada peer (na prática, gerenciar múltiplos peers exige
    //    um mapa de Tunn; simplificamos para o primeiro peer aqui)
    if peers.is_empty() {
        anyhow::bail!("Nenhum peer disponível no grupo");
    }

    let (peer_public, peer_addr) = &peers[0];
    let mut tunn = Tunn::new(
        private_key,
        *peer_public,
        None,
        None,
        0,
        None,
    ).map_err(|e| anyhow::anyhow!("Túnel: {:?}", e))?;

    let mut udp_buf = [0u8; 65535];
    let mut wg_buf = [0u8; 65535];
    let mut out_buf = [0u8; 65535];

    // Handshake inicial
    if let TunnResult::WriteToNetwork(packet) =
        tunn.format_handshake_initiation(&mut wg_buf, false)
    {
        udp.send_to(packet, peer_addr).await?;
    }

    println!("ShadowVPN ativo em {}. Peer: {}", virtual_ip, peer_addr);

    // 4. Loop principal
    loop {
        tokio::select! {
            Some(packet) = framed.next() => {
                let packet = packet?;
                if let TunnResult::WriteToNetwork(data) = tunn.encapsulate(&packet, &mut wg_buf) {
                    udp.send_to(data, peer_addr).await?;
                }
            }
            Ok((len, _)) = udp.recv_from(&mut udp_buf) => {
                match tunn.decapsulate(None, &udp_buf[..len], &mut wg_buf) {
                    TunnResult::WriteToNetwork(response) => {
                        udp.send_to(response, peer_addr).await?;
                    }
                    TunnResult::WriteToTunnel(packet) => {
                        framed.send(BytesMut::from(packet)).await?;
                    }
                    _ => {}
                }
            }
        }
    }
}