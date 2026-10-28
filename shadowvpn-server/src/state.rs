use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand_core::{OsRng as SessionOsRng, RngCore};
use serde::{Deserialize, Serialize};
use shadowvpn_core::{
    clean_hostname, valid_group_name, NetworkSummary, PeerIdentity, PeerInfo,
    VIRTUAL_NETWORK_PREFIX,
};

const NETWORKS_FILE_VERSION: u32 = 2;
const LEGACY_NETWORKS_FILE_VERSION: u32 = 1;
const DEFAULT_MAX_NETWORKS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    InvalidCredentials,
    InvalidIdentity,
    NetworkNotFound,
    NetworkExists,
    GroupFull,
    ServerFull,
    SessionNotFound,
    Internal,
}

#[derive(Debug, Clone)]
pub struct RegisterOutcome {
    pub virtual_ip: Ipv4Addr,
    pub session_id: u64,
    pub session_token: String,
    pub peers: Vec<PeerInfo>,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Mutex<ServerInner>>,
    peer_ttl: Duration,
    relay_port: u16,
    relay_advertise: Option<String>,
    networks_file: Option<PathBuf>,
    admin_token: Option<String>,
    max_networks: usize,
}

struct ServerInner {
    groups: HashMap<String, Group>,
    sessions: HashMap<u64, SessionRef>,
}

#[derive(Serialize, Deserialize)]
struct NetworksFile {
    version: u32,
    networks: BTreeMap<String, StoredNetwork>,
}

#[derive(Serialize, Deserialize)]
struct StoredNetwork {
    password_hash: String,
    #[serde(default)]
    members: BTreeMap<String, String>,
}

#[derive(Clone)]
struct SessionRef {
    group: String,
    public_key: String,
    token: String,
}

struct Group {
    password_hash: String,
    peers: HashMap<String, Peer>,
    members: BTreeMap<String, Ipv4Addr>,
}

struct Peer {
    public_key: String,
    identity: PeerIdentity,
    fingerprint: String,
    hostname: String,
    virtual_ip: Ipv4Addr,
    session_id: u64,
    last_seen: Instant,
    relay_addr: Option<SocketAddr>,
}

impl AppState {
    #[cfg(test)]
    pub fn new(peer_ttl: Duration, relay_port: u16, relay_advertise: Option<String>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ServerInner {
                groups: HashMap::new(),
                sessions: HashMap::new(),
            })),
            peer_ttl,
            relay_port,
            relay_advertise,
            networks_file: None,
            admin_token: None,
            max_networks: DEFAULT_MAX_NETWORKS,
        }
    }

    pub fn with_networks_file(
        peer_ttl: Duration,
        relay_port: u16,
        relay_advertise: Option<String>,
        networks_file: PathBuf,
    ) -> Result<Self> {
        let groups = load_networks(&networks_file)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(ServerInner {
                groups,
                sessions: HashMap::new(),
            })),
            peer_ttl,
            relay_port,
            relay_advertise,
            networks_file: Some(networks_file),
            admin_token: None,
            max_networks: DEFAULT_MAX_NETWORKS,
        })
    }

    pub fn with_admin_token(mut self, admin_token: Option<String>) -> Self {
        self.admin_token = admin_token;
        self
    }

    pub fn with_max_networks(mut self, max_networks: usize) -> Self {
        self.max_networks = max_networks.max(1);
        self
    }

    pub fn admin_token(&self) -> Option<&str> {
        self.admin_token.as_deref()
    }

    pub fn create_network(&self, group_name: &str, password: &str) -> Result<(), StateError> {
        {
            let inner = self.lock();
            if inner.groups.contains_key(group_name) {
                return Err(StateError::NetworkExists);
            }
            if inner.groups.len() >= self.max_networks {
                return Err(StateError::ServerFull);
            }
        }

        let password_hash = hash_password(password).ok_or(StateError::Internal)?;
        let mut inner = self.lock();
        if inner.groups.contains_key(group_name) {
            return Err(StateError::NetworkExists);
        }
        if inner.groups.len() >= self.max_networks {
            return Err(StateError::ServerFull);
        }
        inner.groups.insert(
            group_name.to_owned(),
            Group {
                password_hash,
                peers: HashMap::new(),
                members: BTreeMap::new(),
            },
        );

        if let Some(path) = &self.networks_file {
            if let Err(error) = save_networks(path, &inner.groups) {
                inner.groups.remove(group_name);
                tracing::error!(
                    %group_name,
                    path = %path.display(),
                    error = %error,
                    "não foi possível persistir a rede"
                );
                return Err(StateError::Internal);
            }
        }

        tracing::info!(%group_name, "rede criada");
        Ok(())
    }

    pub fn relay_address(&self, host_header: Option<&str>) -> String {
        if let Some(advertise) = &self.relay_advertise {
            return advertise.clone();
        }

        let host = host_header
            .and_then(hostname_from_header)
            .unwrap_or_else(|| "127.0.0.1".to_string());

        format!("{host}:{}", self.relay_port)
    }

    pub fn register(
        &self,
        group_name: &str,
        password: &str,
        public_key: &str,
        identity: &PeerIdentity,
        hostname: &str,
    ) -> Result<RegisterOutcome, StateError> {
        let fingerprint = shadowvpn_crypto::verify_peer_identity(public_key, group_name, identity)
            .map_err(|_| StateError::InvalidIdentity)?;
        let now = Instant::now();

        let password_hash = {
            let mut inner = self.lock();
            inner.prune(now, self.peer_ttl);
            inner
                .groups
                .get(group_name)
                .map(|group| group.password_hash.clone())
                .ok_or(StateError::NetworkNotFound)?
        };
        if !verify_password(&password_hash, password) {
            return Err(StateError::InvalidCredentials);
        }

        let hostname = display_hostname(hostname);
        let mut inner = self.lock();
        inner.prune(now, self.peer_ttl);

        let (session_id, virtual_ip, session_token, peers, new_member) = {
            let ServerInner { groups, sessions } = &mut *inner;
            let group = groups
                .get_mut(group_name)
                .ok_or(StateError::NetworkNotFound)?;
            if group.password_hash != password_hash {
                return Err(StateError::InvalidCredentials);
            }

            let existing = group
                .peers
                .get(public_key)
                .map(|peer| (peer.session_id, peer.virtual_ip));

            let (session_id, virtual_ip, session_token, new_member) =
                if let Some((session_id, virtual_ip)) = existing {
                    if let Some(peer) = group.peers.get_mut(public_key) {
                        peer.last_seen = now;
                        peer.hostname = hostname;
                        peer.identity = identity.clone();
                        peer.fingerprint = fingerprint;
                    }
                    let token = match sessions.get(&session_id) {
                        Some(session) => session.token.clone(),
                        None => {
                            let token = new_session_token();
                            sessions.insert(
                                session_id,
                                SessionRef {
                                    group: group_name.to_owned(),
                                    public_key: public_key.to_owned(),
                                    token: token.clone(),
                                },
                            );
                            token
                        }
                    };
                    (session_id, virtual_ip, token, false)
                } else {
                    let (virtual_ip, new_member) =
                        if let Some(address) = group.members.get(public_key).copied() {
                            (address, false)
                        } else {
                            let address = allocate_ip(group).ok_or(StateError::GroupFull)?;
                            group.members.insert(public_key.to_owned(), address);
                            (address, true)
                        };
                    let session_id = new_session_id(sessions);
                    let token = new_session_token();
                    group.peers.insert(
                        public_key.to_owned(),
                        Peer {
                            public_key: public_key.to_owned(),
                            identity: identity.clone(),
                            fingerprint,
                            hostname,
                            virtual_ip,
                            session_id,
                            last_seen: now,
                            relay_addr: None,
                        },
                    );
                    sessions.insert(
                        session_id,
                        SessionRef {
                            group: group_name.to_owned(),
                            public_key: public_key.to_owned(),
                            token: token.clone(),
                        },
                    );
                    (session_id, virtual_ip, token, new_member)
                };

            let peers = group
                .peers
                .values()
                .filter(|peer| peer.public_key != public_key)
                .map(peer_info)
                .collect();

            (session_id, virtual_ip, session_token, peers, new_member)
        };

        if new_member {
            if let Some(path) = &self.networks_file {
                if let Err(error) = save_networks(path, &inner.groups) {
                    tracing::error!(
                        %group_name,
                        path = %path.display(),
                        error = %error,
                        "não foi possível persistir o IP virtual do novo membro"
                    );
                    if let Some(group) = inner.groups.get_mut(group_name) {
                        group.peers.remove(public_key);
                        group.members.remove(public_key);
                    }
                    inner.sessions.remove(&session_id);
                    return Err(StateError::Internal);
                }
            }
        }

        Ok(RegisterOutcome {
            virtual_ip,
            session_id,
            session_token,
            peers,
        })
    }

    pub fn heartbeat(
        &self,
        group_name: &str,
        password: &str,
        session_id: u64,
    ) -> Result<Vec<PeerInfo>, StateError> {
        let now = Instant::now();
        let password_hash = {
            let mut inner = self.lock();
            inner.prune(now, self.peer_ttl);
            inner
                .groups
                .get(group_name)
                .map(|group| group.password_hash.clone())
                .ok_or(StateError::NetworkNotFound)?
        };
        if !verify_password(&password_hash, password) {
            return Err(StateError::InvalidCredentials);
        }

        let mut inner = self.lock();
        inner.prune(now, self.peer_ttl);
        let group = inner
            .groups
            .get_mut(group_name)
            .ok_or(StateError::NetworkNotFound)?;
        if group.password_hash != password_hash {
            return Err(StateError::InvalidCredentials);
        }
        touch_session(group, session_id, now)
    }

    pub fn heartbeat_with_token(
        &self,
        group_name: &str,
        session_token: &str,
        session_id: u64,
    ) -> Result<Vec<PeerInfo>, StateError> {
        let now = Instant::now();
        let mut inner = self.lock();
        inner.prune(now, self.peer_ttl);

        let session = inner
            .sessions
            .get(&session_id)
            .cloned()
            .ok_or(StateError::SessionNotFound)?;
        if session.group != group_name || !secure_eq(&session.token, session_token) {
            return Err(StateError::SessionNotFound);
        }

        let group = inner
            .groups
            .get_mut(group_name)
            .ok_or(StateError::NetworkNotFound)?;
        let peer = group
            .peers
            .get_mut(&session.public_key)
            .ok_or(StateError::SessionNotFound)?;
        peer.last_seen = now;

        Ok(group
            .peers
            .values()
            .filter(|peer| peer.session_id != session_id)
            .map(peer_info)
            .collect())
    }

    pub fn unregister(
        &self,
        group_name: &str,
        password: &str,
        session_id: u64,
    ) -> Result<(), StateError> {
        let now = Instant::now();
        let password_hash = {
            let mut inner = self.lock();
            inner.prune(now, self.peer_ttl);
            inner
                .groups
                .get(group_name)
                .map(|group| group.password_hash.clone())
                .ok_or(StateError::NetworkNotFound)?
        };
        if !verify_password(&password_hash, password) {
            return Err(StateError::InvalidCredentials);
        }

        let mut inner = self.lock();
        let ServerInner { groups, sessions } = &mut *inner;
        let group = groups
            .get_mut(group_name)
            .ok_or(StateError::NetworkNotFound)?;
        if group.password_hash != password_hash {
            return Err(StateError::InvalidCredentials);
        }

        let public_key = group
            .peers
            .iter()
            .find(|(_, peer)| peer.session_id == session_id)
            .map(|(public_key, _)| public_key.clone())
            .ok_or(StateError::SessionNotFound)?;

        group.peers.remove(&public_key);
        sessions.remove(&session_id);
        Ok(())
    }

    pub fn unregister_with_token(
        &self,
        group_name: &str,
        session_token: &str,
        session_id: u64,
    ) -> Result<(), StateError> {
        let now = Instant::now();
        let mut inner = self.lock();
        inner.prune(now, self.peer_ttl);

        let session = inner
            .sessions
            .get(&session_id)
            .cloned()
            .ok_or(StateError::SessionNotFound)?;
        if session.group != group_name || !secure_eq(&session.token, session_token) {
            return Err(StateError::SessionNotFound);
        }

        let ServerInner { groups, sessions } = &mut *inner;
        if let Some(group) = groups.get_mut(group_name) {
            group.peers.remove(&session.public_key);
        }
        sessions.remove(&session_id);
        Ok(())
    }

    pub fn list_networks(&self) -> Vec<NetworkSummary> {
        let now = Instant::now();
        let mut inner = self.lock();
        inner.prune(now, self.peer_ttl);

        let mut networks: Vec<NetworkSummary> = inner
            .groups
            .iter()
            .map(|(name, group)| NetworkSummary {
                name: name.clone(),
                online_peers: group.peers.len(),
                known_peers: group.members.len(),
            })
            .collect();
        networks.sort_by(|left, right| left.name.cmp(&right.name));
        networks
    }

    pub fn delete_network(&self, group_name: &str) -> Result<(), StateError> {
        let mut inner = self.lock();
        let group = inner
            .groups
            .remove(group_name)
            .ok_or(StateError::NetworkNotFound)?;

        if let Some(path) = &self.networks_file {
            if let Err(error) = save_networks(path, &inner.groups) {
                inner.groups.insert(group_name.to_owned(), group);
                tracing::error!(
                    %group_name,
                    path = %path.display(),
                    error = %error,
                    "não foi possível persistir a remoção da rede"
                );
                return Err(StateError::Internal);
            }
        }

        inner
            .sessions
            .retain(|_, session| session.group != group_name);
        tracing::info!(%group_name, "rede removida");
        Ok(())
    }

    pub fn set_network_password(&self, group_name: &str, password: &str) -> Result<(), StateError> {
        {
            let inner = self.lock();
            if !inner.groups.contains_key(group_name) {
                return Err(StateError::NetworkNotFound);
            }
        }
        let password_hash = hash_password(password).ok_or(StateError::Internal)?;

        let mut inner = self.lock();
        let group = inner
            .groups
            .get_mut(group_name)
            .ok_or(StateError::NetworkNotFound)?;
        let previous = std::mem::replace(&mut group.password_hash, password_hash);

        if let Some(path) = &self.networks_file {
            if let Err(error) = save_networks(path, &inner.groups) {
                if let Some(group) = inner.groups.get_mut(group_name) {
                    group.password_hash = previous;
                }
                tracing::error!(
                    %group_name,
                    path = %path.display(),
                    error = %error,
                    "não foi possível persistir a nova senha da rede"
                );
                return Err(StateError::Internal);
            }
        }

        if let Some(group) = inner.groups.get_mut(group_name) {
            group.peers.clear();
        }
        inner
            .sessions
            .retain(|_, session| session.group != group_name);
        tracing::info!(%group_name, "senha da rede alterada e sessões encerradas");
        Ok(())
    }

    pub fn relay_bind(&self, session_id: u64, source: SocketAddr) -> bool {
        if session_id == 0 {
            return false;
        }

        let now = Instant::now();
        let mut inner = self.lock();

        let Some(session) = inner.sessions.get(&session_id).cloned() else {
            return false;
        };
        let Some(group) = inner.groups.get_mut(&session.group) else {
            return false;
        };
        let Some(peer) = group.peers.get_mut(&session.public_key) else {
            return false;
        };

        peer.relay_addr = Some(source);
        peer.last_seen = now;
        true
    }

    pub fn relay_route(
        &self,
        sender_session: u64,
        destination_session: u64,
        source: SocketAddr,
    ) -> Option<SocketAddr> {
        if sender_session == 0 || destination_session == 0 || sender_session == destination_session
        {
            return None;
        }

        let now = Instant::now();
        let mut inner = self.lock();

        let sender = inner.sessions.get(&sender_session)?.clone();
        let destination = inner.sessions.get(&destination_session)?.clone();
        if sender.group != destination.group {
            return None;
        }

        let group = inner.groups.get_mut(&sender.group)?;
        let sender_peer = group.peers.get_mut(&sender.public_key)?;
        sender_peer.relay_addr = Some(source);
        sender_peer.last_seen = now;

        group
            .peers
            .get(&destination.public_key)
            .and_then(|peer| peer.relay_addr)
    }

    pub fn cleanup(&self) -> usize {
        let now = Instant::now();
        let mut inner = self.lock();
        inner.prune(now, self.peer_ttl)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ServerInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl ServerInner {
    fn prune(&mut self, now: Instant, ttl: Duration) -> usize {
        let mut expired = Vec::new();
        for group in self.groups.values_mut() {
            group.peers.retain(|_, peer| {
                if now.saturating_duration_since(peer.last_seen) <= ttl {
                    true
                } else {
                    expired.push(peer.session_id);
                    false
                }
            });
        }

        for session_id in &expired {
            self.sessions.remove(session_id);
        }
        expired.len()
    }
}

fn allocate_ip(group: &Group) -> Option<Ipv4Addr> {
    let [a, b, c] = VIRTUAL_NETWORK_PREFIX;
    (2u8..=254)
        .map(|last| Ipv4Addr::new(a, b, c, last))
        .find(|candidate| {
            !group.members.values().any(|address| address == candidate)
                && !group
                    .peers
                    .values()
                    .any(|peer| peer.virtual_ip == *candidate)
        })
}

fn new_session_id(sessions: &HashMap<u64, SessionRef>) -> u64 {
    loop {
        let mut bytes = [0u8; 8];
        SessionOsRng.fill_bytes(&mut bytes);
        let session_id = u64::from_le_bytes(bytes);
        if session_id != 0 && !sessions.contains_key(&session_id) {
            return session_id;
        }
    }
}

fn new_session_token() -> String {
    let mut bytes = [0u8; 32];
    SessionOsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn secure_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.bytes().zip(right.bytes()) {
        difference |= a ^ b;
    }
    difference == 0
}

fn touch_session(
    group: &mut Group,
    session_id: u64,
    now: Instant,
) -> Result<Vec<PeerInfo>, StateError> {
    let peer = group
        .peers
        .values_mut()
        .find(|peer| peer.session_id == session_id)
        .ok_or(StateError::SessionNotFound)?;
    peer.last_seen = now;

    Ok(group
        .peers
        .values()
        .filter(|peer| peer.session_id != session_id)
        .map(peer_info)
        .collect())
}

fn peer_info(peer: &Peer) -> PeerInfo {
    PeerInfo {
        public_key: peer.public_key.clone(),
        identity: peer.identity.clone(),
        fingerprint: peer.fingerprint.clone(),
        session_id: peer.session_id,
        virtual_ip: peer.virtual_ip.to_string(),
        hostname: peer.hostname.clone(),
    }
}

fn display_hostname(value: &str) -> String {
    let cleaned = clean_hostname(value);
    if cleaned.is_empty() {
        "Peer".to_string()
    } else {
        cleaned
    }
}

fn hash_password(password: &str) -> Option<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .ok()
        .map(|hash| hash.to_string())
}

fn verify_password(password_hash: &str, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(password_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn hostname_from_header(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if let Some(rest) = value.strip_prefix('[') {
        let end = rest.find(']')?;
        return Some(format!("[{}]", &rest[..end]));
    }

    let host = value.split([':', '/']).next().unwrap_or_default().trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn load_networks(path: &Path) -> Result<HashMap<String, Group>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let contents = fs::read_to_string(path)
        .with_context(|| format!("não foi possível ler {}", path.display()))?;
    if contents.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let stored: NetworksFile = serde_json::from_str(&contents)
        .with_context(|| format!("arquivo de redes inválido: {}", path.display()))?;
    if stored.version != NETWORKS_FILE_VERSION && stored.version != LEGACY_NETWORKS_FILE_VERSION {
        bail!(
            "versão do arquivo de redes não suportada: {}",
            stored.version
        );
    }

    let mut groups = HashMap::with_capacity(stored.networks.len());
    for (name, network) in stored.networks {
        if !valid_group_name(&name) {
            bail!("nome de rede inválido no arquivo: {name:?}");
        }
        if PasswordHash::new(&network.password_hash).is_err() {
            bail!("hash de senha inválido para a rede {name:?}");
        }

        let mut members = BTreeMap::new();
        for (public_key, encoded_ip) in network.members {
            if !shadowvpn_core::valid_public_key(&public_key) {
                bail!("chave pública inválida nos membros da rede {name:?}");
            }
            let address: Ipv4Addr = encoded_ip
                .parse()
                .with_context(|| format!("IP virtual inválido na rede {name:?}: {encoded_ip}"))?;
            let [prefix_a, prefix_b, prefix_c] = VIRTUAL_NETWORK_PREFIX;
            if address.octets()[..3] != [prefix_a, prefix_b, prefix_c]
                || address.octets()[3] < 2
                || address.octets()[3] == 255
            {
                bail!("IP virtual fora da sub-rede na rede {name:?}: {address}");
            }
            if members.values().any(|existing| existing == &address) {
                bail!("IP virtual duplicado na rede {name:?}: {address}");
            }
            members.insert(public_key, address);
        }

        groups.insert(
            name,
            Group {
                password_hash: network.password_hash,
                peers: HashMap::new(),
                members,
            },
        );
    }

    Ok(groups)
}

fn save_networks(path: &Path, groups: &HashMap<String, Group>) -> Result<()> {
    let networks = groups
        .iter()
        .map(|(name, group)| {
            (
                name.clone(),
                StoredNetwork {
                    password_hash: group.password_hash.clone(),
                    members: group
                        .members
                        .iter()
                        .map(|(public_key, address)| (public_key.clone(), address.to_string()))
                        .collect(),
                },
            )
        })
        .collect();
    let contents = serde_json::to_vec_pretty(&NetworksFile {
        version: NETWORKS_FILE_VERSION,
        networks,
    })
    .context("não foi possível serializar as redes")?;

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("não foi possível criar {}", parent.display()))?;
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("caminho do arquivo de redes inválido")?;
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    {
        use std::io::Write;
        let mut file = options
            .open(&temporary)
            .with_context(|| format!("não foi possível abrir {}", temporary.display()))?;
        file.write_all(&contents)
            .with_context(|| format!("não foi possível escrever {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("não foi possível sincronizar {}", temporary.display()))?;
    }

    fs::rename(&temporary, path)
        .with_context(|| format!("não foi possível substituir {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        let state = AppState::new(Duration::from_secs(75), 8081, None);
        state
            .create_network("grupo", "senha-segura")
            .expect("rede de teste");
        state
    }

    fn key(n: u8) -> String {
        shadowvpn_crypto::public_key_b64(&[n; 32])
    }

    fn peer_identity(n: u8, group: &str) -> PeerIdentity {
        shadowvpn_crypto::create_peer_identity(&[n; 32], group, &key(n)).unwrap()
    }

    fn register(
        state: &AppState,
        group: &str,
        password: &str,
        n: u8,
        hostname: &str,
    ) -> Result<RegisterOutcome, StateError> {
        state.register(group, password, &key(n), &peer_identity(n, group), hostname)
    }

    #[test]
    fn allocates_ips_and_deduplicates_public_keys() {
        let state = state();
        let first = register(&state, "grupo", "senha-segura", 1, "pc-1").unwrap();
        assert_eq!(first.virtual_ip, Ipv4Addr::new(26, 26, 26, 2));

        let second = register(&state, "grupo", "senha-segura", 2, "pc-2").unwrap();
        assert_eq!(second.virtual_ip, Ipv4Addr::new(26, 26, 26, 3));
        assert_eq!(second.peers.len(), 1);

        let again = register(&state, "grupo", "senha-segura", 2, "pc-2").unwrap();
        assert_eq!(again.virtual_ip, second.virtual_ip);
        assert_eq!(again.session_id, second.session_id);
        assert_eq!(again.peers.len(), 1);
    }

    #[test]
    fn rejects_wrong_password() {
        let state = state();
        register(&state, "grupo", "senha-segura", 1, "pc-1").unwrap();
        assert!(matches!(
            register(&state, "grupo", "outra-senha", 2, "pc-2"),
            Err(StateError::InvalidCredentials)
        ));
    }

    #[test]
    fn heartbeat_and_unregister() {
        let state = state();
        let first = register(&state, "grupo", "senha-segura", 1, "pc-1").unwrap();
        let second = register(&state, "grupo", "senha-segura", 2, "pc-2").unwrap();

        let peers = state
            .heartbeat("grupo", "senha-segura", first.session_id)
            .unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].session_id, second.session_id);

        state
            .unregister("grupo", "senha-segura", second.session_id)
            .unwrap();
        let peers = state
            .heartbeat("grupo", "senha-segura", first.session_id)
            .unwrap();
        assert!(peers.is_empty());
    }

    #[test]
    fn relay_only_forwards_inside_group() {
        let state = state();
        state.create_network("a", "senha-segura").unwrap();
        state.create_network("b", "senha-segura").unwrap();
        let first = register(&state, "a", "senha-segura", 1, "pc-1").unwrap();
        let second = register(&state, "a", "senha-segura", 2, "pc-2").unwrap();
        let outsider = register(&state, "b", "senha-segura", 3, "pc-3").unwrap();

        let first_addr: SocketAddr = "203.0.113.10:5000".parse().unwrap();
        let second_addr: SocketAddr = "203.0.113.11:5000".parse().unwrap();
        let outsider_addr: SocketAddr = "203.0.113.12:5000".parse().unwrap();

        assert!(state.relay_bind(first.session_id, first_addr));
        assert!(state.relay_bind(second.session_id, second_addr));
        assert!(state.relay_bind(outsider.session_id, outsider_addr));

        assert_eq!(
            state.relay_route(first.session_id, second.session_id, first_addr),
            Some(second_addr)
        );
        assert_eq!(
            state.relay_route(first.session_id, outsider.session_id, first_addr),
            None
        );
    }

    #[test]
    fn expires_inactive_peers_and_keeps_network() {
        let state = AppState::new(Duration::from_millis(1), 8081, None);
        state
            .create_network("grupo", "senha-segura")
            .expect("rede de teste");
        let first = register(&state, "grupo", "senha-segura", 1, "pc-1").unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(state.cleanup(), 1);
        assert!(matches!(
            state.heartbeat("grupo", "senha-segura", first.session_id),
            Err(StateError::SessionNotFound)
        ));

        let again = register(&state, "grupo", "senha-segura", 2, "pc-2").unwrap();
        assert_eq!(again.virtual_ip, Ipv4Addr::new(26, 26, 26, 3));

        let first_again = register(&state, "grupo", "senha-segura", 1, "pc-1").unwrap();
        assert_eq!(first_again.virtual_ip, first.virtual_ip);
    }

    #[test]
    fn heartbeat_accepts_session_token_without_password() {
        let state = state();
        let first = register(&state, "grupo", "senha-segura", 1, "pc-1").unwrap();
        let second = register(&state, "grupo", "senha-segura", 2, "pc-2").unwrap();
        assert!(!first.session_token.is_empty());

        let peers = state
            .heartbeat_with_token("grupo", &first.session_token, first.session_id)
            .unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].session_id, second.session_id);
        assert!(matches!(
            state.heartbeat_with_token("grupo", "token-errado", first.session_id),
            Err(StateError::SessionNotFound)
        ));

        state
            .unregister_with_token("grupo", &second.session_token, second.session_id)
            .unwrap();
        assert!(state
            .heartbeat_with_token("grupo", &first.session_token, first.session_id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn persists_virtual_ips_across_restart() {
        let directory = tempfile::tempdir().expect("diretório temporário");
        let path = directory.path().join("networks.json");
        let state = AppState::with_networks_file(Duration::from_secs(75), 8081, None, path.clone())
            .expect("estado persistente");
        state
            .create_network("rede-estavel", "senha-segura")
            .expect("criar rede");
        let first = register(&state, "rede-estavel", "senha-segura", 1, "pc-1").unwrap();
        let second = register(&state, "rede-estavel", "senha-segura", 2, "pc-2").unwrap();
        assert_eq!(first.virtual_ip, Ipv4Addr::new(26, 26, 26, 2));
        assert_eq!(second.virtual_ip, Ipv4Addr::new(26, 26, 26, 3));

        drop(state);
        let reloaded = AppState::with_networks_file(Duration::from_secs(75), 8081, None, path)
            .expect("recarregar estado");
        let first_again = register(&reloaded, "rede-estavel", "senha-segura", 1, "pc-1").unwrap();
        let second_again = register(&reloaded, "rede-estavel", "senha-segura", 2, "pc-2").unwrap();
        assert_eq!(first_again.virtual_ip, first.virtual_ip);
        assert_eq!(second_again.virtual_ip, second.virtual_ip);
    }

    #[cfg(unix)]
    #[test]
    fn rolls_back_new_member_when_persistence_fails() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("diretório temporário");
        let path = directory.path().join("networks.json");
        let state = AppState::with_networks_file(Duration::from_secs(75), 8081, None, path.clone())
            .expect("estado persistente");
        state
            .create_network("rede-falha", "senha-segura")
            .expect("criar rede");

        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o500))
            .expect("tornar diretório somente leitura");
        let probe = directory.path().join("probe");
        if fs::write(&probe, b"").is_ok() {
            let _ = fs::remove_file(&probe);
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
                .expect("restaurar permissões");
            return;
        }

        let result = register(&state, "rede-falha", "senha-segura", 1, "pc-1");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("restaurar permissões");

        assert!(matches!(result, Err(StateError::Internal)));
        let networks = state.list_networks();
        assert_eq!(networks.len(), 1);
        assert_eq!(networks[0].known_peers, 0);
        assert_eq!(networks[0].online_peers, 0);

        let reloaded = AppState::with_networks_file(Duration::from_secs(75), 8081, None, path)
            .expect("recarregar estado");
        assert_eq!(reloaded.list_networks()[0].known_peers, 0);
    }

    #[test]
    fn loads_legacy_network_file() {
        let directory = tempfile::tempdir().expect("diretório temporário");
        let path = directory.path().join("networks.json");
        let hash = hash_password("senha-segura").unwrap();
        let contents = serde_json::json!({
            "version": 1,
            "networks": {
                "legada": { "password_hash": hash }
            }
        });
        fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();

        let state = AppState::with_networks_file(Duration::from_secs(75), 8081, None, path)
            .expect("arquivo legado");
        let peer = register(&state, "legada", "senha-segura", 1, "pc-1").unwrap();
        assert_eq!(peer.virtual_ip, Ipv4Addr::new(26, 26, 26, 2));
    }

    #[test]
    fn manages_networks_and_enforces_quota() {
        let state = AppState::new(Duration::from_secs(75), 8081, None)
            .with_admin_token(Some("token-de-admin".into()))
            .with_max_networks(2);
        assert_eq!(state.admin_token(), Some("token-de-admin"));

        state.create_network("primeira", "senha-segura").unwrap();
        let peer = register(&state, "primeira", "senha-segura", 1, "pc-1").unwrap();
        state.create_network("segunda", "senha-segura").unwrap();
        assert!(matches!(
            state.create_network("terceira", "senha-segura"),
            Err(StateError::ServerFull)
        ));

        let networks = state.list_networks();
        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0].name, "primeira");
        assert_eq!(networks[0].online_peers, 1);

        state
            .set_network_password("primeira", "nova-senha")
            .unwrap();
        assert!(matches!(
            state.heartbeat_with_token("primeira", &peer.session_token, peer.session_id),
            Err(StateError::SessionNotFound)
        ));
        assert!(register(&state, "primeira", "nova-senha", 1, "pc-1").is_ok());
        assert!(matches!(
            register(&state, "primeira", "senha-segura", 2, "pc-2"),
            Err(StateError::InvalidCredentials)
        ));

        state.delete_network("primeira").unwrap();
        assert!(matches!(
            register(&state, "primeira", "nova-senha", 1, "pc-1"),
            Err(StateError::NetworkNotFound)
        ));
    }

    #[test]
    fn requires_network_to_exist_before_register() {
        let state = state();
        assert!(matches!(
            register(&state, "rede-nova", "senha-segura", 1, "pc-1"),
            Err(StateError::NetworkNotFound)
        ));
        assert!(register(&state, "grupo", "senha-segura", 1, "pc-1").is_ok());
        assert!(matches!(
            state.create_network("grupo", "outra-senha"),
            Err(StateError::NetworkExists)
        ));
    }

    #[test]
    fn persists_networks_and_hashes_passwords() {
        let directory = tempfile::tempdir().expect("diretório temporário");
        let path = directory.path().join("networks.json");
        let state = AppState::with_networks_file(Duration::from_secs(75), 8081, None, path.clone())
            .expect("estado persistente");
        state
            .create_network("rede-persistente", "senha-segura")
            .expect("criar rede");

        let contents = fs::read_to_string(&path).expect("arquivo de redes");
        assert!(contents.contains("rede-persistente"));
        assert!(!contents.contains("senha-segura"));

        drop(state);
        let reloaded = AppState::with_networks_file(Duration::from_secs(75), 8081, None, path)
            .expect("recarregar estado");
        let peer = register(&reloaded, "rede-persistente", "senha-segura", 1, "pc-1")
            .expect("entrar após reinício");
        assert_eq!(peer.virtual_ip, Ipv4Addr::new(26, 26, 26, 2));
        assert!(matches!(
            register(&reloaded, "rede-persistente", "senha-errada", 2, "pc-2"),
            Err(StateError::InvalidCredentials)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn restricts_network_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("diretório temporário");
        let path = directory.path().join("networks.json");
        let state = AppState::with_networks_file(Duration::from_secs(75), 8081, None, path.clone())
            .expect("estado persistente");
        state
            .create_network("rede-privada", "senha-segura")
            .expect("criar rede");

        let mode = fs::metadata(&path).expect("metadados").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn derives_relay_address_from_host_header() {
        let state = state();
        assert_eq!(
            state.relay_address(Some("vpn.example.com:8080")),
            "vpn.example.com:8081"
        );
        assert_eq!(
            state.relay_address(Some("[2001:db8::1]:8080")),
            "[2001:db8::1]:8081"
        );
        assert_eq!(state.relay_address(None), "127.0.0.1:8081");

        let custom = AppState::new(
            Duration::from_secs(75),
            8081,
            Some("relay.example.com:9000".to_string()),
        );
        assert_eq!(
            custom.relay_address(Some("ignored.example.com:8080")),
            "relay.example.com:9000"
        );
    }
}
