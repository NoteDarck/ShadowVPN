use crate::state::{AppState, StateError};
use axum::{
    extract::{Path, State},
    http::{header::HOST, HeaderMap, StatusCode},
    Json,
};
use shadowvpn_core::{
    clean_hostname, valid_group_name, valid_password, valid_public_key, CreateNetworkRequest,
    CreateNetworkResponse, ErrorResponse, NetworkListResponse, PeersRequest, PeersResponse,
    RegisterRequest, RegisterResponse, RotatePasswordRequest, UnregisterRequest,
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
        StateError::InvalidIdentity => {
            api_error(StatusCode::BAD_REQUEST, "identidade WireGuard inválida")
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
        StateError::ServerFull => api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "o servidor atingiu o limite de redes",
        ),
        StateError::SessionNotFound => {
            api_error(StatusCode::NOT_FOUND, "sessão não encontrada ou expirada")
        }
        StateError::Internal => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "erro interno ao processar a requisição",
        ),
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .trim();
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
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

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected) = state.admin_token() else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "administração desabilitada; defina SHADOWVPN_ADMIN_TOKEN no servidor",
        ));
    };
    let authorized = bearer_token(headers)
        .map(|token| secure_eq(token, expected))
        .unwrap_or(false);
    if authorized {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::UNAUTHORIZED,
            "token de administração ausente ou inválido",
        ))
    }
}

fn require_create_permission(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if state.admin_token().is_some() {
        require_admin(state, headers)?;
    }
    Ok(())
}

pub async fn create_network(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateNetworkRequest>,
) -> Result<(StatusCode, Json<CreateNetworkResponse>), ApiError> {
    require_create_permission(&state, &headers)?;

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

pub async fn admin_list_networks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<NetworkListResponse> {
    require_admin(&state, &headers)?;
    Ok(Json(NetworkListResponse {
        networks: state.list_networks(),
    }))
}

pub async fn admin_delete_network(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_name): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    if !valid_group_name(&group_name) {
        return Err(api_error(StatusCode::BAD_REQUEST, "nome de rede inválido"));
    }
    state.delete_network(&group_name).map_err(state_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn admin_rotate_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_name): Path<String>,
    Json(req): Json<RotatePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    require_admin(&state, &headers)?;
    if !valid_group_name(&group_name) || !valid_password(&req.new_password) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "dados da rede inválidos",
        ));
    }
    state
        .set_network_password(&group_name, &req.new_password)
        .map_err(state_error)?;
    Ok(StatusCode::NO_CONTENT)
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
            &req.identity,
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
        session_token: outcome.session_token,
        relay,
        peers: outcome.peers,
    }))
}

pub async fn peers(
    State(state): State<AppState>,
    Json(req): Json<PeersRequest>,
) -> ApiResult<PeersResponse> {
    let has_token = !req.session_token.is_empty();
    if !valid_group_name(&req.group_name)
        || req.session_id == 0
        || (!has_token && !valid_password(&req.group_password))
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "dados de consulta inválidos",
        ));
    }

    let peers = if has_token {
        state.heartbeat_with_token(&req.group_name, &req.session_token, req.session_id)
    } else {
        state.heartbeat(&req.group_name, &req.group_password, req.session_id)
    }
    .map_err(state_error)?;

    Ok(Json(PeersResponse { peers }))
}

pub async fn unregister(
    State(state): State<AppState>,
    Json(req): Json<UnregisterRequest>,
) -> Result<StatusCode, ApiError> {
    let has_token = !req.session_token.is_empty();
    if !valid_group_name(&req.group_name)
        || req.session_id == 0
        || (!has_token && !valid_password(&req.group_password))
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "dados de desconexão inválidos",
        ));
    }

    if has_token {
        state.unregister_with_token(&req.group_name, &req.session_token, req.session_id)
    } else {
        state.unregister(&req.group_name, &req.group_password, req.session_id)
    }
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
    use std::time::Duration;
    use tower::ServiceExt;

    fn test_state(admin_token: Option<&str>) -> AppState {
        AppState::new(Duration::from_secs(75), 8081, None)
            .with_admin_token(admin_token.map(str::to_owned))
    }

    fn app_with_state(state: AppState) -> Router {
        Router::new()
            .route("/networks", axum::routing::post(create_network))
            .route("/register", axum::routing::post(register))
            .route("/peers", axum::routing::post(peers))
            .route("/unregister", axum::routing::post(unregister))
            .route("/admin/networks", axum::routing::get(admin_list_networks))
            .route(
                "/admin/networks/:name",
                axum::routing::delete(admin_delete_network),
            )
            .route(
                "/admin/networks/:name/password",
                axum::routing::post(admin_rotate_password),
            )
            .with_state(state)
    }

    fn app() -> Router {
        app_with_state(test_state(None))
    }

    fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("requisição")
    }

    fn post_auth(uri: &str, body: serde_json::Value, token: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body.to_string()))
            .expect("requisição")
    }

    fn delete_auth(uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method("DELETE")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("requisição")
    }

    fn get_auth(uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("requisição")
    }

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("corpo");
        serde_json::from_slice(&body).expect("JSON")
    }

    fn key() -> String {
        shadowvpn_crypto::public_key_b64(&[7u8; 32])
    }

    fn identity(group_name: &str) -> serde_json::Value {
        serde_json::to_value(
            shadowvpn_crypto::create_peer_identity(&[7u8; 32], group_name, &key()).unwrap(),
        )
        .unwrap()
    }

    fn register_body(group_name: &str, password: &str) -> serde_json::Value {
        serde_json::json!({
            "group_name": group_name,
            "group_password": password,
            "public_key": key(),
            "identity": identity(group_name),
            "hostname": "pc-1"
        })
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
            .oneshot(post("/register", register_body("rede-api", "senha-segura")))
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
            .oneshot(post("/register", register_body("rede-api", "senha-segura")))
            .await
            .expect("registro após criação");
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["virtual_ip"], "26.26.26.2");
        assert_eq!(body["relay"], "127.0.0.1:8081");
    }

    #[tokio::test]
    async fn rejects_invalid_peer_identity() {
        let state = AppState::new(Duration::from_secs(75), 8081, None);
        state
            .create_network("rede-api", "senha-segura")
            .expect("rede");
        let app = Router::new()
            .route("/register", axum::routing::post(register))
            .with_state(state);
        let mut body = register_body("rede-api", "senha-segura");
        body["identity"]["signature"] = serde_json::json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==");
        let response = app
            .oneshot(post("/register", body))
            .await
            .expect("registro");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
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

    #[tokio::test]
    async fn heartbeat_and_unregister_accept_session_token() {
        let state = test_state(None);
        state
            .create_network("rede-api", "senha-segura")
            .expect("rede");
        let app = app_with_state(state);

        let response = app
            .clone()
            .oneshot(post("/register", register_body("rede-api", "senha-segura")))
            .await
            .expect("registro");
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        let session_id = body["session_id"].as_u64().expect("sessão");
        let session_token = body["session_token"].as_str().expect("token");
        assert!(!session_token.is_empty());

        let response = app
            .clone()
            .oneshot(post(
                "/peers",
                serde_json::json!({
                    "group_name": "rede-api",
                    "session_id": session_id,
                    "session_token": session_token
                }),
            ))
            .await
            .expect("heartbeat por token");
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(post(
                "/unregister",
                serde_json::json!({
                    "group_name": "rede-api",
                    "session_id": session_id,
                    "session_token": session_token
                }),
            ))
            .await
            .expect("desconexão por token");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn admin_routes_require_token_and_manage_network() {
        let state = test_state(Some("token-de-admin"));
        let app = app_with_state(state);

        let response = app
            .clone()
            .oneshot(post("/networks", create_body()))
            .await
            .expect("criar sem token");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .clone()
            .oneshot(post_auth("/networks", create_body(), "token-de-admin"))
            .await
            .expect("criar com token");
        assert_eq!(response.status(), StatusCode::CREATED);

        let response = app
            .clone()
            .oneshot(get_auth("/admin/networks", "token-de-admin"))
            .await
            .expect("listar redes");
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["networks"][0]["name"], "rede-api");

        let response = app
            .clone()
            .oneshot(post_auth(
                "/admin/networks/rede-api/password",
                serde_json::json!({ "new_password": "senha-nova" }),
                "token-de-admin",
            ))
            .await
            .expect("trocar senha");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = app
            .clone()
            .oneshot(post("/register", register_body("rede-api", "senha-segura")))
            .await
            .expect("senha antiga");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .clone()
            .oneshot(post("/register", register_body("rede-api", "senha-nova")))
            .await
            .expect("senha nova");
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(delete_auth("/admin/networks/rede-api", "token-de-admin"))
            .await
            .expect("remover rede");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = app
            .oneshot(post("/register", register_body("rede-api", "senha-nova")))
            .await
            .expect("rede removida");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
