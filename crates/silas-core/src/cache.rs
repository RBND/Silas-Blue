//! In-memory bounded LRU message ring buffer for edit/delete diffing and audit logging.

use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// Default capacity for the LRU message ring buffer (10,000 entries).
pub const DEFAULT_CACHE_CAPACITY: usize = 10_000;

/// In-memory representation of a cached Discord message for change diffing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedMessage {
    pub id: u64,
    pub guild_id: Option<u64>,
    pub channel_id: u64,
    pub author_id: u64,
    pub author_tag: String,
    pub author_avatar: Option<String>,
    pub content: String,
    pub attachments: Vec<String>,
    pub timestamp: i64,
}

/// Thread-safe in-memory LRU ring buffer of recent messages.
///
/// Under high throughput, old entries are automatically pruned when capacity is reached.
#[derive(Debug, Clone)]
pub struct MessageRingBuffer {
    capacity: usize,
    entries: Arc<RwLock<IndexMap<u64, CachedMessage>>>,
}

impl MessageRingBuffer {
    /// Creates a new `MessageRingBuffer` with a fixed capacity.
    /// If capacity is 0, defaults to `DEFAULT_CACHE_CAPACITY`.
    pub fn new(capacity: usize) -> Self {
        let cap = if capacity == 0 {
            DEFAULT_CACHE_CAPACITY
        } else {
            capacity
        };
        Self {
            capacity: cap,
            entries: Arc::new(RwLock::new(IndexMap::with_capacity(cap.min(1024)))),
        }
    }

    /// Returns the maximum capacity of the ring buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the current number of cached messages.
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    /// Returns `true` if the ring buffer contains no messages.
    pub fn is_empty(&self) -> bool {
        self.entries.read().is_empty()
    }

    /// Inserts a message into the cache. If the message ID already exists,
    /// it updates the entry and moves it to the most recently used (MRU) position.
    /// If the cache is full, the least recently used (LRU) message is evicted.
    pub fn insert(&self, message: CachedMessage) {
        let mut guard = self.entries.write();
        let id = message.id;

        // If the key already exists, remove it so inserting puts it at the back (MRU)
        let _ = guard.shift_remove(&id);

        while guard.len() >= self.capacity {
            let _ = guard.shift_remove_index(0);
        }

        let _ = guard.insert(id, message);
    }

    /// Retrieves a cloned copy of the cached message by ID if present.
    pub fn get(&self, id: u64) -> Option<CachedMessage> {
        self.entries.read().get(&id).cloned()
    }

    /// Removes and returns a cached message by ID if present.
    pub fn delete(&self, id: u64) -> Option<CachedMessage> {
        self.entries.write().shift_remove(&id)
    }

    /// Clears all messages from the ring buffer.
    pub fn clear(&self) {
        self.entries.write().clear();
    }
}

impl Default for MessageRingBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_CACHE_CAPACITY)
    }
}

/// Default capacity for the member roles cache (10,000 guild-user pairs).
pub const DEFAULT_MEMBER_ROLE_CACHE_CAPACITY: usize = 10_000;

type MemberRoleMap = IndexMap<(u64, u64), Vec<u64>>;

/// Thread-safe in-memory cache mapping `(guild_id, user_id)` to known role IDs.
///
/// Prevents false "all roles added" diffing bugs when Discord emits `GuildMemberUpdate`
/// for members not present in Serenity's internal cache.
#[derive(Debug, Clone)]
pub struct MemberRoleCache {
    capacity: usize,
    entries: Arc<RwLock<MemberRoleMap>>,
}

impl MemberRoleCache {
    /// Creates a new `MemberRoleCache` with a fixed capacity.
    pub fn new(capacity: usize) -> Self {
        let cap = if capacity == 0 {
            DEFAULT_MEMBER_ROLE_CACHE_CAPACITY
        } else {
            capacity
        };
        Self {
            capacity: cap,
            entries: Arc::new(RwLock::new(IndexMap::with_capacity(cap.min(1024)))),
        }
    }

    /// Retrieves a cloned list of role IDs for `(guild_id, user_id)` if cached.
    pub fn get(&self, guild_id: u64, user_id: u64) -> Option<Vec<u64>> {
        self.entries.read().get(&(guild_id, user_id)).cloned()
    }

    /// Inserts or updates the known role IDs for `(guild_id, user_id)`.
    pub fn insert(&self, guild_id: u64, user_id: u64, roles: Vec<u64>) {
        let mut guard = self.entries.write();
        let key = (guild_id, user_id);
        let _ = guard.shift_remove(&key);
        while guard.len() >= self.capacity {
            let _ = guard.shift_remove_index(0);
        }
        let _ = guard.insert(key, roles);
    }

    /// Removes a member from the cache (e.g. on member leave/kick/ban).
    pub fn remove(&self, guild_id: u64, user_id: u64) -> Option<Vec<u64>> {
        self.entries.write().shift_remove(&(guild_id, user_id))
    }
}

impl Default for MemberRoleCache {
    fn default() -> Self {
        Self::new(DEFAULT_MEMBER_ROLE_CACHE_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_message(id: u64, content: &str) -> CachedMessage {
        CachedMessage {
            id,
            guild_id: Some(1001),
            channel_id: 2001,
            author_id: 3001,
            author_tag: "Alice#0001".to_string(),
            author_avatar: None,
            content: content.to_string(),
            attachments: Vec::new(),
            timestamp: 1_700_000_000 + id as i64,
        }
    }

    #[test]
    fn test_ring_buffer_insert_get_delete() {
        let buffer = MessageRingBuffer::new(3);
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());

        buffer.insert(dummy_message(1, "First"));
        buffer.insert(dummy_message(2, "Second"));

        assert_eq!(buffer.len(), 2);
        assert!(!buffer.is_empty());

        let msg1 = buffer.get(1);
        assert!(msg1.is_some());
        if let Some(m) = msg1 {
            assert_eq!(m.content, "First");
        }

        let deleted = buffer.delete(1);
        assert!(deleted.is_some());
        assert_eq!(buffer.len(), 1);
        assert!(buffer.get(1).is_none());
    }

    #[test]
    fn test_ring_buffer_lru_eviction() {
        let buffer = MessageRingBuffer::new(3);

        buffer.insert(dummy_message(1, "Message 1"));
        buffer.insert(dummy_message(2, "Message 2"));
        buffer.insert(dummy_message(3, "Message 3"));
        assert_eq!(buffer.len(), 3);

        // Inserting 4th message should evict message 1 (LRU)
        buffer.insert(dummy_message(4, "Message 4"));
        assert_eq!(buffer.len(), 3);
        assert!(buffer.get(1).is_none());
        assert!(buffer.get(2).is_some());
        assert!(buffer.get(3).is_some());
        assert!(buffer.get(4).is_some());

        // Re-inserting message 2 updates its position to MRU
        buffer.insert(dummy_message(2, "Message 2 updated"));
        // Now inserting message 5 should evict message 3 (since 2 was renewed)
        buffer.insert(dummy_message(5, "Message 5"));
        assert_eq!(buffer.len(), 3);
        assert!(buffer.get(3).is_none());
        assert!(buffer.get(2).is_some());
        assert!(buffer.get(4).is_some());
        assert!(buffer.get(5).is_some());
    }
}
