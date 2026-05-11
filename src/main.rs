mod auth;
mod captcha;
mod db;
mod routes;
mod session;
mod slug;
mod users;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use sqlx::PgPool;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

pub struct AppState {
    pub pool: PgPool,
    pub rate_limiter: Mutex<HashMap<String, Instant>>,
    pub jwt_secret: Vec<u8>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL required (postgres://user:pass@host:5432/db)"))?;
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".into())
        .parse()?;

    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
        tracing::warn!("JWT_SECRET not set, generating ephemeral secret — sessions invalidate on restart");
        format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4())
    });

    let pool = db::connect(&database_url).await?;
    let state = Arc::new(AppState {
        pool,
        rate_limiter: Mutex::new(HashMap::new()),
        jwt_secret: jwt_secret.into_bytes(),
    });

    let app = routes::router(state).layer(TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("listening on port {port}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
    tokio::select! {
        _ = ctrl_c => {}
        _ = sigterm.recv() => {}
    }
    tracing::info!("shutdown signal received");
}
