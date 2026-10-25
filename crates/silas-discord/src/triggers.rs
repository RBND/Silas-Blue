//! Custom Triggers and Tags engine for Silas Blue.
//!
//! Evaluates dynamic message triggers matching `exact`, `prefix`, `contains`, or `regex` patterns.
//! Executes TagScript formatting, action directives (`{delete}`, `{dm}`, `{react}`, `{role}`),
//! and AI text completions (`{ai: <prompt>}`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use poise::serenity_prelude as serenity;
use regex::Regex;
use serenity::all::{
    Colour, Context as SerenityContext, CreateEmbed, CreateEmbedFooter, CreateMessage,
    Message, Permissions, ReactionType, RoleId,
};
use silas_ai::CompletionRequest;
use silas_core::tagscript::{evaluate_tagscript, TagScriptAction, TagScriptContext};
use silas_core::text::truncate_chars;
use silas_storage::NewCustomTrigger;
use tracing::debug;

use crate::bot::BotState;
use crate::greetings::tagscript_embed_to_serenity;

pub type CommandError = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Arc<BotState>, CommandError>;

/// In-memory cooldown tracking per `(guild_id, trigger_id, user_id)`.
static TRIGGER_COOLDOWNS: std::sync::OnceLock<DashMap<(u64, i64, u64), Instant>> =
    std::sync::OnceLock::new();

fn cooldowns() -> &'static DashMap<(u64, i64, u64), Instant> {
    TRIGGER_COOLDOWNS.get_or_init(DashMap::new)
}

/// Root command group for custom triggers and tags.
#[poise::command(
    slash_command,
    prefix_command,
    guild_only,
    subcommands("get_cmd", "create_cmd", "delete_cmd", "list_cmd")
)]
pub async fn tag(ctx: Context<'_>) -> Result<(), CommandError> {
    ctx.say("Use `/tag get`, `/tag create`, `/tag delete`, or `/tag list`.")
        .await?;
    Ok(())
}

/// Retrieve and execute a custom tag by name.
#[poise::command(slash_command, prefix_command, guild_only, rename = "get")]
pub async fn get_cmd(
    ctx: Context<'_>,
    #[description = "Tag name to execute"] name: String,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let storage = &ctx.data().storage;

    let trigger = match storage.get_custom_trigger(guild_id, &name)? {
        Some(t) => t,
        None => {
            ctx.say(format!("❌ Tag `{name}` not found.")).await?;
            return Ok(());
        }
    };

    if !trigger.enabled {
        ctx.say(format!("⚠️ Tag `{name}` is currently disabled.")).await?;
        return Ok(());
    }

    let author_user = ctx.author();
    let author_id = author_user.id.get();
    let author_name = author_user.name.clone();
    let author_avatar = author_user.avatar_url();
    let author_created_at = author_user.created_at().to_rfc3339();

    let tag_ctx = TagScriptContext {
        author_id,
        author_name: author_name.clone(),
        author_avatar,
        author_created_at,
        target_id: Some(author_id),
        target_name: Some(author_name),
        guild_id: Some(guild_id),
        guild_name: ctx.guild().map(|g| g.name.clone()),
        guild_members: None,
        channel_id: Some(ctx.channel_id().get()),
        channel_name: None,
        custom_vars: std::collections::HashMap::new(),
    };

    let mut output = evaluate_tagscript(&trigger.response_template, &tag_ctx);

    // If TagScript requested an AI prompt completion
    if let Some(ref ai_prompt) = output.ai_prompt {
        let settings = storage.get_app_settings();
        let guild_cfg = storage.get_guild_config(guild_id);
        let model = ctx.data().resolve_model_for_context(&settings, Some(&guild_cfg)).await;

        let req = CompletionRequest {
            prompt: ai_prompt.clone(),
            model,
            system_prompt: Some("You are Silas Blue, an AI assistant in a Discord server.".to_string()),
            history: Vec::new(),
            max_tokens: Some(512),
        };

        if let Ok(comp) = ctx.data().ai.get().send_prompt(req).await {
            let ai_text = comp.content.trim();
            if !ai_text.is_empty() {
                if !output.body.is_empty() {
                    output.body.push_str("\n\n");
                }
                output.body.push_str(ai_text);
            }
        }
    }

    let mut builder = CreateMessage::new();
    if !output.body.trim().is_empty() {
        builder = builder.content(truncate_chars(&output.body, 2000));
    }
    if let Some(ref emb) = output.embed {
        builder = builder.embed(tagscript_embed_to_serenity(emb));
    }

    if trigger.reply_in_dm {
        if let Ok(dm_ch) = author_user.create_dm_channel(&ctx.serenity_context().http).await {
            let _ = dm_ch.send_message(&ctx.serenity_context().http, builder).await;
            ctx.say("📬 Response sent to your DMs.").await?;
            return Ok(());
        }
    }

    let ch = ctx.channel_id();
    ch.send_message(&ctx.serenity_context().http, builder).await?;
    Ok(())
}

/// Create or update a custom trigger / tag.
#[poise::command(slash_command, prefix_command, guild_only, rename = "create")]
pub async fn create_cmd(
    ctx: Context<'_>,
    #[description = "Trigger name (or match pattern)"] name: String,
    #[description = "Response template (supports TagScript: {user}, {embed}, {ai: prompt})"]
    response: String,
    #[description = "Match mode: exact, prefix, contains, regex (default: exact)"]
    match_mode: Option<String>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MANAGE_GUILD) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Manage Server` permission.").await?;
        return Ok(());
    }

    let mode = match_mode.unwrap_or_else(|| "exact".to_string()).to_lowercase();
    let valid_modes = ["exact", "prefix", "contains", "regex"];
    if !valid_modes.contains(&mode.as_str()) {
        ctx.say("❌ Invalid match mode. Choose from: `exact`, `prefix`, `contains`, `regex`.").await?;
        return Ok(());
    }

    if mode == "regex" && Regex::new(&name).is_err() {
        ctx.say("❌ Invalid regex pattern for trigger name.").await?;
        return Ok(());
    }

    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let storage = &ctx.data().storage;

    let new_trigger = NewCustomTrigger {
        guild_id,
        trigger_name: name.clone(),
        match_mode: mode.clone(),
        response_template: response,
        cooldown_seconds: 0,
        allowed_roles: Vec::new(),
        ignored_channels: Vec::new(),
        delete_trigger_message: false,
        reply_in_dm: false,
        enabled: true,
    };

    storage.create_custom_trigger(&new_trigger)?;
    ctx.say(format!("✅ Trigger `{name}` created successfully with mode `{mode}`.")).await?;
    Ok(())
}

/// Delete a custom trigger by name.
#[poise::command(slash_command, prefix_command, guild_only, rename = "delete")]
pub async fn delete_cmd(
    ctx: Context<'_>,
    #[description = "Trigger name to delete"] name: String,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MANAGE_GUILD) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Manage Server` permission.").await?;
        return Ok(());
    }

    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let storage = &ctx.data().storage;

    let deleted = storage.delete_custom_trigger(guild_id, &name)?;
    if deleted {
        ctx.say(format!("🗑️ Trigger `{name}` deleted.")).await?;
    } else {
        ctx.say(format!("❌ Trigger `{name}` not found.")).await?;
    }

    Ok(())
}

/// List all registered custom triggers in the server.
#[poise::command(slash_command, prefix_command, guild_only, rename = "list")]
pub async fn list_cmd(ctx: Context<'_>) -> Result<(), CommandError> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let storage = &ctx.data().storage;

    let triggers = storage.list_custom_triggers(guild_id)?;
    if triggers.is_empty() {
        ctx.say("ℹ️ No custom triggers found in this server.").await?;
        return Ok(());
    }

    let mut embed = CreateEmbed::new()
        .title(format!("Custom Triggers & Tags (Total: {})", triggers.len()))
        .color(Colour::new(0x5865F2))
        .footer(CreateEmbedFooter::new(format!("Guild ID: {guild_id}")));

    for t in triggers.iter().take(15) {
        let status = if t.enabled { "🟢" } else { "⚪" };
        embed = embed.field(
            format!("{status} {} [{}]", t.trigger_name, t.match_mode),
            truncate_chars(&t.response_template, 150),
            true,
        );
    }

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

// =========================================================================
// Passive Message Trigger Evaluator
// =========================================================================

/// Evaluates incoming message content against active guild custom triggers.
pub async fn evaluate_message_triggers(
    ctx: &SerenityContext,
    msg: &Message,
    state: &Arc<BotState>,
) -> bool {
    if msg.author.bot || msg.webhook_id.is_some() || msg.guild_id.is_none() {
        return false;
    }

    let Some(guild_id) = msg.guild_id else {
        return false;
    };
    let gid = guild_id.get();
    let channel_id = msg.channel_id.get();
    let author_id = msg.author.id.get();
    let content = msg.content.trim();

    if content.is_empty() {
        return false;
    }

    let triggers = match state.storage.list_custom_triggers(gid) {
        Ok(t) => t,
        Err(_) => return false,
    };

    if triggers.is_empty() {
        return false;
    }

    for trigger in triggers {
        if !trigger.enabled {
            continue;
        }

        if trigger.ignored_channels.contains(&channel_id) {
            continue;
        }

        // Check allowed roles filter
        if !trigger.allowed_roles.is_empty() {
            let has_allowed_role = match &msg.member {
                Some(m) => m.roles.iter().any(|r| trigger.allowed_roles.contains(&r.get())),
                None => false,
            };
            if !has_allowed_role {
                continue;
            }
        }

        // Match pattern
        let matches = match trigger.match_mode.as_str() {
            "exact" => content.eq_ignore_ascii_case(&trigger.trigger_name),
            "prefix" => content.to_lowercase().starts_with(&trigger.trigger_name.to_lowercase()),
            "contains" => content.to_lowercase().contains(&trigger.trigger_name.to_lowercase()),
            "regex" => {
                if let Ok(re) = Regex::new(&trigger.trigger_name) {
                    re.is_match(content)
                } else {
                    false
                }
            }
            _ => false,
        };

        if !matches {
            continue;
        }

        // Check cooldown
        if trigger.cooldown_seconds > 0 {
            let key = (gid, trigger.id, author_id);
            let now = Instant::now();
            let cd_dur = Duration::from_secs(trigger.cooldown_seconds as u64);

            if let Some(last_used) = cooldowns().get(&key) {
                if now.duration_since(*last_used) < cd_dur {
                    debug!(trigger_id = trigger.id, "Trigger skipped due to cooldown");
                    continue;
                }
            }
            cooldowns().insert(key, now);
        }

        debug!(trigger_id = trigger.id, name = trigger.trigger_name, "Executing custom trigger");

        // Delete trigger message if configured
        if trigger.delete_trigger_message {
            let _ = msg.delete(&ctx.http).await;
        }

        // Build TagScript context
        let author_name = msg.author.name.clone();
        let author_avatar = msg.author.avatar_url();
        let author_created_at = msg.author.created_at().to_rfc3339();

        let tag_ctx = TagScriptContext {
            author_id,
            author_name: author_name.clone(),
            author_avatar,
            author_created_at,
            target_id: Some(author_id),
            target_name: Some(author_name),
            guild_id: Some(gid),
            guild_name: None,
            guild_members: None,
            channel_id: Some(channel_id),
            channel_name: None,
            custom_vars: std::collections::HashMap::new(),
        };

        let mut output = evaluate_tagscript(&trigger.response_template, &tag_ctx);

        // Process AI Tag if present
        if let Some(ref ai_prompt) = output.ai_prompt {
            let settings = state.storage.get_app_settings();
            let guild_cfg = state.storage.get_guild_config(gid);
            let model = state.resolve_model_for_context(&settings, Some(&guild_cfg)).await;

            let req = CompletionRequest {
                prompt: ai_prompt.clone(),
                model,
                system_prompt: Some("You are Silas Blue, an AI assistant in a Discord server.".to_string()),
                history: Vec::new(),
                max_tokens: Some(512),
            };

            if let Ok(comp) = state.ai.get().send_prompt(req).await {
                let ai_text = comp.content.trim();
                if !ai_text.is_empty() {
                    if !output.body.is_empty() {
                        output.body.push_str("\n\n");
                    }
                    output.body.push_str(ai_text);
                }
            }
        }

        // Execute action directives
        let mut send_in_dm = trigger.reply_in_dm;
        for action in output.actions {
            match action {
                TagScriptAction::DeleteMessage => {
                    let _ = msg.delete(&ctx.http).await;
                }
                TagScriptAction::SendDm => {
                    send_in_dm = true;
                }
                TagScriptAction::AddReaction(emoji) => {
                    let r_type = if emoji.starts_with("<:") || emoji.starts_with("<a:") {
                        ReactionType::try_from(emoji.as_str()).ok()
                    } else {
                        Some(ReactionType::Unicode(emoji.clone()))
                    };
                    if let Some(rt) = r_type {
                        let _ = msg.react(&ctx.http, rt).await;
                    }
                }
                TagScriptAction::AddRole(role_str) => {
                    if let Ok(member) = guild_id.member(&ctx.http, author_id).await {
                        if let Ok(rid) = role_str.parse::<u64>() {
                            let _ = member.add_role(&ctx.http, RoleId::new(rid)).await;
                        }
                    }
                }
                TagScriptAction::RemoveRole(role_str) => {
                    if let Ok(member) = guild_id.member(&ctx.http, author_id).await {
                        if let Ok(rid) = role_str.parse::<u64>() {
                            let _ = member.remove_role(&ctx.http, RoleId::new(rid)).await;
                        }
                    }
                }
            }
        }

        let mut builder = CreateMessage::new();
        if !output.body.trim().is_empty() {
            builder = builder.content(truncate_chars(&output.body, 2000));
        }
        if let Some(ref emb) = output.embed {
            builder = builder.embed(tagscript_embed_to_serenity(emb));
        }

        if send_in_dm {
            if let Ok(dm_ch) = msg.author.create_dm_channel(&ctx.http).await {
                let _ = dm_ch.send_message(&ctx.http, builder).await;
                return true;
            }
        }

        let _ = msg.channel_id.send_message(&ctx.http, builder).await;
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches_pattern(pattern: &str, mode: &str, content: &str) -> bool {
        match mode {
            "exact" => content.eq_ignore_ascii_case(pattern),
            "prefix" => content.to_lowercase().starts_with(&pattern.to_lowercase()),
            "contains" => content.to_lowercase().contains(&pattern.to_lowercase()),
            "regex" => {
                if let Ok(re) = Regex::new(pattern) {
                    re.is_match(content)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    #[test]
    fn test_match_modes() {
        // Exact
        assert!(matches_pattern("!help", "exact", "!help"));
        assert!(matches_pattern("!help", "exact", "!HELP"));
        assert!(!matches_pattern("!help", "exact", "!help please"));

        // Prefix
        assert!(matches_pattern("!faq", "prefix", "!faq rules"));
        assert!(matches_pattern("!faq", "prefix", "!FAQ"));
        assert!(!matches_pattern("!faq", "prefix", "check !faq"));

        // Contains
        assert!(matches_pattern("discord.gg", "contains", "join discord.gg/rust"));
        assert!(!matches_pattern("discord.gg", "contains", "hello world"));

        // Regex
        assert!(matches_pattern(r"^!rank\s+\d+$", "regex", "!rank 42"));
        assert!(!matches_pattern(r"^!rank\s+\d+$", "regex", "!rank foo"));
    }
}

