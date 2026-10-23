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
    if state.auth.authenticate_request(&headers, &settings).is_none() {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Unauthorized WebSocket connection.",
        ));
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

    // Send an initial welcome packet
    let welcome = TelemetryEvent::System {
        message: "Connected to Silas Blue live telemetry stream.".to_string(),
    }
    .to_wire_packet();
    if let Ok(json_str) = serde_json::to_string(&welcome) {
        if socket.send(Message::Text(json_str)).await.is_err() {
            return;
        }
    }

    // Stream live events and handle RecvError::Lagged gracefully
    loop {
        match rx.recv().await {
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
