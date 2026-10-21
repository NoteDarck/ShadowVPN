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