//! Reaction and interactive component (buttons/selects) role manager.
//!
//! Supports modes:
//! - `normal`: standard toggle (react adds role, unreact removes role)
//! - `unique`: exclusive choice (grants chosen role and removes other group roles)
//! - `verify`: one-way assignment (react grants role; unreacting does not revoke)
//! - `drop`: reversed role (reacting removes role, unreacting restores role)
//! - `binding`: prerequisite required (only grants role if user possesses `required_role_id`)

use std::sync::Arc;

use poise::serenity_prelude as serenity;
use serenity::all::{
    ButtonStyle, ChannelId, Colour, ComponentInteraction, Context as SerenityContext, CreateActionRow,
    CreateButton, CreateEmbed, CreateEmbedFooter, CreateInteractionResponse,
    CreateInteractionResponseMessage, EditMessage, MessageId, Permissions, Reaction, ReactionType,
    Role, RoleId,
};
use silas_storage::{NewReactionRoleEntry, NewReactionRoleGroup};
use tracing::{debug, warn};

use crate::bot::BotState;

pub type CommandError = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Arc<BotState>, CommandError>;

/// Root command group for reaction roles.
#[poise::command(
    slash_command,
    prefix_command,
    guild_only,
    subcommands("add", "button_cmd", "list", "remove")
)]
pub async fn rr(ctx: Context<'_>) -> Result<(), CommandError> {
    ctx.say("Use `/rr add`, `/rr button`, `/rr list`, or `/rr remove`.")
        .await?;
    Ok(())
}

/// Attach an emoji reaction role to a message.
#[poise::command(slash_command, prefix_command, guild_only, rename = "add")]
pub async fn add(
    ctx: Context<'_>,
    #[description = "Message ID to attach reaction role to"] message_id: String,
    #[description = "Emoji to react with (e.g. ⭐, 🎮)"] emoji: String,
    #[description = "Role to assign"] role: Role,
    #[description = "Group mode: normal, unique, verify, drop, binding (default: normal)"]
    mode: Option<String>,
    #[description = "Required prerequisite role ID (for binding mode)"]
    required_role: Option<Role>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MANAGE_ROLES) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Manage Roles` permission.").await?;
        return Ok(());
    }

    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let channel_id = ctx.channel_id().get();
    let mid: u64 = match message_id.trim().parse() {
        Ok(m) => m,
        Err(_) => {
            ctx.say("❌ Invalid message ID format.").await?;
            return Ok(());
        }
    };

    let group_type = mode.unwrap_or_else(|| "normal".to_string()).to_lowercase();
    let valid_modes = ["normal", "unique", "verify", "drop", "binding"];
    if !valid_modes.contains(&group_type.as_str()) {
        ctx.say("❌ Invalid mode. Choose from: `normal`, `unique`, `verify`, `drop`, `binding`.").await?;
        return Ok(());
    }

    let storage = &ctx.data().storage;

    // Check if group already exists for this message
    let group = match storage.get_reaction_roles_for_message(guild_id, mid)? {
        Some(g) => g,
        None => {
            let new_group = NewReactionRoleGroup {
                guild_id,
                channel_id,
                message_id: mid,
                group_type: group_type.clone(),
                required_role_id: required_role.as_ref().map(|r| r.id.get()),
                component_type: "reaction".to_string(),
            };
            storage.create_reaction_role_group(&new_group)?
        }
    };

    let new_entry = NewReactionRoleEntry {
        group_id: group.id,
        emoji_or_custom_id: emoji.clone(),
        role_id: role.id.get(),
        button_label: None,
        button_style: None,
    };
    storage.add_reaction_role_entry(&new_entry)?;

    // Attempt to react to the target message with the specified emoji
    let ch = ChannelId::new(channel_id);
    let msg_id = MessageId::new(mid);
    if let Ok(target_msg) = ch.message(&ctx.serenity_context().http, msg_id).await {
        let reaction_type = if emoji.starts_with("<:") || emoji.starts_with("<a:") {
            ReactionType::try_from(emoji.as_str()).ok()
        } else {
            Some(ReactionType::Unicode(emoji.clone()))
        };
        if let Some(r_type) = reaction_type {
            let _ = target_msg.react(&ctx.serenity_context().http, r_type).await;
        }
    }

    ctx.say(format!(
        "✅ Reaction role registered!\n**Message:** `{mid}`\n**Emoji:** {emoji}\n**Role:** <@&{}>\n**Mode:** `{group_type}`",
        role.id.get()
    ))
    .await?;

    Ok(())
}

/// Attach an interactive button role to a message.
#[poise::command(slash_command, prefix_command, guild_only, rename = "button")]
pub async fn button_cmd(
    ctx: Context<'_>,
    #[description = "Message ID to attach button to"] message_id: String,
    #[description = "Button label text"] label: String,
    #[description = "Role to assign"] role: Role,
    #[description = "Style: primary, secondary, success, danger (default: primary)"]
    style: Option<String>,
    #[description = "Group mode: normal, unique, verify, drop, binding (default: normal)"]
    mode: Option<String>,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MANAGE_ROLES) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Manage Roles` permission.").await?;
        return Ok(());
    }

    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let channel_id = ctx.channel_id().get();
    let mid: u64 = match message_id.trim().parse() {
        Ok(m) => m,
        Err(_) => {
            ctx.say("❌ Invalid message ID format.").await?;
            return Ok(());
        }
    };

    let group_type = mode.unwrap_or_else(|| "normal".to_string()).to_lowercase();
    let btn_style_str = style.unwrap_or_else(|| "primary".to_string()).to_lowercase();

    let storage = &ctx.data().storage;
    let group = match storage.get_reaction_roles_for_message(guild_id, mid)? {
        Some(g) => g,
        None => {
            let new_group = NewReactionRoleGroup {
                guild_id,
                channel_id,
                message_id: mid,
                group_type: group_type.clone(),
                required_role_id: None,
                component_type: "button".to_string(),
            };
            storage.create_reaction_role_group(&new_group)?
        }
    };

    let custom_id = format!("rr:{}:{}", group.id, role.id.get());
    let new_entry = NewReactionRoleEntry {
        group_id: group.id,
        emoji_or_custom_id: custom_id.clone(),
        role_id: role.id.get(),
        button_label: Some(label.clone()),
        button_style: Some(btn_style_str.clone()),
    };
    storage.add_reaction_role_entry(&new_entry)?;

    // Fetch message and add button to components
    let ch = ChannelId::new(channel_id);
    let msg_id = MessageId::new(mid);
    if let Ok(mut target_msg) = ch.message(&ctx.serenity_context().http, msg_id).await {
        let b_style = match btn_style_str.as_str() {
            "secondary" => ButtonStyle::Secondary,
            "success" => ButtonStyle::Success,
            "danger" => ButtonStyle::Danger,
            _ => ButtonStyle::Primary,
        };

        let new_button = CreateButton::new(custom_id)
            .label(label)
            .style(b_style);

        let row = CreateActionRow::Buttons(vec![new_button]);
        let edit = EditMessage::new().components(vec![row]);
        if let Err(e) = target_msg.edit(&ctx.serenity_context().http, edit).await {
            warn!("Failed to edit message components with button: {e}");
        }
    }

    ctx.say(format!(
        "✅ Button role registered!\n**Message:** `{mid}`\n**Role:** <@&{}>\n**Mode:** `{group_type}`",
        role.id.get()
    ))
    .await?;

    Ok(())
}

/// List all reaction role groups in the current server.
#[poise::command(slash_command, prefix_command, guild_only, rename = "list")]
pub async fn list(ctx: Context<'_>) -> Result<(), CommandError> {
    ctx.defer().await?;
    let guild_id = ctx.guild_id().unwrap_or_default().get();
    let storage = &ctx.data().storage;

    let groups = storage.get_guild_reaction_roles(guild_id)?;
    if groups.is_empty() {
        ctx.say("ℹ️ No reaction roles configured in this server.").await?;
        return Ok(());
    }

    let mut embed = CreateEmbed::new()
        .title(format!("Reaction Roles (Total: {})", groups.len()))
        .color(Colour::new(0x5865F2))
        .footer(CreateEmbedFooter::new(format!("Guild ID: {guild_id}")));

    for g in groups.iter().take(10) {
        let mut entry_desc = String::new();
        for e in &g.entries {
            let label = e.button_label.as_deref().unwrap_or(&e.emoji_or_custom_id);
            entry_desc.push_str(&format!("• `{label}` -> <@&{}>\n", e.role_id));
        }
        if entry_desc.is_empty() {
            entry_desc = "*(No entries)*".to_string();
        }

        embed = embed.field(
            format!("Group #{} [{} | {}]", g.id, g.group_type, g.component_type),
            format!("**Channel:** <#{}>\n**Message:** `{}`\n{entry_desc}", g.channel_id, g.message_id),
            false,
        );
    }

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

/// Delete a reaction role group by group ID.
#[poise::command(slash_command, prefix_command, guild_only, rename = "remove")]
pub async fn remove(
    ctx: Context<'_>,
    #[description = "Group ID to remove"] group_id: i64,
) -> Result<(), CommandError> {
    ctx.defer().await?;
    let member = match ctx.author_member().await {
        Some(m) => m,
        None => return Ok(()),
    };
    let perms = member.permissions.unwrap_or_else(Permissions::empty);
    if !perms.contains(Permissions::MANAGE_ROLES) && !perms.contains(Permissions::ADMINISTRATOR) {
        ctx.say("⛔ You lack `Manage Roles` permission.").await?;
        return Ok(());
    }

    let storage = &ctx.data().storage;
    let deleted = storage.delete_reaction_role_group(group_id)?;
    if deleted {
        ctx.say(format!("🗑️ Deleted reaction role group #{group_id}.")).await?;
    } else {
        ctx.say(format!("❌ Group #{group_id} not found.")).await?;
    }

    Ok(())
}

// =========================================================================
// Event Handlers for Reaction & Component Interactions
// =========================================================================

/// Handles `ReactionAdd` events for emoji reaction roles.
pub async fn handle_reaction_add(
    ctx: &SerenityContext,
    reaction: &Reaction,
    state: &Arc<BotState>,
) {
    let Some(guild_id) = reaction.guild_id else {
        return;
    };
    let Some(user_id) = reaction.user_id else {
        return;
    };

    // Ignore bot reactions
    if user_id == ctx.cache.current_user().id {
        return;
    }

    let message_id = reaction.message_id.get();
    let gid = guild_id.get();

    let group = match state.storage.get_reaction_roles_for_message(gid, message_id) {
        Ok(Some(g)) => g,
        _ => return,
    };

    let emoji_str = reaction.emoji.to_string();
    let matched_entry = group.entries.iter().find(|e| {
        e.emoji_or_custom_id == emoji_str
            || emoji_str.contains(&e.emoji_or_custom_id)
            || e.emoji_or_custom_id.contains(&emoji_str)
    });

    let Some(entry) = matched_entry else {
        return;
    };

    let Ok(member) = guild_id.member(&ctx.http, user_id).await else {
        return;
    };

    let role_id = RoleId::new(entry.role_id);

    match group.group_type.as_str() {
        "normal" | "verify" => {
            let _ = member.add_role(&ctx.http, role_id).await;
        }
        "unique" => {
            // Remove other roles in this group
            for other in &group.entries {
                if other.role_id != entry.role_id {
                    let other_id = RoleId::new(other.role_id);
                    if member.roles.contains(&other_id) {
                        let _ = member.remove_role(&ctx.http, other_id).await;
                    }
                }
            }
            let _ = member.add_role(&ctx.http, role_id).await;
        }
        "drop" => {
            // Drop mode: adding reaction removes role
            let _ = member.remove_role(&ctx.http, role_id).await;
        }
        "binding" => {
            // Requires required_role_id
            if let Some(req_id) = group.required_role_id {
                if member.roles.contains(&RoleId::new(req_id)) {
                    let _ = member.add_role(&ctx.http, role_id).await;
                } else {
                    // Remove the reaction since user lacks required prerequisite
                    let _ = reaction.delete(&ctx.http).await;
                }
            }
        }
        _ => {}
    }
}

/// Handles `ReactionRemove` events for emoji reaction roles.
pub async fn handle_reaction_remove(
    ctx: &SerenityContext,
    reaction: &Reaction,
    state: &Arc<BotState>,
) {
    let Some(guild_id) = reaction.guild_id else {
        return;
    };
    let Some(user_id) = reaction.user_id else {
        return;
    };

    if user_id == ctx.cache.current_user().id {
        return;
    }

    let message_id = reaction.message_id.get();
    let gid = guild_id.get();

    let group = match state.storage.get_reaction_roles_for_message(gid, message_id) {
        Ok(Some(g)) => g,
        _ => return,
    };

    // Verify mode is one-way: removing reaction does NOT remove role
    if group.group_type == "verify" {
        return;
    }

    let emoji_str = reaction.emoji.to_string();
    let matched_entry = group.entries.iter().find(|e| {
        e.emoji_or_custom_id == emoji_str
            || emoji_str.contains(&e.emoji_or_custom_id)
            || e.emoji_or_custom_id.contains(&emoji_str)
    });

    let Some(entry) = matched_entry else {
        return;
    };

    let Ok(member) = guild_id.member(&ctx.http, user_id).await else {
        return;
    };

    let role_id = RoleId::new(entry.role_id);

    match group.group_type.as_str() {
        "normal" | "unique" | "binding" => {
            let _ = member.remove_role(&ctx.http, role_id).await;
        }
        "drop" => {
            // Drop mode: removing reaction restores role
            let _ = member.add_role(&ctx.http, role_id).await;
        }
        _ => {}
    }
}

/// Handles component button clicks for role buttons (`rr:{group_id}:{role_id}`).
pub async fn handle_component_interaction(
    ctx: &SerenityContext,
    comp: &ComponentInteraction,
    state: &Arc<BotState>,
) {
    let custom_id = comp.data.custom_id.as_str();
    if !custom_id.starts_with("rr:") {
        return;
    }

    let Some(guild_id) = comp.guild_id else {
        return;
    };

    let parts: Vec<&str> = custom_id.split(':').collect();
    let group_id_str = match parts.get(1) {
        Some(s) => *s,
        None => return,
    };
    let role_id_str = match parts.get(2) {
        Some(s) => *s,
        None => return,
    };

    let Ok(group_id) = group_id_str.parse::<i64>() else {
        return;
    };
    let Ok(role_id_raw) = role_id_str.parse::<u64>() else {
        return;
    };

    let Ok(member) = guild_id.member(&ctx.http, comp.user.id).await else {
        return;
    };

    let role_id = RoleId::new(role_id_raw);
    let has_role = member.roles.contains(&role_id);

    // Look up group configuration
    let groups = state.storage.get_guild_reaction_roles(guild_id.get()).unwrap_or_default();
    let group = groups.into_iter().find(|g| g.id == group_id);

    let (reply_msg, change_applied) = match group.as_ref().map(|g| g.group_type.as_str()) {
        Some("unique") => {
            if let Some(ref g) = group {
                for other in &g.entries {
                    if other.role_id != role_id_raw {
                        let other_id = RoleId::new(other.role_id);
                        if member.roles.contains(&other_id) {
                            let _ = member.remove_role(&ctx.http, other_id).await;
                        }
                    }
                }
            }
            if has_role {
                let _ = member.remove_role(&ctx.http, role_id).await;
                (format!("Removed role <@&{role_id_raw}>."), true)
            } else {
                let _ = member.add_role(&ctx.http, role_id).await;
                (format!("Assigned exclusive role <@&{role_id_raw}>."), true)
            }
        }
        Some("verify") => {
            if has_role {
                ("You are already verified with this role.".to_string(), false)
            } else {
                let _ = member.add_role(&ctx.http, role_id).await;
                (format!("Assigned role <@&{role_id_raw}>."), true)
            }
        }
        Some("drop") => {
            if has_role {
                let _ = member.remove_role(&ctx.http, role_id).await;
                (format!("Dropped role <@&{role_id_raw}>."), true)
            } else {
                let _ = member.add_role(&ctx.http, role_id).await;
                (format!("Restored role <@&{role_id_raw}>."), true)
            }
        }
        Some("binding") => {
            if let Some(req_id) = group.and_then(|g| g.required_role_id) {
                if !member.roles.contains(&RoleId::new(req_id)) {
                    ("⛔ You lack the prerequisite role required for this button.".to_string(), false)
                } else if has_role {
                    let _ = member.remove_role(&ctx.http, role_id).await;
                    (format!("Removed role <@&{role_id_raw}>."), true)
                } else {
                    let _ = member.add_role(&ctx.http, role_id).await;
                    (format!("Assigned role <@&{role_id_raw}>."), true)
                }
            } else {
                ("Invalid binding group configuration.".to_string(), false)
            }
        }
        _ => {
            // Normal toggle
            if has_role {
                let _ = member.remove_role(&ctx.http, role_id).await;
                (format!("Removed role <@&{role_id_raw}>."), true)
            } else {
                let _ = member.add_role(&ctx.http, role_id).await;
                (format!("Assigned role <@&{role_id_raw}>."), true)
            }
        }
    };

    let resp = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(reply_msg)
            .ephemeral(true),
    );
    let _ = comp.create_response(&ctx.http, resp).await;
    debug!(user_id = comp.user.id.get(), role_id = role_id_raw, change_applied, "Processed component role interaction");
}
