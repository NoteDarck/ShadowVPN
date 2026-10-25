use crate::state::{AppState, StateError};
use axum::{
    extract::State,
    http::{header::HOST, HeaderMap, StatusCode},
    Json,
};
use shadowvpn_core::{
    clean_hostname, valid_group_name, valid_password, valid_public_key, ErrorResponse,
    PeersRequest, PeersResponse, RegisterRequest, RegisterResponse, UnregisterRequest,
};

type ApiError = (StatusCode, Json<ErrorResponse>);
type ApiResult<T> = Result<Json<T>, ApiError>;

fn api_error(status: StatusCode, message: &str) -> ApiError {
    (
        status,
        Json(ErrorResponse {
            error: message.to_string(),
        }),
    )
}

fn state_error(error: StateError) -> ApiError {
    match error {
        StateError::InvalidCredentials => {
            api_error(StatusCode::UNAUTHORIZED, "senha ou grupo inválidos")
        }
        StateError::GroupFull => {
            api_error(StatusCode::CONFLICT, "o grupo atingiu o limite de peers")
        }
        StateError::SessionNotFound => {
            api_error(StatusCode::NOT_FOUND, "sessão não encontrada ou expirada")
        }
        StateError::Internal => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "erro interno ao processar a requisição",
        ),
    }
}

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<RegisterResponse> {
    if !valid_group_name(&req.group_name)
        || !valid_password(&req.group_password)
        || !valid_public_key(&req.public_key)
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "dados de registro inválidos",
        ));
    }

    let hostname = clean_hostname(&req.hostname);
    let outcome = state
        .register(
            &req.group_name,
            &req.group_password,
            &req.public_key,
            &hostname,
        )
        .map_err(state_error)?;

    let host = headers.get(HOST).and_then(|value| value.to_str().ok());
    let relay = state.relay_address(host);

    tracing::info!(
        group = %req.group_name,
        virtual_ip = %outcome.virtual_ip,
        peers = outcome.peers.len(),
        hostname = %hostname,
        "peer registrado"
    );

    Ok(Json(RegisterResponse {
        virtual_ip: outcome.virtual_ip.to_string(),
        session_id: outcome.session_id,
        relay,
        peers: outcome.peers,
    }))
}

pub async fn peers(
    State(state): State<AppState>,
    Json(req): Json<PeersRequest>,
) -> ApiResult<PeersResponse> {
    if !valid_group_name(&req.group_name)
        || !valid_password(&req.group_password)
        || req.session_id == 0
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "dados de consulta inválidos",
        ));
    }

    let peers = state
        .heartbeat(&req.group_name, &req.group_password, req.session_id)
        .map_err(state_error)?;

    Ok(Json(PeersResponse { peers }))
}

pub async fn unregister(
    State(state): State<AppState>,
    Json(req): Json<UnregisterRequest>,
) -> Result<StatusCode, ApiError> {
    if !valid_group_name(&req.group_name)
        || !valid_password(&req.group_password)
        || req.session_id == 0
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "dados de desconexão inválidos",
        ));
    }

    state
        .unregister(&req.group_name, &req.group_password, req.session_id)
        .map_err(state_error)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}
