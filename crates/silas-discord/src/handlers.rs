//! Discord Gateway event handlers (`FullEvent`) implementing message inference, forum auto-reply,
//! interactive pagination, and Discord ToS data deletion compliance.

use crate::bot::{extract_connected_guild_info, BotState, ForumThreadState};
use chrono::Utc;
use poise::serenity_prelude as serenity;
use rand::Rng;
use serenity::all::{
    ChannelId, Context, CreateMessage, FullEvent, GetMessages, GuildChannel, Interaction, Message,
    MessageId, MessageType,
};
use silas_ai::CompletionRequest;
use silas_core::config::{compose_system_prompt, GuildConfig};
use silas_core::events::TelemetryEvent;
use silas_core::memory::ContextKey;
use silas_core::pagination::paginate_markdown;
use silas_core::permissions::{
    has_dm_permission, has_guild_permission, MemberContext, PermissionDomain,
};
use silas_core::quota::{calculate_window_period, evaluate_quota, QuotaReservationGuard};
use silas_core::text::truncate_chars;
use silas_search::grounding::{format_grounded_rag_block, is_refusal_response};
use silas_search::intent::{build_search_query, should_auto_search};
use silas_storage::TokenUsageEntry;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_TRACKED_FORUM_THREADS: usize = 5_000;
const MIN_SPONTANEOUS_COOLDOWN_SECS: u64 = 60;

/// Main Gateway event dispatcher invoked by Poise for every Serenity `FullEvent`.
pub async fn handle_gateway_event(ctx: &Context, event: &FullEvent, state: &Arc<BotState>) {
    match event {
        FullEvent::GuildCreate { guild, is_new } => {
            let bot_uid = state
                .bot_identity
                .read()
                .id
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok());
            let info = extract_connected_guild_info(guild, bot_uid);
            state.connected_guilds.insert(guild.id.get(), info);

            if is_new.unwrap_or(false) {
                state.telemetry.emit(TelemetryEvent::ServerJoined {
                    guild_name: guild.name.clone(),
                    guild_id: guild.id.get().to_string(),
                });
            }
        }
        FullEvent::GuildUpdate { new_data, .. } => {
            if let Some(mut entry) = state.connected_guilds.get_mut(&new_data.id.get()) {
                entry.name = new_data.name.clone();
                entry.icon_url = new_data.icon_url();
            }
        }
        FullEvent::GuildDelete { incomplete, full } => {
            let guild_id = incomplete.id.get();
            let guild_name = full
                .as_ref()
                .map(|g| g.name.clone())
                .or_else(|| state.connected_guilds.get(&guild_id).map(|g| g.name.clone()))
                .unwrap_or_else(|| guild_id.to_string());
            state.connected_guilds.remove(&guild_id);

            // Discord ToS Compliance: If `unavailable == false`, the bot was permanently removed/kicked
            // from the server (or the server was deleted). Immediately purge volatile memory and SQLite data!
            if !incomplete.unavailable {
                state.memory.clear_guild(guild_id);
                if let Err(e) = state.storage.purge_guild_data(guild_id) {
                    tracing::warn!("Failed to purge guild data for removed guild {guild_id}: {e}");
                } else {
                    state.telemetry.emit(TelemetryEvent::ServerLeft {
                        guild_name,
                        guild_id: guild_id.to_string(),
                    });
                }
            }
        }
        FullEvent::MessageDelete {
            channel_id,
            deleted_message_id,
            guild_id,
        } => {
            state
                .memory
                .evict_message(channel_id.get(), deleted_message_id.get());

            crate::logging::log_message_delete(
                &ctx.http,
                &state.storage,
                &state.message_cache,
                channel_id.get(),
                deleted_message_id.get(),
                guild_id.map(|g| g.get()),
            )
            .await;
        }
        FullEvent::MessageDeleteBulk {
            channel_id,
            multiple_deleted_messages_ids,
            guild_id,
        } => {
            let ids: Vec<u64> = multiple_deleted_messages_ids
                .iter()
                .map(|m| m.get())
                .collect();
            state.memory.evict_messages_bulk(channel_id.get(), &ids);

            for mid in &ids {
                crate::logging::log_message_delete(
                    &ctx.http,
                    &state.storage,
                    &state.message_cache,
                    channel_id.get(),
                    *mid,
                    guild_id.map(|g| g.get()),
                )
                .await;
            }
        }
        FullEvent::MessageUpdate { event, .. } => {
            if let (Some(gid), Some(ref new_text)) = (event.guild_id, &event.content) {
                crate::logging::log_message_edit(
                    &ctx.http,
                    &state.storage,
                    &state.message_cache,
                    event.channel_id.get(),
                    event.id.get(),
                    new_text,
                    gid.get(),
                )
                .await;
            }
        }
        FullEvent::ChannelCreate { channel } => {
            crate::logging::log_channel_change(
                &ctx.http,
                &state.storage,
                channel.guild_id.get(),
                "Created",
                &channel.name,
                channel.id.get(),
            )
            .await;
        }
        FullEvent::ChannelDelete { channel, .. } => {
            state.memory.clear_channel(channel.id.get());
            if let Some(mut g) = state.connected_guilds.get_mut(&channel.guild_id.get()) {
                let cid = channel.id.get().to_string();
                g.channels.retain(|c| c.id != cid);
            }
            crate::logging::log_channel_change(
                &ctx.http,
                &state.storage,
                channel.guild_id.get(),
                "Deleted",
                &channel.name,
                channel.id.get(),
            )
            .await;
        }
        FullEvent::ChannelUpdate { old: _, new } => {
            crate::logging::log_channel_change(
                &ctx.http,
                &state.storage,
                new.guild_id.get(),
                "Updated",
                &new.name,
                new.id.get(),
            )
            .await;
        }
        FullEvent::GuildMemberAddition { new_member } => {
            crate::logging::log_member_join(
                &ctx.http,
                &state.storage,
                new_member.guild_id.get(),
                &new_member.user,
                Some(new_member.user.created_at().unix_timestamp()),
            )
            .await;
            crate::greetings::handle_member_join(&ctx.http, &state.storage, new_member).await;
        }
        FullEvent::GuildMemberRemoval {
            guild_id,
            user,
            member_data_if_available: _,
        } => {
            state.member_role_cache.remove(guild_id.get(), user.id.get());
            crate::logging::log_member_leave(&ctx.http, &state.storage, guild_id.get(), user).await;
            crate::greetings::handle_member_remove(&ctx.http, &state.storage, guild_id.get(), user).await;
        }
        FullEvent::GuildMemberUpdate {
            old_if_available,
            new,
            event,
        } => {
            let gid = event.guild_id.get();
            let uid = event.user.id.get();
            let old_nick = old_if_available.as_ref().and_then(|m| m.nick.as_deref());
            let new_nick = new.as_ref().and_then(|m| m.nick.as_deref());

            let new_roles: Vec<u64> = new
                .as_ref()
                .map(|m| m.roles.iter().map(|r| r.get()).collect())
                .unwrap_or_else(|| event.roles.iter().map(|r| r.get()).collect());

            // Retrieve old roles from Serenity cache or our specialized MemberRoleCache.
            // If neither has baseline role knowledge, do NOT assume old roles was empty [],
            // which previously caused false "all roles added" diff logs.
            let cached_baseline = old_if_available
                .as_ref()
                .map(|m| m.roles.iter().map(|r| r.get()).collect::<Vec<u64>>())
                .or_else(|| state.member_role_cache.get(gid, uid));

            let (added_roles, removed_roles) = match cached_baseline {
                Some(old_roles) => {
                    let added: Vec<u64> = new_roles
                        .iter()
                        .filter(|r| !old_roles.contains(r))
                        .copied()
                        .collect();
                    let removed: Vec<u64> = old_roles
                        .iter()
                        .filter(|r| !new_roles.contains(r))
                        .copied()
                        .collect();
                    (added, removed)
                }
                None => {
                    // Cache miss: We don't have a reliable baseline, suppress emitting false additions.
                    (Vec::new(), Vec::new())
                }
            };

            // Update MemberRoleCache with latest known roles
            state.member_role_cache.insert(gid, uid, new_roles.clone());

            crate::logging::log_member_update(
                &ctx.http,
                &state.storage,
                gid,
                &event.user,
                old_nick,
                new_nick,
                &added_roles,
                &removed_roles,
            )
            .await;
        }
        FullEvent::GuildRoleCreate { new } => {
            crate::logging::log_role_change(
                &ctx.http,
                &state.storage,
                new.guild_id.get(),
                "Created",
                &new.name,
                new.id.get(),
            )
            .await;
        }
        FullEvent::GuildRoleDelete {
            guild_id,
            removed_role_id,
            removed_role_data_if_available,
        } => {
            let role_name = removed_role_data_if_available
                .as_ref()
                .map(|r| r.name.as_str())
                .unwrap_or("Unknown Role");
            crate::logging::log_role_change(
                &ctx.http,
                &state.storage,
                guild_id.get(),
                "Deleted",
                role_name,
                removed_role_id.get(),
            )
            .await;
        }
        FullEvent::GuildRoleUpdate {
            old_data_if_available: _,
            new,
        } => {
            crate::logging::log_role_change(
                &ctx.http,
                &state.storage,
                new.guild_id.get(),
                "Updated",
                &new.name,
                new.id.get(),
            )
            .await;
        }
        FullEvent::VoiceStateUpdate { old, new } => {
            if let Some(guild_id) = new.guild_id {
                let user_opt = new.member.as_ref().map(|m| &m.user);
                if let Some(user) = user_opt {
                    let desc = match (old.as_ref().and_then(|o| o.channel_id), new.channel_id) {
                        (None, Some(cid)) => format!("Joined voice channel <#{cid}>"),
                        (Some(cid), None) => format!("Left voice channel <#{cid}>"),
                        (Some(old_cid), Some(new_cid)) if old_cid != new_cid => {
                            format!("Moved voice channel: <#{old_cid}> -> <#{new_cid}>")
                        }
                        _ => {
                            if new.self_mute != old.as_ref().map(|o| o.self_mute).unwrap_or(false) {
                                if new.self_mute {
                                    "Muted self".to_string()
                                } else {
                                    "Unmuted self".to_string()
                                }
                            } else if new.self_deaf != old.as_ref().map(|o| o.self_deaf).unwrap_or(false) {
                                if new.self_deaf {
                                    "Deafened self".to_string()
                                } else {
                                    "Undeafened self".to_string()
                                }
                            } else {
                                String::new()
                            }
                        }
                    };
                    if !desc.is_empty() {
                        crate::logging::log_voice_state(&ctx.http, &state.storage, guild_id.get(), user, &desc).await;
                    }
                }
            }
        }
        FullEvent::ReactionAdd { add_reaction } => {
            crate::reaction_roles::handle_reaction_add(ctx, add_reaction, state).await;
        }
        FullEvent::ReactionRemove { removed_reaction } => {
            crate::reaction_roles::handle_reaction_remove(ctx, removed_reaction, state).await;
        }
        FullEvent::ThreadDelete { thread, .. } => {
            state.memory.clear_channel(thread.id.get());
            state.forum_replied_threads.remove(&thread.id.get());
        }
        FullEvent::InteractionCreate {
            interaction: Interaction::Component(comp),
            ..
        } => {
            let handled = state.pagination.handle_interaction(ctx, comp).await;
            if !handled {
                crate::reaction_roles::handle_component_interaction(ctx, comp, state).await;
            }
        }
        FullEvent::ThreadCreate { thread, .. } => {
            handle_forum_thread_create(ctx, thread, state).await;
        }
        FullEvent::Message { new_message, .. } => {
            // Cache incoming message for LRU audit diffs
            let attachments = new_message
                .attachments
                .iter()
                .map(|a| a.url.clone())
                .collect();

            state.message_cache.insert(silas_core::cache::CachedMessage {
                id: new_message.id.get(),
                guild_id: new_message.guild_id.map(|g| g.get()),
                channel_id: new_message.channel_id.get(),
                author_id: new_message.author.id.get(),
                author_tag: new_message.author.tag(),
                author_avatar: new_message.author.avatar_url(),
                content: new_message.content.clone(),
                attachments,
                timestamp: new_message.timestamp.unix_timestamp(),
            });

            // 1. Automod evaluation
            if let Some(infraction) = crate::automod::evaluate_message(ctx, new_message, state).await {
                crate::automod::handle_infraction(ctx, new_message, state, infraction).await;
                return;
            }

            // 2. Custom triggers / tags evaluation
            if crate::triggers::evaluate_message_triggers(ctx, new_message, state).await {
                return;
            }

            // 3. AI conversation / forum auto-reply / spontaneous chatter
            handle_incoming_message(ctx, new_message, state).await;
        }
        _ => {}
    }
}

/// Handle `FullEvent::ThreadCreate` for Forum Auto-Reply.
async fn handle_forum_thread_create(
    ctx: &Context,
    thread: &GuildChannel,
    state: &Arc<BotState>,
) {
    let Some(parent_id) = thread.parent_id else {
        return;
    };
    let guild_id = thread.guild_id.get();
    let cfg = state.storage.get_guild_config(guild_id);

    if !cfg.forum_auto_reply_enabled || cfg.forum_delivery_mode == "disabled" {
        return;
    }

    // If specific forum channels are monitored, verify parent_id is in the allowed set
    if !cfg.compiled.monitored_forum_channels.is_empty()
        && !cfg.compiled.monitored_forum_channels.contains(&parent_id.get())
    {
        return;
    }

    let thread_id = thread.id.get();

    // Evict oldest forum thread entries if cache exceeds bound
    if state.forum_replied_threads.len() >= MAX_TRACKED_FORUM_THREADS {
        let now = Instant::now();
        state.forum_replied_threads.retain(|_, st| match st {
            ForumThreadState::InFlight(t) | ForumThreadState::Completed(t) => {
                now.duration_since(*t) < Duration::from_secs(3600)
            }
        });
    }

    // Atomic check-and-insert `InFlight` so concurrent MessageCreate or duplicate ThreadCreate cannot double-reply
    if state.forum_replied_threads.contains_key(&thread_id) {
        return;
    }
    state
        .forum_replied_threads
        .insert(thread_id, ForumThreadState::InFlight(Instant::now()));

    // Fetch starter post with up to 3 retries (500ms, 1000ms, 2000ms) in case Discord hasn't indexed the first message yet
    let mut starter_msg: Option<Message> = None;
    for delay_ms in [500u64, 1000, 2000] {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        if let Ok(mut msgs) = thread
            .id
            .messages(&ctx.http, GetMessages::new().limit(1))
            .await
        {
            if let Some(first) = msgs.pop() {
                starter_msg = Some(first);
                break;
            }
        }
    }

    let Some(starter) = starter_msg else {
        state.forum_replied_threads.remove(&thread_id);
        return;
    };

    // Never auto-reply to bots or webhooks
    if starter.author.bot || starter.webhook_id.is_some() {
        state
            .forum_replied_threads
            .insert(thread_id, ForumThreadState::Completed(Instant::now()));
        return;
    }

    let title = thread.name.trim();
    let body = starter.content.trim();
    let combined_prompt = if cfg.forum_include_post_title {
        if body.is_empty() {
            format!("Forum Thread Title: {title}")
        } else {
            format!("Forum Thread Title: {title}\n\nPost Content:\n{body}")
        }
    } else if body.is_empty() {
        title.to_string()
    } else {
        body.to_string()
    };

    let settings = state.storage.get_app_settings();
    let model = state
        .resolve_model_for_context(&settings, Some(&cfg))
        .await;

    let mut rag_block: Option<String> = None;
    let mut citation_footer = String::new();

    if cfg.web_search_enabled && cfg.web_search_auto && settings.web_search_enabled && should_auto_search(&combined_prompt) {
        let search_q = build_search_query(&combined_prompt, "", &[]);
        let results = state.search.search(&search_q, None, &settings).await;
        if !results.is_empty() {
            let (blk, foot) = format_grounded_rag_block(&results);
            rag_block = blk;
            citation_footer = foot;
        }
    }

    let custom_persona = if !cfg.forum_system_prompt.trim().is_empty() {
        Some(cfg.forum_system_prompt.as_str())
    } else if !cfg.system_prompt.trim().is_empty() {
        Some(cfg.system_prompt.as_str())
    } else {
        None
    };

    let system_prompt = compose_system_prompt(
        &settings.system_prompt,
        custom_persona,
        rag_block.as_deref(),
    );

    let _typing = thread.id.start_typing(&ctx.http);

    let req = CompletionRequest {
        prompt: format!("[{}]: {}", starter.author.name, combined_prompt),
        model: model.clone(),
        system_prompt: Some(system_prompt),
        history: Vec::new(),
        max_tokens: None,
    };

    match state.ai.get().send_prompt(req).await {
        Ok(completion) => {
            let mut reply_text = completion.content.trim().to_string();
            if !citation_footer.is_empty() && !is_refusal_response(&reply_text) {
                reply_text.push_str(&citation_footer);
            }

            let interactive = cfg.forum_delivery_mode == "interactive";
            deliver_discord_response(
                ctx,
                thread.id,
                Some(starter.id),
                starter.author.id.get(),
                &reply_text,
                interactive,
                cfg.pagination_max_chars,
                state,
            )
            .await;

            state.memory.add_turn(
                ContextKey::guild_channel(guild_id, thread_id),
                Some(starter.id.get()),
                starter.author.id.get(),
                &combined_prompt,
                &reply_text,
                cfg.memory_turns,
            );

            let guild_name = state
                .connected_guilds
                .get(&guild_id)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| guild_id.to_string());

            state.telemetry.emit(TelemetryEvent::ForumPostReplied {
                guild_name,
                post_title: title.to_string(),
                author: starter.author.name.clone(),
            });

            state
                .forum_replied_threads
                .insert(thread_id, ForumThreadState::Completed(Instant::now()));
        }
        Err(e) => {
            tracing::warn!("Forum auto-reply failed in thread {thread_id}: {e}");
            state.forum_replied_threads.remove(&thread_id);
        }
    }
}

/// Handle `FullEvent::Message` for direct mentions, custom prefixes, replies, DMs, and spontaneous replies.
async fn handle_incoming_message(ctx: &Context, msg: &Message, state: &Arc<BotState>) {
    // 1. Strictly ignore bots, webhooks, and non-regular system messages
    if msg.author.bot || msg.webhook_id.is_some() {
        return;
    }
    if msg.kind != MessageType::Regular && msg.kind != MessageType::InlineReply {
        return;
    }

    let raw_content = msg.content.trim();
    if raw_content.is_empty() {
        return;
    }

    // Ignore prefix commands (`!cmd`) which Poise handles directly
    if let Some(after_bang) = raw_content.strip_prefix('!') {
        if after_bang
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
        {
            return;
        }
    }

    let channel_id = msg.channel_id.get();

    // Skip if this message is inside a forum thread currently being handled by `handle_forum_thread_create`
    if let Some(entry) = state.forum_replied_threads.get(&channel_id) {
        if matches!(*entry.value(), ForumThreadState::InFlight(_)) {
            return;
        }
    }

    let bot_uid = state
        .bot_identity
        .read()
        .id
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let is_dm = msg.guild_id.is_none();
    let guild_id = msg.guild_id.map(|g| g.get());
    let guild_cfg = guild_id.map(|gid| state.storage.get_guild_config(gid));

    // Determine whether the message explicitly or spontaneously triggers Silas Blue
    let (triggered, cleaned_prompt) = evaluate_message_trigger(
        ctx,
        raw_content,
        msg,
        bot_uid,
        is_dm,
        guild_cfg.as_ref(),
        state,
    );

    if !triggered {
        return;
    }

    // If the user mentioned the bot followed by a command name (e.g. `@Silas Blue help`), let Poise handle it
    if is_known_command_invocation(&cleaned_prompt) {
        return;
    }

    let settings = state.storage.get_app_settings();
    let user_id = msg.author.id.get();

    // 2. Channel Whitelist, Permission & Quota check
    if let (Some(gid), Some(cfg)) = (guild_id, guild_cfg.as_ref()) {
        if !is_channel_allowed(ctx, msg, &cfg.compiled.allowed_channels) {
            return;
        }

        let member_info = extract_member_info(ctx, msg, &settings.bot_owner_id);
        let member_ctx = MemberContext {
            user_id,
            is_guild_owner: member_info.is_owner_or_bot_owner,
            is_administrator: member_info.is_admin,
            role_ids: &member_info.role_ids,
            role_names: &member_info.role_names,
        };

        if !has_guild_permission(&member_ctx, cfg, PermissionDomain::ChatReply) {
            return;
        }

        // 3. Quota reservation check
        let mut reservation_guard: Option<QuotaReservationGuard> = None;
        let mut effective_max_tokens: Option<u32> = None;

        if cfg.token_quotas.enabled {
            let tq = &cfg.token_quotas;
            let (win_start, win_end, _) = calculate_window_period(
                &tq.reset_frequency,
                &tq.reset_time,
                tq.reset_timezone_offset,
                tq.reset_day_of_week,
                tq.reset_day_of_month,
                None,
            );
            let committed = state
                .storage
                .get_user_token_usage_async(gid, user_id, win_start, win_end)
                .await
                .unwrap_or(0);
            let in_flight = state.quota_reservations.current_reserved(gid, user_id);
            let total_used = committed.saturating_add(in_flight);

            let quota_res = evaluate_quota(
                &member_ctx,
                cfg,
                &member_info.guild_role_map,
                total_used,
                None,
            );

            if !quota_res.allowed {
                let _ = msg
                    .reply(
                        &ctx.http,
                        format!(
                            "⚠️ You have reached your token quota (**{} / {}** tokens used for role `{}`). Resets <t:{}:R>.",
                            quota_res.used, quota_res.limit, quota_res.role_name, quota_res.reset_epoch
                        ),
                    )
                    .await;
                return;
            }

            effective_max_tokens = quota_res.effective_max_tokens;
            reservation_guard = Some(state.quota_reservations.reserve(
                gid,
                user_id,
                quota_res.remaining,
            ));
        }

        let can_search = has_guild_permission(&member_ctx, cfg, PermissionDomain::WebSearch);

        process_and_reply(
            ctx,
            msg,
            &cleaned_prompt,
            Some(cfg),
            &settings,
            can_search,
            effective_max_tokens,
            reservation_guard,
            state,
        )
        .await;
    } else {
        // DM permission check
        let is_bot_owner = settings.bot_owner_id.trim() == user_id.to_string();
        if !is_bot_owner && !has_dm_permission(settings.allow_dms, PermissionDomain::ChatReply) {
            let _ = msg.reply(&ctx.http, &settings.dm_disabled_message).await;
            return;
        }
        let dm_cfg = Arc::new(state.storage.get_dm_config());
        process_and_reply(
            ctx,
            msg,
            &cleaned_prompt,
            Some(&dm_cfg),
            &settings,
            true,
            None,
            None,
            state,
        )
        .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_and_reply(
    ctx: &Context,
    msg: &Message,
    prompt_text: &str,
    guild_cfg: Option<&Arc<GuildConfig>>,
    settings: &Arc<silas_core::config::AppSettings>,
    can_search: bool,
    effective_max_tokens: Option<u32>,
    reservation: Option<QuotaReservationGuard>,
    state: &Arc<BotState>,
) {
    let channel_id = msg.channel_id.get();
    let guild_id = msg.guild_id.map(|g| g.get());
    let user_id = msg.author.id.get();

    let default_cfg = GuildConfig::default();
    let cfg_ref: &GuildConfig = guild_cfg.map(|c| c.as_ref()).unwrap_or(&default_cfg);

    let guild_name = guild_id
        .and_then(|gid| state.connected_guilds.get(&gid).map(|g| g.name.clone()))
        .unwrap_or_else(|| "Direct Message".to_string());

    state.telemetry.emit(TelemetryEvent::Prompt {
        guild_name: guild_name.clone(),
        user: msg.author.name.clone(),
        prompt_preview: prompt_text.to_string(),
    });

    let model = state
        .resolve_model_for_context(settings, guild_cfg.map(|c| c.as_ref()))
        .await;

    let ctx_key = match guild_id {
        Some(gid) => ContextKey::guild_channel(gid, channel_id),
        None => ContextKey::dm(user_id),
    };

    let history = state.memory.get_history(
        ctx_key,
        cfg_ref.memory_turns,
        Some(cfg_ref.memory_ttl_minutes),
    );

    let replied_context = msg
        .referenced_message
        .as_ref()
        .map(|r| r.content.as_str())
        .unwrap_or("");

    // Evaluate autonomous RAG web search if enabled
    let mut rag_block: Option<String> = None;
    let mut citation_footer = String::new();

    if can_search
        && settings.web_search_enabled
        && settings.web_search_auto
        && cfg_ref.web_search_enabled
        && cfg_ref.web_search_auto
        && should_auto_search(prompt_text)
    {
        let search_q = build_search_query(prompt_text, replied_context, &history);
        let results = state.search.search(&search_q, None, settings).await;
        state.telemetry.emit(TelemetryEvent::WebSearch {
            guild_name: guild_name.clone(),
            query_preview: search_q,
            results_count: results.len(),
        });
        if !results.is_empty() {
            let (blk, foot) = format_grounded_rag_block(&results);
            rag_block = blk;
            citation_footer = foot;
        }
    }

    let custom_persona = if !cfg_ref.system_prompt.trim().is_empty() {
        Some(cfg_ref.system_prompt.as_str())
    } else {
        None
    };

    let system_prompt = compose_system_prompt(
        &settings.system_prompt,
        custom_persona,
        rag_block.as_deref(),
    );

    let _typing = msg.channel_id.start_typing(&ctx.http);

    let req = CompletionRequest {
        prompt: format!("[{}]: {}", msg.author.name, prompt_text),
        model: model.clone(),
        system_prompt: Some(system_prompt),
        history,
        max_tokens: effective_max_tokens,
    };

    match state.ai.get().send_prompt(req).await {
        Ok(completion) => {
            if let Some(gid) = guild_id {
                let _ = state
                    .storage
                    .record_token_usage_async(TokenUsageEntry {
                        guild_id: gid,
                        user_id,
                        total_tokens: completion.usage.total_tokens,
                        prompt_tokens: completion.usage.prompt_tokens,
                        completion_tokens: completion.usage.completion_tokens,
                        model: model.clone(),
                        recorded_at: Utc::now().timestamp(),
                    })
                    .await;
            }
            drop(reservation);

            let mut reply_text = completion.content.trim().to_string();
            if reply_text.is_empty() {
                reply_text = "_(Empty response from model)_".to_string();
            }

            if !citation_footer.is_empty() && !is_refusal_response(&reply_text) {
                reply_text.push_str(&citation_footer);
            }

            deliver_discord_response(
                ctx,
                msg.channel_id,
                Some(msg.id),
                user_id,
                &reply_text,
                cfg_ref.pagination_enabled,
                cfg_ref.pagination_max_chars,
                state,
            )
            .await;

            state.memory.add_turn(
                ctx_key,
                Some(msg.id.get()),
                user_id,
                prompt_text,
                &reply_text,
                cfg_ref.memory_turns,
            );

            state.telemetry.emit(TelemetryEvent::Reply {
                guild_name,
                reply_preview: reply_text,
            });
        }
        Err(e) => {
            drop(reservation);
            let err_str = e.to_string();
            let err_summary = truncate_chars(&err_str, 300);
            let _ = msg
                .reply(
                    &ctx.http,
                    format!("⚠️ Inference backend error: `{err_summary}`"),
                )
                .await;
        }
    }
}

/// Deliver a response to a Discord channel using either interactive button pagination or multi-message chunks
/// with 350ms anti-burst pacing and AutoMod (`200000`) error handling.
#[allow(clippy::too_many_arguments)]
pub async fn deliver_discord_response(
    ctx: &Context,
    channel_id: ChannelId,
    reply_to: Option<MessageId>,
    owner_user_id: u64,
    content: &str,
    pagination_enabled: bool,
    pagination_max_chars: usize,
    state: &Arc<BotState>,
) -> Vec<u64> {
    let max_chars = pagination_max_chars.clamp(500, 1900);
    let pages = paginate_markdown(content, max_chars);
    let mut sent_ids = Vec::new();

    if pagination_enabled && pages.len() > 1 {
        if let Some((first_page, components)) =
            state.pagination.create_view(owner_user_id, pages.clone())
        {
            let mut builder = CreateMessage::new()
                .content(first_page)
                .components(components);
            if let Some(ref_id) = reply_to {
                builder = builder.reference_message((channel_id, ref_id));
            }
            if let Some(id) = send_message_with_automod_guard(ctx, channel_id, builder).await {
                sent_ids.push(id);
            }
            return sent_ids;
        }
    }

    for (idx, chunk) in pages.iter().enumerate() {
        if idx > 0 {
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
        let mut builder = CreateMessage::new().content(chunk.clone());
        if idx == 0 {
            if let Some(ref_id) = reply_to {
                builder = builder.reference_message((channel_id, ref_id));
            }
        }
        if let Some(id) = send_message_with_automod_guard(ctx, channel_id, builder).await {
            sent_ids.push(id);
        }
    }

    sent_ids
}

async fn send_message_with_automod_guard(
    ctx: &Context,
    channel_id: ChannelId,
    builder: CreateMessage,
) -> Option<u64> {
    match channel_id.send_message(&ctx.http, builder).await {
        Ok(sent) => Some(sent.id.get()),
        Err(serenity::Error::Http(http_err)) => {
            let err_str = http_err.to_string();
            if err_str.contains("200000") || err_str.to_lowercase().contains("automod") {
                let fallback = CreateMessage::new().content(
                    "⚠️ This response was blocked by the server's Discord AutoMod rules.",
                );
                let _ = channel_id.send_message(&ctx.http, fallback).await;
            } else {
                tracing::warn!("Discord HTTP error sending message to {channel_id}: {err_str}");
            }
            None
        }
        Err(e) => {
            tracing::warn!("Error sending message to {channel_id}: {e}");
            None
        }
    }
}

/// Determines whether a Discord channel (or thread whose parent channel is configured)
/// is allowed to receive bot LLM replies under guild configuration.
/// If `allowed_channels` is empty, all channels in the guild are allowed.
fn is_channel_allowed(
    ctx: &Context,
    msg: &Message,
    allowed_channels: &HashSet<u64>,
) -> bool {
    if allowed_channels.is_empty() {
        return true;
    }
    let ch_id = msg.channel_id.get();
    if allowed_channels.contains(&ch_id) {
        return true;
    }
    if let Some(gid) = msg.guild_id {
        if let Some(guild) = ctx.cache.guild(gid) {
            if let Some(ch) = guild.channels.get(&msg.channel_id) {
                if let Some(parent) = ch.parent_id {
                    if allowed_channels.contains(&parent.get()) {
                        return true;
                    }
                }
            }
            if let Some(thread) = guild.threads.iter().find(|t| t.id == msg.channel_id) {
                if let Some(parent) = thread.parent_id {
                    if allowed_channels.contains(&parent.get()) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Evaluate if a message triggers the bot via DM, `@mention`, reply to bot, custom prefix, or spontaneous reply mode.
fn evaluate_message_trigger(
    ctx: &Context,
    raw_content: &str,
    msg: &Message,
    bot_uid: u64,
    is_dm: bool,
    guild_cfg: Option<&Arc<GuildConfig>>,
    state: &Arc<BotState>,
) -> (bool, String) {
    if is_dm {
        return (true, strip_bot_mentions(raw_content, bot_uid));
    }

    // Check explicit @Bot mention
    let mention_plain = format!("<@{bot_uid}>");
    let mention_nick = format!("<@!{bot_uid}>");
    if raw_content.contains(&mention_plain) || raw_content.contains(&mention_nick) {
        let cleaned = strip_bot_mentions(raw_content, bot_uid);
        return (
            true,
            if cleaned.is_empty() {
                "Hello!".to_string()
            } else {
                cleaned
            },
        );
    }

    // Check reply to a message authored by Silas Blue
    if let Some(ref referenced) = msg.referenced_message {
        if referenced.author.id.get() == bot_uid {
            return (true, raw_content.to_string());
        }
    }

    // Check case-insensitive prefix (`silas `, `sb!`)
    let lower = raw_content.to_lowercase();
    for prefix in ["silas ", "silas, ", "sb!"] {
        if lower.starts_with(prefix) {
            let cleaned =
                silas_core::text::slice_chars(raw_content, prefix.chars().count(), usize::MAX)
                    .trim()
                    .to_string();
            if !cleaned.is_empty() {
                return (true, cleaned);
            }
        }
    }

    // Check spontaneous reply mode (`random_reply_mode`)
    if let Some(cfg) = guild_cfg {
        if !is_channel_allowed(ctx, msg, &cfg.compiled.allowed_channels) {
            return (false, String::new());
        }
        let mode = cfg.random_reply_mode.trim().to_lowercase();
        if mode != "off" && !mode.is_empty() && cfg.random_reply_value > 0 {
            let channel_id = msg.channel_id.get();
            let now = Instant::now();
            let mut ch_state = state.channel_random_state.entry(channel_id).or_default();

            ch_state.messages_since_reply = ch_state.messages_since_reply.saturating_add(1);
            let elapsed_secs = now.duration_since(ch_state.last_reply_at).as_secs();

            // Enforce minimum 60-second anti-spam cooldown per channel
            if elapsed_secs >= MIN_SPONTANEOUS_COOLDOWN_SECS {
                let should_fire = match mode.as_str() {
                    "messages" => ch_state.messages_since_reply >= cfg.random_reply_value,
                    "minutes" => {
                        elapsed_secs >= u64::from(cfg.random_reply_value).saturating_mul(60)
                    }
                    "probability" => {
                        let prob = cfg.random_reply_value.min(100);
                        let roll = rand::thread_rng().gen_range(1..=100);
                        roll <= prob
                    }
                    _ => false,
                };

                if should_fire {
                    ch_state.messages_since_reply = 0;
                    ch_state.last_reply_at = now;
                    return (true, raw_content.to_string());
                }
            }
        }
    }

    (false, String::new())
}

fn strip_bot_mentions(text: &str, bot_uid: u64) -> String {
    let m1 = format!("<@{bot_uid}>");
    let m2 = format!("<@!{bot_uid}>");
    text.replace(&m1, "").replace(&m2, "").trim().to_string()
}

fn is_known_command_invocation(cleaned_prompt: &str) -> bool {
    let first_word = cleaned_prompt
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches('!')
        .to_lowercase();

    matches!(
        first_word.as_str(),
        "models"
            | "setmodel"
            | "clear"
            | "reset"
            | "forget"
            | "forget_me"
            | "privacy"
            | "recall"
            | "setmemory"
            | "setttl"
            | "search"
            | "web"
            | "ddg"
            | "links"
            | "rawsearch"
            | "autosearch"
            | "setwebsearch"
            | "setprompt"
            | "setpagination"
            | "setnick"
            | "setname"
            | "setforum"
            | "quota"
            | "tokens"
            | "myquota"
            | "config"
            | "ping"
            | "invite"
            | "help"
            | "setglobalname"
            | "update"
    )
}

pub struct ExtractedMemberInfo {
    pub role_ids: Vec<u64>,
    pub role_names: Vec<String>,
    pub guild_role_map: Vec<(u64, String)>,
    pub is_admin: bool,
    pub is_owner_or_bot_owner: bool,
}

pub fn extract_member_info(
    ctx: &Context,
    msg: &Message,
    bot_owner_id: &str,
) -> ExtractedMemberInfo {
    let mut role_ids = Vec::new();
    let mut role_names = Vec::new();
    let mut guild_role_map = Vec::new();
    let mut is_admin = false;
    let mut is_owner_or_bot_owner =
        !bot_owner_id.trim().is_empty() && bot_owner_id.trim() == msg.author.id.get().to_string();

    if let Some(ref member) = msg.member {
        for r in &member.roles {
            role_ids.push(r.get());
        }
        if let Some(perms) = member.permissions {
            if perms.administrator() || perms.manage_guild() {
                is_admin = true;
            }
        }
    }

    if let Some(gid) = msg.guild_id {
        if let Some(guild) = ctx.cache.guild(gid) {
            if guild.owner_id == msg.author.id {
                is_owner_or_bot_owner = true;
            }
            for (rid, role) in &guild.roles {
                guild_role_map.push((rid.get(), role.name.clone()));
            }
            for rid in &role_ids {
                if let Some(role) = guild.roles.get(&serenity::all::RoleId::new(*rid)) {
                    role_names.push(role.name.clone());
                    if role.permissions.administrator() || role.permissions.manage_guild() {
                        is_admin = true;
                    }
                }
            }
        }
    }

    ExtractedMemberInfo {
        role_ids,
        role_names,
        guild_role_map,
        is_admin,
        is_owner_or_bot_owner,
    }
}
