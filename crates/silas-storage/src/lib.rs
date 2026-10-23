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

pub const CURRENT_SCHEMA_VERSION: u32 = 1;
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

    /// Discord ToS `GUILD_DELETE` compliance: deletes `guild_configs` and `token_usage_ledger`
    /// rows when Silas Blue is removed from a server.
    pub fn purge_guild_data(&self, guild_id: u64) -> Result<(), StorageError> {
        self.guild_cache.remove(&guild_id);
        let gid_i64 = guild_id as i64;
        let conn = self.writer.lock();
        conn.execute("DELETE FROM guild_configs WHERE guild_id = ?1", params![gid_i64])?;
        conn.execute(
            "DELETE FROM token_usage_ledger WHERE guild_id = ?1",
            params![gid_i64],
        )?;
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
                 brave_api_key = '';
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

    if let Some(v) = max_ver {
        if v > CURRENT_SCHEMA_VERSION {
            return Err(StorageError::NewerSchemaVersion {
                db_version: v,
                max_supported: CURRENT_SCHEMA_VERSION,
            });
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
            update_notify_owner_dm, update_notify_channel_id
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
            update_notify_channel_id = ?36
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
}
