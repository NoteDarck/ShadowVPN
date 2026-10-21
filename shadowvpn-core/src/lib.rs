use serde::{Deserialize, Serialize};

/// Requisição para registrar um peer em um grupo
#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub group_name: String,
    pub group_password: String,
    /// Chave pública WireGuard em base64
    pub public_key: String,
    /// Endpoint público (IP:porta) para conexão direta
    pub endpoint: String,
}

/// Resposta com os dados da rede virtual
#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterResponse {
    /// IP virtual atribuído (ex: "10.0.0.2")
    pub virtual_ip: String,
    /// Lista de outros peers no mesmo grupo
    pub peers: Vec<PeerInfo>,
}

/// Informações de um peer retornadas pelo servidor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub public_key: String,
    pub endpoint: String,
    pub virtual_ip: String,
}

/// Requisição para consultar peers de um grupo já registrado
#[derive(Debug, Serialize, Deserialize)]
pub struct PeersRequest {
    pub group_name: String,
    pub group_password: String,
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

pub fn valid_public_key(value: &str) -> bool {
    use base64::{engine::general_purpose::STANDARD, Engine};
    STANDARD
        .decode(value)
        .map(|bytes| bytes.len() == 32)
        .unwrap_or(false)
}

pub fn valid_endpoint(value: &str) -> bool {
    value.parse::<std::net::SocketAddr>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_registration_fields() {
        assert!(valid_group_name("team_dev-1"));
        assert!(!valid_group_name("team dev"));
        assert!(valid_password("password"));
        assert!(!valid_password("short"));
        assert!(valid_endpoint("127.0.0.1:51820"));
        assert!(!valid_endpoint("not-an-endpoint"));
    }
}
