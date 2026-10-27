//! Participation tracking, multi-modal point accruals (text, voice, reactions),
//! anti-abuse cooldowns, and automated role rewards reconciler.

use std::time::{Duration, Instant};

use dashmap::DashMap;
use rand::Rng;
use serenity::all::{ChannelId, GuildId, Http, Message, Reaction, RoleId, UserId};
use silas_storage::Storage;
use tracing::{debug, info, warn};

use crate::bot::BotState;

/// In-memory tracker for anti-spam rate limiting and voice channel presence.
#[derive(Default)]
pub struct ActivityTracker {
    /// `(guild_id, user_id) -> last_text_point_awarded_instant`
    pub text_cooldowns: DashMap<(u64, u64), Instant>,
    /// `(guild_id, user_id) -> last_reaction_point_awarded_instant`
    pub reaction_cooldowns: DashMap<(u64, u64), Instant>,
    /// `(guild_id, user_id) -> (voice_channel_id, join_instant)`
    pub voice_states: DashMap<(u64, u64), (u64, Instant)>,
}

impl ActivityTracker {
    pub fn new() -> Self {
        Self {
            text_cooldowns: DashMap::new(),
            reaction_cooldowns: DashMap::new(),
            voice_states: DashMap::new(),
        }
    }

    /// Evaluates incoming message for text participation points.
    pub async fn handle_text_activity(&self, msg: &Message, state: &BotState) {
        if msg.author.bot {
            return;
        }
        let Some(guild_id) = msg.guild_id else {
            return;
        };
        let gid = guild_id.get();
        let uid = msg.author.id.get();

        let cfg = match state.storage.get_activity_config(gid) {
            Ok(c) => c,
            Err(_) => return,
        };

        if !cfg.enabled {
            return;
        }

        // Check channel blacklist
        if cfg.ignored_channels.contains(&msg.channel_id.get()) {
            return;
        }

        // Check min characters
        if (msg.content.trim().chars().count() as i64) < cfg.text_min_chars {
            return;
        }

        // Check member roles for ignored roles and role multipliers
        let mut role_multiplier: f64 = 1.0;
        let mut is_role_ignored = false;

        if let Some(member) = &msg.member {
            for r in &member.roles {
                let rid = r.get();
                if cfg.ignored_roles.contains(&rid) {
                    is_role_ignored = true;
                    break;
                }
                if let Some(&mult) = cfg.role_multipliers.get(&rid.to_string()) {
                    if mult > role_multiplier {
                        role_multiplier = mult;
                    }
                }
            }
        }

        if is_role_ignored {
            return;
        }

        // Anti-spam cooldown check
        let now = Instant::now();
        if let Some(last_time) = self.text_cooldowns.get(&(gid, uid)) {
            if now.duration_since(*last_time) < Duration::from_secs(cfg.text_cooldown_seconds.max(1) as u64) {
                return;
            }
        }
        self.text_cooldowns.insert((gid, uid), now);

        // Channel multiplier
        let chan_multiplier = cfg
            .channel_multipliers
            .get(&msg.channel_id.get().to_string())
            .copied()
            .unwrap_or(1.0);

        // Calculate random points between min and max
        let min_pts = cfg.text_points_min.max(1);
        let max_pts = cfg.text_points_max.max(min_pts);
        let base_pts = if min_pts == max_pts {
            min_pts
        } else {
            rand::thread_rng().gen_range(min_pts..=max_pts)
        };

        let calculated_pts = (base_pts as f64 * role_multiplier * chan_multiplier).round() as i64;
        let final_pts = calculated_pts.max(1);

        if let Err(e) = state.storage.record_text_activity(gid, uid, final_pts) {
            warn!("Failed to record text activity for user {uid} in guild {gid}: {e}");
        }
    }

    /// Evaluates reaction additions for participation points.
    pub async fn handle_reaction_activity(&self, reaction: &Reaction, is_add: bool, state: &BotState) {
        if !is_add {
            return;
        }
        let Some(guild_id) = reaction.guild_id else {
            return;
        };
        let Some(user_id) = reaction.user_id else {
            return;
        };
        let gid = guild_id.get();
        let uid = user_id.get();

        let cfg = match state.storage.get_activity_config(gid) {
            Ok(c) => c,
            Err(_) => return,
        };

        if !cfg.enabled || !cfg.reaction_enabled {
            return;
        }

        if cfg.ignored_channels.contains(&reaction.channel_id.get()) {
            return;
        }

        let now = Instant::now();
        if let Some(last_time) = self.reaction_cooldowns.get(&(gid, uid)) {
            if now.duration_since(*last_time) < Duration::from_secs(cfg.reaction_cooldown_seconds.max(1) as u64) {
                return;
            }
        }
        self.reaction_cooldowns.insert((gid, uid), now);

        // Award reaction giver points
        if cfg.reaction_give_points > 0 {
            let _ = state.storage.record_reaction_activity(gid, uid, cfg.reaction_give_points);
        }

        // Award reaction receiver points if cached message is available and author is different
        if cfg.reaction_receive_points > 0 {
            if let Some(cached) = state.message_cache.get(reaction.message_id.get()) {
                if cached.author_id != uid && cached.author_id != 0 {
                    let _ = state
                        .storage
                        .record_reaction_activity(gid, cached.author_id, cfg.reaction_receive_points);
                }
            }
        }
    }

    /// Tracks voice channel entry, movement, and leave events.
    pub fn handle_voice_state_update(
        &self,
        new_state: &serenity::all::VoiceState,
    ) {
        let Some(guild_id) = new_state.guild_id else {
            return;
        };
        let gid = guild_id.get();
        let uid = new_state.user_id.get();

        let Some(channel_id) = new_state.channel_id else {
            // User left voice
            self.voice_states.remove(&(gid, uid));
            return;
        };

        // If self-deafened or server-deafened, skip tracking
        if new_state.deaf || new_state.self_deaf {
            self.voice_states.remove(&(gid, uid));
            return;
        }

        self.voice_states.insert((gid, uid), (channel_id.get(), Instant::now()));
    }

    /// Recurring background tick (called every 60s) to award voice activity points.
    pub fn tick_voice_activity(&self, storage: &Storage) {
        for entry in self.voice_states.iter() {
            let (gid, uid) = *entry.key();
            let (cid, _) = *entry.value();

            let cfg = match storage.get_activity_config(gid) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if !cfg.enabled || !cfg.voice_enabled {
                continue;
            }

            if cfg.ignored_channels.contains(&cid) {
                continue;
            }

            let chan_mult = cfg
                .channel_multipliers
                .get(&cid.to_string())
                .copied()
                .unwrap_or(1.0);

            let pts = (cfg.voice_points_per_interval as f64 * chan_mult).round() as i64;
            let final_pts = pts.max(1);

            let _ = storage.record_voice_activity(gid, uid, final_pts, 1);
        }
    }
}

/// Checks whether a command is authorized to run in the given channel according to guild restrictions.
pub fn check_channel_restriction(
    storage: &Storage,
    guild_id: u64,
    channel_id: u64,
    command_name: &str,
    is_admin: bool,
) -> Result<(), Vec<u64>> {
    if is_admin {
        return Ok(());
    }

    let restrictions = match storage.get_guild_command_restrictions(guild_id) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };

    if !restrictions.enabled {
        return Ok(());
    }

    // Check command-specific override rules
    if let Ok(rules) = storage.list_command_channel_rules(guild_id) {
        if let Some(rule) = rules.iter().find(|r| r.command_name.eq_ignore_ascii_case(command_name)) {
            if rule.denied_channels.contains(&channel_id) {
                return Err(if !rule.allowed_channels.is_empty() {
                    rule.allowed_channels.clone()
                } else {
                    restrictions.default_allowed_channels.clone()
                });
            }
            if !rule.allowed_channels.is_empty() {
                if rule.allowed_channels.contains(&channel_id) {
                    return Ok(());
                } else {
                    return Err(rule.allowed_channels.clone());
                }
            }
        }
    }

    // Fall back to default guild command channels
    if !restrictions.default_allowed_channels.is_empty() {
        if restrictions.default_allowed_channels.contains(&channel_id) {
            Ok(())
        } else {
            Err(restrictions.default_allowed_channels)
        }
    } else {
        Ok(())
    }
}

/// Evaluates automated activity reward roles for a guild and synchronizes assignments.
pub async fn reconcile_guild_rewards(
    http: &Http,
    storage: &Storage,
    guild_id: u64,
    cache: Option<&serenity::cache::Cache>,
) {
    let gid = GuildId::new(guild_id);
    let cfg = match storage.get_activity_config(guild_id) {
        Ok(c) => c,
        Err(_) => return,
    };

    if !cfg.enabled {
        return;
    }

    let eligible_rules = match storage.get_eligible_rewards(guild_id) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to query eligible rewards for guild {guild_id}: {e}");
            return;
        }
    };

    if eligible_rules.is_empty() {
        return;
    }

    // Determine bot's highest role position to preserve Discord role hierarchy
    let bot_highest_pos: u16 = if let Some(cache) = cache {
        if let Some(guild) = cache.guild(gid) {
            let bot_id = cache.current_user().id;
            guild
                .members
                .get(&bot_id)
                .map(|bm| {
                    bm.roles
                        .iter()
                        .filter_map(|rid| guild.roles.get(rid))
                        .map(|r| r.position)
                        .max()
                        .unwrap_or(0u16)
                })
                .unwrap_or(0u16)
        } else {
            u16::MAX
        }
    } else {
        u16::MAX
    };

    for (role_id, eligible_users, remove_on_drop) in eligible_rules {
        let target_role_id = RoleId::new(role_id);

        // Role hierarchy verification: skip if role position is >= bot's highest role
        if let Some(cache) = cache {
            if let Some(guild) = cache.guild(gid) {
                if let Some(role) = guild.roles.get(&target_role_id) {
                    if role.position >= bot_highest_pos {
                        debug!(
                            "Skipping activity role {role_id} in guild {guild_id}: position ({}) >= bot position ({})",
                            role.position, bot_highest_pos
                        );
                        continue;
                    }
                }
            }
        }

        // Grant role to eligible users
        for &uid in &eligible_users {
            let target_user_id = UserId::new(uid);
            let member_res = gid.member(http, target_user_id).await;
            if let Ok(member) = member_res {
                if !member.roles.contains(&target_role_id) {
                    match member.add_role(http, target_role_id).await {
                        Ok(()) => {
                            info!("Awarded activity reward role {role_id} to user {uid} in guild {guild_id}");
                            if cfg.announce_promotions {
                                if let Some(ann_ch) = cfg.announcement_channel_id {
                                    let msg = format!(
                                        "🎉 Congratulations <@{uid}>! You've earned the <@&{role_id}> role for your active participation!"
                                    );
                                    let _ = ChannelId::new(ann_ch).say(http, &msg).await;
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to add activity role {role_id} to user {uid}: {e}");
                        }
                    }
                }
            }
        }

        // If remove_on_drop is true, revoke role quietly from users who no longer qualify
        let users_to_demote: Vec<u64> = if remove_on_drop {
            if let Some(cache) = cache {
                if let Some(guild) = cache.guild(gid) {
                    guild
                        .members
                        .iter()
                        .filter(|(uid, m)| m.roles.contains(&target_role_id) && !eligible_users.contains(&uid.get()))
                        .map(|(uid, _)| uid.get())
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        for uid in users_to_demote {
            let target_user_id = UserId::new(uid);
            if let Ok(member) = gid.member(http, target_user_id).await {
                if let Err(e) = member.remove_role(http, target_role_id).await {
                    warn!("Failed to demote user {uid} from activity role {role_id}: {e}");
                } else {
                    debug!("Quietly removed activity role {role_id} from demoted user {uid}");
                }
            }
        }
    }
}
