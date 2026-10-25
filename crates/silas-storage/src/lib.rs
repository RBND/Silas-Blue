//! Unified SQLite `STRICT` storage engine (`silas.db`) with lock-free in-memory
//! `ArcSwap<AppSettings>` and `DashMap<u64, Arc<GuildConfig>>` hot caches,
//! serialized single-writer WAL transactions, redacted backup exports, and Discord-ToS erasure.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use hmac::{Hmac, Mac};
use parking_lot::Mutex;
use rand::RngCore;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use tracing::{debug, info};

use silas_core::config::{
    AppSettings, AppSettingsPatch, GuildConfig, GuildConfigPatch, OveragePolicy, TokenQuotaConfig,
    DEFAULT_SYSTEM_PROMPT,
};
use silas_core::reddit::{clean_subreddit_name, NewRedditSubscription, RedditSubscription};

pub const CURRENT_SCHEMA_VERSION: u32 = 3;
const RETENTION_DAYS: i64 = 35;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Database schema version {db_version} is newer than supported version {max_supported}; rollback or upgrade binary")]
    NewerSchemaVersion { db_version: u32, max_supported: u32 },
    #[error("Background task join error: {0}")]
    Join(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateStateRecord {
    pub last_check_epoch: Option<i64>,
    pub latest_version: Option<String>,
    pub latest_tag: Option<String>,
    pub release_notes: Option<String>,
    pub release_url: Option<String>,
    pub update_available: bool,
}

#[derive(Debug, Clone)]
pub struct TokenUsageEntry {
    pub guild_id: u64,
    pub user_id: u64,
    pub total_tokens: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub model: String,
    pub recorded_at: i64,
}

/// Moderation infraction case record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModCase {
    pub id: i64,
    pub guild_id: u64,
    pub case_number: i64,
    pub user_id: u64,
    pub moderator_id: u64,
    pub action_type: String,
    pub reason: String,
    pub duration_seconds: Option<i64>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub active: bool,
}

/// Parameters for creating a new moderation case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewModCase {
    pub guild_id: u64,
    pub user_id: u64,
    pub moderator_id: u64,
    pub action_type: String,
    pub reason: String,
    pub duration_seconds: Option<i64>,
    pub created_at: Option<i64>,
    pub expires_at: Option<i64>,
}

/// Moderation strike entry with decay tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModStrike {
    pub id: i64,
    pub guild_id: u64,
    pub user_id: u64,
    pub case_id: Option<i64>,
    pub strike_count: i64,
    pub reason: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub decayed: bool,
}

/// Parameters for adding a strike to a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewModStrike {
    pub guild_id: u64,
    pub user_id: u64,
    pub case_id: Option<i64>,
    pub strike_count: i64,
    pub reason: String,
    pub created_at: Option<i64>,
    pub expires_at: Option<i64>,
}

/// A single step in an automod punishment ladder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PunishmentStep {
    pub strikes: i64,
    pub action: String,
    pub duration_seconds: Option<i64>,
}

/// Comprehensive configuration for a server's automod engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomodConfig {
    pub guild_id: u64,
    pub enabled: bool,
    pub anti_spam_enabled: bool,
    pub anti_spam_max_messages: i64,
    pub anti_spam_interval_seconds: i64,
    pub anti_invites_enabled: bool,
    pub anti_links_enabled: bool,
    pub anti_caps_enabled: bool,
    pub anti_caps_min_chars: i64,
    pub anti_caps_percent: i64,
    pub anti_mention_max: i64,
    pub blocked_words: Vec<String>,
    pub whitelisted_domains: Vec<String>,
    pub ignored_channels: Vec<u64>,
    pub ignored_roles: Vec<u64>,
    pub strike_weights: HashMap<String, i64>,
    pub strike_decay_hours: i64,
    pub punishment_ladder: Vec<PunishmentStep>,
    pub ai_moderation_enabled: bool,
}

impl AutomodConfig {
    pub fn default_for_guild(guild_id: u64) -> Self {
        let mut weights = HashMap::new();
        let _ = weights.insert("spam".to_string(), 2);
        let _ = weights.insert("invite".to_string(), 3);
        let _ = weights.insert("bad_word".to_string(), 2);
        let _ = weights.insert("caps".to_string(), 1);
        let _ = weights.insert("mention".to_string(), 3);

        Self {
            guild_id,
            enabled: true,
            anti_spam_enabled: true,
            anti_spam_max_messages: 5,
            anti_spam_interval_seconds: 5,
            anti_invites_enabled: true,
            anti_links_enabled: false,
            anti_caps_enabled: false,
            anti_caps_min_chars: 10,
            anti_caps_percent: 70,
            anti_mention_max: 5,
            blocked_words: Vec::new(),
            whitelisted_domains: Vec::new(),
            ignored_channels: Vec::new(),
            ignored_roles: Vec::new(),
            strike_weights: weights,
            strike_decay_hours: 168,
            punishment_ladder: vec![
                PunishmentStep {
                    strikes: 2,
                    action: "warn".to_string(),
                    duration_seconds: None,
                },
                PunishmentStep {
                    strikes: 4,
                    action: "timeout".to_string(),
                    duration_seconds: Some(3600),
                },
                PunishmentStep {
                    strikes: 8,
                    action: "kick".to_string(),
                    duration_seconds: None,
                },
                PunishmentStep {
                    strikes: 10,
                    action: "ban".to_string(),
                    duration_seconds: None,
                },
            ],
            ai_moderation_enabled: false,
        }
    }
}

/// Multi-channel event logging routing and filtering configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub guild_id: u64,
    pub unified_channel_id: Option<u64>,
    pub mod_channel_id: Option<u64>,
    pub message_channel_id: Option<u64>,
    pub member_channel_id: Option<u64>,
    pub server_channel_id: Option<u64>,
    pub voice_channel_id: Option<u64>,
    pub ignored_channels: Vec<u64>,
    pub ignored_roles: Vec<u64>,
    pub log_bots: bool,
    pub embed_color: u32,
}

impl LoggingConfig {
    pub fn default_for_guild(guild_id: u64) -> Self {
        Self {
            guild_id,
            unified_channel_id: None,
            mod_channel_id: None,
            message_channel_id: None,
            member_channel_id: None,
            server_channel_id: None,
            voice_channel_id: None,
            ignored_channels: Vec::new(),
            ignored_roles: Vec::new(),
            log_bots: true,
            embed_color: 3553599,
        }
    }
}

/// Greetings, farewells, and autorole configuration for a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GreetingsConfig {
    pub guild_id: u64,
    pub welcome_channel_id: Option<u64>,
    pub welcome_message: String,
    pub welcome_embed_json: String,
    pub welcome_dm_enabled: bool,
    pub welcome_dm_message: String,
    pub farewell_channel_id: Option<u64>,
    pub farewell_message: String,
    pub farewell_embed_json: String,
    pub member_autorole_ids: Vec<u64>,
    pub bot_autorole_ids: Vec<u64>,
    pub autorole_delay_seconds: i64,
}

impl GreetingsConfig {
    pub fn default_for_guild(guild_id: u64) -> Self {
        Self {
            guild_id,
            welcome_channel_id: None,
            welcome_message: String::new(),
            welcome_embed_json: String::new(),
            welcome_dm_enabled: false,
            welcome_dm_message: String::new(),
            farewell_channel_id: None,
            farewell_message: String::new(),
            farewell_embed_json: String::new(),
            member_autorole_ids: Vec::new(),
            bot_autorole_ids: Vec::new(),
            autorole_delay_seconds: 0,
        }
    }
}

/// A reaction role group bound to a specific Discord message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionRoleGroup {
    pub id: i64,
    pub guild_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
    pub group_type: String,
    pub required_role_id: Option<u64>,
    pub component_type: String,
    pub entries: Vec<ReactionRoleEntry>,
}

/// An emoji or button entry mapped to a role within a reaction role group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionRoleEntry {
    pub id: i64,
    pub group_id: i64,
    pub emoji_or_custom_id: String,
    pub role_id: u64,
    pub button_label: Option<String>,
    pub button_style: Option<String>,
}

/// Parameters for creating a new reaction role group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewReactionRoleGroup {
    pub guild_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
    pub group_type: String,
    pub required_role_id: Option<u64>,
    pub component_type: String,
}

/// Parameters for adding a reaction role entry to an existing group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewReactionRoleEntry {
    pub group_id: i64,
    pub emoji_or_custom_id: String,
    pub role_id: u64,
    pub button_label: Option<String>,
    pub button_style: Option<String>,
}

/// Custom trigger or tag definition with execution options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomTrigger {
    pub id: i64,
    pub guild_id: u64,
    pub trigger_name: String,
    pub match_mode: String,
    pub response_template: String,
    pub cooldown_seconds: i64,
    pub allowed_roles: Vec<u64>,
    pub ignored_channels: Vec<u64>,
    pub delete_trigger_message: bool,
    pub reply_in_dm: bool,
    pub enabled: bool,
}

/// Parameters for creating or updating a custom trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCustomTrigger {
    pub guild_id: u64,
    pub trigger_name: String,
    pub match_mode: String,
    pub response_template: String,
    pub cooldown_seconds: i64,
    pub allowed_roles: Vec<u64>,
    pub ignored_channels: Vec<u64>,
    pub delete_trigger_message: bool,
    pub reply_in_dm: bool,
    pub enabled: bool,
}

/// High-performance unified SQLite storage manager.
/// - Hot reads (`get_app_settings`, `get_guild_config`) hit lock-free RAM (`ArcSwap` / `DashMap`).
/// - All writes are serialized through a single persistent WAL writer connection (`writer`).
/// - Analytical reads (`get_user_token_usage`) execute via `tokio::task::spawn_blocking`.
#[derive(Clone)]
pub struct Storage {
    db_path: PathBuf,
    writer: Arc<Mutex<Connection>>,
    app_cache: Arc<ArcSwap<AppSettings>>,
    guild_cache: Arc<DashMap<u64, Arc<GuildConfig>>>,
}

impl Storage {
    /// Opens or creates the unified SQLite database at `db_path`, applies WAL pragmas,
    /// verifies schema migrations, and warms the in-memory hot caches.
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = db_path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
                set_secure_dir_permissions(parent);
            }
        }

        let conn = Connection::open(&path)?;
        set_secure_file_permissions(&path);
        apply_connection_pragmas(&conn)?;
        initialize_schema(&conn)?;

        let app_settings = load_app_settings_from_conn(&conn)?;
        let all_guilds = load_all_guild_configs_from_conn(&conn)?;

        let guild_cache = DashMap::with_capacity(all_guilds.len().max(16));
        for (gid, cfg) in all_guilds {
            guild_cache.insert(gid, Arc::new(cfg));
        }

        debug!(
            path = %path.display(),
            cached_guilds = guild_cache.len(),
            "Initialized unified SQLite storage in WAL mode"
        );

        Ok(Self {
            db_path: path,
            writer: Arc::new(Mutex::new(conn)),
            app_cache: Arc::new(ArcSwap::from_pointee(app_settings)),
            guild_cache: Arc::new(guild_cache),
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Lock-free, zero-I/O read of the global `AppSettings`.
    #[inline]
    pub fn get_app_settings(&self) -> Arc<AppSettings> {
        self.app_cache.load_full()
    }

    /// Atomically applies `patch` to `AppSettings`, persists to SQLite, and updates the `ArcSwap` cache.
    pub fn update_app_settings(
        &self,
        patch: AppSettingsPatch,
    ) -> Result<Arc<AppSettings>, StorageError> {
        let conn = self.writer.lock();
        let mut current = load_app_settings_from_conn(&conn)?;
        current
            .apply_patch(patch)
            .map_err(StorageError::Validation)?;
        save_app_settings_to_conn(&conn, &current)?;
        let arc_cfg = Arc::new(current);
        self.app_cache.store(Arc::clone(&arc_cfg));
        Ok(arc_cfg)
    }

    /// Async wrapper for `update_app_settings` that executes SQLite I/O on Tokio's blocking pool.
    pub async fn update_app_settings_async(
        &self,
        patch: AppSettingsPatch,
    ) -> Result<Arc<AppSettings>, StorageError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.update_app_settings(patch))
            .await
            .map_err(|e| StorageError::Join(e.to_string()))?
    }

    /// Replaces the entire `AppSettings` (used by `silas-migrate` and initial bootstrap).
    pub fn replace_app_settings(&self, mut settings: AppSettings) -> Result<Arc<AppSettings>, StorageError> {
        if settings.web_secret_key.len() < 32 {
            let mut key = vec![0u8; 32];
            rand::thread_rng().fill_bytes(&mut key);
            settings.web_secret_key = key;
        }
        let conn = self.writer.lock();
        save_app_settings_to_conn(&conn, &settings)?;
        let arc_cfg = Arc::new(settings);
        self.app_cache.store(Arc::clone(&arc_cfg));
        Ok(arc_cfg)
    }

    /// Increments `session_epoch` to immediately revoke all active web sessions.
    pub fn revoke_all_sessions(&self) -> Result<i64, StorageError> {
        let conn = self.writer.lock();
        let mut current = load_app_settings_from_conn(&conn)?;
        current.session_epoch = current.session_epoch.saturating_add(1);
        save_app_settings_to_conn(&conn, &current)?;
        let new_epoch = current.session_epoch;
        self.app_cache.store(Arc::new(current));
        Ok(new_epoch)
    }

    /// Zero-I/O hot read of a server's `GuildConfig` (with pre-compiled permission sets).
    /// If the guild has no persisted config yet, returns a default `GuildConfig` cached in memory.
    #[inline]
    pub fn get_guild_config(&self, guild_id: u64) -> Arc<GuildConfig> {
        if let Some(existing) = self.guild_cache.get(&guild_id) {
            return Arc::clone(existing.value());
        }
        let default_arc = Arc::new(GuildConfig::default());
        self.guild_cache
            .entry(guild_id)
            .or_insert_with(|| Arc::clone(&default_arc));
        default_arc
    }

    /// Returns a virtual `GuildConfig` tailored for Direct Messages (DMs) without writing to disk.
    pub fn get_dm_config(&self) -> GuildConfig {
        let app = self.get_app_settings();
        let mut dm_cfg = GuildConfig::default();
        if !app.dm_default_model.trim().is_empty() {
            dm_cfg.default_model = app.dm_default_model.clone();
        }
        dm_cfg.memory_turns = app.dm_memory_turns;
        dm_cfg.memory_ttl_minutes = app.dm_memory_ttl_minutes;
        dm_cfg.sanitize_and_compile();
        dm_cfg
    }

    /// Non-destructively merges a `GuildConfigPatch` inside a serialized SQLite transaction
    /// and updates the in-memory `DashMap` cache.
    pub fn patch_guild_config(
        &self,
        guild_id: u64,
        patch: GuildConfigPatch,
    ) -> Result<Arc<GuildConfig>, StorageError> {
        let conn = self.writer.lock();
        let mut cfg = load_guild_config_from_conn(&conn, guild_id)?
            .unwrap_or_else(GuildConfig::default);
        cfg.apply_patch(patch);
        save_guild_config_to_conn(&conn, guild_id, &cfg)?;
        let arc_cfg = Arc::new(cfg);
        self.guild_cache.insert(guild_id, Arc::clone(&arc_cfg));
        Ok(arc_cfg)
    }

    /// Async wrapper for `patch_guild_config`.
    pub async fn patch_guild_config_async(
        &self,
        guild_id: u64,
        patch: GuildConfigPatch,
    ) -> Result<Arc<GuildConfig>, StorageError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.patch_guild_config(guild_id, patch))
            .await
            .map_err(|e| StorageError::Join(e.to_string()))?
    }

    /// Upserts a complete `GuildConfig` (used by `silas-migrate` and full config saves).
    pub fn upsert_guild_config(
        &self,
        guild_id: u64,
        mut cfg: GuildConfig,
    ) -> Result<Arc<GuildConfig>, StorageError> {
        cfg.sanitize_and_compile();
        let conn = self.writer.lock();
        save_guild_config_to_conn(&conn, guild_id, &cfg)?;
        let arc_cfg = Arc::new(cfg);
        self.guild_cache.insert(guild_id, Arc::clone(&arc_cfg));
        Ok(arc_cfg)
    }

    /// Returns a list of all guild IDs currently cached in `Storage`.
    pub fn list_guild_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.guild_cache.iter().map(|e| *e.key()).collect();
        ids.sort_unstable();
        ids
    }

    /// Resets token usage records for `guild_id` recorded at or after `window_start`.
    pub fn reset_guild_token_usage(
        &self,
        guild_id: u64,
        window_start: DateTime<Utc>,
    ) -> Result<usize, StorageError> {
        let conn = self.writer.lock();
        let deleted = conn.execute(
            "DELETE FROM token_usage_ledger WHERE guild_id = ?1 AND recorded_at >= ?2",
            params![guild_id as i64, window_start.timestamp()],
        )?;
        Ok(deleted)
    }

    /// Discord ToS `GUILD_DELETE` compliance: deletes guild configs, tokens, cases, strikes, automod, logging, greetings, reaction roles, and triggers.
    pub fn purge_guild_data(&self, guild_id: u64) -> Result<(), StorageError> {
        self.guild_cache.remove(&guild_id);
        let gid_i64 = guild_id as i64;
        let conn = self.writer.lock();
        conn.execute("DELETE FROM guild_configs WHERE guild_id = ?1", params![gid_i64])?;
        conn.execute(
            "DELETE FROM token_usage_ledger WHERE guild_id = ?1",
            params![gid_i64],
        )?;
        conn.execute("DELETE FROM mod_cases WHERE guild_id = ?1", params![gid_i64])?;
        conn.execute("DELETE FROM mod_strikes WHERE guild_id = ?1", params![gid_i64])?;
        conn.execute("DELETE FROM automod_configs WHERE guild_id = ?1", params![gid_i64])?;
        conn.execute("DELETE FROM logging_configs WHERE guild_id = ?1", params![gid_i64])?;
        conn.execute("DELETE FROM greetings_configs WHERE guild_id = ?1", params![gid_i64])?;
        conn.execute("DELETE FROM reaction_role_groups WHERE guild_id = ?1", params![gid_i64])?;
        conn.execute("DELETE FROM custom_triggers WHERE guild_id = ?1", params![gid_i64])?;
        Ok(())
    }

    /// Records token consumption for a completed prompt turn.
    pub fn record_token_usage(&self, entry: &TokenUsageEntry) -> Result<(), StorageError> {
        if entry.total_tokens <= 0 {
            return Ok(());
        }
        let conn = self.writer.lock();
        conn.execute(
            "INSERT INTO token_usage_ledger
             (guild_id, user_id, total_tokens, prompt_tokens, completion_tokens, model, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.guild_id as i64,
                entry.user_id as i64,
                entry.total_tokens,
                entry.prompt_tokens.max(0),
                entry.completion_tokens.max(0),
                entry.model,
                entry.recorded_at,
            ],
        )?;
        Ok(())
    }

    /// Async wrapper for `record_token_usage`.
    pub async fn record_token_usage_async(&self, entry: TokenUsageEntry) -> Result<(), StorageError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.record_token_usage(&entry))
            .await
            .map_err(|e| StorageError::Join(e.to_string()))?
    }

    /// Queries total tokens used by `user_id` (or their HMAC pseudonym if `/forget_me` was run)
    /// within `[window_start, window_end)`.
    pub fn get_user_token_usage(
        &self,
        guild_id: u64,
        user_id: u64,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
    ) -> Result<i64, StorageError> {
        let start_ts = window_start.timestamp();
        let end_ts = window_end.timestamp();
        let pseudo_id = self.compute_user_pseudonym(guild_id, user_id);

        // Open a lightweight read-only WAL connection for concurrent analytical queries
        let ro_conn = if self.db_path == Path::new(":memory:") {
            let conn = self.writer.lock();
            return query_usage_sum(&conn, guild_id as i64, user_id as i64, pseudo_id as i64, start_ts, end_ts);
        } else {
            Connection::open_with_flags(
                &self.db_path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?
        };
        let _ = ro_conn.busy_timeout(std::time::Duration::from_secs(5));
        query_usage_sum(
            &ro_conn,
            guild_id as i64,
            user_id as i64,
            pseudo_id as i64,
            start_ts,
            end_ts,
        )
    }

    /// Async wrapper for `get_user_token_usage` executed on Tokio's blocking pool.
    pub async fn get_user_token_usage_async(
        &self,
        guild_id: u64,
        user_id: u64,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
    ) -> Result<i64, StorageError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            this.get_user_token_usage(guild_id, user_id, window_start, window_end)
        })
        .await
        .map_err(|e| StorageError::Join(e.to_string()))?
    }

    /// Computes a deterministic HMAC-SHA256 pseudonym for `(guild_id, user_id)` with the MSB set
    /// so `/forget_me` erases the user's raw Discord Snowflake from `token_usage_ledger` while
    /// preventing quota-reset abuse within the active cycle.
    pub fn compute_user_pseudonym(&self, guild_id: u64, user_id: u64) -> u64 {
        let app = self.get_app_settings();
        type HmacSha256 = Hmac<Sha256>;
        let Ok(mut mac) = HmacSha256::new_from_slice(&app.web_secret_key) else {
            return user_id | (1u64 << 63);
        };
        mac.update(&guild_id.to_le_bytes());
        mac.update(&user_id.to_le_bytes());
        let bytes = mac.finalize().into_bytes();
        let mut buf = [0u8; 8];
        if let Some(slice) = bytes.get(..8) {
            buf.copy_from_slice(slice);
        }
        // Set MSB (`1 << 63`) so pseudonyms never collide with positive Discord Snowflakes
        u64::from_le_bytes(buf) | (1u64 << 63)
    }

    /// Discord ToS `/forget_me` handler: replaces the user's raw `user_id` Snowflake in
    /// `token_usage_ledger` with their irreversible HMAC pseudonym (preserving only current-cycle
    /// quota fairness without storing their Discord ID).
    pub fn anonymize_user_ledger(&self, user_id: u64) -> Result<usize, StorageError> {
        let conn = self.writer.lock();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT guild_id FROM token_usage_ledger WHERE user_id = ?1",
        )?;
        let guild_ids: Vec<i64> = stmt
            .query_map(params![user_id as i64], |row| row.get(0))?
            .filter_map(Result::ok)
            .collect();
        drop(stmt);

        let mut total_updated = 0usize;
        for gid in guild_ids {
            let pseudo = self.compute_user_pseudonym(gid as u64, user_id) as i64;
            let count = conn.execute(
                "UPDATE token_usage_ledger SET user_id = ?1 WHERE guild_id = ?2 AND user_id = ?3",
                params![pseudo, gid, user_id as i64],
            )?;
            total_updated = total_updated.saturating_add(count);
        }
        Ok(total_updated)
    }

    /// Prunes `token_usage_ledger` rows older than `RETENTION_DAYS` (35 days) and runs a passive WAL checkpoint.
    pub fn prune_old_token_records(&self) -> Result<usize, StorageError> {
        let cutoff = Utc::now().timestamp().saturating_sub(RETENTION_DAYS * 86_400);
        let conn = self.writer.lock();
        let deleted = conn.execute(
            "DELETE FROM token_usage_ledger WHERE recorded_at < ?1",
            params![cutoff],
        )?;
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        if deleted > 0 {
            info!(deleted_rows = deleted, "Pruned expired 35-day token usage records");
        }
        Ok(deleted)
    }

    /// Reads the cached `update_state` singleton row.
    pub fn get_update_state(&self) -> Result<UpdateStateRecord, StorageError> {
        let conn = self.writer.lock();
        let rec = conn
            .query_row(
                "SELECT last_check_epoch, latest_version, latest_tag, release_notes, release_url, update_available
                 FROM update_state WHERE id = 1",
                [],
                |row| {
                    Ok(UpdateStateRecord {
                        last_check_epoch: row.get(0)?,
                        latest_version: row.get(1)?,
                        latest_tag: row.get(2)?,
                        release_notes: row.get(3)?,
                        release_url: row.get(4)?,
                        update_available: row.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .optional()?
            .unwrap_or_default();
        Ok(rec)
    }

    /// Saves the `update_state` singleton row.
    pub fn save_update_state(&self, state: &UpdateStateRecord) -> Result<(), StorageError> {
        let conn = self.writer.lock();
        conn.execute(
            "INSERT INTO update_state (id, last_check_epoch, latest_version, latest_tag, release_notes, release_url, update_available)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                 last_check_epoch = excluded.last_check_epoch,
                 latest_version = excluded.latest_version,
                 latest_tag = excluded.latest_tag,
                 release_notes = excluded.release_notes,
                 release_url = excluded.release_url,
                 update_available = excluded.update_available",
            params![
                state.last_check_epoch,
                state.latest_version,
                state.latest_tag,
                state.release_notes,
                state.release_url,
                i64::from(state.update_available),
            ],
        )?;
        Ok(())
    }

    /// Creates a full online atomic snapshot (`VACUUM INTO`) for internal pre-update rollback.
    /// Never expose this unredacted snapshot over HTTP!
    pub fn create_internal_backup(&self, dest_path: impl AsRef<Path>) -> Result<PathBuf, StorageError> {
        let dest = dest_path.as_ref().to_path_buf();
        if let Some(parent) = dest.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
                set_secure_dir_permissions(parent);
            }
        }
        if dest.exists() {
            let _ = fs::remove_file(&dest);
        }
        let conn = self.writer.lock();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        conn.execute("VACUUM INTO ?1", params![dest.to_string_lossy().as_ref()])?;
        set_secure_file_permissions(&dest);
        Ok(dest)
    }

    /// Creates a **redacted** SQLite snapshot (`CRIT-01`) safe for download via the Web UI:
    /// 1. Runs `VACUUM INTO` to snapshot the database.
    /// 2. Scrubs all secret keys (`discord_token`, `discord_oauth_client_secret`, `web_secret_key`,
    ///    `local_api_key`, `remote_api_key`, `tavily_api_key`, `brave_api_key`).
    /// 3. Runs a second `VACUUM` on the snapshot so zero residual secret bytes remain in B-tree pages.
    pub fn create_redacted_backup(&self, dest_path: impl AsRef<Path>) -> Result<PathBuf, StorageError> {
        let dest = self.create_internal_backup(dest_path)?;
        let snap_conn = Connection::open(&dest)?;
        snap_conn.execute_batch(
            "PRAGMA journal_mode = DELETE;
             PRAGMA secure_delete = ON;
             UPDATE app_settings SET
                 discord_token = '',
                 discord_oauth_client_secret = '',
                 web_secret_key = randomblob(32),
                 local_api_key = '',
                 remote_api_key = '',
                 tavily_api_key = '',
                 brave_api_key = '',
                 reddit_client_secret = '',
                 reddit_webhook_secret = '';
             VACUUM;",
        )?;
        drop(snap_conn);
        set_secure_file_permissions(&dest);
        Ok(dest)
    }

    /// Flushes and truncates the WAL journal cleanly during graceful shutdown.
    pub fn checkpoint_wal(&self) -> Result<(), StorageError> {
        let conn = self.writer.lock();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    /// Creates a new subreddit feed subscription for a guild channel.
    pub fn create_reddit_subscription(
        &self,
        new_sub: &NewRedditSubscription,
        last_seen_post_id: Option<&str>,
    ) -> Result<RedditSubscription, StorageError> {
        let conn = self.writer.lock();
        let sub_name = clean_subreddit_name(&new_sub.subreddit);
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO reddit_subscriptions (
                guild_id, channel_id, subreddit, filter_nsfw, min_score,
                flair_filter, keyword_filter, created_at, last_seen_post_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                new_sub.guild_id as i64,
                new_sub.channel_id as i64,
                sub_name,
                i64::from(new_sub.filter_nsfw),
                new_sub.min_score,
                new_sub.flair_filter,
                new_sub.keyword_filter,
                now,
                last_seen_post_id,
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(RedditSubscription {
            id,
            guild_id: new_sub.guild_id,
            channel_id: new_sub.channel_id,
            subreddit: sub_name,
            filter_nsfw: new_sub.filter_nsfw,
            min_score: new_sub.min_score,
            flair_filter: new_sub.flair_filter.clone(),
            keyword_filter: new_sub.keyword_filter.clone(),
            created_at: now,
            last_seen_post_id: last_seen_post_id.map(|s| s.to_string()),
        })
    }

    /// Deletes a subreddit feed subscription by its unique record ID within a guild.
    pub fn delete_reddit_subscription(
        &self,
        subscription_id: i64,
        guild_id: u64,
    ) -> Result<bool, StorageError> {
        let conn = self.writer.lock();
        let affected = conn.execute(
            "DELETE FROM reddit_subscriptions WHERE id = ?1 AND guild_id = ?2",
            params![subscription_id, guild_id as i64],
        )?;
        if affected > 0 {
            let _ = conn.execute(
                "DELETE FROM reddit_posted_items WHERE subscription_id = ?1",
                params![subscription_id],
            );
        }
        Ok(affected > 0)
    }

    /// Deletes a subreddit feed subscription matching channel and subreddit name.
    pub fn delete_reddit_subscription_by_channel(
        &self,
        guild_id: u64,
        channel_id: u64,
        subreddit: &str,
    ) -> Result<bool, StorageError> {
        let conn = self.writer.lock();
        let sub_name = clean_subreddit_name(subreddit);
        let affected = conn.execute(
            "DELETE FROM reddit_subscriptions WHERE guild_id = ?1 AND channel_id = ?2 AND LOWER(subreddit) = ?3",
            params![guild_id as i64, channel_id as i64, sub_name],
        )?;
        Ok(affected > 0)
    }

    /// Lists all subreddit subscriptions for a specific guild.
    pub fn list_reddit_subscriptions_for_guild(
        &self,
        guild_id: u64,
    ) -> Result<Vec<RedditSubscription>, StorageError> {
        let conn = self.writer.lock();
        let mut stmt = conn.prepare(
            "SELECT id, guild_id, channel_id, subreddit, filter_nsfw, min_score,
                    flair_filter, keyword_filter, created_at, last_seen_post_id
             FROM reddit_subscriptions
             WHERE guild_id = ?1
             ORDER BY subreddit ASC",
        )?;
        let rows = stmt.query_map(params![guild_id as i64], |row| {
            Ok(RedditSubscription {
                id: row.get(0)?,
                guild_id: row.get::<_, i64>(1)? as u64,
                channel_id: row.get::<_, i64>(2)? as u64,
                subreddit: row.get(3)?,
                filter_nsfw: row.get::<_, i64>(4)? != 0,
                min_score: row.get(5)?,
                flair_filter: row.get(6)?,
                keyword_filter: row.get(7)?,
                created_at: row.get(8)?,
                last_seen_post_id: row.get(9)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Lists all subreddit subscriptions across all connected guilds.
    pub fn list_all_reddit_subscriptions(&self) -> Result<Vec<RedditSubscription>, StorageError> {
        let conn = self.writer.lock();
        let mut stmt = conn.prepare(
            "SELECT id, guild_id, channel_id, subreddit, filter_nsfw, min_score,
                    flair_filter, keyword_filter, created_at, last_seen_post_id
             FROM reddit_subscriptions
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RedditSubscription {
                id: row.get(0)?,
                guild_id: row.get::<_, i64>(1)? as u64,
                channel_id: row.get::<_, i64>(2)? as u64,
                subreddit: row.get(3)?,
                filter_nsfw: row.get::<_, i64>(4)? != 0,
                min_score: row.get(5)?,
                flair_filter: row.get(6)?,
                keyword_filter: row.get(7)?,
                created_at: row.get(8)?,
                last_seen_post_id: row.get(9)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Finds all subscriptions for a specific subreddit (case-insensitive).
    pub fn find_subscriptions_for_subreddit(
        &self,
        subreddit: &str,
    ) -> Result<Vec<RedditSubscription>, StorageError> {
        let conn = self.writer.lock();
        let sub_name = clean_subreddit_name(subreddit);
        let mut stmt = conn.prepare(
            "SELECT id, guild_id, channel_id, subreddit, filter_nsfw, min_score,
                    flair_filter, keyword_filter, created_at, last_seen_post_id
             FROM reddit_subscriptions
             WHERE LOWER(subreddit) = ?1",
        )?;
        let rows = stmt.query_map(params![sub_name], |row| {
            Ok(RedditSubscription {
                id: row.get(0)?,
                guild_id: row.get::<_, i64>(1)? as u64,
                channel_id: row.get::<_, i64>(2)? as u64,
                subreddit: row.get(3)?,
                filter_nsfw: row.get::<_, i64>(4)? != 0,
                min_score: row.get(5)?,
                flair_filter: row.get(6)?,
                keyword_filter: row.get(7)?,
                created_at: row.get(8)?,
                last_seen_post_id: row.get(9)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Checks if a post has already been delivered to a given subscription.
    pub fn has_reddit_post_been_delivered(
        &self,
        subscription_id: i64,
        post_id: &str,
    ) -> Result<bool, StorageError> {
        let conn = self.writer.lock();
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM reddit_posted_items WHERE subscription_id = ?1 AND post_id = ?2",
                params![subscription_id, post_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    /// Records that a post was delivered to a subscription and updates the last seen post ID.
    pub fn record_reddit_post_delivered(
        &self,
        subscription_id: i64,
        post_id: &str,
    ) -> Result<(), StorageError> {
        let conn = self.writer.lock();
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT OR IGNORE INTO reddit_posted_items (subscription_id, post_id, posted_at) VALUES (?1, ?2, ?3)",
            params![subscription_id, post_id, now],
        )?;
        conn.execute(
            "UPDATE reddit_subscriptions SET last_seen_post_id = ?1 WHERE id = ?2",
            params![post_id, subscription_id],
        )?;
        Ok(())
    }

    /// Updates only the last seen post ID for a subscription.
    pub fn update_reddit_last_seen(
        &self,
        subscription_id: i64,
        post_id: &str,
    ) -> Result<(), StorageError> {
        let conn = self.writer.lock();
        conn.execute(
            "UPDATE reddit_subscriptions SET last_seen_post_id = ?1 WHERE id = ?2",
            params![post_id, subscription_id],
        )?;
        Ok(())
    }

    /// Prunes delivered post records older than or equal to `retention_days`.
    pub fn prune_old_reddit_posts(&self, retention_days: i64) -> Result<usize, StorageError> {
        let conn = self.writer.lock();
        let cutoff = Utc::now().timestamp() - (retention_days * 86400);
        let count = conn.execute(
            "DELETE FROM reddit_posted_items WHERE posted_at <= ?1",
            params![cutoff],
        )?;
        Ok(count)
    }

    // =========================================================================
    // Moderation Cases & Infractions
    // =========================================================================

    /// Creates a new moderation case with an auto-incremented per-guild case number.
    pub fn create_mod_case(&self, new_case: &NewModCase) -> Result<ModCase, StorageError> {
        let conn = self.writer.lock();
        let next_case_num: i64 = conn.query_row(
            "SELECT COALESCE(MAX(case_number), 0) + 1 FROM mod_cases WHERE guild_id = ?1",
            params![new_case.guild_id as i64],
            |row| row.get(0),
        )?;
        let created_at = new_case.created_at.unwrap_or_else(|| Utc::now().timestamp());
        conn.execute(
            "INSERT INTO mod_cases (
                guild_id, case_number, user_id, moderator_id, action_type,
                reason, duration_seconds, created_at, expires_at, active
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)",
            params![
                new_case.guild_id as i64,
                next_case_num,
                new_case.user_id as i64,
                new_case.moderator_id as i64,
                new_case.action_type,
                new_case.reason,
                new_case.duration_seconds,
                created_at,
                new_case.expires_at,
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(ModCase {
            id,
            guild_id: new_case.guild_id,
            case_number: next_case_num,
            user_id: new_case.user_id,
            moderator_id: new_case.moderator_id,
            action_type: new_case.action_type.clone(),
            reason: new_case.reason.clone(),
            duration_seconds: new_case.duration_seconds,
            created_at,
            expires_at: new_case.expires_at,
            active: true,
        })
    }

    /// Fetches a specific moderation case by guild ID and case number.
    pub fn get_mod_case(
        &self,
        guild_id: u64,
        case_number: i64,
    ) -> Result<Option<ModCase>, StorageError> {
        let conn = self.writer.lock();
        let res = conn
            .query_row(
                "SELECT id, guild_id, case_number, user_id, moderator_id, action_type,
                        reason, duration_seconds, created_at, expires_at, active
                 FROM mod_cases WHERE guild_id = ?1 AND case_number = ?2",
                params![guild_id as i64, case_number],
                |row| {
                    let gid_i64: i64 = row.get(1)?;
                    let uid_i64: i64 = row.get(3)?;
                    let mid_i64: i64 = row.get(4)?;
                    let active_i64: i64 = row.get(10)?;
                    Ok(ModCase {
                        id: row.get(0)?,
                        guild_id: gid_i64 as u64,
                        case_number: row.get(2)?,
                        user_id: uid_i64 as u64,
                        moderator_id: mid_i64 as u64,
                        action_type: row.get(5)?,
                        reason: row.get(6)?,
                        duration_seconds: row.get(7)?,
                        created_at: row.get(8)?,
                        expires_at: row.get(9)?,
                        active: active_i64 != 0,
                    })
                },
            )
            .optional()?;
        Ok(res)
    }

    /// Returns all moderation cases recorded for a given user in a guild, ordered newest first.
    pub fn get_mod_cases_for_user(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<Vec<ModCase>, StorageError> {
        let conn = self.writer.lock();
        let mut stmt = conn.prepare(
            "SELECT id, guild_id, case_number, user_id, moderator_id, action_type,
                    reason, duration_seconds, created_at, expires_at, active
             FROM mod_cases WHERE guild_id = ?1 AND user_id = ?2 ORDER BY case_number DESC",
        )?;
        let rows = stmt.query_map(params![guild_id as i64, user_id as i64], |row| {
            let gid_i64: i64 = row.get(1)?;
            let uid_i64: i64 = row.get(3)?;
            let mid_i64: i64 = row.get(4)?;
            let active_i64: i64 = row.get(10)?;
            Ok(ModCase {
                id: row.get(0)?,
                guild_id: gid_i64 as u64,
                case_number: row.get(2)?,
                user_id: uid_i64 as u64,
                moderator_id: mid_i64 as u64,
                action_type: row.get(5)?,
                reason: row.get(6)?,
                duration_seconds: row.get(7)?,
                created_at: row.get(8)?,
                expires_at: row.get(9)?,
                active: active_i64 != 0,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Retrieves active timed cases (e.g. tempbans, timeouts) whose expiration timestamp is <= `now_timestamp`.
    pub fn get_active_expired_cases(
        &self,
        now_timestamp: i64,
    ) -> Result<Vec<ModCase>, StorageError> {
        let conn = self.writer.lock();
        let mut stmt = conn.prepare(
            "SELECT id, guild_id, case_number, user_id, moderator_id, action_type,
                    reason, duration_seconds, created_at, expires_at, active
             FROM mod_cases WHERE active = 1 AND expires_at IS NOT NULL AND expires_at <= ?1
             ORDER BY expires_at ASC",
        )?;
        let rows = stmt.query_map(params![now_timestamp], |row| {
            let gid_i64: i64 = row.get(1)?;
            let uid_i64: i64 = row.get(3)?;
            let mid_i64: i64 = row.get(4)?;
            let active_i64: i64 = row.get(10)?;
            Ok(ModCase {
                id: row.get(0)?,
                guild_id: gid_i64 as u64,
                case_number: row.get(2)?,
                user_id: uid_i64 as u64,
                moderator_id: mid_i64 as u64,
                action_type: row.get(5)?,
                reason: row.get(6)?,
                duration_seconds: row.get(7)?,
                created_at: row.get(8)?,
                expires_at: row.get(9)?,
                active: active_i64 != 0,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Sets `active = 0` on a mod case (e.g. when an expired punishment is reversed or lifted).
    pub fn deactivate_case(&self, case_id: i64) -> Result<bool, StorageError> {
        let conn = self.writer.lock();
        let affected = conn.execute(
            "UPDATE mod_cases SET active = 0 WHERE id = ?1",
            params![case_id],
        )?;
        Ok(affected > 0)
    }

    /// Updates the reason of a mod case by guild ID and case number.
    pub fn update_case_reason(
        &self,
        guild_id: u64,
        case_number: i64,
        new_reason: &str,
    ) -> Result<bool, StorageError> {
        let conn = self.writer.lock();
        let affected = conn.execute(
            "UPDATE mod_cases SET reason = ?1 WHERE guild_id = ?2 AND case_number = ?3",
            params![new_reason, guild_id as i64, case_number],
        )?;
        Ok(affected > 0)
    }

    // =========================================================================
    // Moderation Strikes
    // =========================================================================

    /// Adds a strike to a user's record.
    pub fn add_strike(&self, new_strike: &NewModStrike) -> Result<ModStrike, StorageError> {
        let created_at = new_strike.created_at.unwrap_or_else(|| Utc::now().timestamp());
        let conn = self.writer.lock();
        conn.execute(
            "INSERT INTO mod_strikes (
                guild_id, user_id, case_id, strike_count, reason, created_at, expires_at, decayed
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            params![
                new_strike.guild_id as i64,
                new_strike.user_id as i64,
                new_strike.case_id,
                new_strike.strike_count,
                new_strike.reason,
                created_at,
                new_strike.expires_at,
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(ModStrike {
            id,
            guild_id: new_strike.guild_id,
            user_id: new_strike.user_id,
            case_id: new_strike.case_id,
            strike_count: new_strike.strike_count,
            reason: new_strike.reason.clone(),
            created_at,
            expires_at: new_strike.expires_at,
            decayed: false,
        })
    }

    /// Computes the sum of active (non-decayed) strikes for a user in a guild.
    pub fn get_active_strikes_count(&self, guild_id: u64, user_id: u64) -> Result<i64, StorageError> {
        let conn = self.writer.lock();
        let count: i64 = conn.query_row(
            "SELECT COALESCE(SUM(strike_count), 0) FROM mod_strikes
             WHERE guild_id = ?1 AND user_id = ?2 AND decayed = 0",
            params![guild_id as i64, user_id as i64],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Retrieves all strike records (active and decayed) for a user in a guild.
    pub fn get_user_strikes(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<Vec<ModStrike>, StorageError> {
        let conn = self.writer.lock();
        let mut stmt = conn.prepare(
            "SELECT id, guild_id, user_id, case_id, strike_count, reason, created_at, expires_at, decayed
             FROM mod_strikes WHERE guild_id = ?1 AND user_id = ?2 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map(params![guild_id as i64, user_id as i64], |row| {
            let gid_i64: i64 = row.get(1)?;
            let uid_i64: i64 = row.get(2)?;
            let decayed_i64: i64 = row.get(8)?;
            Ok(ModStrike {
                id: row.get(0)?,
                guild_id: gid_i64 as u64,
                user_id: uid_i64 as u64,
                case_id: row.get(3)?,
                strike_count: row.get(4)?,
                reason: row.get(5)?,
                created_at: row.get(6)?,
                expires_at: row.get(7)?,
                decayed: decayed_i64 != 0,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Decays strikes in `guild_id` whose expiration timestamp or creation timestamp is <= `cutoff_epoch`.
    pub fn decay_strikes(&self, guild_id: u64, cutoff_epoch: i64) -> Result<usize, StorageError> {
        let conn = self.writer.lock();
        let affected = conn.execute(
            "UPDATE mod_strikes SET decayed = 1
             WHERE guild_id = ?1 AND decayed = 0 AND (expires_at <= ?2 OR (expires_at IS NULL AND created_at <= ?2))",
            params![guild_id as i64, cutoff_epoch],
        )?;
        Ok(affected)
    }

    // =========================================================================
    // Automod Configuration
    // =========================================================================

    /// Fetches the automod configuration for a guild, falling back to sensible defaults.
    pub fn get_automod_config(&self, guild_id: u64) -> Result<AutomodConfig, StorageError> {
        let conn = self.writer.lock();
        let res = conn
            .query_row(
                "SELECT
                    guild_id, enabled, anti_spam_enabled, anti_spam_max_messages, anti_spam_interval_seconds,
                    anti_invites_enabled, anti_links_enabled, anti_caps_enabled, anti_caps_min_chars, anti_caps_percent,
                    anti_mention_max, blocked_words_json, whitelisted_domains_json, ignored_channels_json, ignored_roles_json,
                    strike_weights_json, strike_decay_hours, punishment_ladder_json, ai_moderation_enabled
                 FROM automod_configs WHERE guild_id = ?1",
                params![guild_id as i64],
                |row| {
                    let gid_i64: i64 = row.get(0)?;
                    let enabled_i64: i64 = row.get(1)?;
                    let anti_spam_i64: i64 = row.get(2)?;
                    let anti_inv_i64: i64 = row.get(5)?;
                    let anti_links_i64: i64 = row.get(6)?;
                    let anti_caps_i64: i64 = row.get(7)?;
                    let blocked_json: String = row.get(11)?;
                    let whitelist_json: String = row.get(12)?;
                    let ignored_ch_json: String = row.get(13)?;
                    let ignored_ro_json: String = row.get(14)?;
                    let weights_json: String = row.get(15)?;
                    let ladder_json: String = row.get(17)?;
                    let ai_mod_i64: i64 = row.get(18)?;

                    let blocked_words: Vec<String> =
                        serde_json::from_str(&blocked_json).unwrap_or_default();
                    let whitelisted_domains: Vec<String> =
                        serde_json::from_str(&whitelist_json).unwrap_or_default();
                    let ignored_channels: Vec<u64> =
                        serde_json::from_str(&ignored_ch_json).unwrap_or_default();
                    let ignored_roles: Vec<u64> =
                        serde_json::from_str(&ignored_ro_json).unwrap_or_default();
                    let strike_weights: HashMap<String, i64> =
                        serde_json::from_str(&weights_json).unwrap_or_default();
                    let punishment_ladder: Vec<PunishmentStep> =
                        serde_json::from_str(&ladder_json).unwrap_or_default();

                    Ok(AutomodConfig {
                        guild_id: gid_i64 as u64,
                        enabled: enabled_i64 != 0,
                        anti_spam_enabled: anti_spam_i64 != 0,
                        anti_spam_max_messages: row.get(3)?,
                        anti_spam_interval_seconds: row.get(4)?,
                        anti_invites_enabled: anti_inv_i64 != 0,
                        anti_links_enabled: anti_links_i64 != 0,
                        anti_caps_enabled: anti_caps_i64 != 0,
                        anti_caps_min_chars: row.get(8)?,
                        anti_caps_percent: row.get(9)?,
                        anti_mention_max: row.get(10)?,
                        blocked_words,
                        whitelisted_domains,
                        ignored_channels,
                        ignored_roles,
                        strike_weights,
                        strike_decay_hours: row.get(16)?,
                        punishment_ladder,
                        ai_moderation_enabled: ai_mod_i64 != 0,
                    })
                },
            )
            .optional()?;

        Ok(res.unwrap_or_else(|| AutomodConfig::default_for_guild(guild_id)))
    }

    /// Upserts the automod configuration for a guild.
    pub fn save_automod_config(&self, config: &AutomodConfig) -> Result<(), StorageError> {
        let blocked_json = serde_json::to_string(&config.blocked_words)?;
        let whitelist_json = serde_json::to_string(&config.whitelisted_domains)?;
        let ignored_ch_json = serde_json::to_string(&config.ignored_channels)?;
        let ignored_ro_json = serde_json::to_string(&config.ignored_roles)?;
        let weights_json = serde_json::to_string(&config.strike_weights)?;
        let ladder_json = serde_json::to_string(&config.punishment_ladder)?;

        let conn = self.writer.lock();
        conn.execute(
            "INSERT INTO automod_configs (
                guild_id, enabled, anti_spam_enabled, anti_spam_max_messages, anti_spam_interval_seconds,
                anti_invites_enabled, anti_links_enabled, anti_caps_enabled, anti_caps_min_chars, anti_caps_percent,
                anti_mention_max, blocked_words_json, whitelisted_domains_json, ignored_channels_json, ignored_roles_json,
                strike_weights_json, strike_decay_hours, punishment_ladder_json, ai_moderation_enabled
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
            ON CONFLICT(guild_id) DO UPDATE SET
                enabled = excluded.enabled,
                anti_spam_enabled = excluded.anti_spam_enabled,
                anti_spam_max_messages = excluded.anti_spam_max_messages,
                anti_spam_interval_seconds = excluded.anti_spam_interval_seconds,
                anti_invites_enabled = excluded.anti_invites_enabled,
                anti_links_enabled = excluded.anti_links_enabled,
                anti_caps_enabled = excluded.anti_caps_enabled,
                anti_caps_min_chars = excluded.anti_caps_min_chars,
                anti_caps_percent = excluded.anti_caps_percent,
                anti_mention_max = excluded.anti_mention_max,
                blocked_words_json = excluded.blocked_words_json,
                whitelisted_domains_json = excluded.whitelisted_domains_json,
                ignored_channels_json = excluded.ignored_channels_json,
                ignored_roles_json = excluded.ignored_roles_json,
                strike_weights_json = excluded.strike_weights_json,
                strike_decay_hours = excluded.strike_decay_hours,
                punishment_ladder_json = excluded.punishment_ladder_json,
                ai_moderation_enabled = excluded.ai_moderation_enabled",
            params![
                config.guild_id as i64,
                i64::from(config.enabled),
                i64::from(config.anti_spam_enabled),
                config.anti_spam_max_messages,
                config.anti_spam_interval_seconds,
                i64::from(config.anti_invites_enabled),
                i64::from(config.anti_links_enabled),
                i64::from(config.anti_caps_enabled),
                config.anti_caps_min_chars,
                config.anti_caps_percent,
                config.anti_mention_max,
                blocked_json,
                whitelist_json,
                ignored_ch_json,
                ignored_ro_json,
                weights_json,
                config.strike_decay_hours,
                ladder_json,
                i64::from(config.ai_moderation_enabled),
            ],
        )?;
        Ok(())
    }

    // =========================================================================
    // Multi-Channel Event Logging Configuration
    // =========================================================================

    /// Fetches the logging channel configuration for a guild.
    pub fn get_logging_config(&self, guild_id: u64) -> Result<LoggingConfig, StorageError> {
        let conn = self.writer.lock();
        let res = conn
            .query_row(
                "SELECT guild_id, unified_channel_id, mod_channel_id, message_channel_id,
                        member_channel_id, server_channel_id, voice_channel_id,
                        ignored_channels_json, ignored_roles_json, log_bots, embed_color
                 FROM logging_configs WHERE guild_id = ?1",
                params![guild_id as i64],
                |row| {
                    let gid_i64: i64 = row.get(0)?;
                    let uc_i64: Option<i64> = row.get(1)?;
                    let mc_i64: Option<i64> = row.get(2)?;
                    let msgc_i64: Option<i64> = row.get(3)?;
                    let memc_i64: Option<i64> = row.get(4)?;
                    let sc_i64: Option<i64> = row.get(5)?;
                    let vc_i64: Option<i64> = row.get(6)?;
                    let ign_ch_json: String = row.get(7)?;
                    let ign_ro_json: String = row.get(8)?;
                    let log_bots_i64: i64 = row.get(9)?;
                    let embed_color: i64 = row.get(10)?;

                    let ignored_channels: Vec<u64> =
                        serde_json::from_str(&ign_ch_json).unwrap_or_default();
                    let ignored_roles: Vec<u64> =
                        serde_json::from_str(&ign_ro_json).unwrap_or_default();

                    Ok(LoggingConfig {
                        guild_id: gid_i64 as u64,
                        unified_channel_id: uc_i64.map(|c| c as u64),
                        mod_channel_id: mc_i64.map(|c| c as u64),
                        message_channel_id: msgc_i64.map(|c| c as u64),
                        member_channel_id: memc_i64.map(|c| c as u64),
                        server_channel_id: sc_i64.map(|c| c as u64),
                        voice_channel_id: vc_i64.map(|c| c as u64),
                        ignored_channels,
                        ignored_roles,
                        log_bots: log_bots_i64 != 0,
                        embed_color: embed_color as u32,
                    })
                },
            )
            .optional()?;
        Ok(res.unwrap_or_else(|| LoggingConfig::default_for_guild(guild_id)))
    }

    /// Upserts the logging channel configuration for a guild.
    pub fn save_logging_config(&self, config: &LoggingConfig) -> Result<(), StorageError> {
        let ign_ch_json = serde_json::to_string(&config.ignored_channels)?;
        let ign_ro_json = serde_json::to_string(&config.ignored_roles)?;
        let conn = self.writer.lock();
        conn.execute(
            "INSERT INTO logging_configs (
                guild_id, unified_channel_id, mod_channel_id, message_channel_id,
                member_channel_id, server_channel_id, voice_channel_id,
                ignored_channels_json, ignored_roles_json, log_bots, embed_color
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(guild_id) DO UPDATE SET
                unified_channel_id = excluded.unified_channel_id,
                mod_channel_id = excluded.mod_channel_id,
                message_channel_id = excluded.message_channel_id,
                member_channel_id = excluded.member_channel_id,
                server_channel_id = excluded.server_channel_id,
                voice_channel_id = excluded.voice_channel_id,
                ignored_channels_json = excluded.ignored_channels_json,
                ignored_roles_json = excluded.ignored_roles_json,
                log_bots = excluded.log_bots,
                embed_color = excluded.embed_color",
            params![
                config.guild_id as i64,
                config.unified_channel_id.map(|c| c as i64),
                config.mod_channel_id.map(|c| c as i64),
                config.message_channel_id.map(|c| c as i64),
                config.member_channel_id.map(|c| c as i64),
                config.server_channel_id.map(|c| c as i64),
                config.voice_channel_id.map(|c| c as i64),
                ign_ch_json,
                ign_ro_json,
                i64::from(config.log_bots),
                config.embed_color as i64,
            ],
        )?;
        Ok(())
    }

    // =========================================================================
    // Greetings, Farewells & Autoroles
    // =========================================================================

    /// Fetches the greetings, farewells, and autorole configuration for a guild.
    pub fn get_greetings_config(&self, guild_id: u64) -> Result<GreetingsConfig, StorageError> {
        let conn = self.writer.lock();
        let res = conn
            .query_row(
                "SELECT guild_id, welcome_channel_id, welcome_message, welcome_embed_json,
                        welcome_dm_enabled, welcome_dm_message, farewell_channel_id, farewell_message,
                        farewell_embed_json, member_autorole_ids_json, bot_autorole_ids_json, autorole_delay_seconds
                 FROM greetings_configs WHERE guild_id = ?1",
                params![guild_id as i64],
                |row| {
                    let gid_i64: i64 = row.get(0)?;
                    let wc_i64: Option<i64> = row.get(1)?;
                    let wdm_i64: i64 = row.get(4)?;
                    let fc_i64: Option<i64> = row.get(6)?;
                    let mem_roles_json: String = row.get(9)?;
                    let bot_roles_json: String = row.get(10)?;

                    let member_autorole_ids: Vec<u64> =
                        serde_json::from_str(&mem_roles_json).unwrap_or_default();
                    let bot_autorole_ids: Vec<u64> =
                        serde_json::from_str(&bot_roles_json).unwrap_or_default();

                    Ok(GreetingsConfig {
                        guild_id: gid_i64 as u64,
                        welcome_channel_id: wc_i64.map(|c| c as u64),
                        welcome_message: row.get(2)?,
                        welcome_embed_json: row.get(3)?,
                        welcome_dm_enabled: wdm_i64 != 0,
                        welcome_dm_message: row.get(5)?,
                        farewell_channel_id: fc_i64.map(|c| c as u64),
                        farewell_message: row.get(7)?,
                        farewell_embed_json: row.get(8)?,
                        member_autorole_ids,
                        bot_autorole_ids,
                        autorole_delay_seconds: row.get(11)?,
                    })
                },
            )
            .optional()?;
        Ok(res.unwrap_or_else(|| GreetingsConfig::default_for_guild(guild_id)))
    }

    /// Upserts the greetings, farewells, and autorole configuration for a guild.
    pub fn save_greetings_config(&self, config: &GreetingsConfig) -> Result<(), StorageError> {
        let mem_roles_json = serde_json::to_string(&config.member_autorole_ids)?;
        let bot_roles_json = serde_json::to_string(&config.bot_autorole_ids)?;
        let conn = self.writer.lock();
        conn.execute(
            "INSERT INTO greetings_configs (
                guild_id, welcome_channel_id, welcome_message, welcome_embed_json,
                welcome_dm_enabled, welcome_dm_message, farewell_channel_id, farewell_message,
                farewell_embed_json, member_autorole_ids_json, bot_autorole_ids_json, autorole_delay_seconds
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(guild_id) DO UPDATE SET
                welcome_channel_id = excluded.welcome_channel_id,
                welcome_message = excluded.welcome_message,
                welcome_embed_json = excluded.welcome_embed_json,
                welcome_dm_enabled = excluded.welcome_dm_enabled,
                welcome_dm_message = excluded.welcome_dm_message,
                farewell_channel_id = excluded.farewell_channel_id,
                farewell_message = excluded.farewell_message,
                farewell_embed_json = excluded.farewell_embed_json,
                member_autorole_ids_json = excluded.member_autorole_ids_json,
                bot_autorole_ids_json = excluded.bot_autorole_ids_json,
                autorole_delay_seconds = excluded.autorole_delay_seconds",
            params![
                config.guild_id as i64,
                config.welcome_channel_id.map(|c| c as i64),
                config.welcome_message,
                config.welcome_embed_json,
                i64::from(config.welcome_dm_enabled),
                config.welcome_dm_message,
                config.farewell_channel_id.map(|c| c as i64),
                config.farewell_message,
                config.farewell_embed_json,
                mem_roles_json,
                bot_roles_json,
                config.autorole_delay_seconds,
            ],
        )?;
        Ok(())
    }

    // =========================================================================
    // Reaction Roles & Component Groups
    // =========================================================================

    /// Creates a reaction role group bound to a channel and message.
    pub fn create_reaction_role_group(
        &self,
        group: &NewReactionRoleGroup,
    ) -> Result<ReactionRoleGroup, StorageError> {
        let conn = self.writer.lock();
        conn.execute(
            "INSERT INTO reaction_role_groups (
                guild_id, channel_id, message_id, group_type, required_role_id, component_type
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                group.guild_id as i64,
                group.channel_id as i64,
                group.message_id as i64,
                group.group_type,
                group.required_role_id.map(|r| r as i64),
                group.component_type,
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(ReactionRoleGroup {
            id,
            guild_id: group.guild_id,
            channel_id: group.channel_id,
            message_id: group.message_id,
            group_type: group.group_type.clone(),
            required_role_id: group.required_role_id,
            component_type: group.component_type.clone(),
            entries: Vec::new(),
        })
    }

    /// Adds an entry (emoji or button mapped to a role) to a reaction role group.
    pub fn add_reaction_role_entry(
        &self,
        entry: &NewReactionRoleEntry,
    ) -> Result<ReactionRoleEntry, StorageError> {
        let conn = self.writer.lock();
        conn.execute(
            "INSERT INTO reaction_role_entries (
                group_id, emoji_or_custom_id, role_id, button_label, button_style
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                entry.group_id,
                entry.emoji_or_custom_id,
                entry.role_id as i64,
                entry.button_label,
                entry.button_style,
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(ReactionRoleEntry {
            id,
            group_id: entry.group_id,
            emoji_or_custom_id: entry.emoji_or_custom_id.clone(),
            role_id: entry.role_id,
            button_label: entry.button_label.clone(),
            button_style: entry.button_style.clone(),
        })
    }

    /// Fetches a reaction role group and its mapped entries for a specific message.
    pub fn get_reaction_roles_for_message(
        &self,
        guild_id: u64,
        message_id: u64,
    ) -> Result<Option<ReactionRoleGroup>, StorageError> {
        let conn = self.writer.lock();
        let group_opt = conn
            .query_row(
                "SELECT id, guild_id, channel_id, message_id, group_type, required_role_id, component_type
                 FROM reaction_role_groups WHERE guild_id = ?1 AND message_id = ?2",
                params![guild_id as i64, message_id as i64],
                |row| {
                    let id: i64 = row.get(0)?;
                    let gid_i64: i64 = row.get(1)?;
                    let cid_i64: i64 = row.get(2)?;
                    let mid_i64: i64 = row.get(3)?;
                    let req_i64: Option<i64> = row.get(5)?;
                    Ok(ReactionRoleGroup {
                        id,
                        guild_id: gid_i64 as u64,
                        channel_id: cid_i64 as u64,
                        message_id: mid_i64 as u64,
                        group_type: row.get(4)?,
                        required_role_id: req_i64.map(|r| r as u64),
                        component_type: row.get(6)?,
                        entries: Vec::new(),
                    })
                },
            )
            .optional()?;

        let Some(mut group) = group_opt else {
            return Ok(None);
        };

        let mut stmt = conn.prepare(
            "SELECT id, group_id, emoji_or_custom_id, role_id, button_label, button_style
             FROM reaction_role_entries WHERE group_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![group.id], |row| {
            let eid: i64 = row.get(0)?;
            let gid: i64 = row.get(1)?;
            let rid_i64: i64 = row.get(3)?;
            Ok(ReactionRoleEntry {
                id: eid,
                group_id: gid,
                emoji_or_custom_id: row.get(2)?,
                role_id: rid_i64 as u64,
                button_label: row.get(4)?,
                button_style: row.get(5)?,
            })
        })?;

        for r in rows {
            group.entries.push(r?);
        }

        Ok(Some(group))
    }

    /// Fetches all reaction role groups and entries for a guild.
    pub fn get_guild_reaction_roles(
        &self,
        guild_id: u64,
    ) -> Result<Vec<ReactionRoleGroup>, StorageError> {
        let conn = self.writer.lock();
        let mut group_stmt = conn.prepare(
            "SELECT id, guild_id, channel_id, message_id, group_type, required_role_id, component_type
             FROM reaction_role_groups WHERE guild_id = ?1 ORDER BY id ASC",
        )?;
        let group_rows = group_stmt.query_map(params![guild_id as i64], |row| {
            let id: i64 = row.get(0)?;
            let gid_i64: i64 = row.get(1)?;
            let cid_i64: i64 = row.get(2)?;
            let mid_i64: i64 = row.get(3)?;
            let req_i64: Option<i64> = row.get(5)?;
            Ok(ReactionRoleGroup {
                id,
                guild_id: gid_i64 as u64,
                channel_id: cid_i64 as u64,
                message_id: mid_i64 as u64,
                group_type: row.get(4)?,
                required_role_id: req_i64.map(|r| r as u64),
                component_type: row.get(6)?,
                entries: Vec::new(),
            })
        })?;

        let mut groups = Vec::new();
        for gr in group_rows {
            groups.push(gr?);
        }

        let mut entry_stmt = conn.prepare(
            "SELECT id, group_id, emoji_or_custom_id, role_id, button_label, button_style
             FROM reaction_role_entries WHERE group_id = ?1 ORDER BY id ASC",
        )?;

        for group in &mut groups {
            let entry_rows = entry_stmt.query_map(params![group.id], |row| {
                let eid: i64 = row.get(0)?;
                let gid: i64 = row.get(1)?;
                let rid_i64: i64 = row.get(3)?;
                Ok(ReactionRoleEntry {
                    id: eid,
                    group_id: gid,
                    emoji_or_custom_id: row.get(2)?,
                    role_id: rid_i64 as u64,
                    button_label: row.get(4)?,
                    button_style: row.get(5)?,
                })
            })?;
            for er in entry_rows {
                group.entries.push(er?);
            }
        }

        Ok(groups)
    }

    /// Deletes a reaction role group (and cascades to its entries).
    pub fn delete_reaction_role_group(&self, group_id: i64) -> Result<bool, StorageError> {
        let conn = self.writer.lock();
        let affected = conn.execute(
            "DELETE FROM reaction_role_groups WHERE id = ?1",
            params![group_id],
        )?;
        Ok(affected > 0)
    }

    // =========================================================================
    // Custom Triggers / Tags
    // =========================================================================

    /// Creates or updates a custom trigger in a guild.
    pub fn create_custom_trigger(
        &self,
        trigger: &NewCustomTrigger,
    ) -> Result<CustomTrigger, StorageError> {
        let roles_json = serde_json::to_string(&trigger.allowed_roles)?;
        let channels_json = serde_json::to_string(&trigger.ignored_channels)?;
        let conn = self.writer.lock();
        conn.execute(
            "INSERT INTO custom_triggers (
                guild_id, trigger_name, match_mode, response_template, cooldown_seconds,
                allowed_roles_json, ignored_channels_json, delete_trigger_message, reply_in_dm, enabled
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(guild_id, trigger_name) DO UPDATE SET
                match_mode = excluded.match_mode,
                response_template = excluded.response_template,
                cooldown_seconds = excluded.cooldown_seconds,
                allowed_roles_json = excluded.allowed_roles_json,
                ignored_channels_json = excluded.ignored_channels_json,
                delete_trigger_message = excluded.delete_trigger_message,
                reply_in_dm = excluded.reply_in_dm,
                enabled = excluded.enabled",
            params![
                trigger.guild_id as i64,
                trigger.trigger_name,
                trigger.match_mode,
                trigger.response_template,
                trigger.cooldown_seconds,
                roles_json,
                channels_json,
                i64::from(trigger.delete_trigger_message),
                i64::from(trigger.reply_in_dm),
                i64::from(trigger.enabled),
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(CustomTrigger {
            id,
            guild_id: trigger.guild_id,
            trigger_name: trigger.trigger_name.clone(),
            match_mode: trigger.match_mode.clone(),
            response_template: trigger.response_template.clone(),
            cooldown_seconds: trigger.cooldown_seconds,
            allowed_roles: trigger.allowed_roles.clone(),
            ignored_channels: trigger.ignored_channels.clone(),
            delete_trigger_message: trigger.delete_trigger_message,
            reply_in_dm: trigger.reply_in_dm,
            enabled: trigger.enabled,
        })
    }

    /// Fetches a custom trigger by guild ID and trigger name.
    pub fn get_custom_trigger(
        &self,
        guild_id: u64,
        trigger_name: &str,
    ) -> Result<Option<CustomTrigger>, StorageError> {
        let conn = self.writer.lock();
        let res = conn
            .query_row(
                "SELECT id, guild_id, trigger_name, match_mode, response_template, cooldown_seconds,
                        allowed_roles_json, ignored_channels_json, delete_trigger_message, reply_in_dm, enabled
                 FROM custom_triggers WHERE guild_id = ?1 AND trigger_name = ?2",
                params![guild_id as i64, trigger_name],
                |row| {
                    let id: i64 = row.get(0)?;
                    let gid_i64: i64 = row.get(1)?;
                    let roles_json: String = row.get(6)?;
                    let channels_json: String = row.get(7)?;
                    let del_i64: i64 = row.get(8)?;
                    let dm_i64: i64 = row.get(9)?;
                    let en_i64: i64 = row.get(10)?;
                    let allowed_roles: Vec<u64> =
                        serde_json::from_str(&roles_json).unwrap_or_default();
                    let ignored_channels: Vec<u64> =
                        serde_json::from_str(&channels_json).unwrap_or_default();
                    Ok(CustomTrigger {
                        id,
                        guild_id: gid_i64 as u64,
                        trigger_name: row.get(2)?,
                        match_mode: row.get(3)?,
                        response_template: row.get(4)?,
                        cooldown_seconds: row.get(5)?,
                        allowed_roles,
                        ignored_channels,
                        delete_trigger_message: del_i64 != 0,
                        reply_in_dm: dm_i64 != 0,
                        enabled: en_i64 != 0,
                    })
                },
            )
            .optional()?;
        Ok(res)
    }

    /// Lists all custom triggers registered for a guild.
    pub fn list_custom_triggers(&self, guild_id: u64) -> Result<Vec<CustomTrigger>, StorageError> {
        let conn = self.writer.lock();
        let mut stmt = conn.prepare(
            "SELECT id, guild_id, trigger_name, match_mode, response_template, cooldown_seconds,
                    allowed_roles_json, ignored_channels_json, delete_trigger_message, reply_in_dm, enabled
             FROM custom_triggers WHERE guild_id = ?1 ORDER BY trigger_name ASC",
        )?;
        let rows = stmt.query_map(params![guild_id as i64], |row| {
            let id: i64 = row.get(0)?;
            let gid_i64: i64 = row.get(1)?;
            let roles_json: String = row.get(6)?;
            let channels_json: String = row.get(7)?;
            let del_i64: i64 = row.get(8)?;
            let dm_i64: i64 = row.get(9)?;
            let en_i64: i64 = row.get(10)?;
            let allowed_roles: Vec<u64> =
                serde_json::from_str(&roles_json).unwrap_or_default();
            let ignored_channels: Vec<u64> =
                serde_json::from_str(&channels_json).unwrap_or_default();
            Ok(CustomTrigger {
                id,
                guild_id: gid_i64 as u64,
                trigger_name: row.get(2)?,
                match_mode: row.get(3)?,
                response_template: row.get(4)?,
                cooldown_seconds: row.get(5)?,
                allowed_roles,
                ignored_channels,
                delete_trigger_message: del_i64 != 0,
                reply_in_dm: dm_i64 != 0,
                enabled: en_i64 != 0,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Deletes a custom trigger by guild ID and trigger name.
    pub fn delete_custom_trigger(
        &self,
        guild_id: u64,
        trigger_name: &str,
    ) -> Result<bool, StorageError> {
        let conn = self.writer.lock();
        let affected = conn.execute(
            "DELETE FROM custom_triggers WHERE guild_id = ?1 AND trigger_name = ?2",
            params![guild_id as i64, trigger_name],
        )?;
        Ok(affected > 0)
    }
}

fn query_usage_sum(
    conn: &Connection,
    guild_id: i64,
    user_id: i64,
    pseudo_id: i64,
    start_ts: i64,
    end_ts: i64,
) -> Result<i64, StorageError> {
    let sum: i64 = conn.query_row(
        "SELECT COALESCE(SUM(total_tokens), 0)
         FROM token_usage_ledger
         WHERE guild_id = ?1
           AND (user_id = ?2 OR user_id = ?3)
           AND recorded_at >= ?4
           AND recorded_at < ?5",
        params![guild_id, user_id, pseudo_id, start_ts, end_ts],
        |row| row.get(0),
    )?;
    Ok(sum.max(0))
}

fn apply_connection_pragmas(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA wal_autocheckpoint = 1000;
         PRAGMA secure_delete = ON;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -8000;
         PRAGMA mmap_size = 33554432;",
    )?;
    Ok(())
}

fn initialize_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        ) STRICT;",
    )?;

    let max_ver: Option<u32> = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
        .optional()?
        .flatten();

    let now = Utc::now().timestamp();
    if let Some(v) = max_ver {
        if v > CURRENT_SCHEMA_VERSION {
            return Err(StorageError::NewerSchemaVersion {
                db_version: v,
                max_supported: CURRENT_SCHEMA_VERSION,
            });
        }
        if v < 2 {
            let _ = conn.execute_batch(
                "ALTER TABLE app_settings ADD COLUMN reddit_client_id TEXT NOT NULL DEFAULT '';
                 ALTER TABLE app_settings ADD COLUMN reddit_client_secret TEXT NOT NULL DEFAULT '';
                 ALTER TABLE app_settings ADD COLUMN reddit_webhook_secret TEXT NOT NULL DEFAULT '';
                 ALTER TABLE app_settings ADD COLUMN reddit_poll_interval_minutes INTEGER NOT NULL DEFAULT 2;",
            );
            conn.execute(
                "INSERT OR REPLACE INTO schema_migrations (version, applied_at) VALUES (2, ?1)",
                params![now],
            )?;
        }
        if v < 3 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS mod_cases (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    guild_id INTEGER NOT NULL,
                    case_number INTEGER NOT NULL,
                    user_id INTEGER NOT NULL,
                    moderator_id INTEGER NOT NULL,
                    action_type TEXT NOT NULL,
                    reason TEXT NOT NULL DEFAULT 'No reason provided',
                    duration_seconds INTEGER,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER,
                    active INTEGER NOT NULL DEFAULT 1,
                    UNIQUE(guild_id, case_number)
                ) STRICT;
                CREATE INDEX IF NOT EXISTS idx_mod_cases_user ON mod_cases(guild_id, user_id);
                CREATE INDEX IF NOT EXISTS idx_mod_cases_expires ON mod_cases(expires_at) WHERE active = 1 AND expires_at IS NOT NULL;

                CREATE TABLE IF NOT EXISTS mod_strikes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    guild_id INTEGER NOT NULL,
                    user_id INTEGER NOT NULL,
                    case_id INTEGER,
                    strike_count INTEGER NOT NULL DEFAULT 1,
                    reason TEXT NOT NULL DEFAULT '',
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER,
                    decayed INTEGER NOT NULL DEFAULT 0,
                    FOREIGN KEY(case_id) REFERENCES mod_cases(id) ON DELETE SET NULL
                ) STRICT;
                CREATE INDEX IF NOT EXISTS idx_mod_strikes_user ON mod_strikes(guild_id, user_id) WHERE decayed = 0;

                CREATE TABLE IF NOT EXISTS automod_configs (
                    guild_id INTEGER PRIMARY KEY,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    anti_spam_enabled INTEGER NOT NULL DEFAULT 1,
                    anti_spam_max_messages INTEGER NOT NULL DEFAULT 5,
                    anti_spam_interval_seconds INTEGER NOT NULL DEFAULT 5,
                    anti_invites_enabled INTEGER NOT NULL DEFAULT 1,
                    anti_links_enabled INTEGER NOT NULL DEFAULT 0,
                    anti_caps_enabled INTEGER NOT NULL DEFAULT 0,
                    anti_caps_min_chars INTEGER NOT NULL DEFAULT 10,
                    anti_caps_percent INTEGER NOT NULL DEFAULT 70,
                    anti_mention_max INTEGER NOT NULL DEFAULT 5,
                    blocked_words_json TEXT NOT NULL DEFAULT '[]',
                    whitelisted_domains_json TEXT NOT NULL DEFAULT '[]',
                    ignored_channels_json TEXT NOT NULL DEFAULT '[]',
                    ignored_roles_json TEXT NOT NULL DEFAULT '[]',
                    strike_weights_json TEXT NOT NULL DEFAULT '{\"spam\":2,\"invite\":3,\"bad_word\":2,\"caps\":1,\"mention\":3}',
                    strike_decay_hours INTEGER NOT NULL DEFAULT 168,
                    punishment_ladder_json TEXT NOT NULL DEFAULT '[{\"strikes\":2,\"action\":\"warn\"},{\"strikes\":4,\"action\":\"timeout\",\"duration_seconds\":3600},{\"strikes\":8,\"action\":\"kick\"},{\"strikes\":10,\"action\":\"ban\"}]',
                    ai_moderation_enabled INTEGER NOT NULL DEFAULT 0
                ) STRICT;

                CREATE TABLE IF NOT EXISTS logging_configs (
                    guild_id INTEGER PRIMARY KEY,
                    unified_channel_id INTEGER,
                    mod_channel_id INTEGER,
                    message_channel_id INTEGER,
                    member_channel_id INTEGER,
                    server_channel_id INTEGER,
                    voice_channel_id INTEGER,
                    ignored_channels_json TEXT NOT NULL DEFAULT '[]',
                    ignored_roles_json TEXT NOT NULL DEFAULT '[]',
                    log_bots INTEGER NOT NULL DEFAULT 1,
                    embed_color INTEGER NOT NULL DEFAULT 3553599
                ) STRICT;

                CREATE TABLE IF NOT EXISTS greetings_configs (
                    guild_id INTEGER PRIMARY KEY,
                    welcome_channel_id INTEGER,
                    welcome_message TEXT NOT NULL DEFAULT '',
                    welcome_embed_json TEXT NOT NULL DEFAULT '',
                    welcome_dm_enabled INTEGER NOT NULL DEFAULT 0,
                    welcome_dm_message TEXT NOT NULL DEFAULT '',
                    farewell_channel_id INTEGER,
                    farewell_message TEXT NOT NULL DEFAULT '',
                    farewell_embed_json TEXT NOT NULL DEFAULT '',
                    member_autorole_ids_json TEXT NOT NULL DEFAULT '[]',
                    bot_autorole_ids_json TEXT NOT NULL DEFAULT '[]',
                    autorole_delay_seconds INTEGER NOT NULL DEFAULT 0
                ) STRICT;

                CREATE TABLE IF NOT EXISTS reaction_role_groups (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    guild_id INTEGER NOT NULL,
                    channel_id INTEGER NOT NULL,
                    message_id INTEGER NOT NULL,
                    group_type TEXT NOT NULL DEFAULT 'normal',
                    required_role_id INTEGER,
                    component_type TEXT NOT NULL DEFAULT 'reaction',
                    UNIQUE(guild_id, channel_id, message_id)
                ) STRICT;

                CREATE TABLE IF NOT EXISTS reaction_role_entries (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    group_id INTEGER NOT NULL,
                    emoji_or_custom_id TEXT NOT NULL,
                    role_id INTEGER NOT NULL,
                    button_label TEXT,
                    button_style TEXT,
                    FOREIGN KEY(group_id) REFERENCES reaction_role_groups(id) ON DELETE CASCADE
                ) STRICT;
                CREATE INDEX IF NOT EXISTS idx_rr_lookup ON reaction_role_groups(guild_id, message_id);

                CREATE TABLE IF NOT EXISTS custom_triggers (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    guild_id INTEGER NOT NULL,
                    trigger_name TEXT NOT NULL,
                    match_mode TEXT NOT NULL DEFAULT 'prefix',
                    response_template TEXT NOT NULL,
                    cooldown_seconds INTEGER NOT NULL DEFAULT 0,
                    allowed_roles_json TEXT NOT NULL DEFAULT '[]',
                    ignored_channels_json TEXT NOT NULL DEFAULT '[]',
                    delete_trigger_message INTEGER NOT NULL DEFAULT 0,
                    reply_in_dm INTEGER NOT NULL DEFAULT 0,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    UNIQUE(guild_id, trigger_name)
                ) STRICT;",
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO schema_migrations (version, applied_at) VALUES (3, ?1)",
                params![now],
            )?;
        }
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            discord_token TEXT NOT NULL DEFAULT '',
            discord_oauth_client_id TEXT NOT NULL DEFAULT '',
            discord_oauth_client_secret TEXT NOT NULL DEFAULT '',
            bot_owner_id TEXT NOT NULL DEFAULT '',
            web_secret_key BLOB NOT NULL,
            session_epoch INTEGER NOT NULL DEFAULT 1,

            backend_type TEXT NOT NULL DEFAULT 'ollama',
            active_model TEXT NOT NULL DEFAULT '',
            local_api_key TEXT NOT NULL DEFAULT '',
            ollama_url TEXT NOT NULL DEFAULT 'http://127.0.0.1:11434',
            llamacpp_url TEXT NOT NULL DEFAULT 'http://127.0.0.1:8081',
            remote_preset TEXT NOT NULL DEFAULT 'Xiaomi MiMo',
            remote_base_url TEXT NOT NULL DEFAULT 'https://api.xiaomimimo.com/v1',
            remote_api_key TEXT NOT NULL DEFAULT '',
            remote_model TEXT NOT NULL DEFAULT 'mimo-v2.6-pro',

            system_prompt TEXT NOT NULL,
            allow_dms INTEGER NOT NULL DEFAULT 1,
            dm_default_model TEXT NOT NULL DEFAULT '',
            dm_disabled_message TEXT NOT NULL DEFAULT 'Direct messages with the bot are currently disabled.',
            dm_memory_turns INTEGER NOT NULL DEFAULT 6,
            dm_memory_ttl_minutes INTEGER NOT NULL DEFAULT 30,

            web_search_enabled INTEGER NOT NULL DEFAULT 1,
            web_search_auto INTEGER NOT NULL DEFAULT 1,
            web_search_provider TEXT NOT NULL DEFAULT 'bing',
            web_search_max_results INTEGER NOT NULL DEFAULT 5,
            tavily_api_key TEXT NOT NULL DEFAULT '',
            brave_api_key TEXT NOT NULL DEFAULT '',

            reddit_client_id TEXT NOT NULL DEFAULT '',
            reddit_client_secret TEXT NOT NULL DEFAULT '',
            reddit_webhook_secret TEXT NOT NULL DEFAULT '',
            reddit_poll_interval_minutes INTEGER NOT NULL DEFAULT 2,

            theme TEXT NOT NULL DEFAULT 'retrowave',
            debug_enabled INTEGER NOT NULL DEFAULT 0,

            update_channel TEXT NOT NULL DEFAULT 'stable',
            auto_update_enabled INTEGER NOT NULL DEFAULT 0,
            auto_update_mode TEXT NOT NULL DEFAULT 'notify',
            auto_update_interval_hours INTEGER NOT NULL DEFAULT 24,
            auto_update_check_on_startup INTEGER NOT NULL DEFAULT 1,
            update_notify_owner_dm INTEGER NOT NULL DEFAULT 1,
            update_notify_channel_id TEXT
        ) STRICT;

        CREATE TABLE IF NOT EXISTS guild_configs (
            guild_id INTEGER PRIMARY KEY,
            default_model TEXT NOT NULL DEFAULT '',
            system_prompt TEXT NOT NULL DEFAULT '',

            reply_roles_json TEXT NOT NULL DEFAULT '[\"everyone\"]',
            reply_users_json TEXT NOT NULL DEFAULT '[]',
            change_model_roles_json TEXT NOT NULL DEFAULT '[]',
            change_model_users_json TEXT NOT NULL DEFAULT '[]',
            change_permission_roles_json TEXT NOT NULL DEFAULT '[]',
            change_permission_users_json TEXT NOT NULL DEFAULT '[]',
            search_roles_json TEXT NOT NULL DEFAULT '[\"everyone\"]',
            search_users_json TEXT NOT NULL DEFAULT '[]',

            web_search_enabled INTEGER NOT NULL DEFAULT 1,
            web_search_auto INTEGER NOT NULL DEFAULT 1,

            pagination_enabled INTEGER NOT NULL DEFAULT 1,
            pagination_max_chars INTEGER NOT NULL DEFAULT 1000,
            non_paginated_max_chars INTEGER NOT NULL DEFAULT 0,

            random_reply_mode TEXT NOT NULL DEFAULT 'off',
            random_reply_value INTEGER NOT NULL DEFAULT 0,

            memory_turns INTEGER NOT NULL DEFAULT 6,
            memory_ttl_minutes INTEGER NOT NULL DEFAULT 30,

            forum_auto_reply_enabled INTEGER NOT NULL DEFAULT 0,
            forum_monitored_channels_json TEXT NOT NULL DEFAULT '[]',
            forum_auto_reply_tags_json TEXT NOT NULL DEFAULT '[]',
            forum_ignore_tags_json TEXT NOT NULL DEFAULT '[]',
            forum_system_prompt TEXT NOT NULL DEFAULT '',
            forum_include_post_title INTEGER NOT NULL DEFAULT 1,
            forum_delivery_mode TEXT NOT NULL DEFAULT 'all_chunks',

            quota_enabled INTEGER NOT NULL DEFAULT 0,
            quota_role_limits_json TEXT NOT NULL DEFAULT '{\"everyone\": 10000}',
            quota_reset_frequency TEXT NOT NULL DEFAULT 'daily',
            quota_reset_time TEXT NOT NULL DEFAULT '00:00',
            quota_reset_tz_offset INTEGER NOT NULL DEFAULT 0,
            quota_reset_day_of_week INTEGER NOT NULL DEFAULT 0,
            quota_reset_day_of_month INTEGER NOT NULL DEFAULT 1,
            quota_allow_partial_overage INTEGER NOT NULL DEFAULT 1,
            quota_max_overage_tokens INTEGER NOT NULL DEFAULT 1000,

            updated_at INTEGER NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS token_usage_ledger (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            guild_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            prompt_tokens INTEGER NOT NULL DEFAULT 0,
            completion_tokens INTEGER NOT NULL DEFAULT 0,
            model TEXT NOT NULL DEFAULT '',
            recorded_at INTEGER NOT NULL
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_token_usage_lookup
        ON token_usage_ledger (guild_id, user_id, recorded_at);

        CREATE INDEX IF NOT EXISTS idx_token_usage_recorded_at
        ON token_usage_ledger (recorded_at);

        CREATE TABLE IF NOT EXISTS update_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            last_check_epoch INTEGER,
            latest_version TEXT,
            latest_tag TEXT,
            release_notes TEXT,
            release_url TEXT,
            update_available INTEGER NOT NULL DEFAULT 0
        ) STRICT;

        CREATE TABLE IF NOT EXISTS reddit_subscriptions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            guild_id INTEGER NOT NULL,
            channel_id INTEGER NOT NULL,
            subreddit TEXT NOT NULL,
            filter_nsfw INTEGER NOT NULL DEFAULT 1,
            min_score INTEGER NOT NULL DEFAULT 0,
            flair_filter TEXT,
            keyword_filter TEXT,
            created_at INTEGER NOT NULL,
            last_seen_post_id TEXT
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_reddit_subscriptions_guild
        ON reddit_subscriptions (guild_id);

        CREATE INDEX IF NOT EXISTS idx_reddit_subscriptions_sub
        ON reddit_subscriptions (subreddit);

        CREATE TABLE IF NOT EXISTS reddit_posted_items (
            subscription_id INTEGER NOT NULL,
            post_id TEXT NOT NULL,
            posted_at INTEGER NOT NULL,
            PRIMARY KEY (subscription_id, post_id)
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_reddit_posted_items_posted_at
        ON reddit_posted_items (posted_at);

        CREATE TABLE IF NOT EXISTS mod_cases (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            guild_id INTEGER NOT NULL,
            case_number INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            moderator_id INTEGER NOT NULL,
            action_type TEXT NOT NULL,
            reason TEXT NOT NULL DEFAULT 'No reason provided',
            duration_seconds INTEGER,
            created_at INTEGER NOT NULL,
            expires_at INTEGER,
            active INTEGER NOT NULL DEFAULT 1,
            UNIQUE(guild_id, case_number)
        ) STRICT;
        CREATE INDEX IF NOT EXISTS idx_mod_cases_user ON mod_cases(guild_id, user_id);
        CREATE INDEX IF NOT EXISTS idx_mod_cases_expires ON mod_cases(expires_at) WHERE active = 1 AND expires_at IS NOT NULL;

        CREATE TABLE IF NOT EXISTS mod_strikes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            guild_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            case_id INTEGER,
            strike_count INTEGER NOT NULL DEFAULT 1,
            reason TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            expires_at INTEGER,
            decayed INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY(case_id) REFERENCES mod_cases(id) ON DELETE SET NULL
        ) STRICT;
        CREATE INDEX IF NOT EXISTS idx_mod_strikes_user ON mod_strikes(guild_id, user_id) WHERE decayed = 0;

        CREATE TABLE IF NOT EXISTS automod_configs (
            guild_id INTEGER PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 1,
            anti_spam_enabled INTEGER NOT NULL DEFAULT 1,
            anti_spam_max_messages INTEGER NOT NULL DEFAULT 5,
            anti_spam_interval_seconds INTEGER NOT NULL DEFAULT 5,
            anti_invites_enabled INTEGER NOT NULL DEFAULT 1,
            anti_links_enabled INTEGER NOT NULL DEFAULT 0,
            anti_caps_enabled INTEGER NOT NULL DEFAULT 0,
            anti_caps_min_chars INTEGER NOT NULL DEFAULT 10,
            anti_caps_percent INTEGER NOT NULL DEFAULT 70,
            anti_mention_max INTEGER NOT NULL DEFAULT 5,
            blocked_words_json TEXT NOT NULL DEFAULT '[]',
            whitelisted_domains_json TEXT NOT NULL DEFAULT '[]',
            ignored_channels_json TEXT NOT NULL DEFAULT '[]',
            ignored_roles_json TEXT NOT NULL DEFAULT '[]',
            strike_weights_json TEXT NOT NULL DEFAULT '{\"spam\":2,\"invite\":3,\"bad_word\":2,\"caps\":1,\"mention\":3}',
            strike_decay_hours INTEGER NOT NULL DEFAULT 168,
            punishment_ladder_json TEXT NOT NULL DEFAULT '[{\"strikes\":2,\"action\":\"warn\"},{\"strikes\":4,\"action\":\"timeout\",\"duration_seconds\":3600},{\"strikes\":8,\"action\":\"kick\"},{\"strikes\":10,\"action\":\"ban\"}]',
            ai_moderation_enabled INTEGER NOT NULL DEFAULT 0
        ) STRICT;

        CREATE TABLE IF NOT EXISTS logging_configs (
            guild_id INTEGER PRIMARY KEY,
            unified_channel_id INTEGER,
            mod_channel_id INTEGER,
            message_channel_id INTEGER,
            member_channel_id INTEGER,
            server_channel_id INTEGER,
            voice_channel_id INTEGER,
            ignored_channels_json TEXT NOT NULL DEFAULT '[]',
            ignored_roles_json TEXT NOT NULL DEFAULT '[]',
            log_bots INTEGER NOT NULL DEFAULT 1,
            embed_color INTEGER NOT NULL DEFAULT 3553599
        ) STRICT;

        CREATE TABLE IF NOT EXISTS greetings_configs (
            guild_id INTEGER PRIMARY KEY,
            welcome_channel_id INTEGER,
            welcome_message TEXT NOT NULL DEFAULT '',
            welcome_embed_json TEXT NOT NULL DEFAULT '',
            welcome_dm_enabled INTEGER NOT NULL DEFAULT 0,
            welcome_dm_message TEXT NOT NULL DEFAULT '',
            farewell_channel_id INTEGER,
            farewell_message TEXT NOT NULL DEFAULT '',
            farewell_embed_json TEXT NOT NULL DEFAULT '',
            member_autorole_ids_json TEXT NOT NULL DEFAULT '[]',
            bot_autorole_ids_json TEXT NOT NULL DEFAULT '[]',
            autorole_delay_seconds INTEGER NOT NULL DEFAULT 0
        ) STRICT;

        CREATE TABLE IF NOT EXISTS reaction_role_groups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            guild_id INTEGER NOT NULL,
            channel_id INTEGER NOT NULL,
            message_id INTEGER NOT NULL,
            group_type TEXT NOT NULL DEFAULT 'normal',
            required_role_id INTEGER,
            component_type TEXT NOT NULL DEFAULT 'reaction',
            UNIQUE(guild_id, channel_id, message_id)
        ) STRICT;

        CREATE TABLE IF NOT EXISTS reaction_role_entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            group_id INTEGER NOT NULL,
            emoji_or_custom_id TEXT NOT NULL,
            role_id INTEGER NOT NULL,
            button_label TEXT,
            button_style TEXT,
            FOREIGN KEY(group_id) REFERENCES reaction_role_groups(id) ON DELETE CASCADE
        ) STRICT;
        CREATE INDEX IF NOT EXISTS idx_rr_lookup ON reaction_role_groups(guild_id, message_id);

        CREATE TABLE IF NOT EXISTS custom_triggers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            guild_id INTEGER NOT NULL,
            trigger_name TEXT NOT NULL,
            match_mode TEXT NOT NULL DEFAULT 'prefix',
            response_template TEXT NOT NULL,
            cooldown_seconds INTEGER NOT NULL DEFAULT 0,
            allowed_roles_json TEXT NOT NULL DEFAULT '[]',
            ignored_channels_json TEXT NOT NULL DEFAULT '[]',
            delete_trigger_message INTEGER NOT NULL DEFAULT 0,
            reply_in_dm INTEGER NOT NULL DEFAULT 0,
            enabled INTEGER NOT NULL DEFAULT 1,
            UNIQUE(guild_id, trigger_name)
        ) STRICT;",
    )?;

    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
        params![CURRENT_SCHEMA_VERSION, now],
    )?;

    // Ensure singleton row `id = 1` exists in `app_settings`
    let exists: Option<i64> = conn
        .query_row("SELECT id FROM app_settings WHERE id = 1", [], |r| r.get(0))
        .optional()?;
    if exists.is_none() {
        let mut secret = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        conn.execute(
            "INSERT INTO app_settings (id, web_secret_key, system_prompt) VALUES (1, ?1, ?2)",
            params![secret, DEFAULT_SYSTEM_PROMPT],
        )?;
    }

    Ok(())
}

fn load_app_settings_from_conn(conn: &Connection) -> Result<AppSettings, StorageError> {
    conn.query_row(
        "SELECT
            discord_token, discord_oauth_client_id, discord_oauth_client_secret,
            bot_owner_id, web_secret_key, session_epoch,
            backend_type, active_model, local_api_key, ollama_url, llamacpp_url,
            remote_preset, remote_base_url, remote_api_key, remote_model,
            system_prompt, allow_dms, dm_default_model, dm_disabled_message,
            dm_memory_turns, dm_memory_ttl_minutes,
            web_search_enabled, web_search_auto, web_search_provider, web_search_max_results,
            tavily_api_key, brave_api_key,
            theme, debug_enabled,
            update_channel, auto_update_enabled, auto_update_mode,
            auto_update_interval_hours, auto_update_check_on_startup,
            update_notify_owner_dm, update_notify_channel_id,
            reddit_client_id, reddit_client_secret, reddit_webhook_secret,
            reddit_poll_interval_minutes
         FROM app_settings WHERE id = 1",
        [],
        |row| {
            Ok(AppSettings {
                discord_token: row.get(0)?,
                discord_oauth_client_id: row.get(1)?,
                discord_oauth_client_secret: row.get(2)?,
                bot_owner_id: row.get(3)?,
                web_secret_key: row.get(4)?,
                session_epoch: row.get(5)?,
                backend_type: row.get(6)?,
                active_model: row.get(7)?,
                local_api_key: row.get(8)?,
                ollama_url: row.get(9)?,
                llamacpp_url: row.get(10)?,
                remote_preset: row.get(11)?,
                remote_base_url: row.get(12)?,
                remote_api_key: row.get(13)?,
                remote_model: row.get(14)?,
                system_prompt: row.get(15)?,
                allow_dms: row.get::<_, i64>(16)? != 0,
                dm_default_model: row.get(17)?,
                dm_disabled_message: row.get(18)?,
                dm_memory_turns: row.get::<_, i64>(19)?.clamp(0, 50) as u32,
                dm_memory_ttl_minutes: row.get::<_, i64>(20)?.clamp(1, 1440) as u32,
                web_search_enabled: row.get::<_, i64>(21)? != 0,
                web_search_auto: row.get::<_, i64>(22)? != 0,
                web_search_provider: row.get(23)?,
                web_search_max_results: row.get::<_, i64>(24)?.clamp(1, 10) as u32,
                tavily_api_key: row.get(25)?,
                brave_api_key: row.get(26)?,
                theme: row.get(27)?,
                debug: row.get::<_, i64>(28)? != 0,
                update_channel: row.get(29)?,
                auto_update_enabled: row.get::<_, i64>(30)? != 0,
                auto_update_mode: row.get(31)?,
                auto_update_interval_hours: row.get::<_, i64>(32)?.clamp(1, 720) as u32,
                auto_update_check_on_startup: row.get::<_, i64>(33)? != 0,
                update_notify_owner_dm: row.get::<_, i64>(34)? != 0,
                update_notify_channel_id: row.get(35)?,
                reddit_client_id: row.get(36)?,
                reddit_client_secret: row.get(37)?,
                reddit_webhook_secret: row.get(38)?,
                reddit_poll_interval_minutes: row.get::<_, i64>(39)?.clamp(1, 1440) as u32,
            })
        },
    )
    .map_err(StorageError::from)
}

fn save_app_settings_to_conn(conn: &Connection, s: &AppSettings) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE app_settings SET
            discord_token = ?1,
            discord_oauth_client_id = ?2,
            discord_oauth_client_secret = ?3,
            bot_owner_id = ?4,
            web_secret_key = ?5,
            session_epoch = ?6,
            backend_type = ?7,
            active_model = ?8,
            local_api_key = ?9,
            ollama_url = ?10,
            llamacpp_url = ?11,
            remote_preset = ?12,
            remote_base_url = ?13,
            remote_api_key = ?14,
            remote_model = ?15,
            system_prompt = ?16,
            allow_dms = ?17,
            dm_default_model = ?18,
            dm_disabled_message = ?19,
            dm_memory_turns = ?20,
            dm_memory_ttl_minutes = ?21,
            web_search_enabled = ?22,
            web_search_auto = ?23,
            web_search_provider = ?24,
            web_search_max_results = ?25,
            tavily_api_key = ?26,
            brave_api_key = ?27,
            theme = ?28,
            debug_enabled = ?29,
            update_channel = ?30,
            auto_update_enabled = ?31,
            auto_update_mode = ?32,
            auto_update_interval_hours = ?33,
            auto_update_check_on_startup = ?34,
            update_notify_owner_dm = ?35,
            update_notify_channel_id = ?36,
            reddit_client_id = ?37,
            reddit_client_secret = ?38,
            reddit_webhook_secret = ?39,
            reddit_poll_interval_minutes = ?40
         WHERE id = 1",
        params![
            s.discord_token,
            s.discord_oauth_client_id,
            s.discord_oauth_client_secret,
            s.bot_owner_id,
            s.web_secret_key,
            s.session_epoch,
            s.backend_type,
            s.active_model,
            s.local_api_key,
            s.ollama_url,
            s.llamacpp_url,
            s.remote_preset,
            s.remote_base_url,
            s.remote_api_key,
            s.remote_model,
            s.system_prompt,
            i64::from(s.allow_dms),
            s.dm_default_model,
            s.dm_disabled_message,
            i64::from(s.dm_memory_turns),
            i64::from(s.dm_memory_ttl_minutes),
            i64::from(s.web_search_enabled),
            i64::from(s.web_search_auto),
            s.web_search_provider,
            i64::from(s.web_search_max_results),
            s.tavily_api_key,
            s.brave_api_key,
            s.theme,
            i64::from(s.debug),
            s.update_channel,
            i64::from(s.auto_update_enabled),
            s.auto_update_mode,
            i64::from(s.auto_update_interval_hours),
            i64::from(s.auto_update_check_on_startup),
            i64::from(s.update_notify_owner_dm),
            s.update_notify_channel_id,
            s.reddit_client_id,
            s.reddit_client_secret,
            s.reddit_webhook_secret,
            i64::from(s.reddit_poll_interval_minutes),
        ],
    )?;
    Ok(())
}

fn row_to_guild_config(row: &rusqlite::Row<'_>) -> Result<(u64, GuildConfig), rusqlite::Error> {
    let gid_i64: i64 = row.get(0)?;
    let guild_id = gid_i64 as u64;

    let parse_vec = |idx: usize| -> Vec<String> {
        let s: String = row.get(idx).unwrap_or_default();
        serde_json::from_str(&s).unwrap_or_default()
    };

    let role_limits_str: String = row.get(25)?;
    let role_limits: HashMap<String, i64> =
        serde_json::from_str(&role_limits_str).unwrap_or_default();

    let mode: String = row.get(15)?;
    let val: u32 = row.get::<_, i64>(16)?.max(0) as u32;

    let mut cfg = GuildConfig {
        default_model: row.get(1)?,
        system_prompt: row.get(2)?,
        reply_roles: parse_vec(3),
        reply_users: parse_vec(4),
        change_model_roles: parse_vec(5),
        change_model_users: parse_vec(6),
        change_permission_roles: parse_vec(7),
        change_permission_users: parse_vec(8),
        search_roles: parse_vec(9),
        search_users: parse_vec(10),
        web_search_enabled: row.get::<_, i64>(11)? != 0,
        web_search_auto: row.get::<_, i64>(12)? != 0,
        pagination_enabled: row.get::<_, i64>(13)? != 0,
        pagination_max_chars: row.get::<_, i64>(14)?.clamp(500, 1900) as usize,
        non_paginated_max_chars: row.get::<_, i64>(33)?.max(0) as usize,
        random_prompt_enabled: mode != "off" && val > 0,
        random_prompt_probability: val.min(100),
        random_reply_mode: mode,
        random_reply_value: val,
        memory_turns: row.get::<_, i64>(17)?.clamp(0, 50) as u32,
        memory_ttl_minutes: row.get::<_, i64>(18)?.clamp(1, 1440) as u32,
        forum_auto_reply_enabled: row.get::<_, i64>(19)? != 0,
        forum_monitored_channels: parse_vec(20),
        forum_auto_reply_tags: parse_vec(21),
        forum_ignore_tags: parse_vec(22),
        forum_system_prompt: row.get(23)?,
        forum_include_post_title: row.get::<_, i64>(24)? != 0,
        forum_delivery_mode: row.get(34)?,
        token_quotas: TokenQuotaConfig {
            enabled: row.get::<_, i64>(26)? != 0,
            role_limits,
            reset_frequency: row.get(27)?,
            reset_time: row.get(28)?,
            reset_timezone_offset: row.get::<_, i64>(29)?.clamp(-12, 14) as i32,
            reset_day_of_week: row.get::<_, i64>(30)?.clamp(0, 6) as u32,
            reset_day_of_month: row.get::<_, i64>(31)?.clamp(1, 28) as u32,
            overage_policy: OveragePolicy {
                allow_partial_overage: row.get::<_, i64>(32)? != 0,
                max_overage_tokens: row.get(35)?,
            },
        },
        compiled: Default::default(),
    };
    cfg.sanitize_and_compile();
    Ok((guild_id, cfg))
}

const SELECT_GUILD_COLUMNS: &str = "
    guild_id, default_model, system_prompt,
    reply_roles_json, reply_users_json,
    change_model_roles_json, change_model_users_json,
    change_permission_roles_json, change_permission_users_json,
    search_roles_json, search_users_json,
    web_search_enabled, web_search_auto,
    pagination_enabled, pagination_max_chars,
    random_reply_mode, random_reply_value,
    memory_turns, memory_ttl_minutes,
    forum_auto_reply_enabled, forum_monitored_channels_json,
    forum_auto_reply_tags_json, forum_ignore_tags_json,
    forum_system_prompt, forum_include_post_title,
    quota_role_limits_json, quota_enabled,
    quota_reset_frequency, quota_reset_time,
    quota_reset_tz_offset, quota_reset_day_of_week,
    quota_reset_day_of_month, quota_allow_partial_overage,
    non_paginated_max_chars, forum_delivery_mode, quota_max_overage_tokens
";

fn load_guild_config_from_conn(
    conn: &Connection,
    guild_id: u64,
) -> Result<Option<GuildConfig>, StorageError> {
    let sql = format!("SELECT {SELECT_GUILD_COLUMNS} FROM guild_configs WHERE guild_id = ?1");
    let res = conn
        .query_row(&sql, params![guild_id as i64], row_to_guild_config)
        .optional()?;
    Ok(res.map(|(_, cfg)| cfg))
}

fn load_all_guild_configs_from_conn(
    conn: &Connection,
) -> Result<Vec<(u64, GuildConfig)>, StorageError> {
    let sql = format!("SELECT {SELECT_GUILD_COLUMNS} FROM guild_configs");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_guild_config)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn save_guild_config_to_conn(
    conn: &Connection,
    guild_id: u64,
    c: &GuildConfig,
) -> Result<(), StorageError> {
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT INTO guild_configs (
            guild_id, default_model, system_prompt,
            reply_roles_json, reply_users_json,
            change_model_roles_json, change_model_users_json,
            change_permission_roles_json, change_permission_users_json,
            search_roles_json, search_users_json,
            web_search_enabled, web_search_auto,
            pagination_enabled, pagination_max_chars, non_paginated_max_chars,
            random_reply_mode, random_reply_value,
            memory_turns, memory_ttl_minutes,
            forum_auto_reply_enabled, forum_monitored_channels_json,
            forum_auto_reply_tags_json, forum_ignore_tags_json,
            forum_system_prompt, forum_include_post_title, forum_delivery_mode,
            quota_enabled, quota_role_limits_json,
            quota_reset_frequency, quota_reset_time,
            quota_reset_tz_offset, quota_reset_day_of_week,
            quota_reset_day_of_month, quota_allow_partial_overage,
            quota_max_overage_tokens, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
            ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29,
            ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37
        )
        ON CONFLICT(guild_id) DO UPDATE SET
            default_model = excluded.default_model,
            system_prompt = excluded.system_prompt,
            reply_roles_json = excluded.reply_roles_json,
            reply_users_json = excluded.reply_users_json,
            change_model_roles_json = excluded.change_model_roles_json,
            change_model_users_json = excluded.change_model_users_json,
            change_permission_roles_json = excluded.change_permission_roles_json,
            change_permission_users_json = excluded.change_permission_users_json,
            search_roles_json = excluded.search_roles_json,
            search_users_json = excluded.search_users_json,
            web_search_enabled = excluded.web_search_enabled,
            web_search_auto = excluded.web_search_auto,
            pagination_enabled = excluded.pagination_enabled,
            pagination_max_chars = excluded.pagination_max_chars,
            non_paginated_max_chars = excluded.non_paginated_max_chars,
            random_reply_mode = excluded.random_reply_mode,
            random_reply_value = excluded.random_reply_value,
            memory_turns = excluded.memory_turns,
            memory_ttl_minutes = excluded.memory_ttl_minutes,
            forum_auto_reply_enabled = excluded.forum_auto_reply_enabled,
            forum_monitored_channels_json = excluded.forum_monitored_channels_json,
            forum_auto_reply_tags_json = excluded.forum_auto_reply_tags_json,
            forum_ignore_tags_json = excluded.forum_ignore_tags_json,
            forum_system_prompt = excluded.forum_system_prompt,
            forum_include_post_title = excluded.forum_include_post_title,
            forum_delivery_mode = excluded.forum_delivery_mode,
            quota_enabled = excluded.quota_enabled,
            quota_role_limits_json = excluded.quota_role_limits_json,
            quota_reset_frequency = excluded.quota_reset_frequency,
            quota_reset_time = excluded.quota_reset_time,
            quota_reset_tz_offset = excluded.quota_reset_tz_offset,
            quota_reset_day_of_week = excluded.quota_reset_day_of_week,
            quota_reset_day_of_month = excluded.quota_reset_day_of_month,
            quota_allow_partial_overage = excluded.quota_allow_partial_overage,
            quota_max_overage_tokens = excluded.quota_max_overage_tokens,
            updated_at = excluded.updated_at",
        params![
            guild_id as i64,
            c.default_model,
            c.system_prompt,
            serde_json::to_string(&c.reply_roles)?,
            serde_json::to_string(&c.reply_users)?,
            serde_json::to_string(&c.change_model_roles)?,
            serde_json::to_string(&c.change_model_users)?,
            serde_json::to_string(&c.change_permission_roles)?,
            serde_json::to_string(&c.change_permission_users)?,
            serde_json::to_string(&c.search_roles)?,
            serde_json::to_string(&c.search_users)?,
            i64::from(c.web_search_enabled),
            i64::from(c.web_search_auto),
            i64::from(c.pagination_enabled),
            c.pagination_max_chars as i64,
            c.non_paginated_max_chars as i64,
            c.random_reply_mode,
            i64::from(c.random_reply_value),
            i64::from(c.memory_turns),
            i64::from(c.memory_ttl_minutes),
            i64::from(c.forum_auto_reply_enabled),
            serde_json::to_string(&c.forum_monitored_channels)?,
            serde_json::to_string(&c.forum_auto_reply_tags)?,
            serde_json::to_string(&c.forum_ignore_tags)?,
            c.forum_system_prompt,
            i64::from(c.forum_include_post_title),
            c.forum_delivery_mode,
            i64::from(c.token_quotas.enabled),
            serde_json::to_string(&c.token_quotas.role_limits)?,
            c.token_quotas.reset_frequency,
            c.token_quotas.reset_time,
            i64::from(c.token_quotas.reset_timezone_offset),
            i64::from(c.token_quotas.reset_day_of_week),
            i64::from(c.token_quotas.reset_day_of_month),
            i64::from(c.token_quotas.overage_policy.allow_partial_overage),
            c.token_quotas.overage_policy.max_overage_tokens,
            now,
        ],
    )?;
    Ok(())
}

#[cfg(unix)]
fn set_secure_dir_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_secure_dir_permissions(_path: &Path) {}

#[cfg(unix)]
fn set_secure_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_secure_file_permissions(_path: &Path) {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_storage_guild_patch_and_usage_ledger() {
        let storage = Storage::open(":memory:").expect("open in-memory db");
        let gid = 1279634736951853126u64;

        let patch = GuildConfigPatch {
            system_prompt: Some("Test Persona".to_owned()),
            memory_turns: Some(12),
            ..Default::default()
        };
        let updated = storage.patch_guild_config(gid, patch).expect("patch guild");
        assert_eq!(updated.system_prompt, "Test Persona");
        assert_eq!(updated.memory_turns, 12);

        let now = Utc::now();
        storage
            .record_token_usage(&TokenUsageEntry {
                guild_id: gid,
                user_id: 999888777,
                total_tokens: 450,
                prompt_tokens: 150,
                completion_tokens: 300,
                model: "mimo-v2.6-pro".to_owned(),
                recorded_at: now.timestamp(),
            })
            .expect("record usage");

        let used = storage
            .get_user_token_usage(
                gid,
                999888777,
                now - Duration::hours(1),
                now + Duration::hours(1),
            )
            .expect("query usage");
        assert_eq!(used, 450);

        // Verify /forget_me anonymization preserves cycle quota sum without keeping raw user_id
        let count = storage.anonymize_user_ledger(999888777).expect("anonymize");
        assert_eq!(count, 1);

        let used_after = storage
            .get_user_token_usage(
                gid,
                999888777,
                now - Duration::hours(1),
                now + Duration::hours(1),
            )
            .expect("query usage after anonymize");
        assert_eq!(used_after, 450);
    }

    #[test]
    fn test_redacted_backup_scrubs_secrets() {
        let temp_dir = std::env::temp_dir().join(format!("silas_test_{}", rand::random::<u64>()));
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let db_path = temp_dir.join("silas.db");
        let backup_path = temp_dir.join("redacted.db");

        let storage = Storage::open(&db_path).expect("open file db");
        storage
            .update_app_settings(AppSettingsPatch {
                discord_token: Some("SUPER_SECRET_BOT_TOKEN".to_owned()),
                remote_api_key: Some("SK_SECRET_REMOTE_KEY".to_owned()),
                ..Default::default()
            })
            .expect("save secrets");

        storage
            .create_redacted_backup(&backup_path)
            .expect("create redacted backup");

        let verify_storage = Storage::open(&backup_path).expect("open redacted backup");
        let app = verify_storage.get_app_settings();
        assert!(app.discord_token.is_empty());
        assert!(app.remote_api_key.is_empty());

        drop(verify_storage);
        drop(storage);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_reddit_subscriptions_storage() {
        let temp_dir = std::env::temp_dir().join(format!("silas_test_reddit_{}", rand::random::<u64>()));
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let db_path = temp_dir.join("silas.db");

        let storage = Storage::open(&db_path).expect("open file db");

        let new_sub = NewRedditSubscription {
            guild_id: 12345,
            channel_id: 67890,
            subreddit: "r/rust".to_string(),
            filter_nsfw: true,
            min_score: 10,
            flair_filter: Some("News".to_string()),
            keyword_filter: None,
            backfill_count: 0,
        };

        let created = storage
            .create_reddit_subscription(&new_sub, Some("initial_post_id"))
            .expect("create subscription");

        assert_eq!(created.subreddit, "rust");
        assert_eq!(created.guild_id, 12345);
        assert_eq!(created.channel_id, 67890);

        let list = storage
            .list_reddit_subscriptions_for_guild(12345)
            .expect("list subs");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, created.id);

        let matching = storage
            .find_subscriptions_for_subreddit("rust")
            .expect("find subs");
        assert_eq!(matching.len(), 1);

        // Dedup check
        assert!(!storage.has_reddit_post_been_delivered(created.id, "post_1").expect("dedup check"));
        storage.record_reddit_post_delivered(created.id, "post_1").expect("record delivered");
        assert!(storage.has_reddit_post_been_delivered(created.id, "post_1").expect("dedup check"));

        // Pruning
        let pruned = storage.prune_old_reddit_posts(0).expect("prune");
        assert_eq!(pruned, 1);

        // Delete
        let deleted = storage.delete_reddit_subscription(created.id, 12345).expect("delete sub");
        assert!(deleted);
        let list_empty = storage
            .list_reddit_subscriptions_for_guild(12345)
            .expect("list subs");
        assert_eq!(list_empty.len(), 0);

        drop(storage);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_v3_schema_migration_and_operations() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = std::env::temp_dir().join(format!("silas_test_v3_{}", rand::random::<u64>()));
        fs::create_dir_all(&temp_dir)?;
        let db_path = temp_dir.join("silas.db");

        // 1. Setup a v2 database by opening initially, then simulating a v2 state
        {
            let storage = Storage::open(&db_path)?;
            let conn = storage.writer.lock();
            conn.execute_batch(
                "UPDATE schema_migrations SET version = 2;
                 DROP TABLE IF EXISTS mod_cases;
                 DROP TABLE IF EXISTS mod_strikes;
                 DROP TABLE IF EXISTS automod_configs;
                 DROP TABLE IF EXISTS logging_configs;
                 DROP TABLE IF EXISTS greetings_configs;
                 DROP TABLE IF EXISTS reaction_role_entries;
                 DROP TABLE IF EXISTS reaction_role_groups;
                 DROP TABLE IF EXISTS custom_triggers;",
            )?;
        }

        // 2. Open via Storage to trigger v3 migration
        let storage = Storage::open(&db_path)?;

        // Verify schema version is now 3
        {
            let conn = storage.writer.lock();
            let current_v: u32 = conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))?;
            assert_eq!(current_v, 3);
        }

        let guild_id = 999_888_777u64;
        let user_id = 111_222_333u64;
        let mod_id = 444_555_666u64;

        // 3. Test Mod Cases
        let new_case = NewModCase {
            guild_id,
            user_id,
            moderator_id: mod_id,
            action_type: "tempban".to_string(),
            reason: "Excessive spam".to_string(),
            duration_seconds: Some(3600),
            created_at: Some(1_700_000_000),
            expires_at: Some(1_700_003_600),
        };
        let created_case = storage.create_mod_case(&new_case)?;
        assert_eq!(created_case.case_number, 1);
        assert_eq!(created_case.guild_id, guild_id);
        assert!(created_case.active);

        let fetched_case = storage.get_mod_case(guild_id, 1)?;
        assert!(fetched_case.is_some());
        if let Some(c) = fetched_case {
            assert_eq!(c.reason, "Excessive spam");
            assert_eq!(c.action_type, "tempban");
        }

        let user_cases = storage.get_mod_cases_for_user(guild_id, user_id)?;
        assert_eq!(user_cases.len(), 1);

        let expired_cases = storage.get_active_expired_cases(1_700_005_000)?;
        assert_eq!(expired_cases.len(), 1);

        let reason_updated = storage.update_case_reason(guild_id, 1, "Updated reason")?;
        assert!(reason_updated);

        let deactivated = storage.deactivate_case(created_case.id)?;
        assert!(deactivated);
        let expired_after_deact = storage.get_active_expired_cases(1_700_005_000)?;
        assert!(expired_after_deact.is_empty());

        // 4. Test Strikes
        let new_strike = NewModStrike {
            guild_id,
            user_id,
            case_id: Some(created_case.id),
            strike_count: 2,
            reason: "First strike warning".to_string(),
            created_at: Some(1_700_000_000),
            expires_at: Some(1_700_010_000),
        };
        let created_strike = storage.add_strike(&new_strike)?;
        assert_eq!(created_strike.strike_count, 2);
        assert!(!created_strike.decayed);

        let active_count = storage.get_active_strikes_count(guild_id, user_id)?;
        assert_eq!(active_count, 2);

        let user_strikes = storage.get_user_strikes(guild_id, user_id)?;
        assert_eq!(user_strikes.len(), 1);

        let decayed = storage.decay_strikes(guild_id, 1_700_020_000)?;
        assert_eq!(decayed, 1);
        let count_after_decay = storage.get_active_strikes_count(guild_id, user_id)?;
        assert_eq!(count_after_decay, 0);

        // 5. Test Automod Config
        let default_am = storage.get_automod_config(guild_id)?;
        assert!(default_am.enabled);
        assert!(default_am.anti_spam_enabled);

        let mut custom_am = default_am;
        custom_am.anti_spam_max_messages = 8;
        custom_am.blocked_words.push("badterm".to_string());
        storage.save_automod_config(&custom_am)?;

        let reloaded_am = storage.get_automod_config(guild_id)?;
        assert_eq!(reloaded_am.anti_spam_max_messages, 8);
        assert_eq!(reloaded_am.blocked_words, vec!["badterm".to_string()]);

        // 6. Test Logging Config
        let default_log = storage.get_logging_config(guild_id)?;
        assert!(default_log.log_bots);

        let mut custom_log = default_log;
        custom_log.mod_channel_id = Some(12345);
        custom_log.ignored_channels.push(9999);
        storage.save_logging_config(&custom_log)?;

        let reloaded_log = storage.get_logging_config(guild_id)?;
        assert_eq!(reloaded_log.mod_channel_id, Some(12345));
        assert_eq!(reloaded_log.ignored_channels, vec![9999]);

        // 7. Test Greetings Config
        let default_greet = storage.get_greetings_config(guild_id)?;
        assert!(!default_greet.welcome_dm_enabled);

        let mut custom_greet = default_greet;
        custom_greet.welcome_channel_id = Some(55555);
        custom_greet.welcome_message = "Welcome {user.name}!".to_string();
        custom_greet.member_autorole_ids.push(77777);
        storage.save_greetings_config(&custom_greet)?;

        let reloaded_greet = storage.get_greetings_config(guild_id)?;
        assert_eq!(reloaded_greet.welcome_channel_id, Some(55555));
        assert_eq!(reloaded_greet.welcome_message, "Welcome {user.name}!");
        assert_eq!(reloaded_greet.member_autorole_ids, vec![77777]);

        // 8. Test Reaction Roles
        let new_rr_group = NewReactionRoleGroup {
            guild_id,
            channel_id: 11111,
            message_id: 22222,
            group_type: "unique".to_string(),
            required_role_id: None,
            component_type: "button".to_string(),
        };
        let created_group = storage.create_reaction_role_group(&new_rr_group)?;
        assert_eq!(created_group.group_type, "unique");

        let entry = NewReactionRoleEntry {
            group_id: created_group.id,
            emoji_or_custom_id: "color_red".to_string(),
            role_id: 88888,
            button_label: Some("Red Team".to_string()),
            button_style: Some("danger".to_string()),
        };
        let created_entry = storage.add_reaction_role_entry(&entry)?;
        assert_eq!(created_entry.emoji_or_custom_id, "color_red");

        let fetched_rr = storage.get_reaction_roles_for_message(guild_id, 22222)?;
        assert!(fetched_rr.is_some());
        if let Some(g) = fetched_rr {
            assert_eq!(g.entries.len(), 1);
            if let Some(first_entry) = g.entries.first() {
                assert_eq!(first_entry.role_id, 88888);
            }
        }

        let all_guild_rr = storage.get_guild_reaction_roles(guild_id)?;
        assert_eq!(all_guild_rr.len(), 1);

        // 9. Test Custom Triggers
        let new_trigger = NewCustomTrigger {
            guild_id,
            trigger_name: "!ping".to_string(),
            match_mode: "prefix".to_string(),
            response_template: "Pong! {user.mention}".to_string(),
            cooldown_seconds: 5,
            allowed_roles: vec![101010],
            ignored_channels: vec![],
            delete_trigger_message: true,
            reply_in_dm: false,
            enabled: true,
        };
        let created_trigger = storage.create_custom_trigger(&new_trigger)?;
        assert_eq!(created_trigger.trigger_name, "!ping");

        let fetched_trigger = storage.get_custom_trigger(guild_id, "!ping")?;
        assert!(fetched_trigger.is_some());
        if let Some(t) = fetched_trigger {
            assert_eq!(t.response_template, "Pong! {user.mention}");
            assert!(t.delete_trigger_message);
        }

        let trigger_list = storage.list_custom_triggers(guild_id)?;
        assert_eq!(trigger_list.len(), 1);

        let trigger_deleted = storage.delete_custom_trigger(guild_id, "!ping")?;
        assert!(trigger_deleted);
        assert!(storage.get_custom_trigger(guild_id, "!ping")?.is_none());

        // 10. Test Guild Purge deletes all module data
        storage.purge_guild_data(guild_id)?;
        assert!(storage.get_mod_cases_for_user(guild_id, user_id)?.is_empty());
        assert_eq!(storage.get_active_strikes_count(guild_id, user_id)?, 0);
        assert!(storage.get_guild_reaction_roles(guild_id)?.is_empty());

        drop(storage);
        let _ = fs::remove_dir_all(&temp_dir);
        Ok(())
    }
}
