//! Standalone migration utility (`silas-migrate`) converting legacy Silas Blue v3 configurations
//! (`config/*.json`, `config/bot_token.txt`, `config/web_secret.key`, and `state/usage.db`)
//! or Carl.gg bot backup exports into a clean-room unified SQLite database (`data/silas.db`).

mod carl;

use clap::{Parser, Subcommand};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use silas_core::config::{AppSettings, GuildConfig};
use silas_storage::{Storage, TokenUsageEntry};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "silas-migrate",
    version = env!("CARGO_PKG_VERSION"),
    about = "Silas Blue state and configuration migration suite"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to the legacy v3 root directory (for legacy v3 migration without subcommand)
    #[arg(long, default_value = ".")]
    from: PathBuf,

    /// Destination path for the v4 unified SQLite database
    #[arg(long, default_value = "data/silas.db")]
    to: PathBuf,

    /// Parse and validate without writing to disk
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Overwrite the destination database if it already exists
    #[arg(long, default_value_t = false)]
    force: bool,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Migrate legacy Silas Blue v3 JSON/SQLite state into a unified SQLite database
    V3(MigrateArgs),
    /// Import configuration from a Carl.gg bot JSON backup
    ImportCarl(CarlArgs),
}

#[derive(Debug, clap::Args)]
struct MigrateArgs {
    /// Path to the legacy v3 root directory (containing `config/` and/or `state/`)
    #[arg(long, default_value = ".")]
    from: PathBuf,

    /// Destination path for the v4 unified SQLite database
    #[arg(long, default_value = "data/silas.db")]
    to: PathBuf,

    /// Parse and validate all legacy v3 files without writing to disk
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Overwrite the destination database if it already exists
    #[arg(long, default_value_t = false)]
    force: bool,
}

#[derive(Debug, clap::Args)]
struct CarlArgs {
    /// Path to the Carl.gg backup JSON file
    #[arg(long)]
    file: PathBuf,

    /// Target Guild ID (e.g. 295767175426473984)
    #[arg(long, default_value_t = 0)]
    guild_id: u64,

    /// Target SQLite database path (default: data/silas.db)
    #[arg(long, default_value = "data/silas.db")]
    db: PathBuf,

    /// Parse and validate without writing to database
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let res = match cli.command {
        Some(Commands::V3(args)) => run_v3_migration(args).await,
        Some(Commands::ImportCarl(args)) => run_carl_import(args),
        None => {
            let args = MigrateArgs {
                from: cli.from,
                to: cli.to,
                dry_run: cli.dry_run,
                force: cli.force,
            };
            run_v3_migration(args).await
        }
    };

    if let Err(e) = res {
        eprintln!("Migration error: {e}");
        std::process::exit(1);
    }
}

fn run_carl_import(args: CarlArgs) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Carl.gg Configuration Importer ===");
    println!("Reading backup file: {}", args.file.display());
    let raw = std::fs::read_to_string(&args.file)?;
    let parsed = carl::parse_carl_backup(&raw, args.guild_id)?;
    println!("Target Guild ID: {}", parsed.guild_id);
    println!(
        "Mode: {}",
        if args.dry_run {
            "DRY RUN (Simulated, no changes written)"
        } else {
            "LIVE EXECUTION"
        }
    );

    let report = carl::execute_carl_import(&parsed, &args.db, args.dry_run)?;

    println!("\n--- Import Summary ---");
    println!("Guild ID: {}", report.guild_id);
    println!(
        "Welcome / Greetings: {} (Channel: {:?})",
        if report.welcome_configured {
            "Configured"
        } else {
            "None"
        },
        report.welcome_channel_id
    );
    println!(
        "Logging Channel: {} (Channel: {:?})",
        if report.logging_configured {
            "Configured"
        } else {
            "None"
        },
        report.logging_channel_id
    );
    println!("Automod Blocked Words: {}", report.automod_words_count);
    println!(
        "Automod Ignored Channels: {}",
        report.automod_ignored_channels_count
    );
    println!("Automod Ignored Roles: {}", report.automod_ignored_roles_count);
    println!(
        "Reaction Role Groups: {} (Entries: {})",
        report.reaction_role_groups_count, report.reaction_role_entries_count
    );
    println!("Custom Triggers: {}", report.triggers_count);
    println!("Tags: {}", report.tags_count);
    println!(
        "Total Trigger Rules: {}",
        report.triggers_count + report.tags_count
    );

    if args.dry_run {
        println!("\n[DRY RUN COMPLETE] Zero database changes applied.");
    } else {
        println!(
            "\n[SUCCESS] All settings successfully imported into {}!",
            args.db.display()
        );
    }

    Ok(())
}

async fn run_v3_migration(args: MigrateArgs) -> Result<(), Box<dyn std::error::Error>> {

    let config_dir = args.from.join("config");
    let state_dir = args.from.join("state");

    if !config_dir.exists() && !state_dir.exists() {
        return Err(format!(
            "Source directory '{}' does not contain a 'config/' or 'state/' folder.",
            args.from.display()
        )
        .into());
    }

    if !args.dry_run && args.to.exists() && !args.force {
        return Err(format!(
            "Destination database '{}' already exists. Pass --force to overwrite or --dry-run to preview.",
            args.to.display()
        )
        .into());
    }

    // 1. Parse AppSettings from config/app_config.json, config/bot_token.txt, config/web_secret.key
    let mut app_settings = AppSettings::default();

    let token_path = config_dir.join("bot_token.txt");
    if token_path.exists() {
        if let Ok(raw_tok) = std::fs::read_to_string(&token_path) {
            let trimmed = raw_tok.trim();
            if !trimmed.is_empty() {
                app_settings.discord_token = trimmed.to_string();
            }
        }
    }

    let secret_path = config_dir.join("web_secret.key");
    if secret_path.exists() {
        if let Ok(raw_sec) = std::fs::read(&secret_path) {
            let trimmed = raw_sec.strip_suffix(b"\n").unwrap_or(&raw_sec);
            let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
            if trimmed.len() >= 32 {
                app_settings.web_secret_key = trimmed.to_vec();
            }
        }
    }

    let app_cfg_path = config_dir.join("app_config.json");
    if app_cfg_path.exists() {
        let raw = std::fs::read_to_string(&app_cfg_path)?;
        let val: Value = serde_json::from_str(&raw)?;
        map_v3_app_config(&val, &mut app_settings);
    }

    // 2. Parse GuildConfigs from config/<guild_id>.json
    let mut guild_configs: Vec<(u64, GuildConfig)> = Vec::new();
    if config_dir.exists() {
        for entry in std::fs::read_dir(&config_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if stem.eq_ignore_ascii_case("app_config") || stem.eq_ignore_ascii_case("example") {
                continue;
            }

            let guild_id = if stem.eq_ignore_ascii_case("default_config") {
                0u64
            } else if let Ok(id) = stem.parse::<u64>() {
                id
            } else {
                continue;
            };

            let raw = std::fs::read_to_string(&path)?;
            let val: Value = serde_json::from_str(&raw)?;
            let cfg = map_v3_guild_config(&val);
            guild_configs.push((guild_id, cfg));
        }
    }
    guild_configs.sort_by_key(|(gid, _)| *gid);

    // 3. Read legacy token usage records from state/usage.db (Strictly Read-Only)
    let usage_db_path = state_dir.join("usage.db");
    let usage_entries = if usage_db_path.exists() {
        read_v3_usage_ledger(&usage_db_path)?
    } else {
        Vec::new()
    };

    println!("========================================================================");
    println!("  Silas Blue v3 -> v4 Migration Summary");
    println!("  Source Directory      : {}", args.from.display());
    println!("  Destination DB        : {}", args.to.display());
    println!(
        "  Bot Token Present     : {}",
        !app_settings.discord_token.is_empty()
    );
    println!("  Active AI Provider    : {}", app_settings.backend_type);
    println!(
        "  Active Model          : {}",
        app_settings.effective_global_model()
    );
    println!("  Bot Owner ID          : {}", app_settings.bot_owner_id);
    println!("  Server Configs Found  : {}", guild_configs.len());
    println!("  Token Usage Records   : {}", usage_entries.len());
    println!("========================================================================");

    if args.dry_run {
        println!("Dry run complete. No files were created or modified.");
        return Ok(());
    }

    // 4. Write atomically via temporary SQLite file `<to>.tmp`, verify integrity, and rename
    if let Some(parent) = args.to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_db = args.to.with_extension("db.tmp");
    let tmp_wal = PathBuf::from(format!("{}-wal", tmp_db.display()));
    let tmp_shm = PathBuf::from(format!("{}-shm", tmp_db.display()));
    let _ = std::fs::remove_file(&tmp_db);
    let _ = std::fs::remove_file(&tmp_wal);
    let _ = std::fs::remove_file(&tmp_shm);

    {
        let storage = Storage::open(&tmp_db)?;
        storage.replace_app_settings(app_settings)?;

        for (gid, gcfg) in guild_configs {
            storage.upsert_guild_config(gid, gcfg)?;
        }

        for item in &usage_entries {
            storage.record_token_usage(item)?;
        }

        storage.checkpoint_wal()?;
    }

    // Verify PRAGMA integrity_check on the staged database
    {
        let verify_conn = Connection::open_with_flags(&tmp_db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let integrity: String = verify_conn.query_row("PRAGMA integrity_check;", [], |r| r.get(0))?;
        if !integrity.eq_ignore_ascii_case("ok") {
            return Err(format!("SQLite integrity check failed: {integrity}").into());
        }
    }

    let dest_wal = PathBuf::from(format!("{}-wal", args.to.display()));
    let dest_shm = PathBuf::from(format!("{}-shm", args.to.display()));
    let _ = std::fs::remove_file(&dest_wal);
    let _ = std::fs::remove_file(&dest_shm);
    if args.to.exists() {
        std::fs::remove_file(&args.to)?;
    }
    std::fs::rename(&tmp_db, &args.to)?;
    let _ = std::fs::remove_file(&tmp_wal);
    let _ = std::fs::remove_file(&tmp_shm);

    println!(
        "Successfully migrated v3 configuration and state into '{}'.",
        args.to.display()
    );
    Ok(())
}

fn map_v3_app_config(val: &Value, out: &mut AppSettings) {
    if let Some(s) = get_str(val, &["backend_type", "ai_provider", "provider"]) {
        out.backend_type = s;
    }
    if let Some(s) = get_str(val, &["active_model", "default_model"]) {
        out.active_model = s;
    }
    if let Some(s) = get_str(val, &["api_key", "local_api_key"]) {
        out.local_api_key = s;
    }
    if let Some(s) = get_str(val, &["ollama_url"]) {
        out.ollama_url = s;
    }
    if let Some(s) = get_str(val, &["llamacpp_url"]) {
        out.llamacpp_url = s;
    }
    if let Some(s) = get_str(val, &["remote_preset", "remote_provider"]) {
        out.remote_preset = s;
    }
    if let Some(s) = get_str(val, &["remote_base_url"]) {
        out.remote_base_url = s;
    }
    if let Some(s) = get_str(val, &["remote_api_key"]) {
        if !s.is_empty() {
            out.remote_api_key = s;
        }
    }
    if let Some(s) = get_str(val, &["remote_model"]) {
        out.remote_model = s;
    }
    if let Some(s) = get_str(val, &["system_prompt"]) {
        out.system_prompt = s;
    }
    if let Some(b) = val.get("allow_dms").and_then(|v| v.as_bool()) {
        out.allow_dms = b;
    }
    if let Some(s) = get_str(val, &["dm_default_model"]) {
        out.dm_default_model = s;
    }
    if let Some(s) = get_str(val, &["dm_disabled_message"]) {
        out.dm_disabled_message = s;
    }
    if let Some(n) = val.get("dm_memory_turns").and_then(|v| v.as_u64()) {
        out.dm_memory_turns = (n as u32).min(50);
    }
    if let Some(n) = val.get("dm_memory_ttl_minutes").and_then(|v| v.as_u64()) {
        out.dm_memory_ttl_minutes = (n as u32).clamp(1, 1440);
    }
    if let Some(b) = val.get("web_search_enabled").and_then(|v| v.as_bool()) {
        out.web_search_enabled = b;
    }
    if let Some(b) = val.get("web_search_auto").and_then(|v| v.as_bool()) {
        out.web_search_auto = b;
    }
    if let Some(s) = get_str(val, &["web_search_provider", "search_provider"]) {
        out.web_search_provider = s;
    }
    if let Some(n) = val.get("web_search_max_results").and_then(|v| v.as_u64()) {
        out.web_search_max_results = (n as u32).clamp(1, 10);
    }
    if let Some(s) = get_str(val, &["tavily_api_key"]) {
        out.tavily_api_key = s;
    }
    if let Some(s) = get_str(val, &["brave_api_key"]) {
        out.brave_api_key = s;
    }
    if let Some(s) = get_str(val, &["discord_oauth_client_id", "discord_client_id"]) {
        out.discord_oauth_client_id = s;
    }
    if let Some(s) = get_str(
        val,
        &["discord_oauth_client_secret", "discord_client_secret"],
    ) {
        out.discord_oauth_client_secret = s;
    }
    if let Some(s) = get_str(val, &["bot_owner_id"]) {
        out.bot_owner_id = s;
    } else if let Some(n) = val.get("bot_owner_id").and_then(|v| v.as_u64()) {
        out.bot_owner_id = n.to_string();
    }
    if let Some(s) = get_str(val, &["theme"]) {
        out.theme = s;
    }
    if let Some(b) = val.get("debug").and_then(|v| v.as_bool()) {
        out.debug = b;
    }
    if let Some(s) = get_str(val, &["update_channel"]) {
        out.update_channel = s;
    }
    if let Some(b) = val.get("auto_update_enabled").and_then(|v| v.as_bool()) {
        out.auto_update_enabled = b;
    }
    if let Some(s) = get_str(val, &["auto_update_mode"]) {
        out.auto_update_mode = s;
    }
    if let Some(n) = val
        .get("auto_update_interval_hours")
        .and_then(|v| v.as_u64())
    {
        out.auto_update_interval_hours = (n as u32).clamp(1, 720);
    }
}

fn map_v3_guild_config(val: &Value) -> GuildConfig {
    let mut cfg = GuildConfig::default();

    if let Some(s) = get_str(val, &["default_model", "model"]) {
        cfg.default_model = s;
    }
    if let Some(s) = get_str(val, &["system_prompt"]) {
        if !s.is_empty() {
            cfg.system_prompt = s;
        }
    }
    if let Some(b) = val.get("web_search_enabled").and_then(|v| v.as_bool()) {
        cfg.web_search_enabled = b;
    }
    if let Some(b) = val.get("web_search_auto").and_then(|v| v.as_bool()) {
        cfg.web_search_auto = b;
    }
    if let Some(turns) = val.get("memory_turns").and_then(|v| v.as_u64()) {
        cfg.memory_turns = (turns as u32).min(50);
    }
    if let Some(ttl) = val.get("memory_ttl_minutes").and_then(|v| v.as_u64()) {
        cfg.memory_ttl_minutes = (ttl as u32).clamp(1, 1440);
    }
    if let Some(b) = val.get("pagination_enabled").and_then(|v| v.as_bool()) {
        cfg.pagination_enabled = b;
    }
    if let Some(chars) = val.get("pagination_max_chars").and_then(|v| v.as_u64()) {
        cfg.pagination_max_chars = (chars as usize).clamp(500, 1900);
    }
    if let Some(chars) = val.get("non_paginated_max_chars").and_then(|v| v.as_u64()) {
        cfg.non_paginated_max_chars = chars as usize;
    }

    // Role & user permission lists
    if let Some(list) = get_string_vec(val, "reply_roles") {
        cfg.reply_roles = list;
    }
    if let Some(list) = get_string_vec(val, "reply_users") {
        cfg.reply_users = list;
    }
    if let Some(list) = get_string_vec(val, "change_model_roles") {
        cfg.change_model_roles = list;
    }
    if let Some(list) = get_string_vec(val, "change_model_users") {
        cfg.change_model_users = list;
    }
    if let Some(list) = get_string_vec(val, "change_permission_roles") {
        cfg.change_permission_roles = list;
    }
    if let Some(list) = get_string_vec(val, "change_permission_users") {
        cfg.change_permission_users = list;
    }
    if let Some(list) = get_string_vec(val, "search_roles") {
        cfg.search_roles = list;
    }
    if let Some(list) = get_string_vec(val, "search_users") {
        cfg.search_users = list;
    }

    // Spontaneous reply settings (supporting both v3 random_prompt_* and v3.5.9+ random_reply_*)
    if let Some(b) = val.get("random_prompt_enabled").and_then(|v| v.as_bool()) {
        cfg.random_prompt_enabled = b;
    }
    if let Some(p) = val
        .get("random_prompt_probability")
        .and_then(|v| v.as_u64())
    {
        cfg.random_prompt_probability = (p as u32).min(100);
    }
    if let Some(mode) = get_str(val, &["random_reply_mode"]) {
        cfg.random_reply_mode = mode;
    } else if cfg.random_prompt_enabled && cfg.random_prompt_probability > 0 {
        cfg.random_reply_mode = "probability".to_string();
        cfg.random_reply_value = cfg.random_prompt_probability;
    }
    if let Some(rv) = val.get("random_reply_value").and_then(|v| v.as_u64()) {
        cfg.random_reply_value = rv as u32;
    }

    // Forum auto-reply settings
    if let Some(b) = val
        .get("forum_auto_reply_enabled")
        .or_else(|| val.get("forum_auto_reply"))
        .and_then(|v| v.as_bool())
    {
        cfg.forum_auto_reply_enabled = b;
    }
    if let Some(list) = get_string_vec(val, "forum_monitored_channels") {
        cfg.forum_monitored_channels = list;
    }
    if let Some(list) = get_string_vec(val, "forum_auto_reply_tags") {
        cfg.forum_auto_reply_tags = list;
    }
    if let Some(list) = get_string_vec(val, "forum_ignore_tags") {
        cfg.forum_ignore_tags = list;
    }
    if let Some(s) = get_str(val, &["forum_system_prompt"]) {
        cfg.forum_system_prompt = s;
    }
    if let Some(b) = val.get("forum_include_post_title").and_then(|v| v.as_bool()) {
        cfg.forum_include_post_title = b;
    }
    if let Some(s) = get_str(val, &["forum_delivery_mode"]) {
        cfg.forum_delivery_mode = s;
    }

    // Token quota settings
    if let Some(tq_obj) = val.get("token_quotas").and_then(|v| v.as_object()) {
        if let Some(b) = tq_obj.get("enabled").and_then(|v| v.as_bool()) {
            cfg.token_quotas.enabled = b;
        }
        if let Some(rl_obj) = tq_obj.get("role_limits").and_then(|v| v.as_object()) {
            let mut map = HashMap::new();
            for (k, v) in rl_obj {
                if let Some(lim) = v.as_i64() {
                    map.insert(k.clone(), lim);
                }
            }
            if !map.is_empty() {
                cfg.token_quotas.role_limits = map;
            }
        }
        if let Some(freq) = tq_obj.get("reset_frequency").and_then(|v| v.as_str()) {
            cfg.token_quotas.reset_frequency = freq.to_string();
        }
        if let Some(rt) = tq_obj.get("reset_time").and_then(|v| v.as_str()) {
            cfg.token_quotas.reset_time = rt.to_string();
        }
        if let Some(tz) = tq_obj.get("reset_timezone_offset").and_then(|v| v.as_i64()) {
            cfg.token_quotas.reset_timezone_offset = tz as i32;
        }
        if let Some(dow) = tq_obj.get("reset_day_of_week").and_then(|v| v.as_u64()) {
            cfg.token_quotas.reset_day_of_week = dow as u32;
        }
        if let Some(dom) = tq_obj.get("reset_day_of_month").and_then(|v| v.as_u64()) {
            cfg.token_quotas.reset_day_of_month = dom as u32;
        }
        if let Some(ov_obj) = tq_obj.get("overage_policy").and_then(|v| v.as_object()) {
            if let Some(b) = ov_obj.get("allow_partial_overage").and_then(|v| v.as_bool()) {
                cfg.token_quotas.overage_policy.allow_partial_overage = b;
            }
            if let Some(m) = ov_obj.get("max_overage_tokens").and_then(|v| v.as_i64()) {
                cfg.token_quotas.overage_policy.max_overage_tokens = m;
            }
        }
    }

    cfg.sanitize_and_compile();
    cfg
}

fn read_v3_usage_ledger(path: &Path) -> Result<Vec<TokenUsageEntry>, rusqlite::Error> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut rows_out = Vec::new();

    // Check if v3 `token_usage` or `token_records` table exists
    let has_token_usage: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='token_usage')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);

    if has_token_usage {
        let mut stmt = conn.prepare(
            "SELECT guild_id, user_id, tokens_used, prompt_tokens, completion_tokens, COALESCE(model, ''), timestamp
             FROM token_usage WHERE tokens_used > 0",
        )?;
        let mapped = stmt.query_map([], |row| {
            let gid_str: String = row.get(0)?;
            let uid_str: String = row.get(1)?;
            let total: i64 = row.get(2)?;
            let prompt: i64 = row.get(3).unwrap_or(0);
            let completion: i64 = row.get(4).unwrap_or(0);
            let model: String = row.get(5)?;
            let ts: f64 = row.get(6)?;
            Ok(TokenUsageEntry {
                guild_id: gid_str.trim().parse::<u64>().unwrap_or(0),
                user_id: uid_str.trim().parse::<u64>().unwrap_or(0),
                total_tokens: total.max(0),
                prompt_tokens: prompt.max(0),
                completion_tokens: completion.max(0),
                model,
                recorded_at: ts as i64,
            })
        })?;
        for r in mapped.flatten() {
            if r.guild_id > 0 && r.user_id > 0 && r.total_tokens > 0 {
                rows_out.push(r);
            }
        }
    }

    Ok(rows_out)
}

fn get_str(val: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = val.get(*k).and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn get_string_vec(val: &Value, key: &str) -> Option<Vec<String>> {
    let arr = val.get(key)?.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(s) = item.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        } else if let Some(n) = item.as_u64() {
            out.push(n.to_string());
        }
    }
    Some(out)
}
