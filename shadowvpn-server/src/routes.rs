use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use shadowvpn_core::{
    valid_endpoint, valid_group_name, valid_password, valid_public_key, PeerInfo, PeersRequest,
    RegisterRequest, RegisterResponse,
};

fn bad(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message })),
    )
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, (StatusCode, Json<serde_json::Value>)> {
    if !valid_group_name(&req.group_name)
        || !valid_password(&req.group_password)
        || !valid_public_key(&req.public_key)
        || !valid_endpoint(&req.endpoint)
    {
        return Err(bad("dados de registro inválidos"));
    }
    if let Some(stored) = state.group_passwords.get(&req.group_name) {
        if stored.value() != &req.group_password {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "senha do grupo incorreta" })),
            ));
        }
    } else {
        state
            .group_passwords
            .insert(req.group_name.clone(), req.group_password.clone());
    }
    let peer = PeerInfo {
        public_key: req.public_key,
        endpoint: req.endpoint,
        virtual_ip: String::new(),
    };
    let (virtual_ip, peers) = state.register_peer(&req.group_name, peer);
    Ok(Json(RegisterResponse { virtual_ip, peers }))
}

pub async fn list_peers(
    State(state): State<AppState>,
    Json(req): Json<PeersRequest>,
) -> Result<Json<Vec<PeerInfo>>, (StatusCode, Json<serde_json::Value>)> {
    if !valid_group_name(&req.group_name) || !valid_password(&req.group_password) {
        return Err(bad("dados de consulta inválidos"));
    }
    match state.group_passwords.get(&req.group_name) {
        Some(stored) if stored.value() == &req.group_password => {
            Ok(Json(state.peers(&req.group_name)))
        }
        Some(_) => Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "senha do grupo incorreta" })),
        )),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "grupo não encontrado" })),
        )),
    }
}
