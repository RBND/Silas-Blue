//! Real-time telemetry event bus for WebSocket streaming.
//! Truncates prompt and reply previews in memory so raw user conversations are never persisted to logs.

use chrono::Local;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::text::{ sanitize_single_line, truncate_with_suffix };

/// Maximum preview characters emitted to the live Web Console stream.
const MAX_PREVIEW_CHARS: usize = 180;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum TelemetryEvent {
    ConfigChange {
        guild_id: String,
        field: String,
        value: String,
    },
    Prompt {
        guild_name: String,
        user: String,
        prompt_preview: String,
    },
    Reply {
        guild_name: String,
        reply_preview: String,
    },
    ForumPostReplied {
        guild_name: String,
        post_title: String,
        author: String,
    },
    WebSearch {
        guild_name: String,
        query_preview: String,
        results_count: usize,
    },
    ServerJoined {
        guild_name: String,
        guild_id: String,
    },
    ServerLeft {
        guild_name: String,
        guild_id: String,
    },
    System {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireLogPacket {
    #[serde(rename = "type")]
    pub category: String,
    pub timestamp: String,
    pub formatted: String,
}

impl TelemetryEvent {
    pub fn to_wire_packet(&self) -> WireLogPacket {
        let now_str = Local::now().format("%H:%M:%S").to_string();
        let (category, formatted) = match self {
            Self::ConfigChange {
                guild_id,
                field,
                value,
            } => (
                "config",
                format!(
                    "[{now_str}] [Config Change] Guild {}: {} -> {}",
                    sanitize_single_line(guild_id, 32),
                    sanitize_single_line(field, 64),
                     truncate_with_suffix(value, 100, "...")
                ),
            ),
            Self::Prompt {
                guild_name,
                user,
                prompt_preview,
            } => (
                "prompt",
                format!(
                    "[{now_str}] [{}] {}: {}",
                    sanitize_single_line(guild_name, 64),
                    sanitize_single_line(user, 64),
                    truncate_with_suffix(prompt_preview, MAX_PREVIEW_CHARS, "...")
                ),
            ),
            Self::Reply {
                guild_name,
                reply_preview,
            } => (
                "reply",
                format!(
                    "[{now_str}] [{}] Silas Blue -> {}",
                    sanitize_single_line(guild_name, 64),
                    truncate_with_suffix(reply_preview, MAX_PREVIEW_CHARS, "...")
                ),
            ),
            Self::ForumPostReplied {
                guild_name,
                post_title,
                author,
            } => (
                "reply",
                format!(
                    "[{now_str}] [{}] Auto-replied to forum post \"{}\" by {}",
                    sanitize_single_line(guild_name, 64),
                    sanitize_single_line(post_title, 100),
                    sanitize_single_line(author, 64)
                ),
            ),
            Self::WebSearch {
                guild_name,
                query_preview,
                results_count,
            } => (
                "search",
                format!(
                    "[{now_str}] [{}] Web Search: \"{}\" ({} results)",
                    sanitize_single_line(guild_name, 64),
                    truncate_with_suffix(query_preview, 60, "..."),
                    results_count
                ),
            ),
            Self::ServerJoined {
                guild_name,
                guild_id,
            } => (
                "server",
                format!(
                    "[{now_str}] [Discord] Joined server: {} ({})",
                    sanitize_single_line(guild_name, 64),
                    sanitize_single_line(guild_id, 32)
                ),
            ),
            Self::ServerLeft {
                guild_name,
                guild_id,
            } => (
                "server",
                format!(
                    "[{now_str}] [Discord] Left server: {} ({})",
                    sanitize_single_line(guild_name, 64),
                    sanitize_single_line(guild_id, 32)
                ),
            ),
            Self::System { message } => (
                "system",
                format!(
                    "[{now_str}] [System] {}",
                    truncate_with_suffix(message, 240, "...")
                ),
            ),
        };

        WireLogPacket {
            category: category.to_owned(),
            timestamp: now_str,
            formatted,
        }
    }
}

/// Broadcast bus for live WebSocket log streaming with bounded backpressure.
#[derive(Clone)]
pub struct TelemetryBus {
    sender: broadcast::Sender<WireLogPacket>,
}

impl Default for TelemetryBus {
    fn default() -> Self {
        Self::new(512)
    }
}

impl TelemetryBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(32));
        Self { sender }
    }

    pub fn emit(&self, event: TelemetryEvent) {
        let _ = self.sender.send(event.to_wire_packet());
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WireLogPacket> {
        self.sender.subscribe()
    }
}
