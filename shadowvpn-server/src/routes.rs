use crate::state::{AppState, StateError};
use axum::{
    extract::State,
    http::{header::HOST, HeaderMap, StatusCode},
    Json,
};
use shadowvpn_core::{
    clean_hostname, valid_group_name, valid_password, valid_public_key, CreateNetworkRequest,
    CreateNetworkResponse, ErrorResponse, PeersRequest, PeersResponse, RegisterRequest,
    RegisterResponse, UnregisterRequest,
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
        StateError::NetworkNotFound => api_error(
            StatusCode::NOT_FOUND,
            "rede não encontrada; crie a rede primeiro",
        ),
        StateError::NetworkExists => {
            api_error(StatusCode::CONFLICT, "já existe uma rede com esse nome")
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

pub async fn create_network(
    State(state): State<AppState>,
    Json(req): Json<CreateNetworkRequest>,
) -> Result<(StatusCode, Json<CreateNetworkResponse>), ApiError> {
    if !valid_group_name(&req.group_name) || !valid_password(&req.group_password) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "dados da rede inválidos",
        ));
    }

    state
        .create_network(&req.group_name, &req.group_password)
        .map_err(state_error)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateNetworkResponse {
            group_name: req.group_name,
        }),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, body::Body, http::Request, Router};
    use base64::{engine::general_purpose::STANDARD, Engine};
    use std::time::Duration;
    use tower::ServiceExt;

    fn app() -> Router {
        let state = AppState::new(Duration::from_secs(75), 8081, None);
        Router::new()
            .route("/networks", axum::routing::post(create_network))
            .route("/register", axum::routing::post(register))
            .with_state(state)
    }

    fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("requisição")
    }

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("corpo");
        serde_json::from_slice(&body).expect("JSON")
    }

    fn key() -> String {
        STANDARD.encode([7u8; 32])
    }

    fn create_body() -> serde_json::Value {
        serde_json::json!({
            "group_name": "rede-api",
            "group_password": "senha-segura"
        })
    }

    #[tokio::test]
    async fn creates_network_and_rejects_duplicate() {
        let app = app();
        let response = app
            .clone()
            .oneshot(post("/networks", create_body()))
            .await
            .expect("criar rede");
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            body_json(response).await,
            serde_json::json!({ "group_name": "rede-api" })
        );

        let response = app
            .oneshot(post("/networks", create_body()))
            .await
            .expect("rede duplicada");
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn register_requires_existing_network() {
        let response = app()
            .oneshot(post(
                "/register",
                serde_json::json!({
                    "group_name": "rede-api",
                    "group_password": "senha-segura",
                    "public_key": key(),
                    "hostname": "pc-1"
                }),
            ))
            .await
            .expect("registro");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let state = AppState::new(Duration::from_secs(75), 8081, None);
        state
            .create_network("rede-api", "senha-segura")
            .expect("rede");
        let app = Router::new()
            .route("/register", axum::routing::post(register))
            .with_state(state);
        let response = app
            .oneshot(post(
                "/register",
                serde_json::json!({
                    "group_name": "rede-api",
                    "group_password": "senha-segura",
                    "public_key": key(),
                    "hostname": "pc-1"
                }),
            ))
            .await
            .expect("registro após criação");
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["virtual_ip"], "26.26.26.2");
        assert_eq!(body["relay"], "127.0.0.1:8081");
    }

    #[tokio::test]
    async fn rejects_invalid_network_data() {
        let response = app()
            .oneshot(post(
                "/networks",
                serde_json::json!({
                    "group_name": "nome inválido!",
                    "group_password": "curta"
                }),
            ))
            .await
            .expect("dados inválidos");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
