mod routes;
mod state;

use axum::{routing::post, Router};
use state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let state = AppState::new();

    let app = Router::new()
        .route("/register", post(routes::register))
        .route("/peers", post(routes::list_peers))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .unwrap();

    tracing::info!("ShadowVPN coordination server on 0.0.0.0:8080");
    axum::serve(listener, app).await.unwrap();
}