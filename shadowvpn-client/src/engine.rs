use crate::api::{Api, ApiError};
use crate::config::ClientConfig;
use crate::identity;
use crate::tunnel::{self, TunFramed};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use futures::{SinkExt, StreamExt};
use reqwest::StatusCode;
use shadowvpn_core::{
    encode_relay_packet, parse_relay_packet, valid_public_key, PeerIdentity, PeerInfo,
    PeersResponse, RelayKind, HEARTBEAT_SECS, RELAY_HEADER_LEN, VIRTUAL_NETWORK_PREFIX,
};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::time::{interval, interval_at, sleep, Instant, MissedTickBehavior};

const PACKET_BUFFER: usize = 65_536;
const TIMER_INTERVAL: Duration = Duration::from_millis(250);
const WIREGUARD_KEEPALIVE_SECS: u16 = 25;
const HEARTBEAT_FAILURES_BEFORE_RECONNECT: u32 = 3;
const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(1);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct FatalConnectError(anyhow::Error);

impl fmt::Display for FatalConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:#}", self.0)
    }
}

impl std::error::Error for FatalConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

#[derive(Debug, Clone)]
pub enum ClientEvent {
    Registered {
        hostname: String,
        virtual_ip: String,
        relay: String,
        public_key: String,
        identity_fingerprint: String,
        latency_ms: u64,
        peers: Vec<PeerInfo>,
    },
    Peers(Vec<PeerInfo>),
    Latency(u64),
    Status(String),
    Reconnecting {
        attempt: u32,
        delay_secs: u64,
        message: String,
    },
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
    let identity_proof =
        shadowvpn_crypto::create_peer_identity(secret.as_bytes(), &config.group_name, &public_key)?;
    let identity_fingerprint =
        shadowvpn_crypto::verify_peer_identity(&public_key, &config.group_name, &identity_proof)?;
    let hostname = local_hostname();
    let api = Api::new(&config.server)?;
    let mut attempt = 0u32;

    loop {
        if *shutdown.borrow() {
            break;
        }

        match connect_once(
            &config,
            &api,
            &secret,
            &public_key,
            &identity_proof,
            &identity_fingerprint,
            &hostname,
            &shutdown,
            &events,
            &mut attempt,
        )
        .await
        {
            Ok(()) => break,
            Err(error) if is_fatal_error(&error) => return Err(error),
            Err(error) => {
                if *shutdown.borrow() {
                    break;
                }

                attempt = attempt.saturating_add(1);
                let delay = reconnect_delay(attempt);
                let message = format!("{error:#}");
                tracing::warn!(
                    attempt,
                    delay_secs = delay.as_secs(),
                    error = %message,
                    "conexão caiu; tentando novamente"
                );
                emit(&events, ClientEvent::Peers(Vec::new()));
                emit(
                    &events,
                    ClientEvent::Reconnecting {
                        attempt,
                        delay_secs: delay.as_secs().max(1),
                        message,
                    },
                );

                let mut wait_for_shutdown = shutdown.clone();
                tokio::select! {
                    _ = sleep(delay) => {}
                    _ = wait_for_shutdown.changed() => {}
                }
            }
        }
    }

    emit(&events, ClientEvent::Status("Desconectado".into()));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn connect_once(
    config: &ClientConfig,
    api: &Api,
    secret: &StaticSecret,
    public_key: &str,
    identity_proof: &PeerIdentity,
    identity_fingerprint: &str,
    hostname: &str,
    shutdown: &watch::Receiver<bool>,
    events: &Option<Sender<ClientEvent>>,
    attempt: &mut u32,
) -> Result<()> {
    emit(
        events,
        ClientEvent::Status("Registrando no grupo...".into()),
    );
    let register_started = std::time::Instant::now();
    let registered = match api
        .register(
            &config.group_name,
            &config.group_password,
            public_key,
            identity_proof,
            hostname,
        )
        .await
    {
        Ok(registered) => registered,
        Err(error) if api_status(&error) == Some(StatusCode::NOT_FOUND) => {
            return Err(anyhow::Error::new(FatalConnectError(error)));
        }
        Err(error) => return Err(error),
    };
    *attempt = 0;
    let register_latency_ms = register_started.elapsed().as_millis() as u64;
    let own_session = registered.session_id;
    let session_token = registered.session_token.clone();

    let own_ip: Ipv4Addr = match registered.virtual_ip.parse() {
        Ok(address) => address,
        Err(error) => {
            cleanup_registration(api, &config.group_name, &session_token, own_session).await;
            return Err(anyhow::Error::new(FatalConnectError(anyhow::anyhow!(
                "IP virtual inválido {}: {error}",
                registered.virtual_ip
            ))));
        }
    };
    let relay = match resolve_relay(&registered.relay).await {
        Ok(relay) => relay,
        Err(error) => {
            cleanup_registration(api, &config.group_name, &session_token, own_session).await;
            return Err(error);
        }
    };

    let framed = match tunnel::create(own_ip) {
        Ok(framed) => framed,
        Err(error) => {
            cleanup_registration(api, &config.group_name, &session_token, own_session).await;
            return Err(anyhow::Error::new(FatalConnectError(error)));
        }
    };

    let udp = match bind_udp(relay).await {
        Ok(udp) => udp,
        Err(error) => {
            cleanup_registration(api, &config.group_name, &session_token, own_session).await;
            return Err(error);
        }
    };
    let mut peers = PeerSet::default();
    let peer_list = peers.reconcile(&registered.peers, secret, &config.group_name);
    if let Err(error) = send_hello(&udp, relay, own_session).await {
        cleanup_registration(api, &config.group_name, &session_token, own_session).await;
        return Err(error);
    }

    emit(
        events,
        ClientEvent::Registered {
            hostname: hostname.to_string(),
            virtual_ip: own_ip.to_string(),
            relay: relay.to_string(),
            public_key: public_key.to_string(),
            identity_fingerprint: identity_fingerprint.to_string(),
            latency_ms: register_latency_ms,
            peers: peer_list.clone(),
        },
    );
    emit(events, ClientEvent::Peers(peer_list));

    tracing::info!(%own_ip, %relay, peers = peers.len(), "ShadowVPN conectado");

    let result = session_loop(
        &udp,
        relay,
        secret,
        &session_token,
        own_session,
        framed,
        &mut peers,
        shutdown,
        events,
        api,
        &config.group_name,
    )
    .await;

    emit(events, ClientEvent::Status("Desconectando...".into()));
    cleanup_registration(api, &config.group_name, &session_token, own_session).await;

    tracing::info!("ShadowVPN desconectado");
    result
}

fn reconnect_delay(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(5);
    let millis = RECONNECT_BASE_DELAY
        .as_millis()
        .saturating_mul(1u128 << exponent);
    let capped = millis.min(RECONNECT_MAX_DELAY.as_millis());
    Duration::from_millis(capped as u64)
}

fn api_status(error: &anyhow::Error) -> Option<StatusCode> {
    error.chain().find_map(|cause| {
        cause
            .downcast_ref::<ApiError>()
            .map(|api_error| api_error.status)
    })
}

fn is_fatal_error(error: &anyhow::Error) -> bool {
    if error.chain().any(|cause| cause.is::<FatalConnectError>()) {
        return true;
    }
    matches!(
        api_status(error),
        Some(
            StatusCode::BAD_REQUEST
                | StatusCode::UNAUTHORIZED
                | StatusCode::FORBIDDEN
                | StatusCode::NOT_FOUND
        )
    )
}

#[allow(clippy::too_many_arguments)]
async fn session_loop(
    udp: &UdpSocket,
    relay: SocketAddr,
    secret: &StaticSecret,
    session_token: &str,
    own_session: u64,
    mut framed: TunFramed,
    peers: &mut PeerSet,
    shutdown: &watch::Receiver<bool>,
    events: &Option<Sender<ClientEvent>>,
    api: &Api,
    group_name: &str,
) -> Result<()> {
    let mut udp_buffer = vec![0u8; PACKET_BUFFER];
    let mut timer = interval(TIMER_INTERVAL);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let (heartbeat_tx, mut heartbeat_rx) =
        tokio::sync::mpsc::channel::<(u64, anyhow::Result<PeersResponse>)>(1);
    let heartbeat_api = api.clone();
    let heartbeat_group = group_name.to_string();
    let heartbeat_token = session_token.to_string();
    let heartbeat_session = own_session;
    let mut heartbeat_tasks = tokio::task::JoinSet::new();
    heartbeat_tasks.spawn(async move {
        let period = Duration::from_secs(HEARTBEAT_SECS);
        let mut ticker = interval_at(Instant::now() + period, period);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let started = std::time::Instant::now();
            let result = heartbeat_api
                .heartbeat(&heartbeat_group, &heartbeat_token, heartbeat_session)
                .await;
            let latency_ms = started.elapsed().as_millis() as u64;
            if heartbeat_tx.send((latency_ms, result)).await.is_err() {
                break;
            }
        }
    });

    let mut failed_heartbeats = 0u32;

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
                    own_session,
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
                    own_session,
                ).await?;
            }
            _ = timer.tick() => {
                let outputs = peers.update_timers();
                for (destination, payload) in outputs {
                    send_relay(
                        udp,
                        relay,
                        own_session,
                        destination,
                        RelayKind::Data,
                        &payload,
                    ).await?;
                }
            }
            result = heartbeat_rx.recv() => {
                let Some((latency_ms, result)) = result else {
                    bail!("a tarefa de heartbeat encerrou inesperadamente");
                };
                match result {
                    Ok(response) => {
                        failed_heartbeats = 0;
                        emit(events, ClientEvent::Latency(latency_ms));
                        let refreshed =
                            peers.reconcile(&response.peers, secret, group_name);
                        emit(events, ClientEvent::Peers(refreshed));
                        send_hello(udp, relay, own_session).await?;
                        tracing::debug!(peers = peers.len(), "heartbeat enviado");
                    }
                    Err(error) => {
                        failed_heartbeats = failed_heartbeats.saturating_add(1);
                        if api_status(&error) == Some(StatusCode::NOT_FOUND) {
                            bail!("sessão expirou no servidor; reconectando");
                        }
                        tracing::warn!(
                            failed_heartbeats,
                            %error,
                            "falha no heartbeat"
                        );
                        if failed_heartbeats >= HEARTBEAT_FAILURES_BEFORE_RECONNECT {
                            return Err(error).context("heartbeat falhou repetidamente");
                        }
                        emit(
                            events,
                            ClientEvent::Status(format!(
                                "Falha no heartbeat ({failed_heartbeats}/{HEARTBEAT_FAILURES_BEFORE_RECONNECT}): {error:#}"
                            )),
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

async fn cleanup_registration(api: &Api, group_name: &str, session_token: &str, session_id: u64) {
    if let Err(error) = api.unregister(group_name, session_token, session_id).await {
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
    fn reconcile(
        &mut self,
        infos: &[PeerInfo],
        secret: &StaticSecret,
        group_name: &str,
    ) -> Vec<PeerInfo> {
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

            let fingerprint = match shadowvpn_crypto::verify_peer_identity(
                &info.public_key,
                group_name,
                &info.identity,
            ) {
                Ok(fingerprint) if fingerprint == info.fingerprint => fingerprint,
                Ok(_) => {
                    tracing::warn!(peer = %info.public_key, "fingerprint divergente do servidor");
                    continue;
                }
                Err(error) => {
                    tracing::warn!(peer = %info.public_key, %error, "identidade do peer inválida");
                    continue;
                }
            };

            if self
                .peers
                .get(&info.public_key)
                .map(|peer| peer.info.fingerprint != fingerprint)
                .unwrap_or(false)
            {
                tracing::warn!(peer = %info.public_key, "identidade do peer mudou; recriando túnel");
                self.remove(&info.public_key);
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
                let psk = match shadowvpn_crypto::derive_pairwise_psk(
                    secret.as_bytes(),
                    &info.public_key,
                    group_name,
                ) {
                    Ok(psk) => psk,
                    Err(error) => {
                        tracing::warn!(peer = %info.public_key, %error, "PSK do peer inválido");
                        continue;
                    }
                };
                let index = (self.next_index & 0x00FF_FFFF).max(1);
                self.next_index = (index + 1) & 0x00FF_FFFF;
                let tunn = Tunn::new(
                    secret.clone(),
                    public_key,
                    Some(psk),
                    Some(WIREGUARD_KEEPALIVE_SECS),
                    index,
                    None,
                );
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

    const TEST_GROUP: &str = "rede-teste";

    #[test]
    fn classifies_fatal_and_retryable_connect_errors() {
        let unauthorized: anyhow::Error = ApiError {
            status: StatusCode::UNAUTHORIZED,
            message: "senha inválida".into(),
        }
        .into();
        assert!(is_fatal_error(&unauthorized));

        let unavailable: anyhow::Error = ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "indisponível".into(),
        }
        .into();
        assert!(!is_fatal_error(&unavailable));

        let tun = anyhow::Error::new(FatalConnectError(anyhow::anyhow!("sem TUN")));
        assert!(is_fatal_error(&tun));
    }

    #[test]
    fn reconnect_backoff_is_capped() {
        assert_eq!(reconnect_delay(1), Duration::from_secs(1));
        assert_eq!(reconnect_delay(2), Duration::from_secs(2));
        assert_eq!(reconnect_delay(3), Duration::from_secs(4));
        assert_eq!(reconnect_delay(6), Duration::from_secs(30));
        assert_eq!(reconnect_delay(100), Duration::from_secs(30));
    }

    fn peer_info(
        secret: &StaticSecret,
        session_id: u64,
        virtual_ip: &str,
        hostname: &str,
    ) -> PeerInfo {
        let public_key = BASE64.encode(PublicKey::from(secret).as_bytes());
        let identity =
            shadowvpn_crypto::create_peer_identity(secret.as_bytes(), TEST_GROUP, &public_key)
                .unwrap();
        let fingerprint =
            shadowvpn_crypto::verify_peer_identity(&public_key, TEST_GROUP, &identity).unwrap();
        PeerInfo {
            public_key,
            identity,
            fingerprint,
            session_id,
            virtual_ip: virtual_ip.into(),
            hostname: hostname.into(),
        }
    }

    #[test]
    fn peer_set_reconciles_and_removes_peers() {
        let secret = StaticSecret::random_from_rng(rand_core::OsRng);
        let peer_secret = StaticSecret::random_from_rng(rand_core::OsRng);
        let info = peer_info(&peer_secret, 7, "26.26.26.9", "pc-teste");
        let peer_key = info.public_key.clone();

        let mut peers = PeerSet::default();
        let listed = peers.reconcile(&[info], &secret, TEST_GROUP);
        assert_eq!(listed.len(), 1);
        assert_eq!(peers.by_session.get(&7), Some(&peer_key));
        assert_eq!(
            peers.by_ip.get(&"26.26.26.9".parse().unwrap()),
            Some(&peer_key)
        );

        let listed = peers.reconcile(&[], &secret, TEST_GROUP);
        assert!(listed.is_empty());
        assert!(peers.by_session.is_empty());
        assert!(peers.by_ip.is_empty());
    }

    #[test]
    fn peer_set_rejects_invalid_identity() {
        let secret = StaticSecret::random_from_rng(rand_core::OsRng);
        let peer_secret = StaticSecret::random_from_rng(rand_core::OsRng);
        let mut info = peer_info(&peer_secret, 7, "26.26.26.9", "pc-teste");
        info.identity.signature = BASE64.encode([0u8; 64]);

        let mut peers = PeerSet::default();
        assert!(peers.reconcile(&[info], &secret, TEST_GROUP).is_empty());
        assert!(peers.peers.is_empty());
    }

    #[test]
    fn broadcast_detection_uses_virtual_subnet() {
        assert_eq!(broadcast_address(), Ipv4Addr::new(26, 26, 26, 255));
    }

    #[test]
    fn wireguard_peer_handshake_and_data() {
        let secret_a = StaticSecret::random_from_rng(rand_core::OsRng);
        let secret_b = StaticSecret::random_from_rng(rand_core::OsRng);
        let info_a = peer_info(&secret_a, 11, "26.26.26.2", "pc-a");
        let info_b = peer_info(&secret_b, 22, "26.26.26.3", "pc-b");
        let key_a = info_a.public_key.clone();
        let key_b = info_b.public_key.clone();

        let mut peers_a = PeerSet::default();
        peers_a.reconcile(&[info_b], &secret_a, TEST_GROUP);
        let mut peers_b = PeerSet::default();
        peers_b.reconcile(&[info_a], &secret_b, TEST_GROUP);

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
