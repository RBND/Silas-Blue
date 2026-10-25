//! Multi-channel event logging for moderation actions, message edits/deletions,
//! member lifecycle, server changes, and voice state updates.

use chrono::Utc;
use poise::serenity_prelude as serenity;
use serenity::all::{
    ChannelId, Colour, CreateEmbed, CreateEmbedAuthor, CreateEmbedFooter, CreateMessage, Http,
    Timestamp, User,
};
use silas_core::cache::MessageRingBuffer;
use silas_core::text::truncate_chars;
use silas_storage::Storage;
use tracing::warn;

const COLOR_MOD: u32 = 0xED4245; // Discord Red
const COLOR_WARN: u32 = 0xFEE75C; // Discord Yellow
const COLOR_TIMEOUT: u32 = 0xE67E22; // Orange
const COLOR_INFO: u32 = 0x5865F2; // Discord Blurple
const COLOR_SUCCESS: u32 = 0x57F287; // Discord Green
const COLOR_MEMBER_LEAVE: u32 = 0x95A5A6; // Gray

/// Emits an audit log entry for moderation actions (warn, timeout, kick, ban, unban, etc.).
#[allow(clippy::too_many_arguments)]
pub async fn log_mod_action(
    http: &Http,
    storage: &Storage,
    guild_id: u64,
    case_number: Option<i64>,
    action: &str,
    target_id: u64,
    target_tag: &str,
    moderator_id: u64,
    moderator_tag: &str,
    reason: &str,
    duration_str: Option<&str>,
) {
    let cfg = match storage.get_logging_config(guild_id) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read logging config for guild {guild_id}: {e}");
            return;
        }
    };

    let target_channel_id = match cfg.mod_channel_id.or(cfg.unified_channel_id) {
        Some(cid) => cid,
        None => return,
    };

    let title = match case_number {
        Some(num) => format!("Case #{num} | {action}"),
        None => action.to_string(),
    };

    let color = match action.to_lowercase().as_str() {
        "warn" => COLOR_WARN,
        "timeout" | "untimeout" => COLOR_TIMEOUT,
        "kick" | "ban" | "tempban" | "softban" => COLOR_MOD,
        "unban" => COLOR_SUCCESS,
        _ => cfg.embed_color,
    };

    let now_ts = Timestamp::from_unix_timestamp(Utc::now().timestamp())
        .unwrap_or_else(|_| Timestamp::now());

    let mut embed = CreateEmbed::new()
        .title(title)
        .color(Colour::new(color))
        .field("Target", format!("{target_tag} (`{target_id}`)"), true)
        .field("Moderator", format!("{moderator_tag} (`{moderator_id}`)"), true)
        .field("Reason", truncate_chars(reason, 1000), false)
        .timestamp(now_ts)
        .footer(CreateEmbedFooter::new(format!("User ID: {target_id}")));

    if let Some(dur) = duration_str {
        embed = embed.field("Duration", dur, true);
    }

    let ch = ChannelId::new(target_channel_id);
    if let Err(e) = ch.send_message(http, CreateMessage::new().embed(embed)).await {
        warn!("Failed to deliver mod log to channel {target_channel_id}: {e}");
    }
}

/// Logs message deletions using information retrieved from the in-memory ring buffer cache.
pub async fn log_message_delete(
    http: &Http,
    storage: &Storage,
    cache: &MessageRingBuffer,
    channel_id: u64,
    message_id: u64,
    guild_id_opt: Option<u64>,
) {
    let cached = cache.get(message_id);
    let guild_id = match guild_id_opt.or_else(|| cached.as_ref().and_then(|m| m.guild_id)) {
        Some(gid) => gid,
        None => return,
    };

    let cfg = match storage.get_logging_config(guild_id) {
        Ok(c) => c,
        Err(_) => return,
    };

    if cfg.ignored_channels.contains(&channel_id) {
        return;
    }

    let target_channel_id = match cfg.message_channel_id.or(cfg.unified_channel_id) {
        Some(cid) => cid,
        None => return,
    };

    // Don't log self-deletions inside log channels to prevent feedback loops
    if target_channel_id == channel_id {
        return;
    }

    let now_ts = Timestamp::from_unix_timestamp(Utc::now().timestamp())
        .unwrap_or_else(|_| Timestamp::now());

    let mut embed = CreateEmbed::new()
        .title("Message Deleted")
        .color(Colour::new(COLOR_MOD))
        .field("Channel", format!("<#{channel_id}>"), true)
        .timestamp(now_ts)
        .footer(CreateEmbedFooter::new(format!("Message ID: {message_id}")));

    if let Some(msg) = cached {
        embed = embed.author(CreateEmbedAuthor::new(format!("{} (`{}`)", msg.author_tag, msg.author_id)));
        let content = if msg.content.trim().is_empty() {
            "*(No text content)*".to_string()
        } else {
            truncate_chars(&msg.content, 1000).to_string()
        };
        embed = embed.field("Content", content, false);

        if !msg.attachments.is_empty() {
            let list = msg.attachments.join("\n");
            embed = embed.field("Attachments", truncate_chars(&list, 1000), false);
        }
    } else {
        embed = embed.description("*(Message content was not cached)*");
    }

    let ch = ChannelId::new(target_channel_id);
    let _ = ch.send_message(http, CreateMessage::new().embed(embed)).await;
}

/// Logs message edits with before/after diffs using information from the message cache.
pub async fn log_message_edit(
    http: &Http,
    storage: &Storage,
    cache: &MessageRingBuffer,
    channel_id: u64,
    message_id: u64,
    new_content: &str,
    guild_id: u64,
) {
    let cfg = match storage.get_logging_config(guild_id) {
        Ok(c) => c,
        Err(_) => return,
    };

    if cfg.ignored_channels.contains(&channel_id) {
        return;
    }

    let target_channel_id = match cfg.message_channel_id.or(cfg.unified_channel_id) {
        Some(cid) => cid,
        None => return,
    };

    if target_channel_id == channel_id {
        return;
    }

    let cached = cache.get(message_id);
    let old_content = match cached.as_ref() {
        Some(m) => m.content.as_str(),
        None => "*(Original content not cached)*",
    };

    // If content did not change (e.g. only embeds were updated by Discord), do not log
    if old_content == new_content {
        return;
    }

    let now_ts = Timestamp::from_unix_timestamp(Utc::now().timestamp())
        .unwrap_or_else(|_| Timestamp::now());

    let mut embed = CreateEmbed::new()
        .title("Message Edited")
        .color(Colour::new(COLOR_WARN))
        .field("Channel", format!("<#{channel_id}>"), true)
        .field("Before", truncate_chars(old_content, 1000), false)
        .field("After", truncate_chars(new_content, 1000), false)
        .timestamp(now_ts)
        .footer(CreateEmbedFooter::new(format!("Message ID: {message_id}")));

    if let Some(msg) = cached {
        embed = embed.author(CreateEmbedAuthor::new(format!("{} (`{}`)", msg.author_tag, msg.author_id)));
    }

    let ch = ChannelId::new(target_channel_id);
    let _ = ch.send_message(http, CreateMessage::new().embed(embed)).await;
}

/// Logs member join events.
pub async fn log_member_join(
    http: &Http,
    storage: &Storage,
    guild_id: u64,
    user: &User,
    account_created_at: Option<i64>,
) {
    let cfg = match storage.get_logging_config(guild_id) {
        Ok(c) => c,
        Err(_) => return,
    };

    if user.bot && !cfg.log_bots {
        return;
    }

    let target_channel_id = match cfg.member_channel_id.or(cfg.unified_channel_id) {
        Some(cid) => cid,
        None => return,
    };

    let now_ts = Timestamp::from_unix_timestamp(Utc::now().timestamp())
        .unwrap_or_else(|_| Timestamp::now());

    let created_field = match account_created_at {
        Some(ts) => format!("<t:{ts}:R> (<t:{ts}:f>)"),
        None => "Unknown".to_string(),
    };

    let embed = CreateEmbed::new()
        .title("Member Joined")
        .color(Colour::new(COLOR_SUCCESS))
        .author(CreateEmbedAuthor::new(format!("{} (`{}`)", user.tag(), user.id)))
        .field("User", format!("<@{}>", user.id), true)
        .field("Account Created", created_field, false)
        .timestamp(now_ts)
        .footer(CreateEmbedFooter::new(format!("User ID: {}", user.id)));

    let ch = ChannelId::new(target_channel_id);
    let _ = ch.send_message(http, CreateMessage::new().embed(embed)).await;
}

/// Logs member departure / leave events.
pub async fn log_member_leave(
    http: &Http,
    storage: &Storage,
    guild_id: u64,
    user: &User,
) {
    let cfg = match storage.get_logging_config(guild_id) {
        Ok(c) => c,
        Err(_) => return,
    };

    if user.bot && !cfg.log_bots {
        return;
    }

    let target_channel_id = match cfg.member_channel_id.or(cfg.unified_channel_id) {
        Some(cid) => cid,
        None => return,
    };

    let now_ts = Timestamp::from_unix_timestamp(Utc::now().timestamp())
        .unwrap_or_else(|_| Timestamp::now());

    let embed = CreateEmbed::new()
        .title("Member Left")
        .color(Colour::new(COLOR_MEMBER_LEAVE))
        .author(CreateEmbedAuthor::new(format!("{} (`{}`)", user.tag(), user.id)))
        .field("User", format!("{user} (`{}`)", user.id), true)
        .timestamp(now_ts)
        .footer(CreateEmbedFooter::new(format!("User ID: {}", user.id)));

    let ch = ChannelId::new(target_channel_id);
    let _ = ch.send_message(http, CreateMessage::new().embed(embed)).await;
}

/// Logs member profile updates (nickname changes, role assignments/removals).
#[allow(clippy::too_many_arguments)]
pub async fn log_member_update(
    http: &Http,
    storage: &Storage,
    guild_id: u64,
    user: &User,
    old_nick: Option<&str>,
    new_nick: Option<&str>,
    added_roles: &[u64],
    removed_roles: &[u64],
) {
    let cfg = match storage.get_logging_config(guild_id) {
        Ok(c) => c,
        Err(_) => return,
    };

    if user.bot && !cfg.log_bots {
        return;
    }

    let target_channel_id = match cfg.member_channel_id.or(cfg.unified_channel_id) {
        Some(cid) => cid,
        None => return,
    };

    if old_nick == new_nick && added_roles.is_empty() && removed_roles.is_empty() {
        return;
    }

    let now_ts = Timestamp::from_unix_timestamp(Utc::now().timestamp())
        .unwrap_or_else(|_| Timestamp::now());

    let mut embed = CreateEmbed::new()
        .title("Member Updated")
        .color(Colour::new(COLOR_INFO))
        .author(CreateEmbedAuthor::new(format!("{} (`{}`)", user.tag(), user.id)))
        .timestamp(now_ts)
        .footer(CreateEmbedFooter::new(format!("User ID: {}", user.id)));

    if old_nick != new_nick {
        let before = old_nick.unwrap_or("*(None)*");
        let after = new_nick.unwrap_or("*(None)*");
        embed = embed.field("Nickname Change", format!("{before} -> {after}"), false);
    }

    if !added_roles.is_empty() {
        let roles: Vec<String> = added_roles.iter().map(|r| format!("<@&{r}>")).collect();
        embed = embed.field("Roles Added", roles.join(" "), false);
    }

    if !removed_roles.is_empty() {
        let roles: Vec<String> = removed_roles.iter().map(|r| format!("<@&{r}>")).collect();
        embed = embed.field("Roles Removed", roles.join(" "), false);
    }

    let ch = ChannelId::new(target_channel_id);
    let _ = ch.send_message(http, CreateMessage::new().embed(embed)).await;
}

/// Logs channel creation, deletion, or modification events.
pub async fn log_channel_change(
    http: &Http,
    storage: &Storage,
    guild_id: u64,
    action: &str,
    channel_name: &str,
    channel_id: u64,
) {
    let cfg = match storage.get_logging_config(guild_id) {
        Ok(c) => c,
        Err(_) => return,
    };

    let target_channel_id = match cfg.server_channel_id.or(cfg.unified_channel_id) {
        Some(cid) => cid,
        None => return,
    };

    let now_ts = Timestamp::from_unix_timestamp(Utc::now().timestamp())
        .unwrap_or_else(|_| Timestamp::now());

    let color = if action.contains("Delete") {
        COLOR_MOD
    } else if action.contains("Create") {
        COLOR_SUCCESS
    } else {
        COLOR_INFO
    };

    let embed = CreateEmbed::new()
        .title(format!("Channel {action}"))
        .color(Colour::new(color))
        .field("Name", channel_name, true)
        .field("Channel", format!("<#{channel_id}>"), true)
        .timestamp(now_ts)
        .footer(CreateEmbedFooter::new(format!("Channel ID: {channel_id}")));

    let ch = ChannelId::new(target_channel_id);
    let _ = ch.send_message(http, CreateMessage::new().embed(embed)).await;
}

/// Logs role creation, deletion, or modification events.
pub async fn log_role_change(
    http: &Http,
    storage: &Storage,
    guild_id: u64,
    action: &str,
    role_name: &str,
    role_id: u64,
) {
    let cfg = match storage.get_logging_config(guild_id) {
        Ok(c) => c,
        Err(_) => return,
    };

    let target_channel_id = match cfg.server_channel_id.or(cfg.unified_channel_id) {
        Some(cid) => cid,
        None => return,
    };

    let now_ts = Timestamp::from_unix_timestamp(Utc::now().timestamp())
        .unwrap_or_else(|_| Timestamp::now());

    let color = if action.contains("Delete") {
        COLOR_MOD
    } else if action.contains("Create") {
        COLOR_SUCCESS
    } else {
        COLOR_INFO
    };

    let embed = CreateEmbed::new()
        .title(format!("Role {action}"))
        .color(Colour::new(color))
        .field("Role", format!("{role_name} (`{role_id}`)"), true)
        .timestamp(now_ts)
        .footer(CreateEmbedFooter::new(format!("Role ID: {role_id}")));

    let ch = ChannelId::new(target_channel_id);
    let _ = ch.send_message(http, CreateMessage::new().embed(embed)).await;
}

/// Logs voice channel transitions (join, leave, switch, mute, deafen).
pub async fn log_voice_state(
    http: &Http,
    storage: &Storage,
    guild_id: u64,
    user: &User,
    change_description: &str,
) {
    let cfg = match storage.get_logging_config(guild_id) {
        Ok(c) => c,
        Err(_) => return,
    };

    if user.bot && !cfg.log_bots {
        return;
    }

    let target_channel_id = match cfg.voice_channel_id.or(cfg.unified_channel_id) {
        Some(cid) => cid,
        None => return,
    };

    let now_ts = Timestamp::from_unix_timestamp(Utc::now().timestamp())
        .unwrap_or_else(|_| Timestamp::now());

    let embed = CreateEmbed::new()
        .title("Voice State Update")
        .color(Colour::new(COLOR_INFO))
        .author(CreateEmbedAuthor::new(format!("{} (`{}`)", user.tag(), user.id)))
        .description(change_description)
        .timestamp(now_ts)
        .footer(CreateEmbedFooter::new(format!("User ID: {}", user.id)));

    let ch = ChannelId::new(target_channel_id);
    let _ = ch.send_message(http, CreateMessage::new().embed(embed)).await;
}
