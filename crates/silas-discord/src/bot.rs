//! Discord gateway lifecycle controller (`BotController`) and shared state (`BotState`).

use crate::commands;
use crate::handlers;
use crate::pagination_ui::PaginationStore;
use dashmap::DashMap;
use parking_lot::RwLock;
use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};
use serenity::all::{
    ChannelType, ClientBuilder, CreateAllowedMentions, EditProfile, GatewayIntents, GuildId, Http,
    ShardManager,
};
use silas_ai::ProviderManager;
use silas_core::config::{AppSettings, GuildConfig};
use silas_core::events::{TelemetryBus, TelemetryEvent};
use silas_core::memory::ConversationMemory;
use silas_core::quota::QuotaReservationManager;
use silas_search::SearchCoordinator;
use silas_storage::Storage;
use silas_updater::BotUpdater;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::task::JoinHandle;

/// State of a forum thread auto-reply to prevent duplicate replies during `ThreadCreate` vs `MessageCreate`.
#[derive(Debug, Clone, Copy)]
pub enum ForumThreadState {
    InFlight(Instant),
    Completed(Instant),
}

/// Per-channel state for spontaneous (`random_reply_mode`) replies.
#[derive(Debug, Clone)]
pub struct ChannelRandomState {
    pub messages_since_reply: u32,
    pub last_reply_at: Instant,
}

impl Default for ChannelRandomState {
    fn default() -> Self {
        Self {
            messages_since_reply: 0,
            last_reply_at: Instant::now()
                .checked_sub(std::time::Duration::from_secs(3600))
                .unwrap_or_else(Instant::now),
        }
    }
}

/// Summary of a connected Discord guild for the Web Console.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedGuildInfo {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub member_count: u64,
    pub bot_nickname: Option<String>,
    pub channels: Vec<GuildChannelInfo>,
    pub roles: Vec<GuildRoleInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuildChannelInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuildRoleInfo {
    pub id: String,
    pub name: String,
    pub position: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BotUserIdentity {
    pub id: Option<String>,
    pub username: Option<String>,
    pub global_name: Option<String>,
    pub avatar_url: Option<String>,
}

/// Shared state accessible by all Poise commands and Serenity event handlers.
pub struct BotState {
    pub storage: Storage,
    pub ai: Arc<ProviderManager>,
    pub search: Arc<SearchCoordinator>,
    pub memory: Arc<ConversationMemory>,
    pub quota_reservations: Arc<QuotaReservationManager>,
    pub updater: Arc<BotUpdater>,
    pub telemetry: TelemetryBus,
    pub pagination: Arc<PaginationStore>,
    pub forum_replied_threads: DashMap<u64, ForumThreadState>,
    pub channel_random_state: DashMap<u64, ChannelRandomState>,
    pub connected_guilds: Arc<DashMap<u64, ConnectedGuildInfo>>,
    pub bot_identity: Arc<RwLock<BotUserIdentity>>,
    pub gateway_ready: Arc<AtomicBool>,
    pub started_at: Instant,
}

impl BotState {
    /// Deterministic model resolution across Remote vs Local backends and Guild vs DM contexts.
    pub async fn resolve_model_for_context(
        &self,
        settings: &AppSettings,
        guild_cfg: Option<&GuildConfig>,
    ) -> String {
        let override_model = match guild_cfg {
            Some(cfg) => cfg.default_model.as_str(),
            None => settings.dm_default_model.as_str(),
        };
        self.ai
            .resolve_model_for_turn(override_model, settings)
            .await
    }
}

struct RunningBot {
    shard_manager: Arc<ShardManager>,
    http: Arc<Http>,
    join_handle: JoinHandle<()>,
    started_at: Instant,
}

/// Thread-safe controller managing the Serenity/Poise gateway client lifecycle.
#[derive(Clone)]
pub struct BotController {
    inner: Arc<tokio::sync::Mutex<Option<RunningBot>>>,
    storage: Storage,
    ai: Arc<ProviderManager>,
    search: Arc<SearchCoordinator>,
    memory: Arc<ConversationMemory>,
    quota_reservations: Arc<QuotaReservationManager>,
    updater: Arc<BotUpdater>,
    telemetry: TelemetryBus,
    connected_guilds: Arc<DashMap<u64, ConnectedGuildInfo>>,
    bot_identity: Arc<RwLock<BotUserIdentity>>,
    gateway_ready: Arc<AtomicBool>,
}

impl BotController {
    pub fn new(
        storage: Storage,
        ai: Arc<ProviderManager>,
        search: Arc<SearchCoordinator>,
        memory: Arc<ConversationMemory>,
        quota_reservations: Arc<QuotaReservationManager>,
        updater: Arc<BotUpdater>,
        telemetry: TelemetryBus,
    ) -> Self {
        Self {
            inner: Arc::new(tokio::sync::Mutex::new(None)),
            storage,
            ai,
            search,
            memory,
            quota_reservations,
            updater,
            telemetry,
            connected_guilds: Arc::new(DashMap::new()),
            bot_identity: Arc::new(RwLock::new(BotUserIdentity::default())),
            gateway_ready: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn memory(&self) -> &Arc<ConversationMemory> {
        &self.memory
    }

    pub fn ai(&self) -> &Arc<ProviderManager> {
        &self.ai
    }

    pub fn search(&self) -> &Arc<SearchCoordinator> {
        &self.search
    }

    pub fn updater(&self) -> &Arc<BotUpdater> {
        &self.updater
    }

    pub fn is_running(&self) -> bool {
        self.gateway_ready.load(Ordering::Acquire)
    }

    pub fn bot_identity(&self) -> BotUserIdentity {
        self.bot_identity.read().clone()
    }

    pub fn connected_guilds_list(&self) -> Vec<ConnectedGuildInfo> {
        let mut list: Vec<ConnectedGuildInfo> = self
            .connected_guilds
            .iter()
            .map(|e| e.value().clone())
            .collect();
        list.sort_by_key(|a| a.name.to_lowercase());
        list
    }

    pub fn guild_metadata(&self, guild_id: u64) -> Option<ConnectedGuildInfo> {
        self.connected_guilds
            .get(&guild_id)
            .map(|e| e.value().clone())
    }

    /// Generate the least-privilege Discord OAuth2 bot invite link.
    /// Permissions integer `274878221312` grants:
    /// View Channels (1024) | Send Messages (2048) | Embed Links (16384) | Attach Files (32768) |
    /// Read Message History (65536) | Add Reactions (64) | Send Messages in Threads (274877906944).
    pub fn get_invite_url(&self) -> Option<String> {
        let settings = self.storage.get_app_settings();
        let client_id = if !settings.discord_oauth_client_id.trim().is_empty() {
            Some(settings.discord_oauth_client_id.trim().to_string())
        } else {
            self.bot_identity.read().id.clone()
        }?;
        if client_id.is_empty() {
            return None;
        }
        Some(format!(
            "https://discord.com/oauth2/authorize?client_id={client_id}&permissions=274878221312&scope=bot%20applications.commands"
        ))
    }

    /// Start the Discord Gateway client if not already running.
    pub async fn start(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        if let Some(existing) = guard.as_ref() {
            if !existing.join_handle.is_finished() {
                return Ok(());
            }
        }
        *guard = None;

        let settings = self.storage.get_app_settings();
        let token = settings.discord_token.trim().to_string();
        if token.is_empty() {
            return Err("Discord bot token is not configured.".to_string());
        }

        let state = Arc::new(BotState {
            storage: self.storage.clone(),
            ai: Arc::clone(&self.ai),
            search: Arc::clone(&self.search),
            memory: Arc::clone(&self.memory),
            quota_reservations: Arc::clone(&self.quota_reservations),
            updater: Arc::clone(&self.updater),
            telemetry: self.telemetry.clone(),
            pagination: Arc::new(PaginationStore::new()),
            forum_replied_threads: DashMap::new(),
            channel_random_state: DashMap::new(),
            connected_guilds: Arc::clone(&self.connected_guilds),
            bot_identity: Arc::clone(&self.bot_identity),
            gateway_ready: Arc::clone(&self.gateway_ready),
            started_at: Instant::now(),
        });

        let state_for_setup = Arc::clone(&state);

        // Least-privilege GatewayIntents: never request GUILD_PRESENCES or GUILD_MEMBERS.
        let intents = GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;

        let allowed_mentions = CreateAllowedMentions::new()
            .everyone(false)
            .all_roles(false)
            .all_users(false)
            .replied_user(true);

        let framework = poise::Framework::builder()
            .options(poise::FrameworkOptions {
                commands: commands::all_commands(),
                prefix_options: poise::PrefixFrameworkOptions {
                    prefix: Some("!".into()),
                    mention_as_prefix: true,
                    case_insensitive_commands: true,
                    ..Default::default()
                },
                allowed_mentions: Some(allowed_mentions),
                event_handler: |ctx, event, _framework, data| {
                    Box::pin(async move {
                        handlers::handle_gateway_event(ctx, event, data).await;
                        Ok(())
                    })
                },
                ..Default::default()
            })
            .setup(move |ctx, ready, framework| {
                let st = Arc::clone(&state_for_setup);
                Box::pin(async move {
                    {
                        let mut id_guard = st.bot_identity.write();
                        id_guard.id = Some(ready.user.id.get().to_string());
                        id_guard.username = Some(ready.user.name.clone());
                        id_guard.global_name = ready.user.global_name.clone();
                        id_guard.avatar_url = ready.user.avatar_url();
                    }
                    st.gateway_ready.store(true, Ordering::Release);

                    if let Err(e) =
                        poise::builtins::register_globally(ctx, &framework.options().commands).await
                    {
                        tracing::warn!("Failed to register global slash commands: {e}");
                    }

                    st.telemetry.emit(TelemetryEvent::System {
                        message: format!(
                            "Connected to Discord Gateway as {} ({}) across {} guilds.",
                            ready.user.name,
                            ready.user.id,
                            ready.guilds.len()
                        ),
                    });

                    Ok(st)
                })
            })
            .build();

        let mut cache_settings = serenity::cache::Settings::default();
        cache_settings.max_messages = 0;
        cache_settings.cache_users = false;
        cache_settings.cache_guilds = true;
        cache_settings.cache_channels = true;

        let mut client = ClientBuilder::new(token, intents)
            .framework(framework)
            .cache_settings(cache_settings)
            .await
            .map_err(|e| format!("Failed to build Discord client: {e}"))?;

        let shard_manager = Arc::clone(&client.shard_manager);
        let http = Arc::clone(&client.http);
        let gateway_ready = Arc::clone(&self.gateway_ready);
        let telemetry = self.telemetry.clone();

        let join_handle = tokio::spawn(async move {
            if let Err(e) = client.start().await {
                telemetry.emit(TelemetryEvent::System {
                    message: format!("Discord Gateway client exited with error: {e}"),
                });
            }
            gateway_ready.store(false, Ordering::Release);
        });

        *guard = Some(RunningBot {
            shard_manager,
            http,
            join_handle,
            started_at: Instant::now(),
        });

        Ok(())
    }

    /// Gracefully shut down all Gateway shards and wait for the client task to exit.
    pub async fn stop(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        if let Some(running) = guard.take() {
            running.shard_manager.shutdown_all().await;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), running.join_handle)
                .await;
            self.gateway_ready.store(false, Ordering::Release);
            self.connected_guilds.clear();
            self.telemetry.emit(TelemetryEvent::System {
                message: "Discord Gateway client stopped.".to_string(),
            });
        }
        Ok(())
    }

    /// Stop and restart the Discord Gateway client atomically.
    pub async fn restart(&self) -> Result<(), String> {
        self.stop().await?;
        self.start().await
    }

    /// Update the bot's nickname in a specific guild.
    pub async fn update_guild_nickname(
        &self,
        guild_id: u64,
        nickname: Option<&str>,
    ) -> Result<(), String> {
        let http = {
            let guard = self.inner.lock().await;
            guard.as_ref().map(|r| Arc::clone(&r.http))
        }
        .ok_or_else(|| "Bot is not currently connected to Discord.".to_string())?;

        let clean_nick = nickname.map(str::trim).filter(|s| !s.is_empty());
        GuildId::new(guild_id)
            .edit_nickname(&http, clean_nick)
            .await
            .map_err(|e| format!("Failed to update guild nickname: {e}"))?;

        if let Some(mut entry) = self.connected_guilds.get_mut(&guild_id) {
            entry.bot_nickname = clean_nick.map(ToString::to_string);
        }

        Ok(())
    }

    /// Update the bot's global username via Discord REST API.
    pub async fn update_global_username(&self, new_username: &str) -> Result<String, String> {
        let http = {
            let guard = self.inner.lock().await;
            guard.as_ref().map(|r| Arc::clone(&r.http))
        }
        .ok_or_else(|| "Bot is not currently connected to Discord.".to_string())?;

        let trimmed = new_username.trim();
        if trimmed.len() < 2 || trimmed.len() > 32 {
            return Err("Username must be between 2 and 32 characters.".to_string());
        }

        let builder = EditProfile::new().username(trimmed);
        let updated = http
            .edit_profile(&builder)
            .await
            .map_err(|e| format!("Discord API rejected username change: {e}"))?;

        {
            let mut id_guard = self.bot_identity.write();
            id_guard.username = Some(updated.name.clone());
        }

        Ok(updated.name.clone())
    }

    /// Return uptime seconds if running.
    pub async fn uptime_seconds(&self) -> u64 {
        let guard = self.inner.lock().await;
        guard
            .as_ref()
            .map(|r| r.started_at.elapsed().as_secs())
            .unwrap_or(0)
    }
}

/// Convert a Serenity `Guild` from cache/create event into `ConnectedGuildInfo`.
pub fn extract_connected_guild_info(
    guild: &serenity::all::Guild,
    bot_user_id: Option<u64>,
) -> ConnectedGuildInfo {
    let mut channels: Vec<GuildChannelInfo> = guild
        .channels
        .values()
        .filter_map(|ch| {
            let kind = match ch.kind {
                ChannelType::Text => "text",
                ChannelType::Forum => "forum",
                ChannelType::News => "announcement",
                _ => return None,
            };
            Some(GuildChannelInfo {
                id: ch.id.get().to_string(),
                name: ch.name.clone(),
                kind: kind.to_string(),
            })
        })
        .collect();
    channels.sort_by_key(|a| a.name.to_lowercase());

    let mut roles: Vec<GuildRoleInfo> = guild
        .roles
        .values()
        .filter(|r| r.id.get() != guild.id.get())
        .map(|r| GuildRoleInfo {
            id: r.id.get().to_string(),
            name: r.name.clone(),
            position: r.position,
        })
        .collect();
    roles.sort_by_key(|a| std::cmp::Reverse(a.position));

    let bot_nickname = bot_user_id
        .and_then(|uid| guild.members.get(&serenity::all::UserId::new(uid)))
        .and_then(|m| m.nick.clone());

    ConnectedGuildInfo {
        id: guild.id.get().to_string(),
        name: guild.name.clone(),
        icon_url: guild.icon_url(),
        member_count: guild.member_count,
        bot_nickname,
        channels,
        roles,
    }
}
