use serde::{Deserialize, Serialize};

/// Prefixo IPv4 usado pela rede virtual. O gateway é .1 e os peers começam em .2.
pub const VIRTUAL_NETWORK_PREFIX: [u8; 3] = [26, 26, 26];
pub const VIRTUAL_NETMASK: [u8; 4] = [255, 255, 255, 0];
pub const VIRTUAL_GATEWAY: [u8; 4] = [26, 26, 26, 1];
pub const DEFAULT_HTTP_PORT: u16 = 8080;
pub const DEFAULT_RELAY_PORT: u16 = 8081;
pub const DEFAULT_PEER_TTL_SECS: u64 = 75;
pub const HEARTBEAT_SECS: u64 = 20;

pub const RELAY_MAGIC: [u8; 4] = *b"SVPN";
pub const RELAY_VERSION: u8 = 1;
pub const RELAY_HEADER_LEN: usize = 22;
pub const INVITE_SCHEME: &str = "shadowvpn";

/// Prova XEdDSA de posse da chave privada WireGuard.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerIdentity {
    /// Assinatura XEdDSA em base64.
    pub signature: String,
}

/// Convite que reúne o servidor de coordenação e o nome da rede.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Invite {
    pub server: String,
    pub network: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteError {
    InvalidLink,
    UnsupportedScheme,
    MissingServer,
    MissingNetwork,
    InvalidServer,
    InvalidNetwork,
}

impl std::fmt::Display for InviteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InviteError::InvalidLink => formatter.write_str("o convite não é um link válido"),
            InviteError::UnsupportedScheme => {
                formatter.write_str("o convite deve começar com shadowvpn://")
            }
            InviteError::MissingServer => formatter.write_str("o convite não inclui o servidor"),
            InviteError::MissingNetwork => formatter.write_str("o convite não inclui a rede"),
            InviteError::InvalidServer => formatter.write_str("o servidor do convite é inválido"),
            InviteError::InvalidNetwork => {
                formatter.write_str("o nome da rede do convite é inválido")
            }
        }
    }
}

impl std::error::Error for InviteError {}

impl Invite {
    pub fn new(server: &str, network: &str) -> Result<Self, InviteError> {
        let server = normalize_server(server).ok_or(InviteError::InvalidServer)?;
        let network = network.trim();
        if !valid_group_name(network) {
            return Err(InviteError::InvalidNetwork);
        }
        Ok(Self {
            server,
            network: network.to_owned(),
        })
    }

    pub fn parse(value: &str) -> Result<Self, InviteError> {
        let value = value.trim();
        let url = url::Url::parse(value).map_err(|_| InviteError::InvalidLink)?;
        if url.scheme() != INVITE_SCHEME {
            return Err(InviteError::UnsupportedScheme);
        }

        let mut server = None;
        let mut network = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "server" => server = Some(value.into_owned()),
                "network" => network = Some(value.into_owned()),
                _ => {}
            }
        }

        let server = server.ok_or(InviteError::MissingServer)?;
        let network = network.ok_or(InviteError::MissingNetwork)?;
        Self::new(&server, &network)
    }

    pub fn to_url(&self) -> String {
        let mut url = url::Url::parse("shadowvpn://join").expect("URL de convite fixa");
        url.query_pairs_mut()
            .append_pair("server", &self.server)
            .append_pair("network", &self.network);
        url.to_string()
    }
}

fn normalize_server(value: &str) -> Option<String> {
    let server = value.trim().trim_end_matches('/');
    let url = url::Url::parse(server).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(server.to_owned())
}

/// Requisição para criar uma rede no servidor.
#[derive(Clone, Serialize, Deserialize)]
pub struct CreateNetworkRequest {
    pub group_name: String,
    pub group_password: String,
}

/// Resposta da criação de uma rede.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateNetworkResponse {
    pub group_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSummary {
    pub name: String,
    pub online_peers: usize,
    pub known_peers: usize,
    #[serde(default)]
    pub blocked_peers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkListResponse {
    pub networks: Vec<NetworkSummary>,
}

/// Membro conhecido ou revogado de uma rede.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkMember {
    pub public_key: String,
    pub fingerprint: String,
    #[serde(default)]
    pub virtual_ip: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    pub online: bool,
    pub blocked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberListResponse {
    pub members: Vec<NetworkMember>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MemberKeyRequest {
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotatePasswordRequest {
    pub new_password: String,
}

/// Requisição para entrar em um grupo.
#[derive(Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub group_name: String,
    pub group_password: String,
    /// Chave pública WireGuard em base64.
    pub public_key: String,
    /// Prova de posse da chave privada WireGuard.
    pub identity: PeerIdentity,
    /// Nome exibido no grupo (hostname do computador).
    #[serde(default)]
    pub hostname: String,
}

/// Resposta do registro no grupo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    /// IP virtual atribuído (ex.: "26.26.26.2").
    pub virtual_ip: String,
    /// Identificador opaco usado no relay UDP.
    pub session_id: u64,
    /// Segredo da sessão para heartbeat e desconexão sem reenviar a senha.
    #[serde(default)]
    pub session_token: String,
    /// Endereço do relay UDP (ex.: "vpn.example.com:8081").
    pub relay: String,
    /// Outros peers ativos no mesmo grupo.
    pub peers: Vec<PeerInfo>,
}

/// Informações de um peer ativo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub public_key: String,
    pub identity: PeerIdentity,
    pub fingerprint: String,
    pub session_id: u64,
    pub virtual_ip: String,
    /// Nome exibido no grupo (hostname do computador).
    #[serde(default)]
    pub hostname: String,
}

/// Requisição de heartbeat/consulta de peers.
#[derive(Clone, Serialize, Deserialize)]
pub struct PeersRequest {
    pub group_name: String,
    /// Mantido para compatibilidade com clientes antigos.
    #[serde(default)]
    pub group_password: String,
    pub session_id: u64,
    /// Preferido: autentica a sessão sem reenviar a senha da rede.
    #[serde(default)]
    pub session_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeersResponse {
    pub peers: Vec<PeerInfo>,
}

/// Remove um peer do grupo ao desconectar.
#[derive(Clone, Serialize, Deserialize)]
pub struct UnregisterRequest {
    pub group_name: String,
    /// Mantido para compatibilidade com clientes antigos.
    #[serde(default)]
    pub group_password: String,
    pub session_id: u64,
    /// Preferido: autentica a sessão sem reenviar a senha da rede.
    #[serde(default)]
    pub session_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Tipo de datagrama do relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayKind {
    /// Mantém o mapeamento NAT do cliente no servidor.
    Hello,
    /// Datagrama WireGuard encapsulado.
    Data,
}

impl RelayKind {
    pub fn as_u8(self) -> u8 {
        match self {
            RelayKind::Hello => 1,
            RelayKind::Data => 2,
        }
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(RelayKind::Hello),
            2 => Some(RelayKind::Data),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayPacket<'a> {
    pub kind: RelayKind,
    pub sender: u64,
    pub destination: u64,
    pub payload: &'a [u8],
}

/// Serializa um datagrama do relay em `out`, retornando o tamanho usado.
pub fn encode_relay_packet(
    kind: RelayKind,
    sender: u64,
    destination: u64,
    payload: &[u8],
    out: &mut [u8],
) -> Option<usize> {
    let total = RELAY_HEADER_LEN.checked_add(payload.len())?;
    if out.len() < total {
        return None;
    }

    out[0..4].copy_from_slice(&RELAY_MAGIC);
    out[4] = RELAY_VERSION;
    out[5] = kind.as_u8();
    out[6..14].copy_from_slice(&sender.to_le_bytes());
    out[14..22].copy_from_slice(&destination.to_le_bytes());
    out[RELAY_HEADER_LEN..total].copy_from_slice(payload);
    Some(total)
}

pub fn parse_relay_packet(datagram: &[u8]) -> Option<RelayPacket<'_>> {
    if datagram.len() < RELAY_HEADER_LEN || datagram[0..4] != RELAY_MAGIC {
        return None;
    }
    if datagram[4] != RELAY_VERSION {
        return None;
    }
    let kind = RelayKind::from_u8(datagram[5])?;
    let sender = u64::from_le_bytes(datagram[6..14].try_into().ok()?);
    let destination = u64::from_le_bytes(datagram[14..22].try_into().ok()?);
    Some(RelayPacket {
        kind,
        sender,
        destination,
        payload: &datagram[RELAY_HEADER_LEN..],
    })
}

pub fn valid_group_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn valid_password(value: &str) -> bool {
    (8..=128).contains(&value.len())
}

/// Limpa um hostname para exibição: remove controles, espaços nas pontas e limita o tamanho.
pub fn clean_hostname(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(64)
        .collect();
    cleaned.trim().to_string()
}

pub fn valid_public_key(value: &str) -> bool {
    use base64::{engine::general_purpose::STANDARD, Engine};
    STANDARD
        .decode(value)
        .map(|bytes| bytes.len() == 32)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_registration_fields() {
        assert!(valid_group_name("team_dev-1"));
        assert!(!valid_group_name("team dev"));
        assert!(!valid_group_name(""));
        assert!(valid_password("password"));
        assert!(!valid_password("short"));
        assert!(valid_public_key(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        ));
        assert!(!valid_public_key("não-é-uma-chave"));
        assert_eq!(clean_hostname("  notebook\n"), "notebook");
        assert_eq!(clean_hostname("pc \u{1}"), "pc");
        assert_eq!(clean_hostname(""), "");
    }

    #[test]
    fn invite_roundtrip_and_validation() {
        let invite = Invite::new("http://vpn.example.com:8080/", "minha-rede").unwrap();
        let link = invite.to_url();
        assert_eq!(Invite::parse(&link).unwrap(), invite);

        assert_eq!(
            Invite::parse("https://example.com/?server=x&network=y"),
            Err(InviteError::UnsupportedScheme)
        );
        assert_eq!(
            Invite::parse("shadowvpn://join?network=minha-rede"),
            Err(InviteError::MissingServer)
        );
        assert_eq!(
            Invite::parse("shadowvpn://join?server=http%3A%2F%2Fexample.com"),
            Err(InviteError::MissingNetwork)
        );
        assert_eq!(
            Invite::new("ftp://example.com", "minha-rede"),
            Err(InviteError::InvalidServer)
        );
        assert_eq!(
            Invite::new("http://example.com", "nome inválido"),
            Err(InviteError::InvalidNetwork)
        );
    }

    #[test]
    fn relay_packet_roundtrip() {
        let payload = b"wireguard";
        let mut out = [0u8; 64];
        let len =
            encode_relay_packet(RelayKind::Data, 42, 7, payload, &mut out).expect("cabe no buffer");
        let parsed = parse_relay_packet(&out[..len]).expect("pacote válido");

        assert_eq!(parsed.kind, RelayKind::Data);
        assert_eq!(parsed.sender, 42);
        assert_eq!(parsed.destination, 7);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn relay_packet_rejects_invalid_input() {
        assert!(parse_relay_packet(b"SVPN").is_none());

        let mut out = [0u8; RELAY_HEADER_LEN];
        encode_relay_packet(RelayKind::Hello, 1, 0, &[], &mut out).expect("cabe");
        out[0] = b'X';
        assert!(parse_relay_packet(&out).is_none());

        let mut out = [0u8; RELAY_HEADER_LEN];
        encode_relay_packet(RelayKind::Hello, 1, 0, &[], &mut out).expect("cabe");
        out[5] = 99;
        assert!(parse_relay_packet(&out).is_none());
    }

    #[test]
    fn relay_packet_requires_enough_space() {
        let mut out = [0u8; RELAY_HEADER_LEN];
        assert!(encode_relay_packet(RelayKind::Data, 1, 2, b"x", &mut out).is_none());
    }
}
