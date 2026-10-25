use crate::state::AppState;
use shadowvpn_core::{parse_relay_packet, RelayKind};
use std::net::SocketAddr;
use tokio::net::UdpSocket;

const MAX_DATAGRAM: usize = 65_536;

pub async fn run(state: AppState, socket: UdpSocket) {
    let mut buffer = vec![0u8; MAX_DATAGRAM];

    loop {
        let (len, source) = match socket.recv_from(&mut buffer).await {
            Ok(received) => received,
            Err(error) => {
                tracing::warn!(%error, "falha ao receber datagrama no relay");
                continue;
            }
        };

        let Some(packet) = parse_relay_packet(&buffer[..len]) else {
            continue;
        };

        match packet.kind {
            RelayKind::Hello => {
                state.relay_bind(packet.sender, source);
            }
            RelayKind::Data => {
                if let Some(destination) =
                    state.relay_route(packet.sender, packet.destination, source)
                {
                    if let Err(error) = socket.send_to(&buffer[..len], destination).await {
                        tracing::debug!(%error, %destination, "falha ao encaminhar datagrama");
                    }
                }
            }
        }
    }
}

pub async fn bind(address: SocketAddr) -> std::io::Result<UdpSocket> {
    let socket = UdpSocket::bind(address).await?;
    tracing::info!(local = %socket.local_addr()?, "relay UDP ativo");
    Ok(socket)
}
