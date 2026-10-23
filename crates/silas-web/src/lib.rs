//! Embedded Axum Web Console (`silas-web`) with loopback OTP, Discord OAuth2, CSRF protection,
//! live WebSocket telemetry, and compiled-in SPA assets (`rust-embed`).

pub mod auth;
pub mod routes;
pub mod ws;

pub use auth::{AuthManager, SessionClaims};
pub use routes::{build_router, EmbeddedAssets, WebState};

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

/// Bind and serve the embedded Web Console on `addr`.
pub async fn serve_web_console(
    addr: SocketAddr,
    state: Arc<WebState>,
) -> Result<(), std::io::Error> {
    let listener = TcpListener::bind(addr).await?;
    let router = build_router(state);
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}
