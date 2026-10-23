//! Complete 23-command hybrid (`/` slash + `!` / `@Bot` prefix) command suite for Silas Blue.

use crate::bot::BotState;
use poise::serenity_prelude as serenity;
use serenity::all::EditProfile;
use silas_ai::CompletionRequest;
use silas_core::config::{
    compose_system_prompt, AppSettingsPatch, GuildConfig, GuildConfigPatch, TokenQuotaPatch,
};
use silas_core::memory::ContextKey;
use silas_core::permissions::{
    has_dm_permission, has_guild_permission, MemberContext, PermissionDomain,
};
use silas_core::quota::{calculate_window_period, evaluate_quota};
use silas_core::text::truncate_chars;
use silas_search::grounding::{format_grounded_rag_block, is_refusal_response};
use silas_search::intent::{build_search_query, should_auto_search};
use std::sync::Arc;

pub type CommandError = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Arc<BotState>, CommandError>;

/// Return the full vector of registered Poise commands.
pub fn all_commands() -> Vec<poise::Command<Arc<BotState>, CommandError>> {
    vec![
        models(),
        setmodel(),
        clear(),
        forget_me(),
        privacy(),
        recall(),
        setmemory(),
        setttl(),
        search(),
        links(),
        autosearch(),
        setwebsearch(),
        setprompt(),
        setpagination(),
        setnick(),
        setforum(),
        quota(),
        config(),
        ping(),
        invite(),
        help(),
        setglobalname(),
        update(),
    ]
}

struct CallerRoleInfo {
    role_ids: Vec<u64>,
    role_names: Vec<String>,
    guild_role_map: Vec<(u64, String)>,
    is_admin: bool,
    is_owner_or_bot_owner: bool,
}

async fn extract_caller_roles(ctx: Context<'_>, bot_owner_id: &str) -> CallerRoleInfo {
    let mut role_ids = Vec::new();
    let mut role_names = Vec::new();
    let mut guild_role_map = Vec::new();
    let mut is_admin = false;
    let user_id = ctx.author().id.get();
    let mut is_owner_or_bot_owner =
        !bot_owner_id.trim().is_empty() && bot_owner_id.trim() == user_id.to_string();

    if let Some(member) = ctx.author_member().await {
        for r in &member.roles {
            role_ids.push(r.get());
        }
        if let Some(perms) = member.permissions {
            if perms.administrator() || perms.manage_guild() {
                is_admin = true;
            }
        }
    }

    if let Some(gid) = ctx.guild_id() {
        if let Some(guild) = ctx.serenity_context().cache.guild(gid) {
            if guild.owner_id == ctx.author().id {
                is_owner_or_bot_owner = true;
            }
            for (rid, role) in &guild.roles {
                guild_role_map.push((rid.get(), role.name.clone()));
            }
            for rid in &role_ids {
                if let Some(role) = guild.roles.get(&serenity::RoleId::new(*rid)) {
                    role_names.push(role.name.clone());
                    if role.permissions.administrator() || role.permissions.manage_guild() {
                        is_admin = true;
                    }
                }
            }
        }
    }

    CallerRoleInfo {
        role_ids,
        role_names,
        guild_role_map,
        is_admin,
        is_owner_or_bot_owner,
    }
}

/// Helper to check whether the caller has permission for `domain` in the current context.
async fn check_caller_permission(
    ctx: Context<'_>,
    domain: PermissionDomain,
) -> Result<bool, CommandError> {
    let state = ctx.data();
    let settings = state.storage.get_app_settings();
    let user_id = ctx.author().id.get();

    let Some(guild_id) = ctx.guild_id() else {
        let is_bot_owner = settings.bot_owner_id.trim() == user_id.to_string();
        let allowed = is_bot_owner || has_dm_permission(settings.allow_dms, domain);
        if !allowed {
            ctx.say("⛔ You do not have permission to run this command in DMs.")
                .await?;
        }
        return Ok(allowed);
    };

    let cfg = state.storage.get_guild_config(guild_id.get());
    let info = extract_caller_roles(ctx, &settings.bot_owner_id).await;
    let member_ctx = MemberContext {
        user_id,
        is_guild_owner: info.is_owner_or_bot_owner,
        is_administrator: info.is_admin,
        role_ids: &info.role_ids,
        role_names: &info.role_names,
    };

    let allowed = has_guild_permission(&member_ctx, &cfg, domain);
    if !allowed {
        ctx.say("⛔ You do not have permission to run this command.")
            .await?;
    }
    Ok(allowed)
}

/// List available AI models on the active backend.
#[poise::command(slash_command, prefix_command)]
pub async fn models(ctx: Context<'_>) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ChangeModel).await? {
        return Ok(());
    }
    let _ = ctx.defer().await;

    let state = ctx.data();
    let settings = state.storage.get_app_settings();
    let guild_cfg = ctx.guild_id().map(|g| state.storage.get_guild_config(g.get()));
    let active_model = state
        .resolve_model_for_context(&settings, guild_cfg.as_deref())
        .await;

    let list = state.ai.list_models_cached(true).await;
    if !list.is_empty() {
        let mut out = format!(
            "**Available Models** (Backend: `{}` | Active: `{}`):\n",
            settings.backend_type, active_model
        );
        for m in list.iter().take(35) {
            let marker = if *m == active_model { "✅" } else { "•" };
            out.push_str(&format!("{marker} `{m}`\n"));
        }
        ctx.say(out).await?;
    } else {
        ctx.say(format!(
            "No models reported by backend `{}` (Active model: `{}`).",
            settings.backend_type, active_model
        ))
        .await?;
    }
    Ok(())
}

/// Set the active AI model for this server (or DM).
#[poise::command(slash_command, prefix_command)]
pub async fn setmodel(
    ctx: Context<'_>,
    #[description = "Model identifier (e.g. llama3.2:latest, gpt-4o-mini)"]
    #[rest]
    model_name: String,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ChangeModel).await? {
        return Ok(());
    }
    let clean = model_name.trim().to_string();
    if clean.is_empty() {
        ctx.say("⚠️ Please provide a model name (e.g. `/setmodel llama3.2:latest`).")
            .await?;
        return Ok(());
    }

    let state = ctx.data();
    let settings = state.storage.get_app_settings();

    if settings.backend_type.eq_ignore_ascii_case("remote") {
        let updated = state
            .storage
            .update_app_settings_async(AppSettingsPatch {
                remote_model: Some(clean.clone()),
                ..Default::default()
            })
            .await?;
        state.ai.reload(&updated);
        ctx.say(format!("✅ Updated remote model to `{clean}`.")).await?;
    } else if let Some(gid) = ctx.guild_id() {
        state
            .storage
            .patch_guild_config_async(
                gid.get(),
                GuildConfigPatch {
                    default_model: Some(clean.clone()),
                    ..Default::default()
                },
            )
            .await?;
        ctx.say(format!("✅ Set default model for this server to `{clean}`."))
            .await?;
    } else {
        state
            .storage
            .update_app_settings_async(AppSettingsPatch {
                dm_default_model: Some(clean.clone()),
                ..Default::default()
            })
            .await?;
        ctx.say(format!("✅ Set default DM model to `{clean}`.")).await?;
    }

    Ok(())
}

/// Clear conversation memory in the current channel.
#[poise::command(slash_command, prefix_command, aliases("reset", "forget"))]
pub async fn clear(ctx: Context<'_>) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ChatReply).await? {
        return Ok(());
    }
    let key = match ctx.guild_id() {
        Some(gid) => ContextKey::guild_channel(gid.get(), ctx.channel_id().get()),
        None => ContextKey::dm(ctx.author().id.get()),
    };
    ctx.data().memory.clear(key);
    ctx.say("🧹 Cleared conversation memory for this channel.").await?;
    Ok(())
}

/// Erase your messages from volatile memory and anonymize your token ledger records.
#[poise::command(slash_command, prefix_command)]
pub async fn forget_me(ctx: Context<'_>) -> Result<(), CommandError> {
    let user_id = ctx.author().id.get();
    let state = ctx.data();
    let evicted = state.memory.clear_user_all(user_id);
    let anonymized = state.storage.anonymize_user_ledger(user_id).unwrap_or(0);

    ctx.say(format!(
        "🛡️ **Privacy Erasure Complete**:\n• Removed **{evicted}** message turn(s) from volatile conversation memory across all channels.\n• Pseudonymized **{anonymized}** historical token quota ledger record(s)."
    ))
    .await?;
    Ok(())
}

/// View Silas Blue's data retention policy and privacy controls.
#[poise::command(slash_command, prefix_command)]
pub async fn privacy(ctx: Context<'_>) -> Result<(), CommandError> {
    let state = ctx.data();
    let cfg = ctx
        .guild_id()
        .map(|g| state.storage.get_guild_config(g.get()))
        .unwrap_or_else(|| Arc::new(GuildConfig::default()));

    let msg = format!(
        "🔒 **Silas Blue Privacy & Data Retention Policy**\n\
        • **Message Content**: Stored **exclusively in volatile RAM** (never written to disk or SQLite).\n\
        • **Current Channel Memory**: `{}` (Max `{}` turns, TTL: `{}` minutes).\n\
        • **Automatic Deletion**: Deleting a message in Discord, deleting a channel, or removing Silas Blue from a server immediately purges the corresponding memory and server records.\n\
        • **Self-Service Erasure**: Run `/clear` to wipe this channel's context, or `/forget_me` to erase all your messages across all channels and pseudonymize your token usage ledger.",
        if cfg.memory_turns > 0 { "Enabled" } else { "Disabled" },
        cfg.memory_turns,
        cfg.memory_ttl_minutes
    );
    ctx.say(msg).await?;
    Ok(())
}

/// Show conversation memory status and message count for the current channel.
#[poise::command(slash_command, prefix_command)]
pub async fn recall(ctx: Context<'_>) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ChatReply).await? {
        return Ok(());
    }
    let state = ctx.data();
    let cfg = ctx
        .guild_id()
        .map(|g| state.storage.get_guild_config(g.get()))
        .unwrap_or_else(|| Arc::new(GuildConfig::default()));

    let key = match ctx.guild_id() {
        Some(gid) => ContextKey::guild_channel(gid.get(), ctx.channel_id().get()),
        None => ContextKey::dm(ctx.author().id.get()),
    };
    let hist = state
        .memory
        .get_history(key, cfg.memory_turns, Some(cfg.memory_ttl_minutes));

    ctx.say(format!(
        "🧠 **Channel Memory Status**\n\
        • **Enabled**: `{}`\n\
        • **Stored Messages**: `{} / {}` (`{}` turns)\n\
        • **TTL**: `{}` minutes",
        cfg.memory_turns > 0,
        hist.len(),
        cfg.memory_turns.saturating_mul(2),
        cfg.memory_turns,
        cfg.memory_ttl_minutes
    ))
    .await?;
    Ok(())
}

/// Configure channel conversation memory (`on`, `off`, or `max <1-50>`).
#[poise::command(slash_command, prefix_command)]
pub async fn setmemory(
    ctx: Context<'_>,
    #[description = "'on', 'off', or 'max <1-50>'"]
    #[rest]
    arg: String,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ManageBot).await? {
        return Ok(());
    }
    let Some(gid) = ctx.guild_id() else {
        ctx.say("⚠️ Server memory settings can only be modified inside a server.")
            .await?;
        return Ok(());
    };

    let state = ctx.data();
    let clean = arg.trim().to_lowercase();

    if clean == "on" || clean == "enable" || clean == "true" {
        state
            .storage
            .patch_guild_config_async(
                gid.get(),
                GuildConfigPatch {
                    memory_turns: Some(6),
                    ..Default::default()
                },
            )
            .await?;
        ctx.say("✅ Conversation memory **enabled** (6 turns) for this server.")
            .await?;
    } else if clean == "off" || clean == "disable" || clean == "false" {
        state.memory.clear_guild(gid.get());
        state
            .storage
            .patch_guild_config_async(
                gid.get(),
                GuildConfigPatch {
                    memory_turns: Some(0),
                    ..Default::default()
                },
            )
            .await?;
        ctx.say("✅ Conversation memory **disabled** and cleared for this server.")
            .await?;
    } else if let Some(num_str) = clean
        .strip_prefix("max")
        .map(str::trim)
        .or(Some(clean.as_str()))
    {
        if let Ok(turns) = num_str.parse::<isize>() {
            let clamped = turns.clamp(0, 50);
            state
                .storage
                .patch_guild_config_async(
                    gid.get(),
                    GuildConfigPatch {
                        memory_turns: Some(clamped),
                        ..Default::default()
                    },
                )
                .await?;
            ctx.say(format!(
                "✅ Set maximum remembered turns per channel to **{clamped}**."
            ))
            .await?;
        } else {
            ctx.say("⚠️ Usage: `/setmemory on`, `/setmemory off`, or `/setmemory max 10`.")
                .await?;
        }
    }
    Ok(())
}

/// Set conversation memory expiration timeout in minutes (`1-1440`).
#[poise::command(slash_command, prefix_command)]
pub async fn setttl(
    ctx: Context<'_>,
    #[description = "Expiration timeout in minutes (1-1440)"] minutes: isize,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ManageBot).await? {
        return Ok(());
    }
    let Some(gid) = ctx.guild_id() else {
        ctx.say("⚠️ Server TTL settings can only be modified inside a server.")
            .await?;
        return Ok(());
    };

    let clamped = minutes.clamp(1, 1440);
    ctx.data()
        .storage
        .patch_guild_config_async(
            gid.get(),
            GuildConfigPatch {
                memory_ttl_minutes: Some(clamped),
                ..Default::default()
            },
        )
        .await?;

    ctx.say(format!(
        "✅ Set conversation memory TTL to **{clamped} minutes**."
    ))
    .await?;
    Ok(())
}

/// Search the live web and synthesize an AI answer with verified citations.
#[poise::command(slash_command, prefix_command, aliases("web", "ddg"))]
pub async fn search(
    ctx: Context<'_>,
    #[description = "Search query"]
    #[rest]
    query: String,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::WebSearch).await? {
        return Ok(());
    }
    let clean = query.trim();
    if clean.is_empty() {
        ctx.say("⚠️ Please provide a search query.").await?;
        return Ok(());
    }
    let _ = ctx.defer().await;

    let state = ctx.data();
    let settings = state.storage.get_app_settings();
    let guild_cfg = ctx.guild_id().map(|g| state.storage.get_guild_config(g.get()));
    let default_cfg = GuildConfig::default();
    let cfg_ref = guild_cfg.as_deref().unwrap_or(&default_cfg);

    let results = state.search.search(clean, Some(5), &settings).await;
    if results.is_empty() {
        ctx.say(format!("🔍 No web search results found for `{clean}`."))
            .await?;
        return Ok(());
    }

    let model = state
        .resolve_model_for_context(&settings, guild_cfg.as_deref())
        .await;
    let (rag_block, citation_footer) = format_grounded_rag_block(&results);
    let custom_persona = if !cfg_ref.system_prompt.trim().is_empty() {
        Some(cfg_ref.system_prompt.as_str())
    } else {
        None
    };
    let sys_prompt = compose_system_prompt(
        &settings.system_prompt,
        custom_persona,
        rag_block.as_deref(),
    );

    let req = CompletionRequest {
        prompt: clean.to_string(),
        model,
        system_prompt: Some(sys_prompt),
        history: Vec::new(),
        max_tokens: None,
    };

    match state.ai.get().send_prompt(req).await {
        Ok(completion) => {
            let mut reply = completion.content.trim().to_string();
            if !is_refusal_response(&reply) {
                reply.push_str(&citation_footer);
            }
            ctx.say(truncate_chars(&reply, 1950)).await?;
        }
        Err(e) => {
            ctx.say(format!(
                "⚠️ AI synthesis failed: `{}`{}",
                truncate_chars(&e.to_string(), 200),
                citation_footer
            ))
            .await?;
        }
    }

    Ok(())
}

/// Fetch raw web search links without AI synthesis.
#[poise::command(slash_command, prefix_command, aliases("rawsearch"))]
pub async fn links(
    ctx: Context<'_>,
    #[description = "Search query"]
    #[rest]
    query: String,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::WebSearch).await? {
        return Ok(());
    }
    let clean = query.trim();
    if clean.is_empty() {
        ctx.say("⚠️ Please provide a search query.").await?;
        return Ok(());
    }
    let _ = ctx.defer().await;

    let state = ctx.data();
    let settings = state.storage.get_app_settings();
    let results = state.search.search(clean, Some(5), &settings).await;

    if !results.is_empty() {
        let mut out = format!("🔍 **Search Results for `{clean}`**:\n");
        for (idx, r) in results.iter().enumerate() {
            out.push_str(&format!(
                "{}. **[{}](<{}>)**\n   {}\n",
                idx.saturating_add(1),
                truncate_chars(&r.title, 90),
                r.url,
                truncate_chars(&r.snippet, 160)
            ));
        }
        ctx.say(truncate_chars(&out, 1950)).await?;
    } else {
        ctx.say(format!("🔍 No results found for `{clean}`.")).await?;
    }
    Ok(())
}

/// Test whether a message would trigger autonomous RAG web search.
#[poise::command(slash_command, prefix_command)]
pub async fn autosearch(
    ctx: Context<'_>,
    #[description = "Sample prompt to evaluate"]
    #[rest]
    sample_prompt: String,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ChatReply).await? {
        return Ok(());
    }
    let should = should_auto_search(&sample_prompt);
    let q = build_search_query(&sample_prompt, "", &[]);
    ctx.say(format!(
        "🧭 **Auto-Search Classifier Diagnostics**\n\
        • **Should Search**: `{should}`\n\
        • **Synthesized Query**: `{}`",
        if q.is_empty() { "(none)" } else { &q }
    ))
    .await?;
    Ok(())
}

/// Enable or disable autonomous RAG web search (`on` / `off`) for this server.
#[poise::command(slash_command, prefix_command)]
pub async fn setwebsearch(
    ctx: Context<'_>,
    #[description = "'on' or 'off'"] state_arg: String,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ManageBot).await? {
        return Ok(());
    }
    let Some(gid) = ctx.guild_id() else {
        ctx.say("⚠️ Server web search settings can only be changed inside a server.")
            .await?;
        return Ok(());
    };

    let clean = state_arg.trim().to_lowercase();
    let enabled = match clean.as_str() {
        "on" | "true" | "enable" | "enabled" => true,
        "off" | "false" | "disable" | "disabled" => false,
        _ => {
            ctx.say("⚠️ Usage: `/setwebsearch on` or `/setwebsearch off`.")
                .await?;
            return Ok(());
        }
    };

    ctx.data()
        .storage
        .patch_guild_config_async(
            gid.get(),
            GuildConfigPatch {
                web_search_enabled: Some(enabled),
                web_search_auto: Some(enabled),
                ..Default::default()
            },
        )
        .await?;

    ctx.say(format!(
        "✅ Autonomous web search is now **{}** for this server.",
        if enabled { "enabled" } else { "disabled" }
    ))
    .await?;
    Ok(())
}

/// View, set, or reset (`reset` / `default`) the custom AI persona system prompt for this server.
#[poise::command(slash_command, prefix_command)]
pub async fn setprompt(
    ctx: Context<'_>,
    #[description = "New system prompt, or 'reset' to restore default (leave empty to view current)"]
    #[rest]
    prompt: Option<String>,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ManageBot).await? {
        return Ok(());
    }
    let Some(gid) = ctx.guild_id() else {
        ctx.say("⚠️ Custom server prompts can only be configured inside a server.")
            .await?;
        return Ok(());
    };

    let state = ctx.data();
    let cfg = state.storage.get_guild_config(gid.get());

    let Some(raw) = prompt
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        let current_p = if cfg.system_prompt.trim().is_empty() {
            "(Using global default Silas Blue persona)"
        } else {
            cfg.system_prompt.as_str()
        };
        ctx.say(format!(
            "📝 **Current Server System Prompt**:\n```\n{}\n```",
            truncate_chars(current_p, 1800)
        ))
        .await?;
        return Ok(());
    };

    let new_prompt = if raw.eq_ignore_ascii_case("reset") || raw.eq_ignore_ascii_case("default") {
        String::new()
    } else {
        truncate_chars(&raw, 4000).to_string()
    };

    state
        .storage
        .patch_guild_config_async(
            gid.get(),
            GuildConfigPatch {
                system_prompt: Some(new_prompt.clone()),
                ..Default::default()
            },
        )
        .await?;

    if new_prompt.is_empty() {
        ctx.say("✅ Restored server system prompt to the default Silas Blue persona.")
            .await?;
    } else {
        ctx.say("✅ Updated server system prompt (immutable safety guardrails remain enforced).")
            .await?;
    }
    Ok(())
}

/// Configure interactive button pagination (`on`, `off`, or `chars <500-1900>`).
#[poise::command(slash_command, prefix_command)]
pub async fn setpagination(
    ctx: Context<'_>,
    #[description = "'on', 'off', or 'chars <500-1900>'"]
    #[rest]
    arg: String,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ManageBot).await? {
        return Ok(());
    }
    let Some(gid) = ctx.guild_id() else {
        ctx.say("⚠️ Server pagination settings can only be changed inside a server.")
            .await?;
        return Ok(());
    };

    let state = ctx.data();
    let clean = arg.trim().to_lowercase();

    if clean == "on" || clean == "enable" {
        state
            .storage
            .patch_guild_config_async(
                gid.get(),
                GuildConfigPatch {
                    pagination_enabled: Some(true),
                    ..Default::default()
                },
            )
            .await?;
        ctx.say("✅ Interactive button pagination **enabled**.").await?;
    } else if clean == "off" || clean == "disable" {
        state
            .storage
            .patch_guild_config_async(
                gid.get(),
                GuildConfigPatch {
                    pagination_enabled: Some(false),
                    ..Default::default()
                },
            )
            .await?;
        ctx.say("✅ Interactive button pagination **disabled**.").await?;
    } else if let Some(num_str) = clean
        .strip_prefix("chars")
        .map(str::trim)
        .or(Some(clean.as_str()))
    {
        if let Ok(chars) = num_str.parse::<isize>() {
            let clamped = chars.clamp(500, 1900);
            state
                .storage
                .patch_guild_config_async(
                    gid.get(),
                    GuildConfigPatch {
                        pagination_max_chars: Some(clamped),
                        ..Default::default()
                    },
                )
                .await?;
            ctx.say(format!(
                "✅ Set pagination page size to **{clamped} characters**."
            ))
            .await?;
        } else {
            ctx.say("⚠️ Usage: `/setpagination on`, `/setpagination off`, or `/setpagination chars 1800`.")
                .await?;
        }
    }
    Ok(())
}

/// Change or reset (`reset`) Silas Blue's nickname in this server.
#[poise::command(slash_command, prefix_command, aliases("setname"))]
pub async fn setnick(
    ctx: Context<'_>,
    #[description = "New server nickname, or 'reset' to clear"]
    #[rest]
    nickname: String,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ManageBot).await? {
        return Ok(());
    }
    let Some(gid) = ctx.guild_id() else {
        ctx.say("⚠️ Server nicknames can only be changed inside a server.")
            .await?;
        return Ok(());
    };

    let clean = nickname.trim();
    let target_nick = if clean.is_empty()
        || clean.eq_ignore_ascii_case("reset")
        || clean.eq_ignore_ascii_case("clear")
    {
        None
    } else {
        Some(truncate_chars(clean, 32).to_string())
    };

    gid.edit_nickname(&ctx.serenity_context().http, target_nick.as_deref())
        .await?;

    if let Some(mut entry) = ctx.data().connected_guilds.get_mut(&gid.get()) {
        entry.bot_nickname = target_nick.clone();
    }

    if let Some(n) = target_nick {
        ctx.say(format!("✅ Changed server nickname to **{n}**.")).await?;
    } else {
        ctx.say("✅ Reset server nickname to default.").await?;
    }
    Ok(())
}

/// Configure Forum Channel auto-reply (`on`, `off`, `mode all_chunks|interactive`, `prompt <text>`).
#[poise::command(slash_command, prefix_command)]
pub async fn setforum(
    ctx: Context<'_>,
    #[description = "'on', 'off', 'mode all_chunks|interactive', or 'prompt <text>'"]
    #[rest]
    arg: String,
) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ManageBot).await? {
        return Ok(());
    }
    let Some(gid) = ctx.guild_id() else {
        ctx.say("⚠️ Forum settings can only be configured inside a server.")
            .await?;
        return Ok(());
    };

    let state = ctx.data();
    let trimmed = arg.trim();
    let lower = trimmed.to_lowercase();

    if lower == "on" || lower == "enable" {
        state
            .storage
            .patch_guild_config_async(
                gid.get(),
                GuildConfigPatch {
                    forum_auto_reply_enabled: Some(true),
                    ..Default::default()
                },
            )
            .await?;
        ctx.say("✅ Forum thread auto-reply **enabled**.").await?;
    } else if lower == "off" || lower == "disable" {
        state
            .storage
            .patch_guild_config_async(
                gid.get(),
                GuildConfigPatch {
                    forum_auto_reply_enabled: Some(false),
                    ..Default::default()
                },
            )
            .await?;
        ctx.say("✅ Forum thread auto-reply **disabled**.").await?;
    } else if let Some(m) = lower.strip_prefix("mode ").map(str::trim) {
        state
            .storage
            .patch_guild_config_async(
                gid.get(),
                GuildConfigPatch {
                    forum_delivery_mode: Some(m.to_string()),
                    ..Default::default()
                },
            )
            .await?;
        ctx.say(format!("✅ Updated Forum delivery mode to `{m}`."))
            .await?;
    } else if let Some(custom_prompt) = trimmed
        .strip_prefix("prompt ")
        .or_else(|| trimmed.strip_prefix("PROMPT "))
    {
        state
            .storage
            .patch_guild_config_async(
                gid.get(),
                GuildConfigPatch {
                    forum_system_prompt: Some(truncate_chars(custom_prompt.trim(), 4000).to_string()),
                    ..Default::default()
                },
            )
            .await?;
        ctx.say("✅ Updated Forum auto-reply system prompt.").await?;
    } else {
        ctx.say("⚠️ Usage: `/setforum on`, `/setforum off`, `/setforum mode interactive|all_chunks`, or `/setforum prompt <instructions>`.")
            .await?;
    }
    Ok(())
}

/// View your current token quota usage or (for admins) manage server token limits.
#[poise::command(slash_command, prefix_command, aliases("tokens", "myquota"))]
pub async fn quota(
    ctx: Context<'_>,
    #[description = "Optional admin subcommand: 'on', 'off', 'reset'"]
    #[rest]
    subcommand: Option<String>,
) -> Result<(), CommandError> {
    let Some(gid) = ctx.guild_id() else {
        ctx.say("ℹ️ Token quotas apply only within Discord servers.")
            .await?;
        return Ok(());
    };

    let state = ctx.data();
    let settings = state.storage.get_app_settings();
    let cfg = state.storage.get_guild_config(gid.get());

    if let Some(sub) = subcommand
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        if !check_caller_permission(ctx, PermissionDomain::ManageBot).await? {
            return Ok(());
        }
        let lower = sub.to_lowercase();
        if lower == "on" || lower == "enable" {
            state
                .storage
                .patch_guild_config_async(
                    gid.get(),
                    GuildConfigPatch {
                        token_quotas: Some(TokenQuotaPatch {
                            enabled: Some(true),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )
                .await?;
            ctx.say("✅ Server token quotas **enabled**.").await?;
            return Ok(());
        } else if lower == "off" || lower == "disable" {
            state
                .storage
                .patch_guild_config_async(
                    gid.get(),
                    GuildConfigPatch {
                        token_quotas: Some(TokenQuotaPatch {
                            enabled: Some(false),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )
                .await?;
            ctx.say("✅ Server token quotas **disabled**.").await?;
            return Ok(());
        } else if lower.starts_with("reset") {
            let tq = &cfg.token_quotas;
            let (win_start, _, _) = calculate_window_period(
                &tq.reset_frequency,
                &tq.reset_time,
                tq.reset_timezone_offset,
                tq.reset_day_of_week,
                tq.reset_day_of_month,
                None,
            );
            state.storage.reset_guild_token_usage(gid.get(), win_start)?;
            ctx.say("✅ Reset all token usage counters for the current window.")
                .await?;
            return Ok(());
        }
    }

    let info = extract_caller_roles(ctx, &settings.bot_owner_id).await;
    let user_id = ctx.author().id.get();
    let member_ctx = MemberContext {
        user_id,
        is_guild_owner: info.is_owner_or_bot_owner,
        is_administrator: info.is_admin,
        role_ids: &info.role_ids,
        role_names: &info.role_names,
    };

    let tq = &cfg.token_quotas;
    let (win_start, win_end, _) = calculate_window_period(
        &tq.reset_frequency,
        &tq.reset_time,
        tq.reset_timezone_offset,
        tq.reset_day_of_week,
        tq.reset_day_of_month,
        None,
    );
    let used = state
        .storage
        .get_user_token_usage_async(gid.get(), user_id, win_start, win_end)
        .await
        .unwrap_or(0);

    let eval = evaluate_quota(&member_ctx, &cfg, &info.guild_role_map, used, None);

    if !tq.enabled {
        ctx.say(format!(
            "📊 **Token Usage** (Quotas currently disabled on this server)\n• **Tokens Used ({})**: `{used}`",
            tq.reset_frequency
        ))
        .await?;
    } else if eval.limit == -1 {
        ctx.say(format!(
            "📊 **Token Quota Status**\n• **Tier**: `Unlimited` (`{}`)\n• **Tokens Used ({})**: `{used}`",
            eval.role_name, tq.reset_frequency
        ))
        .await?;
    } else {
        ctx.say(format!(
            "📊 **Token Quota Status**\n• **Role Tier**: `{}`\n• **Used**: `{} / {}` tokens (`{}` remaining)\n• **Schedule**: `{}` (Resets <t:{}:R>)",
            eval.role_name, eval.used, eval.limit, eval.remaining, tq.reset_frequency, eval.reset_epoch
        ))
        .await?;
    }

    Ok(())
}

/// Display the active configuration summary for this server.
#[poise::command(slash_command, prefix_command)]
pub async fn config(ctx: Context<'_>) -> Result<(), CommandError> {
    if !check_caller_permission(ctx, PermissionDomain::ManageBot).await? {
        return Ok(());
    }
    let state = ctx.data();
    let settings = state.storage.get_app_settings();
    let cfg = ctx
        .guild_id()
        .map(|g| state.storage.get_guild_config(g.get()))
        .unwrap_or_else(|| Arc::new(GuildConfig::default()));

    let active_model = state.resolve_model_for_context(&settings, Some(&cfg)).await;

    let summary = format!(
        "⚙️ **Silas Blue v{} — Server Configuration**\n\
        • **AI Backend**: `{}` | **Active Model**: `{}`\n\
        • **Autonomous Web Search**: `{}` (Provider: `{}`)\n\
        • **Conversation Memory**: `{}` turns (TTL `{}`m)\n\
        • **Interactive Pagination**: `{}` (`{}` chars/page)\n\
        • **Spontaneous Reply Mode**: `{}` (Value: `{}`)\n\
        • **Forum Auto-Reply**: `{}` (Mode: `{}`)\n\
        • **Token Quotas**: `{}` (Schedule: `{}`)",
        silas_updater::CURRENT_VERSION,
        settings.backend_type,
        active_model,
        if cfg.web_search_enabled {
            "Enabled"
        } else {
            "Disabled"
        },
        settings.web_search_provider,
        cfg.memory_turns,
        cfg.memory_ttl_minutes,
        if cfg.pagination_enabled {
            "Enabled"
        } else {
            "Disabled"
        },
        cfg.pagination_max_chars,
        cfg.random_reply_mode,
        cfg.random_reply_value,
        if cfg.forum_auto_reply_enabled {
            "Enabled"
        } else {
            "Disabled"
        },
        cfg.forum_delivery_mode,
        if cfg.token_quotas.enabled {
            "Enabled"
        } else {
            "Disabled"
        },
        cfg.token_quotas.reset_frequency,
    );

    ctx.say(summary).await?;
    Ok(())
}

/// Check gateway latency and AI backend status.
#[poise::command(slash_command, prefix_command)]
pub async fn ping(ctx: Context<'_>) -> Result<(), CommandError> {
    let start = std::time::Instant::now();
    let handle = ctx.say("🏓 Pinging...").await?;
    let rtt_ms = start.elapsed().as_millis();

    let state = ctx.data();
    let settings = state.storage.get_app_settings();
    let guild_cfg = ctx.guild_id().map(|g| state.storage.get_guild_config(g.get()));
    let model = state
        .resolve_model_for_context(&settings, guild_cfg.as_deref())
        .await;

    handle
        .edit(
            ctx,
            poise::CreateReply::default().content(format!(
                "🏓 **Pong!** REST RTT: `{}ms` | Version: `v{}` | Backend: `{}` (`{}`)",
                rtt_ms,
                silas_updater::CURRENT_VERSION,
                settings.backend_type,
                model
            )),
        )
        .await?;
    Ok(())
}

/// Generate the least-privilege Discord OAuth2 invite link for Silas Blue.
#[poise::command(slash_command, prefix_command)]
pub async fn invite(ctx: Context<'_>) -> Result<(), CommandError> {
    let state = ctx.data();
    let settings = state.storage.get_app_settings();
    let client_id = if !settings.discord_oauth_client_id.trim().is_empty() {
        Some(settings.discord_oauth_client_id.trim().to_string())
    } else {
        state.bot_identity.read().id.clone()
    };

    if let Some(cid) = client_id.filter(|s| !s.is_empty()) {
        let url = format!(
            "https://discord.com/oauth2/authorize?client_id={cid}&permissions=274878221312&scope=bot%20applications.commands"
        );
        ctx.say(format!("🔗 **Invite Silas Blue to your server**:\n<{url}>"))
            .await?;
    } else {
        ctx.say("⚠️ Discord Client ID is not yet available.").await?;
    }
    Ok(())
}

/// Display the categorized command reference guide.
#[poise::command(slash_command, prefix_command)]
pub async fn help(ctx: Context<'_>) -> Result<(), CommandError> {
    let guide = format!(
        "🤖 **Silas Blue v{} — Command Reference**\n\n\
        **Chat & Memory**\n\
        • `@Silas Blue <message>` or `silas <message>` — Chat with Silas Blue\n\
        • `/clear` (`!reset`, `!forget`) — Clear channel conversation memory\n\
        • `/recall` — View channel memory status and message count\n\
        • `/forget_me` — Erase all your messages across all channels & anonymize quota logs\n\
        • `/privacy` — View data retention policy and privacy guarantees\n\n\
        **Web Search & RAG**\n\
        • `/search <query>` (`!web`, `!ddg`) — Search the web and synthesize an AI answer with citations\n\
        • `/links <query>` (`!rawsearch`) — Fetch raw search links without AI synthesis\n\
        • `/autosearch <prompt>` — Test whether a prompt triggers autonomous web search\n\n\
        **Models & Quotas**\n\
        • `/models` — List available LLM models on the active backend\n\
        • `/setmodel <model>` — Switch the active model for this server or DM\n\
        • `/quota` (`!tokens`, `!myquota`) — Check your token quota usage and reset timer\n\n\
        **Server Administration**\n\
        • `/config` — View complete server configuration\n\
        • `/setprompt [prompt|reset]` — View, customize, or reset the server AI persona\n\
        • `/setmemory <on|off|max N>` & `/setttl <minutes>` — Configure channel memory\n\
        • `/setwebsearch <on|off>` — Toggle autonomous RAG web search\n\
        • `/setpagination <on|off|chars N>` — Configure interactive pagination buttons\n\
        • `/setforum <on|off|mode ...|prompt ...>` — Configure Forum auto-reply\n\
        • `/setnick <nickname|reset>` — Change the bot's server nickname\n\
        • `/ping` & `/invite` — Check latency or generate an invite link",
        silas_updater::CURRENT_VERSION
    );
    ctx.say(guide).await?;
    Ok(())
}

/// (Owner-Only) Change Silas Blue's global Discord username.
#[poise::command(slash_command, prefix_command)]
pub async fn setglobalname(
    ctx: Context<'_>,
    #[description = "New global Discord username (2-32 chars)"]
    #[rest]
    new_name: String,
) -> Result<(), CommandError> {
    let state = ctx.data();
    let settings = state.storage.get_app_settings();
    if settings.bot_owner_id.trim() != ctx.author().id.get().to_string() {
        ctx.say("⛔ Only the configured Bot Owner can change the global username.")
            .await?;
        return Ok(());
    }

    let trimmed = new_name.trim();
    if trimmed.len() < 2 || trimmed.len() > 32 {
        ctx.say("⚠️ Global username must be between 2 and 32 characters.")
            .await?;
        return Ok(());
    }

    let builder = EditProfile::new().username(trimmed);
    match ctx.serenity_context().http.edit_profile(&builder).await {
        Ok(user) => {
            state.bot_identity.write().username = Some(user.name.clone());
            ctx.say(format!(
                "✅ Updated global bot username to **{}**.",
                user.name
            ))
            .await?;
        }
        Err(e) => {
            ctx.say(format!(
                "⚠️ Discord rejected global username update: `{}`",
                truncate_chars(&e.to_string(), 200)
            ))
            .await?;
        }
    }
    Ok(())
}

/// (Owner-Only) Check for or apply cryptographically verified Silas Blue binary updates.
#[poise::command(slash_command, prefix_command)]
pub async fn update(
    ctx: Context<'_>,
    #[description = "'check', 'apply', or 'channel stable|prerelease'"]
    #[rest]
    action: Option<String>,
) -> Result<(), CommandError> {
    let state = ctx.data();
    let settings = state.storage.get_app_settings();
    if settings.bot_owner_id.trim() != ctx.author().id.get().to_string() {
        ctx.say("⛔ Only the configured Bot Owner can run `/update`.")
            .await?;
        return Ok(());
    }
    let _ = ctx.defer().await;

    let cmd = action
        .unwrap_or_else(|| "check".to_string())
        .trim()
        .to_lowercase();

    if let Some(ch) = cmd.strip_prefix("channel ").map(str::trim) {
        if ch == "stable" || ch == "prerelease" || ch == "latest" {
            state
                .storage
                .update_app_settings_async(AppSettingsPatch {
                    update_channel: Some(ch.to_string()),
                    ..Default::default()
                })
                .await?;
            ctx.say(format!("✅ Switched update channel to **{ch}**."))
                .await?;
        } else {
            ctx.say("⚠️ Valid channels are `stable` or `prerelease`.")
                .await?;
        }
        return Ok(());
    }

    if cmd == "apply" || cmd == "now" || cmd == "install" {
        match state.updater.apply_update(None).await {
            Ok(msg) => {
                ctx.say(format!("✅ {msg}")).await?;
            }
            Err(e) => {
                ctx.say(format!("⚠️ Update failed: `{e}`")).await?;
            }
        }
        return Ok(());
    }

    let res = state.updater.check_for_updates(None).await;
    if let Some(err) = res.error {
        ctx.say(format!(
            "⚠️ Update check encountered an error: `{err}` (Current: `v{}`)",
            res.current_version
        ))
        .await?;
    } else if res.update_available {
        ctx.say(format!(
            "🚀 **Update Available**: `v{}` (Current: `v{}`, Channel: `{}`)\n• Release: <{}>\n• Run `/update apply` to verify and install.",
            res.latest_version, res.current_version, res.channel, res.release_url
        ))
        .await?;
    } else {
        ctx.say(format!(
            "✅ Silas Blue is up to date (`v{}`, Channel: `{}`).",
            res.current_version, res.channel
        ))
        .await?;
    }

    Ok(())
}
