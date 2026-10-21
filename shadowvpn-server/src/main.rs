mod routes;
mod state;

use axum::{routing::post, Router};
use state::AppState;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let state = AppState::new();

    let app = Router::new()
        .route("/register", post(routes::register))
        .route("/peers", post(routes::list_peers))
        .with_state(state);

    let addr: SocketAddr = std::env::var("SHADOWVPN_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()
        .expect("SHADOWVPN_LISTEN deve ser IP:porta válido");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    tracing::info!(%addr, "ShadowVPN coordination server iniciado");
    axum::serve(listener, app).await.unwrap();
}
