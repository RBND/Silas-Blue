//! Volatile multi-turn conversation memory engine with strict channel/DM isolation,
//! LRU session capacity bounds (`MAX_ACTIVE_SESSIONS = 1,000`), inactivity TTL expiration,
//! citation-preserving truncation, and Discord-ToS message/channel/guild deletion eviction.

use std::time::{Duration, Instant};

use indexmap::IndexMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::text::{char_count, truncate_chars};

/// Hard cap on active conversation sessions in RAM to prevent OOM under spam.
pub const MAX_ACTIVE_SESSIONS: usize = 1_000;

/// Default character budget for historical assistant responses in memory.
pub const DEFAULT_MAX_ASSISTANT_CHARS: usize = 1_500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextKey {
    GuildChannel { guild_id: u64, channel_id: u64 },
    DirectMessage { user_id: u64 },
}

impl ContextKey {
    pub fn guild_channel(guild_id: u64, channel_id: u64) -> Self {
        Self::GuildChannel {
            guild_id,
            channel_id,
        }
    }

    pub fn dm(user_id: u64) -> Self {
        Self::DirectMessage { user_id }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct TurnEntry {
    pub message_id: Option<u64>,
    pub author_id: u64,
    pub user_prompt: String,
    pub assistant_response: String,
}

#[derive(Debug, Clone)]
struct SessionBuffer {
    last_active: Instant,
    turns: Vec<TurnEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total_sessions: usize,
    pub dm_sessions: usize,
    pub guild_sessions: usize,
    pub total_stored_messages: usize,
}

/// Thread-safe, poison-free volatile conversation memory engine.
pub struct ConversationMemory {
    default_ttl: Duration,
    max_assistant_chars: usize,
    buffers: RwLock<IndexMap<ContextKey, SessionBuffer>>,
}

impl Default for ConversationMemory {
    fn default() -> Self {
        Self::new(30, DEFAULT_MAX_ASSISTANT_CHARS)
    }
}

impl ConversationMemory {
    pub fn new(default_ttl_minutes: u32, max_assistant_chars: usize) -> Self {
        Self {
            default_ttl: Duration::from_secs(u64::from(default_ttl_minutes).saturating_mul(60)),
            max_assistant_chars: max_assistant_chars.max(200),
            buffers: RwLock::new(IndexMap::new()),
        }
    }

    /// Retrieves up to `max_turns` conversation pairs (`2 * max_turns` messages) for `key`.
    /// Evicts the session if inactivity TTL has elapsed.
    pub fn get_history(
        &self,
        key: ContextKey,
        max_turns: u32,
        ttl_minutes: Option<u32>,
    ) -> Vec<ChatMessage> {
        if max_turns == 0 {
            return Vec::new();
        }

        let ttl = ttl_minutes.map_or(self.default_ttl, |m| {
            Duration::from_secs(u64::from(m).saturating_mul(60))
        });

        let mut map = self.buffers.write();
        let now = Instant::now();

        let Some(buf) = map.get_mut(&key) else {
            return Vec::new();
        };

        if !ttl.is_zero() && now.duration_since(buf.last_active) > ttl {
            map.shift_remove(&key);
            return Vec::new();
        }

        buf.last_active = now;
        let keep_turns = (max_turns.min(50)) as usize;
        let start_idx = buf.turns.len().saturating_sub(keep_turns);
        let slice = buf.turns.get(start_idx..).unwrap_or(&[]);

        let mut out = Vec::with_capacity(slice.len().saturating_mul(2));
        for turn in slice {
            out.push(ChatMessage {
                role: "user".to_owned(),
                content: turn.user_prompt.clone(),
            });
            out.push(ChatMessage {
                role: "assistant".to_owned(),
                content: turn.assistant_response.clone(),
            });
        }
        out
    }

    /// Appends a completed user/assistant turn to `key`, compressing verbose assistant
    /// replies while preserving `\n\n**Sources:**` citations intact.
    pub fn add_turn(
        &self,
        key: ContextKey,
        message_id: Option<u64>,
        author_id: u64,
        user_prompt: &str,
        assistant_response: &str,
        max_turns: u32,
    ) {
        if max_turns == 0 {
            return;
        }

        let compressed_assistant = self.compress_assistant_reply(assistant_response);
        let mut map = self.buffers.write();
        let now = Instant::now();

        // Enforce hard LRU session cap (`MAX_ACTIVE_SESSIONS = 1,000`)
        if !map.contains_key(&key) && map.len() >= MAX_ACTIVE_SESSIONS {
            // Sweep expired first; if still at capacity, evict oldest entry at index 0
            let ttl = self.default_ttl;
            map.retain(|_, v| now.duration_since(v.last_active) <= ttl);
            if map.len() >= MAX_ACTIVE_SESSIONS {
                map.shift_remove_index(0);
            }
        }

        // Move or insert at the back of IndexMap to maintain LRU order
        let mut buf = map.shift_remove(&key).unwrap_or_else(|| SessionBuffer {
            last_active: now,
            turns: Vec::new(),
        });
        buf.last_active = now;
        buf.turns.push(TurnEntry {
            message_id,
            author_id,
            user_prompt: user_prompt.trim().to_owned(),
            assistant_response: compressed_assistant,
        });

        let limit = (max_turns.min(50)) as usize;
        if buf.turns.len() > limit {
            let excess = buf.turns.len().saturating_sub(limit);
            buf.turns.drain(0..excess);
        }

        map.insert(key, buf);
    }

    fn compress_assistant_reply(&self, text: &str) -> String {
        let trimmed = text.trim();
        if char_count(trimmed) <= self.max_assistant_chars {
            return trimmed.to_owned();
        }

        const SOURCES_MARKER: &str = "\n\n**Sources:**";
        const TRUNC_NOTE: &str = "... [truncated for brevity]";

        if let Some((body, sources_part)) = trimmed.split_once(SOURCES_MARKER) {
            let sources_suffix = format!("{SOURCES_MARKER}{sources_part}");
            let sources_chars = char_count(&sources_suffix);
            let budget_for_body = self
                .max_assistant_chars
                .saturating_sub(sources_chars)
                .saturating_sub(30)
                .max(100);

            if char_count(body) > budget_for_body {
                let short_body = truncate_chars(body, budget_for_body).trim_end();
                return format!("{short_body}{TRUNC_NOTE}{sources_suffix}");
            }
            return format!("{body}{sources_suffix}");
        }

        let short = truncate_chars(trimmed, self.max_assistant_chars).trim_end();
        format!("{short}{TRUNC_NOTE}")
    }

    /// Clears conversation memory for a specific `ContextKey`.
    pub fn clear(&self, key: ContextKey) -> bool {
        self.buffers.write().shift_remove(&key).is_some()
    }

    /// Clears all conversation buffers across the entire process.
    pub fn clear_all(&self) {
        self.buffers.write().clear();
    }

    /// Discord ToS `MESSAGE_DELETE` handler: immediately evicts any turn triggered by `message_id`.
    pub fn evict_message(&self, channel_id: u64, message_id: u64) -> usize {
        self.evict_messages_bulk(channel_id, &[message_id])
    }

    /// Discord ToS `MESSAGE_DELETE_BULK` handler: evicts all turns matching `message_ids`.
    pub fn evict_messages_bulk(&self, channel_id: u64, message_ids: &[u64]) -> usize {
        if message_ids.is_empty() {
            return 0;
        }
        let mut removed = 0usize;
        let mut map = self.buffers.write();
        for (key, buf) in map.iter_mut() {
            let matches_channel = match key {
                ContextKey::GuildChannel {
                    channel_id: cid, ..
                } => *cid == channel_id,
                ContextKey::DirectMessage { .. } => true,
            };
            if matches_channel {
                let before = buf.turns.len();
                buf.turns.retain(|t| {
                    t.message_id
                        .is_none_or(|mid| !message_ids.contains(&mid))
                });
                removed = removed.saturating_add(before.saturating_sub(buf.turns.len()));
            }
        }
        map.retain(|_, buf| !buf.turns.is_empty());
        removed
    }

    /// Discord ToS `CHANNEL_DELETE` / `THREAD_DELETE` handler.
    pub fn clear_channel(&self, channel_id: u64) {
        let mut map = self.buffers.write();
        map.retain(|k, _| match k {
            ContextKey::GuildChannel {
                channel_id: cid, ..
            } => *cid != channel_id,
            ContextKey::DirectMessage { .. } => true,
        });
    }

    /// Discord ToS `GUILD_DELETE` handler: purges all channel buffers for `guild_id`.
    pub fn clear_guild(&self, guild_id: u64) {
        let mut map = self.buffers.write();
        map.retain(|k, _| match k {
            ContextKey::GuildChannel { guild_id: gid, .. } => *gid != guild_id,
            ContextKey::DirectMessage { .. } => true,
        });
    }

    /// Self-service `/forget_me` handler: purges a user's DM buffer and any guild turns authored by `user_id`.
    pub fn clear_user_all(&self, user_id: u64) -> usize {
        let mut removed = 0usize;
        let mut map = self.buffers.write();
        if let Some(dm_buf) = map.shift_remove(&ContextKey::DirectMessage { user_id }) {
            removed = removed.saturating_add(dm_buf.turns.len());
        }
        for (_, buf) in map.iter_mut() {
            let before = buf.turns.len();
            buf.turns.retain(|t| t.author_id != user_id);
            removed = removed.saturating_add(before.saturating_sub(buf.turns.len()));
        }
        map.retain(|_, buf| !buf.turns.is_empty());
        removed
    }

    pub fn stats(&self) -> MemoryStats {
        let map = self.buffers.read();
        let total_sessions = map.len();
        let mut dm_sessions = 0usize;
        let mut total_stored_messages = 0usize;

        for (k, buf) in map.iter() {
            if matches!(k, ContextKey::DirectMessage { .. }) {
                dm_sessions = dm_sessions.saturating_add(1);
            }
            total_stored_messages =
                total_stored_messages.saturating_add(buf.turns.len().saturating_mul(2));
        }

        MemoryStats {
            total_sessions,
            dm_sessions,
            guild_sessions: total_sessions.saturating_sub(dm_sessions),
            total_stored_messages,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_isolation_and_sources_preservation() {
        let mem = ConversationMemory::new(30, 300);
        let key = ContextKey::guild_channel(10, 20);
        let long_reply = format!(
            "{}\n\n**Sources:**\n• [Rust Docs](<https://doc.rust-lang.org>)",
            "A".repeat(600)
        );
        mem.add_turn(key, Some(9001), 42, "Tell me about Rust", &long_reply, 6);

        let hist = mem.get_history(key, 6, None);
        assert_eq!(hist.len(), 2);
        assert!(hist[1].content.contains("... [truncated for brevity]"));
        assert!(hist[1]
            .content
            .ends_with("\n\n**Sources:**\n• [Rust Docs](<https://doc.rust-lang.org>)"));

        // Verify MESSAGE_DELETE eviction
        assert_eq!(mem.evict_message(20, 9001), 1);
        assert!(mem.get_history(key, 6, None).is_empty());
    }
}
