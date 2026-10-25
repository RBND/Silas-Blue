//! Authenticated WebSocket telemetry stream (`/api/ws/logs`) with `Origin` validation and lag recovery.

use crate::routes::WebState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use silas_core::events::TelemetryEvent;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

pub async fn ws_logs_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<Arc<WebState>>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let settings = state.storage.get_app_settings();
    let Some(claims) = state.auth.authenticate_request(&headers, &settings) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Unauthorized WebSocket connection.",
        ));
    };

    // Global telemetry stream is restricted to bot owner
    if !claims.is_owner && !state.auth.is_bot_owner(&claims.user_id, &settings) {
        return Err((StatusCode::NOT_FOUND, "Not Found"));
    }

    // Validate Origin header matches Host to prevent Cross-Site WebSocket Hijacking (CSWSH)
    if let (Some(origin_val), Some(host_val)) = (
        headers
            .get(axum::http::header::ORIGIN)
            .and_then(|v| v.to_str().ok()),
        headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok()),
    ) {
        if let Ok(parsed_origin) = url::Url::parse(origin_val) {
            let origin_host_port = match (parsed_origin.host_str(), parsed_origin.port()) {
                (Some(h), Some(p)) => format!("{h}:{p}"),
                (Some(h), None) => h.to_string(),
                _ => String::new(),
            };
            if !origin_host_port.eq_ignore_ascii_case(host_val.trim()) {
                return Err((
                    StatusCode::FORBIDDEN,
                    "WebSocket Origin does not match Host.",
                ));
            }
        }
    }

    Ok(ws.on_upgrade(move |socket| handle_ws_client(socket, state)))
}

async fn handle_ws_client(mut socket: WebSocket, state: Arc<WebState>) {
    let mut rx = state.telemetry.subscribe();
    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(25));
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Stream live events, maintain 25s ping keepalive, and drain client pong frames
    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if socket.send(Message::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }
            msg_opt = socket.recv() => {
                match msg_opt {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => { /* drain incoming client pong/text */ }
                    Some(Err(_)) => break,
                }
            }
            recv_res = rx.recv() => {
                match recv_res {
                    Ok(packet) => {
                        if let Ok(json_str) = serde_json::to_string(&packet) {
                            if socket.send(Message::Text(json_str)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        let warn_pkt = TelemetryEvent::System {
                            message: format!("Skipped {skipped} telemetry events due to slow browser tab."),
                        }
                        .to_wire_packet();
                        if let Ok(json_str) = serde_json::to_string(&warn_pkt) {
                            if socket.send(Message::Text(json_str)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }
}
