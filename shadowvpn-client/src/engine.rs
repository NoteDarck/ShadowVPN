use crate::api::Api;
use crate::config::ClientConfig;
use crate::identity;
use crate::tunnel::{self, TunFramed};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use futures::{SinkExt, StreamExt};
use shadowvpn_core::{
    encode_relay_packet, parse_relay_packet, valid_public_key, PeerInfo, RelayKind, HEARTBEAT_SECS,
    RELAY_HEADER_LEN, VIRTUAL_NETWORK_PREFIX,
};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::time::{interval, interval_at, Instant, MissedTickBehavior};

const PACKET_BUFFER: usize = 65_536;
const TIMER_INTERVAL: Duration = Duration::from_millis(250);
const WIREGUARD_KEEPALIVE_SECS: u16 = 25;

#[derive(Debug, Clone)]
pub enum ClientEvent {
    Registered {
        hostname: String,
        virtual_ip: String,
        relay: String,
        public_key: String,
        peers: Vec<PeerInfo>,
    },
    Peers(Vec<PeerInfo>),
    Status(String),
    Error(String),
    Finished,
}

pub struct ClientHandle {
    stop: watch::Sender<bool>,
    events: Receiver<ClientEvent>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ClientHandle {
    pub fn spawn(config: ClientConfig) -> Result<Self> {
        config.validate()?;
        let config = config.normalized();

        let (event_tx, event_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = watch::channel(false);
        let thread_events = event_tx.clone();

        let thread = std::thread::Builder::new()
            .name("shadowvpn-client".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = thread_events.send(ClientEvent::Error(format!("runtime: {error}")));
                        let _ = thread_events.send(ClientEvent::Finished);
                        return;
                    }
                };

                if let Err(error) =
                    runtime.block_on(run(config, stop_rx, Some(thread_events.clone())))
                {
                    let _ = thread_events.send(ClientEvent::Error(format!("{error:#}")));
                }
                let _ = thread_events.send(ClientEvent::Finished);
            })
            .context("não foi possível iniciar a thread do cliente")?;

        Ok(Self {
            stop: stop_tx,
            events: event_rx,
            thread: Some(thread),
        })
    }

    pub fn try_recv(&self) -> Option<ClientEvent> {
        self.events.try_recv().ok()
    }

    pub fn stop(&mut self) {
        let _ = self.stop.send(true);
    }

    pub fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .map(std::thread::JoinHandle::is_finished)
            .unwrap_or(true)
    }
}

impl Drop for ClientHandle {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

pub async fn run(
    config: ClientConfig,
    shutdown: watch::Receiver<bool>,
    events: Option<Sender<ClientEvent>>,
) -> Result<()> {
    config.validate()?;
    let config = config.normalized();

    emit(
        &events,
        ClientEvent::Status("Carregando identidade...".into()),
    );

    let identity_path = config
        .identity_path
        .clone()
        .map(Ok)
        .unwrap_or_else(identity::default_identity_path)?;
    let secret = identity::load_or_create(&identity_path)?;
    let public_key = identity::public_key_b64(&secret);
    let hostname = local_hostname();

    let api = Api::new(&config.server)?;

    emit(
        &events,
        ClientEvent::Status("Registrando no grupo...".into()),
    );
    let registered = api
        .register(
            &config.group_name,
            &config.group_password,
            &public_key,
            &hostname,
        )
        .await?;

    let own_ip: Ipv4Addr = registered
        .virtual_ip
        .parse()
        .with_context(|| format!("IP virtual inválido: {}", registered.virtual_ip))?;
    let relay = match resolve_relay(&registered.relay).await {
        Ok(relay) => relay,
        Err(error) => {
            cleanup_registration(&api, &config, registered.session_id).await;
            return Err(error);
        }
    };
    let mut own_session = registered.session_id;

    let framed = match tunnel::create(own_ip) {
        Ok(framed) => framed,
        Err(error) => {
            cleanup_registration(&api, &config, own_session).await;
            return Err(error);
        }
    };

    let udp = match bind_udp(relay).await {
        Ok(udp) => udp,
        Err(error) => {
            cleanup_registration(&api, &config, own_session).await;
            return Err(error);
        }
    };
    let mut peers = PeerSet::default();
    let peer_list = peers.reconcile(&registered.peers, &secret);
    if let Err(error) = send_hello(&udp, relay, own_session).await {
        cleanup_registration(&api, &config, own_session).await;
        return Err(error);
    }

    emit(
        &events,
        ClientEvent::Registered {
            hostname: hostname.clone(),
            virtual_ip: own_ip.to_string(),
            relay: relay.to_string(),
            public_key: public_key.clone(),
            peers: peer_list.clone(),
        },
    );
    emit(&events, ClientEvent::Peers(peer_list));

    tracing::info!(%own_ip, %relay, peers = peers.len(), "ShadowVPN conectado");

    let result = session_loop(
        &udp,
        relay,
        own_ip,
        &secret,
        &mut own_session,
        framed,
        &mut peers,
        &shutdown,
        &events,
        &api,
        &config,
        &public_key,
        &hostname,
    )
    .await;

    emit(&events, ClientEvent::Status("Desconectando...".into()));
    cleanup_registration(&api, &config, own_session).await;

    tracing::info!("ShadowVPN desconectado");
    result
}

#[allow(clippy::too_many_arguments)]
async fn session_loop(
    udp: &UdpSocket,
    relay: SocketAddr,
    own_ip: Ipv4Addr,
    secret: &StaticSecret,
    own_session: &mut u64,
    mut framed: TunFramed,
    peers: &mut PeerSet,
    shutdown: &watch::Receiver<bool>,
    events: &Option<Sender<ClientEvent>>,
    api: &Api,
    config: &ClientConfig,
    public_key: &str,
    hostname: &str,
) -> Result<()> {
    let mut udp_buffer = vec![0u8; PACKET_BUFFER];
    let mut timer = interval(TIMER_INTERVAL);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let (heartbeat_tx, mut heartbeat_rx) = tokio::sync::mpsc::channel(1);
    let heartbeat_api = api.clone();
    let heartbeat_config = config.clone();
    let heartbeat_public_key = public_key.to_string();
    let heartbeat_hostname = hostname.to_string();
    let mut heartbeat_tasks = tokio::task::JoinSet::new();
    heartbeat_tasks.spawn(async move {
        let period = Duration::from_secs(HEARTBEAT_SECS);
        let mut ticker = interval_at(Instant::now() + period, period);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let result = heartbeat_api
                .register(
                    &heartbeat_config.group_name,
                    &heartbeat_config.group_password,
                    &heartbeat_public_key,
                    &heartbeat_hostname,
                )
                .await;
            if heartbeat_tx.send(result).await.is_err() {
                break;
            }
        }
    });

    loop {
        if *shutdown.borrow() {
            break;
        }

        tokio::select! {
            packet = framed.next() => {
                let packet = match packet {
                    Some(Ok(packet)) => packet,
                    Some(Err(error)) => return Err(error).context("erro ao ler da interface TUN"),
                    None => bail!("a interface TUN foi encerrada"),
                };

                handle_tun_packet(
                    packet,
                    peers,
                    udp,
                    relay,
                    *own_session,
                ).await?;
            }
            received = udp.recv_from(&mut udp_buffer) => {
                let (len, from) = received.context("erro ao receber do relay")?;
                if from.ip() != relay.ip() {
                    tracing::trace!(%from, "datagrama ignorado: origem diferente do relay");
                    continue;
                }
                handle_relay_packet(
                    &udp_buffer[..len],
                    peers,
                    &mut framed,
                    udp,
                    relay,
                    *own_session,
                ).await?;
            }
            _ = timer.tick() => {
                let outputs = peers.update_timers();
                for (destination, payload) in outputs {
                    send_relay(
                        udp,
                        relay,
                        *own_session,
                        destination,
                        RelayKind::Data,
                        &payload,
                    ).await?;
                }
            }
            result = heartbeat_rx.recv() => {
                let Some(result) = result else {
                    bail!("a tarefa de heartbeat encerrou inesperadamente");
                };
                match result {
                    Ok(response) => {
                        if response.virtual_ip != own_ip.to_string() {
                            bail!("o servidor atribuiu um novo IP virtual; reconecte");
                        }
                        if response.session_id != *own_session {
                            *own_session = response.session_id;
                        }
                        let refreshed = peers.reconcile(&response.peers, secret);
                        emit(events, ClientEvent::Peers(refreshed));
                        send_hello(udp, relay, *own_session).await?;
                        tracing::debug!(peers = peers.len(), "heartbeat enviado");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "falha no heartbeat; mantendo túnel");
                        emit(
                            events,
                            ClientEvent::Status(format!("Falha no heartbeat: {error:#}")),
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

async fn handle_tun_packet(
    packet: Vec<u8>,
    peers: &mut PeerSet,
    udp: &UdpSocket,
    relay: SocketAddr,
    own_session: u64,
) -> Result<()> {
    let Some(IpAddr::V4(destination)) = Tunn::dst_address(&packet) else {
        return Ok(());
    };

    if destination == broadcast_address() || destination == Ipv4Addr::BROADCAST {
        for peer in peers.peers.values_mut() {
            let destination_session = peer.info.session_id;
            if let Some(payload) = encapsulate(&mut peer.tunn, &packet) {
                send_relay(
                    udp,
                    relay,
                    own_session,
                    destination_session,
                    RelayKind::Data,
                    &payload,
                )
                .await?;
            }
        }
        return Ok(());
    }

    let Some(peer_key) = peers.by_ip.get(&destination).cloned() else {
        tracing::trace!(%destination, "sem peer para o IP de destino");
        return Ok(());
    };

    let Some(peer) = peers.peers.get_mut(&peer_key) else {
        return Ok(());
    };
    let destination_session = peer.info.session_id;
    let Some(payload) = encapsulate(&mut peer.tunn, &packet) else {
        return Ok(());
    };

    send_relay(
        udp,
        relay,
        own_session,
        destination_session,
        RelayKind::Data,
        &payload,
    )
    .await?;

    Ok(())
}

async fn handle_relay_packet(
    datagram: &[u8],
    peers: &mut PeerSet,
    framed: &mut TunFramed,
    udp: &UdpSocket,
    relay: SocketAddr,
    own_session: u64,
) -> Result<()> {
    let Some(packet) = parse_relay_packet(datagram) else {
        return Ok(());
    };
    if packet.kind != RelayKind::Data || packet.sender == own_session || packet.sender == 0 {
        return Ok(());
    }

    let Some(peer_key) = peers.by_session.get(&packet.sender).cloned() else {
        tracing::trace!(sender = packet.sender, "peer desconhecido no relay");
        return Ok(());
    };
    let Some(peer) = peers.peers.get_mut(&peer_key) else {
        return Ok(());
    };

    let (network, tunnel) = drain_decapsulate(&mut peer.tunn, packet.payload, relay.ip());

    for payload in network {
        send_relay(
            udp,
            relay,
            own_session,
            packet.sender,
            RelayKind::Data,
            &payload,
        )
        .await?;
    }

    for packet in tunnel {
        framed
            .send(packet)
            .await
            .context("erro ao escrever pacote na interface TUN")?;
    }

    Ok(())
}

fn encapsulate(tunn: &mut Tunn, packet: &[u8]) -> Option<Vec<u8>> {
    let mut output = vec![0u8; PACKET_BUFFER];
    match tunn.encapsulate(packet, &mut output) {
        TunnResult::WriteToNetwork(data) => Some(data.to_vec()),
        TunnResult::Err(error) => {
            tracing::debug!(?error, "erro ao encapsular pacote WireGuard");
            None
        }
        _ => None,
    }
}

fn drain_decapsulate(
    tunn: &mut Tunn,
    datagram: &[u8],
    source: IpAddr,
) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut network = Vec::new();
    let mut tunnel = Vec::new();
    let mut output = vec![0u8; PACKET_BUFFER];
    let mut input = datagram;

    loop {
        let result = tunn.decapsulate(Some(source), input, &mut output);
        input = &[];

        match result {
            TunnResult::Done => break,
            TunnResult::Err(error) => {
                tracing::debug!(?error, "erro ao desencapsular pacote WireGuard");
                break;
            }
            TunnResult::WriteToNetwork(data) => network.push(data.to_vec()),
            TunnResult::WriteToTunnelV4(data, _) => tunnel.push(data.to_vec()),
            TunnResult::WriteToTunnelV6(data, _) => tunnel.push(data.to_vec()),
        }
    }

    (network, tunnel)
}

async fn send_hello(udp: &UdpSocket, relay: SocketAddr, session_id: u64) -> Result<()> {
    send_relay(udp, relay, session_id, 0, RelayKind::Hello, &[]).await
}

async fn send_relay(
    udp: &UdpSocket,
    relay: SocketAddr,
    sender: u64,
    destination: u64,
    kind: RelayKind,
    payload: &[u8],
) -> Result<()> {
    let mut datagram = vec![0u8; RELAY_HEADER_LEN + payload.len()];
    let len = encode_relay_packet(kind, sender, destination, payload, &mut datagram)
        .context("datagrama grande demais para o relay")?;
    udp.send_to(&datagram[..len], relay)
        .await
        .context("falha ao enviar ao relay")?;
    Ok(())
}

async fn cleanup_registration(api: &Api, config: &ClientConfig, session_id: u64) {
    if let Err(error) = api
        .unregister(&config.group_name, &config.group_password, session_id)
        .await
    {
        tracing::debug!(%error, "não foi possível remover o peer no servidor");
    }
}

async fn bind_udp(relay: SocketAddr) -> Result<UdpSocket> {
    let bind_address = if relay.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    UdpSocket::bind(bind_address)
        .await
        .context("não foi possível abrir o socket UDP")
}

async fn resolve_relay(value: &str) -> Result<SocketAddr> {
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host(value)
        .await
        .with_context(|| format!("não foi possível resolver o relay {value}"))?
        .collect();

    addresses
        .iter()
        .find(|address| address.is_ipv4())
        .or_else(|| addresses.first())
        .copied()
        .with_context(|| format!("o relay {value} não resolveu para nenhum endereço"))
}

fn local_hostname() -> String {
    let value = std::env::var("SHADOWVPN_HOSTNAME")
        .ok()
        .or_else(|| {
            hostname::get()
                .ok()
                .and_then(|value| value.into_string().ok())
        })
        .or_else(|| {
            let variable = if cfg!(windows) {
                "COMPUTERNAME"
            } else {
                "HOSTNAME"
            };
            std::env::var(variable).ok()
        })
        .unwrap_or_default();
    let cleaned = shadowvpn_core::clean_hostname(&value);
    if cleaned.is_empty() {
        "ShadowVPN".to_string()
    } else {
        cleaned
    }
}

fn broadcast_address() -> Ipv4Addr {
    let [a, b, c] = VIRTUAL_NETWORK_PREFIX;
    Ipv4Addr::new(a, b, c, 255)
}

fn emit(events: &Option<Sender<ClientEvent>>, event: ClientEvent) {
    if let Some(sender) = events {
        let _ = sender.send(event);
    }
}

#[derive(Default)]
struct PeerSet {
    peers: HashMap<String, Peer>,
    by_session: HashMap<u64, String>,
    by_ip: HashMap<Ipv4Addr, String>,
    next_index: u32,
}

struct Peer {
    info: PeerInfo,
    tunn: Tunn,
}

impl PeerSet {
    fn reconcile(&mut self, infos: &[PeerInfo], secret: &StaticSecret) -> Vec<PeerInfo> {
        let mut seen = HashSet::new();

        for info in infos {
            if info.session_id == 0 || !valid_public_key(&info.public_key) {
                continue;
            }
            let Ok(address) = info.virtual_ip.parse::<Ipv4Addr>() else {
                continue;
            };
            if !seen.insert(info.public_key.clone()) {
                continue;
            }
            if let Some(owner) = self.by_ip.get(&address) {
                if owner != &info.public_key {
                    tracing::warn!(%address, "IP duplicado recebido do servidor");
                    continue;
                }
            }

            if let Some(peer) = self.peers.get_mut(&info.public_key) {
                self.by_session.remove(&peer.info.session_id);
                if let Ok(old_ip) = peer.info.virtual_ip.parse::<Ipv4Addr>() {
                    self.by_ip.remove(&old_ip);
                }
                peer.info = info.clone();
            } else {
                let Ok(public_key) = decode_public_key(&info.public_key) else {
                    continue;
                };
                let index = (self.next_index & 0x00FF_FFFF).max(1);
                self.next_index = (index + 1) & 0x00FF_FFFF;
                let tunn = match Tunn::new(
                    secret.clone(),
                    public_key,
                    None,
                    Some(WIREGUARD_KEEPALIVE_SECS),
                    index,
                    None,
                ) {
                    Ok(tunn) => tunn,
                    Err(error) => {
                        tracing::warn!(%error, "não foi possível criar túnel para peer");
                        continue;
                    }
                };
                self.peers.insert(
                    info.public_key.clone(),
                    Peer {
                        info: info.clone(),
                        tunn,
                    },
                );
            }

            self.by_session
                .insert(info.session_id, info.public_key.clone());
            self.by_ip.insert(address, info.public_key.clone());
        }

        let stale: Vec<String> = self
            .peers
            .keys()
            .filter(|key| !seen.contains(*key))
            .cloned()
            .collect();
        for key in stale {
            self.remove(&key);
        }

        let mut peers: Vec<PeerInfo> = self.peers.values().map(|peer| peer.info.clone()).collect();
        peers.sort_by_key(|peer| peer.virtual_ip.parse::<Ipv4Addr>().ok());
        peers
    }

    fn update_timers(&mut self) -> Vec<(u64, Vec<u8>)> {
        let mut outputs = Vec::new();
        let mut failed = Vec::new();

        for (key, peer) in self.peers.iter_mut() {
            let mut output = vec![0u8; PACKET_BUFFER];
            match peer.tunn.update_timers(&mut output) {
                TunnResult::WriteToNetwork(data) => {
                    outputs.push((peer.info.session_id, data.to_vec()));
                }
                TunnResult::Err(error) => {
                    tracing::debug!(?error, "túnel WireGuard expirou");
                    failed.push(key.clone());
                }
                _ => {}
            }
        }

        for key in failed {
            self.remove(&key);
        }

        outputs
    }

    fn remove(&mut self, key: &str) {
        if let Some(peer) = self.peers.remove(key) {
            self.by_session.remove(&peer.info.session_id);
            if let Ok(address) = peer.info.virtual_ip.parse::<Ipv4Addr>() {
                self.by_ip.remove(&address);
            }
        }
    }

    fn len(&self) -> usize {
        self.peers.len()
    }
}

fn decode_public_key(encoded: &str) -> Result<PublicKey> {
    let bytes = BASE64
        .decode(encoded)
        .context("chave pública WireGuard inválida")?;
    let key: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("chave pública WireGuard inválida"))?;
    Ok(PublicKey::from(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_set_reconciles_and_removes_peers() {
        let secret = StaticSecret::random_from_rng(rand_core::OsRng);
        let public = PublicKey::from(&secret);
        let peer_key = BASE64.encode(public.as_bytes());

        let mut peers = PeerSet::default();
        let infos = vec![PeerInfo {
            public_key: peer_key.clone(),
            session_id: 7,
            virtual_ip: "26.26.26.9".into(),
            hostname: "pc-teste".into(),
        }];

        let listed = peers.reconcile(&infos, &secret);
        assert_eq!(listed.len(), 1);
        assert_eq!(peers.by_session.get(&7), Some(&peer_key));
        assert_eq!(
            peers.by_ip.get(&"26.26.26.9".parse().unwrap()),
            Some(&peer_key)
        );

        let listed = peers.reconcile(&[], &secret);
        assert!(listed.is_empty());
        assert!(peers.by_session.is_empty());
        assert!(peers.by_ip.is_empty());
    }

    #[test]
    fn broadcast_detection_uses_virtual_subnet() {
        assert_eq!(broadcast_address(), Ipv4Addr::new(26, 26, 26, 255));
    }

    #[test]
    fn wireguard_peer_handshake_and_data() {
        let secret_a = StaticSecret::random_from_rng(rand_core::OsRng);
        let public_a = PublicKey::from(&secret_a);
        let secret_b = StaticSecret::random_from_rng(rand_core::OsRng);
        let public_b = PublicKey::from(&secret_b);
        let key_a = BASE64.encode(public_a.as_bytes());
        let key_b = BASE64.encode(public_b.as_bytes());

        let info_a = PeerInfo {
            public_key: key_a.clone(),
            session_id: 11,
            virtual_ip: "26.26.26.2".into(),
            hostname: "pc-a".into(),
        };
        let info_b = PeerInfo {
            public_key: key_b.clone(),
            session_id: 22,
            virtual_ip: "26.26.26.3".into(),
            hostname: "pc-b".into(),
        };

        let mut peers_a = PeerSet::default();
        peers_a.reconcile(&[info_b], &secret_a);
        let mut peers_b = PeerSet::default();
        peers_b.reconcile(&[info_a], &secret_b);

        let packet = ipv4_udp_packet();
        let relay = IpAddr::V4(Ipv4Addr::LOCALHOST);

        let peer_b_from_a = peers_a.peers.get_mut(&key_b).expect("peer B");
        let initiation = encapsulate(&mut peer_b_from_a.tunn, &packet).expect("handshake init");
        let peer_a_from_b = peers_b.peers.get_mut(&key_a).expect("peer A");
        let (network, tunnel) = drain_decapsulate(&mut peer_a_from_b.tunn, &initiation, relay);
        assert!(tunnel.is_empty());
        let response = network.into_iter().next().expect("handshake response");

        let peer_b_from_a = peers_a.peers.get_mut(&key_b).expect("peer B");
        let (network, tunnel) = drain_decapsulate(&mut peer_b_from_a.tunn, &response, relay);
        assert!(tunnel.is_empty());
        let keepalive = network.into_iter().next().expect("keepalive");

        let peer_a_from_b = peers_b.peers.get_mut(&key_a).expect("peer A");
        let (network, tunnel) = drain_decapsulate(&mut peer_a_from_b.tunn, &keepalive, relay);
        assert!(network.is_empty());
        assert!(tunnel.is_empty());

        let peer_b_from_a = peers_a.peers.get_mut(&key_b).expect("peer B");
        let data = encapsulate(&mut peer_b_from_a.tunn, &packet).expect("dados cifrados");
        let peer_a_from_b = peers_b.peers.get_mut(&key_a).expect("peer A");
        let (network, tunnel) = drain_decapsulate(&mut peer_a_from_b.tunn, &data, relay);
        assert!(network.is_empty());
        assert_eq!(tunnel, vec![packet]);
    }

    fn ipv4_udp_packet() -> Vec<u8> {
        let mut packet = vec![0u8; 28];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&28u16.to_be_bytes());
        packet[8] = 64;
        packet[9] = 17;
        packet[12..16].copy_from_slice(&[26, 26, 26, 2]);
        packet[16..20].copy_from_slice(&[26, 26, 26, 3]);
        packet[20..22].copy_from_slice(&1234u16.to_be_bytes());
        packet[22..24].copy_from_slice(&5678u16.to_be_bytes());
        packet[24..26].copy_from_slice(&8u16.to_be_bytes());
        packet
    }
}
