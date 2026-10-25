//! Moderation commands suite for Silas Blue:
//! `/warn`, `/delwarn`, `/warnings`, `/strikes`, `/timeout`, `/untimeout`,
//! `/kick`, `/softban`, `/ban`, `/tempban`, `/unban`, `/purge`, `/slowmode`,
//! `/lock`, `/unlock`, `/reason`, `/case`.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use poise::serenity_prelude as serenity;
use serenity::all::{
    ChannelId, Colour, CreateEmbed, CreateEmbedFooter, EditChannel, EditMember,
    GetMessages, GuildId, Member, MessageId, PermissionOverwrite, PermissionOverwriteType,
    Permissions, RoleId, Timestamp, User, UserId,
};
use silas_core::text::truncate_chars;
use silas_storage::{NewModCase, NewModStrike};

use crate::bot::BotState;
use crate::logging::log_mod_action;

pub type CommandError = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Arc<BotState>, CommandError>;

/// Exports all moderation commands for registration in the Poise framework.
pub fn mod_commands() -> Vec<poise::Command<Arc<BotState>, CommandError>> {
    vec![
        warn(),
        delwarn(),
        warnings(),
        strikes(),
        timeout(),
        untimeout(),
        kick(),
        softban(),
        ban(),
        tempban(),
        unban(),
        purge(),
        slowmode(),
        lock(),
        unlock(),
        reason(),
        case(),
    ]
}

/// Helper to parse duration strings like "10m", "1h", "2d", "1w", "300" safely without string slicing.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut digits = String::new();
    let mut unit = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            unit.push(ch);
        }
    }
    let num: u64 = digits.parse().ok()?;
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        _ => return None,
    };
    num.checked_mul(mult).map(Duration::from_secs)
}

/// Validates permission hierarchy between caller, target member, and bot.
async fn validate_hierarchy(
    ctx: Context<'_>,
    target_user: &User,
    required_perm: Permissions,
) -> Result<Option<Member>, String> {
    let Some(guild_id) = ctx.guild_id() else {
        return Err("This command can only be used inside a Discord server.".to_string());
    };

    let settings = ctx.data().storage.get_app_settings();
    let caller_id = ctx.author().id;

    // 1. Caller cannot moderate themselves
    if target_user.id == caller_id {
        return Err("You cannot moderate yourself.".to_string());
    }

    // 2. Cannot moderate bot owner
    if !settings.bot_owner_id.trim().is_empty()
        && target_user.id.to_string() == settings.bot_owner_id.trim()
    {
        return Err("You cannot moderate the Silas bot owner.".to_string());
    }

    // Fetch guild information without holding a CacheRef across await boundaries
    let (guild_owner_id, role_positions) = {
        let cache = &ctx.serenity_context().cache;
        match cache.guild(guild_id) {
            Some(g) => {
                let owner = g.owner_id;
                let mut map = std::collections::HashMap::new();
                for (id, role) in &g.roles {
                    let _ = map.insert(*id, role.position);
                }
                (owner, map)
            }
            None => {
                return Err("Failed to retrieve server metadata from cache.".to_string());
            }
        }
    };

    // 3. Cannot moderate guild owner
    if target_user.id == guild_owner_id {
        return Err("You cannot moderate the server owner.".to_string());
    }

    let is_caller_owner = caller_id == guild_owner_id
        || (!settings.bot_owner_id.trim().is_empty()
            && caller_id.to_string() == settings.bot_owner_id.trim());

    // 4. Verify caller permissions
    if !is_caller_owner {
        let caller_member = guild_id
            .member(&ctx.serenity_context().http, caller_id)
            .await
            .map_err(|e| format!("Failed to fetch caller member details: {e}"))?;

        let caller_perms = caller_member.permissions.unwrap_or_else(Permissions::empty);
        if !caller_perms.contains(Permissions::ADMINISTRATOR)
            && !caller_perms.contains(required_perm)
        {
            return Err("You lack the required Discord permissions to run this command.".to_string());
        }

        // Fetch target member if currently in guild
        if let Ok(target_member) = guild_id.member(&ctx.serenity_context().http, target_user.id).await {
            // Find highest role positions
            let caller_top_role = caller_member
                .roles
                .iter()
                .filter_map(|r| role_positions.get(r).copied())
                .max()
                .unwrap_or(0);

            let target_top_role = target_member
                .roles
                .iter()
                .filter_map(|r| role_positions.get(r).copied())
                .max()
                .unwrap_or(0);

            if target_top_role >= caller_top_role {
                return Err("You cannot moderate a member with an equal or higher role than yours.".to_string());
            }

            // Check bot's role vs target
            let bot_id = ctx.serenity_context().cache.current_user().id;
            let bot_member = guild_id
                .member(&ctx.serenity_context().http, bot_id)
                .await
                .map_err(|e| format!("Failed to fetch bot member details: {e}"))?;

            let bot_top_role = bot_member
                .roles
                .iter()
                .filter_map(|r| role_positions.get(r).copied())
                .max()
                .unwrap_or(0);

            if target_top_role >= bot_top_role {
                return Err("I cannot moderate this user because their role is equal to or higher than my highest role.".to_string());
            }

            return Ok(Some(target_member));
        }
    } else if let Ok(target_member) = guild_id.member(&ctx.serenity_context().http, target_user.id).await {
        return Ok(Some(target_member));
    }

    Ok(None)
}

/// Dispatches a direct message notification to the target user before punishment.
async fn notify_user_dm(
    ctx: Context<'_>,
    target_user: &User,
    action: &str,
    reason: &str,
) -> bool {
    let guild_name = ctx
        .guild()
        .map(|g| g.name.clone())
        .unwrap_or_else(|| "Discord Server".to_string());

    let message_text = format!(
        "⚠️ You received a moderation action in **{guild_name}**:\n**Action:** {action}\n**Reason:** {reason}"
    );

    match target_user.create_dm_channel(&ctx.serenity_context().http).await {
        Ok(channel) => channel.say(&ctx.serenity_context().http, message_text).await.is_ok(),
        Err(_) => false,
    }
}

// =========================================================================
// Commands
// =========================================================================

/// Warn a member in the server and log an infraction strike.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn warn(
    ctx: Context<'_>,
    #[description = "The user to warn"] user: User,
    #[rest]
    #[description = "Reason for the warning"]
    reason: Option<String>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let target_user = user;
    let target_id = target_user.id.get();
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let reason = reason.unwrap_or_else(|| "No reason provided".to_string());

    if let Err(err_msg) = validate_hierarchy(ctx, &target_user, Permissions::MODERATE_MEMBERS).await {
        ctx.say(format!("⛔ {err_msg}")).await?;
        return Ok(());
    }

    let dm_sent = notify_user_dm(ctx, &target_user, "Warn", &reason).await;

    // Create database mod case
    let storage = &ctx.data().storage;
    let new_case = NewModCase {
        guild_id,
        user_id: target_id,
        moderator_id: ctx.author().id.get(),
        action_type: "warn".to_string(),
        reason: reason.clone(),
        duration_seconds: None,
        created_at: Some(Utc::now().timestamp()),
        expires_at: None,
    };
    let case = storage.create_mod_case(&new_case)?;

    // Add strike
    let strike = NewModStrike {
        guild_id,
        user_id: target_id,
        case_id: Some(case.id),
        strike_count: 1,
        reason: reason.clone(),
        created_at: Some(Utc::now().timestamp()),
        expires_at: None,
    };
    let _ = storage.add_strike(&strike);

    log_mod_action(
        &ctx.serenity_context().http,
        storage,
        guild_id,
        Some(case.case_number),
        "Warn",
        target_id,
        &target_user.tag(),
        ctx.author().id.get(),
        &ctx.author().tag(),
        &reason,
        None,
    )
    .await;

    let dm_status = if dm_sent { " (User DMed)" } else { " *(DMs closed)*" };
    ctx.say(format!(
        "⚠️ **Warned** {} [`{target_id}`] | Case #{}{}\n**Reason:** {}",
        target_user.tag(),
        case.case_number,
        dm_status,
        reason
    ))
    .await?;

    Ok(())
}

/// Delete a warning case by case number.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn delwarn(
    ctx: Context<'_>,
    #[description = "The case number to remove"] case_number: i64,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let storage = &ctx.data().storage;

    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MODERATE_MEMBERS) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Moderate Members` permission to delete warnings.").await?;
        return Ok(());
    }

    let case = match storage.get_mod_case(guild_id, case_number)? {
        Some(c) => c,
        None => {
            ctx.say(format!("❌ Case #{case_number} was not found.")).await?;
            return Ok(());
        }
    };

    let _ = storage.deactivate_case(case.id);

    log_mod_action(
        &ctx.serenity_context().http,
        storage,
        guild_id,
        Some(case_number),
        "Case Deleted",
        case.user_id,
        &format!("<@{}>", case.user_id),
        ctx.author().id.get(),
        &ctx.author().tag(),
        &format!("Warning case #{case_number} deleted by moderator"),
        None,
    )
    .await;

    ctx.say(format!("🗑️ Case #{case_number} has been deleted.")).await?;
    Ok(())
}

/// View warning history for a user.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn warnings(
    ctx: Context<'_>,
    #[description = "The user to inspect"] user: User,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let storage = &ctx.data().storage;

    let cases = storage.get_mod_cases_for_user(guild_id, user.id.get())?;
    let warnings: Vec<_> = cases.into_iter().filter(|c| c.action_type == "warn").collect();

    if warnings.is_empty() {
        ctx.say(format!("✅ **{}** has no warnings on record.", user.tag())).await?;
        return Ok(());
    }

    let mut embed = CreateEmbed::new()
        .title(format!("Warnings for {} (Total: {})", user.tag(), warnings.len()))
        .color(Colour::new(0xFEE75C))
        .footer(CreateEmbedFooter::new(format!("User ID: {}", user.id)));

    for (idx, w) in warnings.iter().take(10).enumerate() {
        let active_mark = if w.active { "🟢 Active" } else { "⚪ Inactive" };
        embed = embed.field(
            format!("Case #{} - {}", w.case_number, active_mark),
            format!(
                "**Mod:** <@{}>\n**Reason:** {}\n**Date:** <t:{}:d>",
                w.moderator_id,
                truncate_chars(&w.reason, 200),
                w.created_at
            ),
            false,
        );
        if idx >= 9 {
            break;
        }
    }

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

/// View active strike count and details for a user.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn strikes(
    ctx: Context<'_>,
    #[description = "The user to inspect"] user: User,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let storage = &ctx.data().storage;

    let active_count = storage.get_active_strikes_count(guild_id, user.id.get())?;
    let strike_entries = storage.get_user_strikes(guild_id, user.id.get())?;

    let mut embed = CreateEmbed::new()
        .title(format!("Moderation Strikes for {}", user.tag()))
        .color(Colour::new(if active_count > 0 { 0xED4245 } else { 0x57F287 }))
        .field("Active Strikes", active_count.to_string(), true)
        .field("Total Records", strike_entries.len().to_string(), true)
        .footer(CreateEmbedFooter::new(format!("User ID: {}", user.id)));

    for s in strike_entries.iter().take(8) {
        let status = if s.decayed { "🍂 Decayed" } else { "⚡ Active" };
        embed = embed.field(
            format!("+{} Strikes ({status})", s.strike_count),
            format!("**Reason:** {}\n**Date:** <t:{}:R>", s.reason, s.created_at),
            false,
        );
    }

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

/// Timeout a member for a given duration.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn timeout(
    ctx: Context<'_>,
    #[description = "User to timeout"] user: User,
    #[description = "Duration (e.g. 10m, 1h, 1d)"] duration: String,
    #[rest]
    #[description = "Reason"]
    reason: Option<String>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let target_user = user;
    let target_id = target_user.id.get();
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let reason = reason.unwrap_or_else(|| "No reason provided".to_string());

    let dur = match parse_duration(&duration) {
        Some(d) => d,
        None => {
            ctx.say("❌ Invalid duration format. Examples: `10m`, `1h`, `1d`, `7d`.")
                .await?;
            return Ok(());
        }
    };

    if dur > Duration::from_secs(28 * 86400) {
        ctx.say("❌ Discord timeouts cannot exceed 28 days.").await?;
        return Ok(());
    }

    if let Err(err_msg) = validate_hierarchy(ctx, &target_user, Permissions::MODERATE_MEMBERS).await {
        ctx.say(format!("⛔ {err_msg}")).await?;
        return Ok(());
    }

    let dm_sent = notify_user_dm(ctx, &target_user, &format!("Timeout ({duration})"), &reason).await;

    let expires_at_ts = Utc::now().timestamp().saturating_add(dur.as_secs() as i64);
    let serenity_ts = match Timestamp::from_unix_timestamp(expires_at_ts) {
        Ok(t) => t,
        Err(_) => {
            ctx.say("❌ Could not compute timeout expiration timestamp.").await?;
            return Ok(());
        }
    };

    let edit = EditMember::new().disable_communication_until_datetime(serenity_ts);
    if let Err(e) = GuildId::new(guild_id).edit_member(&ctx.serenity_context().http, UserId::new(target_id), edit).await {
        ctx.say(format!("❌ Failed to apply timeout in Discord: {e}")).await?;
        return Ok(());
    }

    let storage = &ctx.data().storage;
    let new_case = NewModCase {
        guild_id,
        user_id: target_id,
        moderator_id: ctx.author().id.get(),
        action_type: "timeout".to_string(),
        reason: reason.clone(),
        duration_seconds: Some(dur.as_secs() as i64),
        created_at: Some(Utc::now().timestamp()),
        expires_at: Some(expires_at_ts),
    };
    let case = storage.create_mod_case(&new_case)?;

    log_mod_action(
        &ctx.serenity_context().http,
        storage,
        guild_id,
        Some(case.case_number),
        "Timeout",
        target_id,
        &target_user.tag(),
        ctx.author().id.get(),
        &ctx.author().tag(),
        &reason,
        Some(&duration),
    )
    .await;

    let dm_status = if dm_sent { " (User DMed)" } else { " *(DMs closed)*" };
    ctx.say(format!(
        "⏳ **Timed out** {} [`{target_id}`] for **{duration}** | Case #{}{}\n**Reason:** {}",
        target_user.tag(),
        case.case_number,
        dm_status,
        reason
    ))
    .await?;

    Ok(())
}

/// Remove a timeout from a member.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn untimeout(
    ctx: Context<'_>,
    #[description = "User to untimeout"] user: User,
    #[rest]
    #[description = "Reason"]
    reason: Option<String>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let target_user = user;
    let target_id = target_user.id.get();
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let reason = reason.unwrap_or_else(|| "No reason provided".to_string());

    if let Err(err_msg) = validate_hierarchy(ctx, &target_user, Permissions::MODERATE_MEMBERS).await {
        ctx.say(format!("⛔ {err_msg}")).await?;
        return Ok(());
    }

    let edit = EditMember::new().enable_communication();
    if let Err(e) = GuildId::new(guild_id).edit_member(&ctx.serenity_context().http, UserId::new(target_id), edit).await {
        ctx.say(format!("❌ Failed to lift timeout in Discord: {e}")).await?;
        return Ok(());
    }

    let storage = &ctx.data().storage;
    let new_case = NewModCase {
        guild_id,
        user_id: target_id,
        moderator_id: ctx.author().id.get(),
        action_type: "untimeout".to_string(),
        reason: reason.clone(),
        duration_seconds: None,
        created_at: Some(Utc::now().timestamp()),
        expires_at: None,
    };
    let case = storage.create_mod_case(&new_case)?;

    log_mod_action(
        &ctx.serenity_context().http,
        storage,
        guild_id,
        Some(case.case_number),
        "Untimeout",
        target_id,
        &target_user.tag(),
        ctx.author().id.get(),
        &ctx.author().tag(),
        &reason,
        None,
    )
    .await;

    ctx.say(format!(
        "🔊 **Lifted timeout** for {} [`{target_id}`] | Case #{}\n**Reason:** {}",
        target_user.tag(),
        case.case_number,
        reason
    ))
    .await?;

    Ok(())
}

/// Kick a member from the server.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn kick(
    ctx: Context<'_>,
    #[description = "Member to kick"] user: User,
    #[rest]
    #[description = "Reason for kick"]
    reason: Option<String>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let target_user = user;
    let target_id = target_user.id.get();
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let reason = reason.unwrap_or_else(|| "No reason provided".to_string());

    if let Err(err_msg) = validate_hierarchy(ctx, &target_user, Permissions::KICK_MEMBERS).await {
        ctx.say(format!("⛔ {err_msg}")).await?;
        return Ok(());
    }

    let dm_sent = notify_user_dm(ctx, &target_user, "Kick", &reason).await;

    if let Err(e) = GuildId::new(guild_id).kick_with_reason(&ctx.serenity_context().http, UserId::new(target_id), &reason).await {
        ctx.say(format!("❌ Failed to kick user: {e}")).await?;
        return Ok(());
    }

    let storage = &ctx.data().storage;
    let new_case = NewModCase {
        guild_id,
        user_id: target_id,
        moderator_id: ctx.author().id.get(),
        action_type: "kick".to_string(),
        reason: reason.clone(),
        duration_seconds: None,
        created_at: Some(Utc::now().timestamp()),
        expires_at: None,
    };
    let case = storage.create_mod_case(&new_case)?;

    log_mod_action(
        &ctx.serenity_context().http,
        storage,
        guild_id,
        Some(case.case_number),
        "Kick",
        target_id,
        &target_user.tag(),
        ctx.author().id.get(),
        &ctx.author().tag(),
        &reason,
        None,
    )
    .await;

    let dm_status = if dm_sent { " (User DMed)" } else { " *(DMs closed)*" };
    ctx.say(format!(
        "👢 **Kicked** {} [`{target_id}`] | Case #{}{}\n**Reason:** {}",
        target_user.tag(),
        case.case_number,
        dm_status,
        reason
    ))
    .await?;

    Ok(())
}

/// Softban a user: bans, purges 1-7 days of messages, and immediately unbans.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn softban(
    ctx: Context<'_>,
    #[description = "Member to softban"] user: User,
    #[description = "Days of messages to delete (1-7, default 1)"] delete_days: Option<u8>,
    #[rest]
    #[description = "Reason"]
    reason: Option<String>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let target_user = user;
    let target_id = target_user.id.get();
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let reason = reason.unwrap_or_else(|| "No reason provided".to_string());
    let days = delete_days.unwrap_or(1).clamp(1, 7);

    if let Err(err_msg) = validate_hierarchy(ctx, &target_user, Permissions::BAN_MEMBERS).await {
        ctx.say(format!("⛔ {err_msg}")).await?;
        return Ok(());
    }

    let dm_sent = notify_user_dm(ctx, &target_user, "Softban", &reason).await;

    let guild = GuildId::new(guild_id);
    let user_id = UserId::new(target_id);

    if let Err(e) = guild.ban_with_reason(&ctx.serenity_context().http, user_id, days, &reason).await {
        ctx.say(format!("❌ Failed to ban for softban: {e}")).await?;
        return Ok(());
    }

    // Immediately unban
    if let Err(e) = guild.unban(&ctx.serenity_context().http, user_id).await {
        ctx.say(format!("⚠️ User was banned but unban failed: {e}")).await?;
    }

    let storage = &ctx.data().storage;
    let new_case = NewModCase {
        guild_id,
        user_id: target_id,
        moderator_id: ctx.author().id.get(),
        action_type: "softban".to_string(),
        reason: reason.clone(),
        duration_seconds: None,
        created_at: Some(Utc::now().timestamp()),
        expires_at: None,
    };
    let case = storage.create_mod_case(&new_case)?;

    log_mod_action(
        &ctx.serenity_context().http,
        storage,
        guild_id,
        Some(case.case_number),
        "Softban",
        target_id,
        &target_user.tag(),
        ctx.author().id.get(),
        &ctx.author().tag(),
        &reason,
        None,
    )
    .await;

    let dm_status = if dm_sent { " (User DMed)" } else { " *(DMs closed)*" };
    ctx.say(format!(
        "🧹 **Softbanned** {} [`{target_id}`] (purged {days}d messages) | Case #{}{}\n**Reason:** {}",
        target_user.tag(),
        case.case_number,
        dm_status,
        reason
    ))
    .await?;

    Ok(())
}

/// Permanently ban a user from the server.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn ban(
    ctx: Context<'_>,
    #[description = "User to ban"] user: User,
    #[description = "Days of messages to delete (0-7, default 0)"] delete_days: Option<u8>,
    #[rest]
    #[description = "Reason"]
    reason: Option<String>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let target_user = user;
    let target_id = target_user.id.get();
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let reason = reason.unwrap_or_else(|| "No reason provided".to_string());
    let days = delete_days.unwrap_or(0).clamp(0, 7);

    if let Err(err_msg) = validate_hierarchy(ctx, &target_user, Permissions::BAN_MEMBERS).await {
        ctx.say(format!("⛔ {err_msg}")).await?;
        return Ok(());
    }

    let dm_sent = notify_user_dm(ctx, &target_user, "Ban", &reason).await;

    let guild = GuildId::new(guild_id);
    let user_id = UserId::new(target_id);

    if let Err(e) = guild.ban_with_reason(&ctx.serenity_context().http, user_id, days, &reason).await {
        ctx.say(format!("❌ Failed to ban user: {e}")).await?;
        return Ok(());
    }

    let storage = &ctx.data().storage;
    let new_case = NewModCase {
        guild_id,
        user_id: target_id,
        moderator_id: ctx.author().id.get(),
        action_type: "ban".to_string(),
        reason: reason.clone(),
        duration_seconds: None,
        created_at: Some(Utc::now().timestamp()),
        expires_at: None,
    };
    let case = storage.create_mod_case(&new_case)?;

    log_mod_action(
        &ctx.serenity_context().http,
        storage,
        guild_id,
        Some(case.case_number),
        "Ban",
        target_id,
        &target_user.tag(),
        ctx.author().id.get(),
        &ctx.author().tag(),
        &reason,
        None,
    )
    .await;

    let dm_status = if dm_sent { " (User DMed)" } else { " *(DMs closed)*" };
    ctx.say(format!(
        "🔨 **Banned** {} [`{target_id}`] | Case #{}{}\n**Reason:** {}",
        target_user.tag(),
        case.case_number,
        dm_status,
        reason
    ))
    .await?;

    Ok(())
}

/// Temporarily ban a user with automatic unban upon expiration.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn tempban(
    ctx: Context<'_>,
    #[description = "User to ban"] user: User,
    #[description = "Duration (e.g. 1d, 7d, 12h)"] duration: String,
    #[description = "Days of messages to delete (0-7, default 0)"] delete_days: Option<u8>,
    #[rest]
    #[description = "Reason"]
    reason: Option<String>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let target_user = user;
    let target_id = target_user.id.get();
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let reason = reason.unwrap_or_else(|| "No reason provided".to_string());
    let days = delete_days.unwrap_or(0).clamp(0, 7);

    let dur = match parse_duration(&duration) {
        Some(d) => d,
        None => {
            ctx.say("❌ Invalid duration format. Examples: `12h`, `1d`, `7d`.")
                .await?;
            return Ok(());
        }
    };

    if let Err(err_msg) = validate_hierarchy(ctx, &target_user, Permissions::BAN_MEMBERS).await {
        ctx.say(format!("⛔ {err_msg}")).await?;
        return Ok(());
    }

    let dm_sent = notify_user_dm(ctx, &target_user, &format!("Tempban ({duration})"), &reason).await;

    let guild = GuildId::new(guild_id);
    let user_id = UserId::new(target_id);

    if let Err(e) = guild.ban_with_reason(&ctx.serenity_context().http, user_id, days, &reason).await {
        ctx.say(format!("❌ Failed to tempban user: {e}")).await?;
        return Ok(());
    }

    let expires_at_ts = Utc::now().timestamp().saturating_add(dur.as_secs() as i64);
    let storage = &ctx.data().storage;
    let new_case = NewModCase {
        guild_id,
        user_id: target_id,
        moderator_id: ctx.author().id.get(),
        action_type: "tempban".to_string(),
        reason: reason.clone(),
        duration_seconds: Some(dur.as_secs() as i64),
        created_at: Some(Utc::now().timestamp()),
        expires_at: Some(expires_at_ts),
    };
    let case = storage.create_mod_case(&new_case)?;

    log_mod_action(
        &ctx.serenity_context().http,
        storage,
        guild_id,
        Some(case.case_number),
        "Tempban",
        target_id,
        &target_user.tag(),
        ctx.author().id.get(),
        &ctx.author().tag(),
        &reason,
        Some(&duration),
    )
    .await;

    let dm_status = if dm_sent { " (User DMed)" } else { " *(DMs closed)*" };
    ctx.say(format!(
        "⏳ **Tempbanned** {} [`{target_id}`] for **{duration}** | Case #{}{}\n**Reason:** {}",
        target_user.tag(),
        case.case_number,
        dm_status,
        reason
    ))
    .await?;

    Ok(())
}

/// Unban a user by their Discord user ID.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn unban(
    ctx: Context<'_>,
    #[description = "User ID to unban"] user_id: String,
    #[rest]
    #[description = "Reason"]
    reason: Option<String>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let reason = reason.unwrap_or_else(|| "No reason provided".to_string());

    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::BAN_MEMBERS) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Ban Members` permission to unban users.").await?;
        return Ok(());
    }

    let uid: u64 = match user_id.trim().parse() {
        Ok(u) => u,
        Err(_) => {
            ctx.say("❌ Invalid user ID. Please provide a numeric Discord Snowflake ID.").await?;
            return Ok(());
        }
    };

    let guild = GuildId::new(guild_id);
    if let Err(e) = guild.unban(&ctx.serenity_context().http, UserId::new(uid)).await {
        ctx.say(format!("❌ Failed to unban user: {e}")).await?;
        return Ok(());
    }

    let storage = &ctx.data().storage;
    let new_case = NewModCase {
        guild_id,
        user_id: uid,
        moderator_id: ctx.author().id.get(),
        action_type: "unban".to_string(),
        reason: reason.clone(),
        duration_seconds: None,
        created_at: Some(Utc::now().timestamp()),
        expires_at: None,
    };
    let case = storage.create_mod_case(&new_case)?;

    log_mod_action(
        &ctx.serenity_context().http,
        storage,
        guild_id,
        Some(case.case_number),
        "Unban",
        uid,
        &format!("<@{uid}>"),
        ctx.author().id.get(),
        &ctx.author().tag(),
        &reason,
        None,
    )
    .await;

    ctx.say(format!("🕊️ **Unbanned** `<@{uid}>` | Case #{}\n**Reason:** {reason}", case.case_number)).await?;
    Ok(())
}

/// Purge messages from the current channel (optionally filtered by user).
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn purge(
    ctx: Context<'_>,
    #[description = "Number of messages to delete (1-100)"] count: u64,
    #[description = "Filter deletion to a specific user"] user: Option<User>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MANAGE_MESSAGES) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Manage Messages` permission to purge messages.").await?;
        return Ok(());
    }

    let count = count.clamp(1, 100);
    let channel_id = ctx.channel_id();

    let fetched = channel_id
        .messages(&ctx.serenity_context().http, GetMessages::new().limit(count as u8))
        .await
        .map_err(|e| format!("Failed to fetch messages: {e}"))?;

    let message_ids: Vec<MessageId> = match user {
        Some(u) => fetched
            .into_iter()
            .filter(|m| m.author.id == u.id)
            .map(|m| m.id)
            .collect(),
        None => fetched.into_iter().map(|m| m.id).collect(),
    };

    let deleted_count = message_ids.len();
    if deleted_count > 0 {
        if deleted_count == 1 {
            if let Some(first_id) = message_ids.first() {
                let _ = channel_id.delete_message(&ctx.serenity_context().http, *first_id).await;
            }
        } else {
            let _ = channel_id.delete_messages(&ctx.serenity_context().http, message_ids).await;
        }
    }

    let reply = ctx.say(format!("🧹 Purged {deleted_count} messages.")).await?;
    // Auto-delete confirmation message after 4 seconds
    tokio::time::sleep(Duration::from_secs(4)).await;
    let _ = reply.delete(ctx).await;
    Ok(())
}

/// Set slowmode rate limit for the current channel.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn slowmode(
    ctx: Context<'_>,
    #[description = "Slowmode duration in seconds (0 to disable, max 21600)"] seconds: u16,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MANAGE_CHANNELS) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Manage Channels` permission.").await?;
        return Ok(());
    }

    let secs = seconds.min(21600);
    let edit = EditChannel::new().rate_limit_per_user(secs);
    ctx.channel_id().edit(&ctx.serenity_context().http, edit).await?;

    if secs == 0 {
        ctx.say("⏱️ Slowmode has been **disabled** for this channel.").await?;
    } else {
        ctx.say(format!("⏱️ Slowmode set to **{secs} seconds** per user.")).await?;
    }

    Ok(())
}

/// Lock the current channel from member messages.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn lock(
    ctx: Context<'_>,
    #[description = "Optional channel to lock (defaults to current)"] channel: Option<ChannelId>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MANAGE_CHANNELS) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Manage Channels` permission.").await?;
        return Ok(());
    }

    let target_channel = channel.unwrap_or_else(|| ctx.channel_id());
    let guild_id = ctx.guild_id().unwrap_or_default();
    let everyone_id = RoleId::new(guild_id.get());

    let overwrite = PermissionOverwrite {
        allow: Permissions::empty(),
        deny: Permissions::SEND_MESSAGES,
        kind: PermissionOverwriteType::Role(everyone_id),
    };

    target_channel
        .create_permission(&ctx.serenity_context().http, overwrite)
        .await?;

    ctx.say(format!("🔒 Channel <#{}> has been locked.", target_channel.get())).await?;
    Ok(())
}

/// Unlock the current channel allowing member messages.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn unlock(
    ctx: Context<'_>,
    #[description = "Optional channel to unlock (defaults to current)"] channel: Option<ChannelId>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MANAGE_CHANNELS) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Manage Channels` permission.").await?;
        return Ok(());
    }

    let target_channel = channel.unwrap_or_else(|| ctx.channel_id());
    let guild_id = ctx.guild_id().unwrap_or_default();
    let everyone_id = RoleId::new(guild_id.get());

    let _ = target_channel
        .delete_permission(&ctx.serenity_context().http, PermissionOverwriteType::Role(everyone_id))
        .await;

    ctx.say(format!("🔓 Channel <#{}> has been unlocked.", target_channel.get())).await?;
    Ok(())
}

/// Update the reason for an existing moderation case.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn reason(
    ctx: Context<'_>,
    #[description = "Case number to update"] case_number: i64,
    #[rest]
    #[description = "New reason for the case"]
    new_reason: String,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let storage = &ctx.data().storage;

    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MODERATE_MEMBERS) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Moderate Members` permission.").await?;
        return Ok(());
    }

    let updated = storage.update_case_reason(guild_id, case_number, &new_reason)?;
    if !updated {
        ctx.say(format!("❌ Case #{case_number} was not found.")).await?;
        return Ok(());
    }

    ctx.say(format!("📝 Updated reason for Case #{case_number}: {new_reason}")).await?;
    Ok(())
}

/// Display full details for a moderation case.
#[poise::command(slash_command, prefix_command, guild_only)]
pub async fn case(
    ctx: Context<'_>,
    #[description = "Case number to inspect"] case_number: i64,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let storage = &ctx.data().storage;

    let case = match storage.get_mod_case(guild_id, case_number)? {
        Some(c) => c,
        None => {
            ctx.say(format!("❌ Case #{case_number} was not found.")).await?;
            return Ok(());
        }
    };

    let status = if case.active { "🟢 Active" } else { "⚪ Inactive / Expired" };
    let color = match case.action_type.to_lowercase().as_str() {
        "warn" => 0xFEE75C,
        "timeout" => 0xE67E22,
        "kick" | "ban" | "tempban" | "softban" => 0xED4245,
        _ => 0x5865F2,
    };

    let mut embed = CreateEmbed::new()
        .title(format!("Case #{case_number} | {}", case.action_type.to_uppercase()))
        .color(Colour::new(color))
        .field("Target", format!("<@{}> (`{}`)", case.user_id, case.user_id), true)
        .field("Moderator", format!("<@{}> (`{}`)", case.moderator_id, case.moderator_id), true)
        .field("Status", status, true)
        .field("Reason", &case.reason, false)
        .field("Date", format!("<t:{}:F>", case.created_at), false)
        .footer(CreateEmbedFooter::new(format!("Case ID: {}", case.id)));

    if let Some(dur) = case.duration_seconds {
        embed = embed.field("Duration", format!("{dur} seconds"), true);
    }
    if let Some(exp) = case.expires_at {
        embed = embed.field("Expires", format!("<t:{exp}:R>"), true);
    }

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("30"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("0s"), Some(Duration::from_secs(0)));
    }

    #[test]
    fn test_parse_duration_minutes_hours_days_weeks() {
        assert_eq!(parse_duration("10m"), Some(Duration::from_secs(600)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("1d"), Some(Duration::from_secs(86400)));
        assert_eq!(parse_duration("7d"), Some(Duration::from_secs(7 * 86400)));
        assert_eq!(parse_duration("2w"), Some(Duration::from_secs(14 * 86400)));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("   "), None);
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("10x"), None);
        assert_eq!(parse_duration("-5m"), None);
    }

    #[test]
    fn test_mod_commands_suite_count() {
        let cmds = mod_commands();
        assert_eq!(cmds.len(), 17);
    }
}

