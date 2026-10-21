use axum::{extract::State, Json};
use shadowvpn_core::{PeerInfo, PeersRequest, RegisterRequest, RegisterResponse};
use crate::state::AppState;

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Json<RegisterResponse> {
    // Valida ou define a senha do grupo
    if let Some(stored) = state.group_passwords.get(&req.group_name) {
        if stored.value() != &req.group_password {
            return Json(RegisterResponse {
                virtual_ip: String::new(),
                peers: vec![],
            });
        }
    } else {
        state.group_passwords.insert(req.group_name.clone(), req.group_password.clone());
    }

    let virtual_ip = state.allocate_ip(&req.group_name);
    let peer = PeerInfo {
        public_key: req.public_key,
        endpoint: req.endpoint,
        virtual_ip: virtual_ip.clone(),
    };

    let mut peers = state.groups.entry(req.group_name.clone()).or_default();
    peers.push(peer);

    // Retorna todos os outros peers (exclui o recém-registrado)
    let others: Vec<PeerInfo> = peers.iter()
        .filter(|p| p.virtual_ip != virtual_ip)
        .cloned()
        .collect();

    Json(RegisterResponse {
        virtual_ip,
        peers: others,
    })
}

pub async fn list_peers(
    State(state): State<AppState>,
    Json(req): Json<PeersRequest>,
) -> Json<Vec<PeerInfo>> {
    if let Some(stored) = state.group_passwords.get(&req.group_name) {
        if stored.value() != &req.group_password {
            return Json(vec![]);
        }
    } else {
        return Json(vec![]);
    }

    let peers = state.groups.get(&req.group_name)
        .map(|p| p.clone())
        .unwrap_or_default();
    Json(peers)
}