//! Greetings, farewells, and autorole subsystem for Silas Blue.
//!
//! Evaluates dynamic templates via TagScript for welcome and farewell messages,
//! dispatches join DMs, and assigns human or bot autoroles with optional delay timers.

use std::sync::Arc;
use std::time::Duration;

use poise::serenity_prelude as serenity;
use serenity::all::{
    ChannelId, Colour, CreateEmbed, CreateEmbedFooter, CreateMessage, GuildId, Http, Member,
    RoleId, User,
};
use silas_core::tagscript::{evaluate_tagscript, TagScriptContext, TagScriptEmbed};
use silas_storage::Storage;
use tracing::{debug, info, warn};

/// Converts a parsed TagScript embed into a Serenity `CreateEmbed` structure.
pub fn tagscript_embed_to_serenity(t_embed: &TagScriptEmbed) -> CreateEmbed {
    let mut embed = CreateEmbed::new();
    if let Some(ref title) = t_embed.title {
        embed = embed.title(title);
    }
    if let Some(ref desc) = t_embed.description {
        embed = embed.description(desc);
    }
    if let Some(col) = t_embed.color {
        embed = embed.color(Colour::new(col));
    }
    if let Some(ref img) = t_embed.image_url {
        embed = embed.image(img);
    }
    if let Some(ref thumb) = t_embed.thumbnail_url {
        embed = embed.thumbnail(thumb);
    }
    if let Some(ref foot) = t_embed.footer {
        embed = embed.footer(CreateEmbedFooter::new(foot));
    }
    for field in &t_embed.fields {
        embed = embed.field(&field.name, &field.value, field.inline);
    }
    embed
}

/// Handler for `GuildMemberAddition` (member join).
pub async fn handle_member_join(
    http: &Arc<Http>,
    storage: &Storage,
    new_member: &Member,
) {
    let guild_id = new_member.guild_id.get();
    let config = match storage.get_greetings_config(guild_id) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to query greetings config for guild {guild_id}: {e}");
            return;
        }
    };

    let user = &new_member.user;
    let author_id = user.id.get();
    let author_name = user.name.clone();
    let author_avatar = user.avatar_url();
    let author_created_at = user.created_at().to_rfc3339();

    let ctx = TagScriptContext {
        author_id,
        author_name: author_name.clone(),
        author_avatar,
        author_created_at,
        target_id: Some(author_id),
        target_name: Some(author_name),
        guild_id: Some(guild_id),
        guild_name: None,
        guild_members: None,
        channel_id: config.welcome_channel_id,
        channel_name: None,
        custom_vars: std::collections::HashMap::new(),
    };

    // 1. Send Welcome Channel Message
    if let Some(welcome_channel) = config.welcome_channel_id {
        if !config.welcome_message.trim().is_empty() {
            let output = evaluate_tagscript(&config.welcome_message, &ctx);
            let mut builder = CreateMessage::new();
            if !output.body.trim().is_empty() {
                builder = builder.content(&output.body);
            }
            if let Some(ref emb) = output.embed {
                builder = builder.embed(tagscript_embed_to_serenity(emb));
            }

            let ch = ChannelId::new(welcome_channel);
            if let Err(e) = ch.send_message(http, builder).await {
                warn!("Failed to send welcome message to channel {welcome_channel}: {e}");
            } else {
                debug!(guild_id, user_id = author_id, "Sent welcome message");
            }
        }
    }

    // 2. Send Welcome Direct Message (if enabled)
    if config.welcome_dm_enabled && !config.welcome_dm_message.trim().is_empty() {
        let output = evaluate_tagscript(&config.welcome_dm_message, &ctx);
        let mut builder = CreateMessage::new();
        if !output.body.trim().is_empty() {
            builder = builder.content(&output.body);
        }
        if let Some(ref emb) = output.embed {
            builder = builder.embed(tagscript_embed_to_serenity(emb));
        }

        match user.create_dm_channel(http).await {
            Ok(dm_ch) => {
                if let Err(e) = dm_ch.send_message(http, builder).await {
                    debug!("Could not send welcome DM to user {author_id}: {e}");
                }
            }
            Err(e) => {
                debug!("Failed to open welcome DM channel with user {author_id}: {e}");
            }
        }
    }

    // 3. Autorole Assignment
    let role_ids = if user.bot {
        config.bot_autorole_ids
    } else {
        config.member_autorole_ids
    };

    if !role_ids.is_empty() {
        let http_clone = Arc::clone(http);
        let delay_secs = config.autorole_delay_seconds.max(0) as u64;
        let member_user_id = user.id;

        tokio::spawn(async move {
            if delay_secs > 0 {
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            }

            let serenity_roles: Vec<RoleId> = role_ids.into_iter().map(RoleId::new).collect();
            let guild = GuildId::new(guild_id);

            match guild.member(&http_clone, member_user_id).await {
                Ok(mem) => {
                    if let Err(e) = mem.add_roles(&http_clone, &serenity_roles).await {
                        warn!(
                            guild_id,
                            user_id = member_user_id.get(),
                            "Failed to assign autoroles: {e}"
                        );
                    } else {
                        info!(
                            guild_id,
                            user_id = member_user_id.get(),
                            "Assigned autoroles successfully"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        guild_id,
                        user_id = member_user_id.get(),
                        "Could not fetch member for autorole assignment: {e}"
                    );
                }
            }
        });
    }
}

/// Handler for `GuildMemberRemoval` (member leave/kick/ban).
pub async fn handle_member_remove(
    http: &Arc<Http>,
    storage: &Storage,
    guild_id: u64,
    user: &User,
) {
    let config = match storage.get_greetings_config(guild_id) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to query greetings config for guild {guild_id}: {e}");
            return;
        }
    };

    let Some(farewell_channel) = config.farewell_channel_id else {
        return;
    };

    if config.farewell_message.trim().is_empty() {
        return;
    }

    let author_id = user.id.get();
    let author_name = user.name.clone();
    let author_avatar = user.avatar_url();
    let author_created_at = user.created_at().to_rfc3339();

    let ctx = TagScriptContext {
        author_id,
        author_name: author_name.clone(),
        author_avatar,
        author_created_at,
        target_id: Some(author_id),
        target_name: Some(author_name),
        guild_id: Some(guild_id),
        guild_name: None,
        guild_members: None,
        channel_id: Some(farewell_channel),
        channel_name: None,
        custom_vars: std::collections::HashMap::new(),
    };

    let output = evaluate_tagscript(&config.farewell_message, &ctx);
    let mut builder = CreateMessage::new();
    if !output.body.trim().is_empty() {
        builder = builder.content(&output.body);
    }
    if let Some(ref emb) = output.embed {
        builder = builder.embed(tagscript_embed_to_serenity(emb));
    }

    let ch = ChannelId::new(farewell_channel);
    if let Err(e) = ch.send_message(http, builder).await {
        warn!("Failed to send farewell message to channel {farewell_channel}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silas_core::tagscript::TagScriptEmbedField;

    #[test]
    fn test_tagscript_embed_conversion() {
        let ts_embed = TagScriptEmbed {
            title: Some("Welcome Title".to_string()),
            description: Some("Welcome description text".to_string()),
            color: Some(0x57F287),
            image_url: Some("https://example.com/banner.png".to_string()),
            thumbnail_url: Some("https://example.com/thumb.png".to_string()),
            footer: Some("Welcome footer".to_string()),
            fields: vec![
                TagScriptEmbedField {
                    name: "Rules".to_string(),
                    value: "Be kind".to_string(),
                    inline: true,
                },
            ],
        };

        let serenity_embed = tagscript_embed_to_serenity(&ts_embed);
        // Verify serenity embed serialization does not panic
        let json = serde_json::to_value(&serenity_embed);
        assert!(json.is_ok());
    }
}

