//! High-performance automod evaluator for incoming `MessageCreate` events.
//!
//! Evaluates spam bursts, Discord invites, unauthorized links, blocked words,
//! excessive uppercase characters, and mass mentions.
//! Enforces weighted strikes, strike decay, and automated escalation ladder punishments.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use dashmap::DashMap;
use poise::serenity_prelude as serenity;
use regex::Regex;
use serenity::all::{Context, EditMember, Message, Permissions, Timestamp};
use silas_storage::{NewModCase, NewModStrike, PunishmentStep};
use tracing::debug;

use crate::bot::BotState;
use crate::logging::log_mod_action;

static INVITE_REGEX: OnceLock<Regex> = OnceLock::new();
static URL_REGEX: OnceLock<Regex> = OnceLock::new();

fn get_invite_regex() -> &'static Regex {
    INVITE_REGEX.get_or_init(|| {
        Regex::new(r"(?i)(?:discord\.gg|discord(?:app)?\.com/invite)/([a-zA-Z0-9\-]+)")
            .unwrap_or_else(|_| Regex::new("a^").unwrap_or_else(|_| unreachable!()))
    })
}

fn get_url_regex() -> &'static Regex {
    URL_REGEX.get_or_init(|| {
        Regex::new(r"(?i)https?://(?:www\.)?([a-zA-Z0-9\-\.]+)")
            .unwrap_or_else(|_| Regex::new("a^").unwrap_or_else(|_| unreachable!()))
    })
}

/// In-memory sliding window message tracker for burst anti-spam.
pub struct SpamRateTracker {
    history: DashMap<(u64, u64), Vec<Instant>>,
    last_pruned: AtomicU64,
}

impl Default for SpamRateTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SpamRateTracker {
    pub fn new() -> Self {
        Self {
            history: DashMap::new(),
            last_pruned: AtomicU64::new(0),
        }
    }

    /// Records a message and returns `true` if the burst rate limit was violated.
    pub fn record_and_check(
        &self,
        guild_id: u64,
        user_id: u64,
        max_messages: i64,
        interval_secs: i64,
    ) -> bool {
        let now = Instant::now();
        let key = (guild_id, user_id);
        let max_msgs = max_messages.max(1) as usize;
        let window = Duration::from_secs(interval_secs.max(1) as u64);

        let is_spam = {
            let mut entry = self.history.entry(key).or_default();
            let list = entry.value_mut();

            // Prune entries outside the sliding window
            list.retain(|t| now.duration_since(*t) <= window);
            list.push(now);

            list.len() > max_msgs
        };

        // Periodically prune stale keys to keep memory bounded (after lock is released)
        let prev_prune = self.last_pruned.load(Ordering::Relaxed);
        let now_sec = Utc::now().timestamp() as u64;
        if now_sec.saturating_sub(prev_prune) > 300
            && self
                .last_pruned
                .compare_exchange(prev_prune, now_sec, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
        {
            self.history.retain(|_, v| {
                v.retain(|t| now.duration_since(*t) <= Duration::from_secs(300));
                !v.is_empty()
            });
        }

        is_spam
    }
}

static GLOBAL_SPAM_TRACKER: OnceLock<SpamRateTracker> = OnceLock::new();

pub fn spam_tracker() -> &'static SpamRateTracker {
    GLOBAL_SPAM_TRACKER.get_or_init(SpamRateTracker::new)
}

/// Infraction classification result.
#[derive(Debug, Clone)]
pub struct AutomodInfraction {
    pub rule_type: &'static str,
    pub reason: String,
    pub strike_weight: i64,
}

/// High-efficiency evaluator invoked on every incoming message.
/// Returns `Some(infraction)` if the message violated an enabled rule, or `None` if clean.
pub async fn evaluate_message(
    _ctx: &Context,
    msg: &Message,
    state: &Arc<BotState>,
) -> Option<AutomodInfraction> {
    // 1. Ignore bots, webhooks, and private messages
    if msg.author.bot || msg.webhook_id.is_some() || msg.guild_id.is_none() {
        return None;
    }

    let guild_id_serenity = msg.guild_id?;
    let guild_id = guild_id_serenity.get();

    // Never moderate bot owner or server owner
    let settings = state.storage.get_app_settings();
    if !settings.bot_owner_id.trim().is_empty()
        && msg.author.id.to_string() == settings.bot_owner_id.trim()
    {
        return None;
    }
    if let Some(guild) = _ctx.cache.guild(guild_id_serenity) {
        if guild.owner_id == msg.author.id {
            return None;
        }
    }
    let config = state.storage.get_automod_config(guild_id).ok()?;

    if !config.enabled {
        return None;
    }

    // 2. Channel and role whitelist checks
    let channel_id = msg.channel_id.get();
    if config.ignored_channels.contains(&channel_id) {
        return None;
    }

    // Check if member has ignored roles or elevated administrative permissions
    if let Some(member) = &msg.member {
        for role_id in &member.roles {
            if config.ignored_roles.contains(&role_id.get()) {
                return None;
            }
        }
        if let Some(perms) = member.permissions {
            if perms.contains(Permissions::ADMINISTRATOR)
                || perms.contains(Permissions::MANAGE_GUILD)
                || perms.contains(Permissions::MODERATE_MEMBERS)
            {
                return None;
            }
        }
    }

    let author_id = msg.author.id.get();
    let content = msg.content.trim();

    // 3. Spam Burst Check
    if config.anti_spam_enabled
        && spam_tracker().record_and_check(
            guild_id,
            author_id,
            config.anti_spam_max_messages,
            config.anti_spam_interval_seconds,
        )
    {
        let weight = *config.strike_weights.get("spam").unwrap_or(&2);
        return Some(AutomodInfraction {
            rule_type: "spam",
            reason: format!(
                "Sending messages too quickly (burst limit: {} per {}s)",
                config.anti_spam_max_messages, config.anti_spam_interval_seconds
            ),
            strike_weight: weight,
        });
    }

    // 4. Discord Invites Check
    if config.anti_invites_enabled {
        let invite_regex = get_invite_regex();
        if let Some(caps) = invite_regex.captures(content) {
            let invite_code = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let is_whitelisted = config
                .whitelisted_domains
                .iter()
                .any(|d| d.eq_ignore_ascii_case(invite_code));

            if !is_whitelisted {
                let weight = *config.strike_weights.get("invite").unwrap_or(&3);
                return Some(AutomodInfraction {
                    rule_type: "invite",
                    reason: "Unauthorized Discord invite link".to_string(),
                    strike_weight: weight,
                });
            }
        }
    }

    // 5. Unauthorized Links Check
    if config.anti_links_enabled {
        let url_regex = get_url_regex();
        for caps in url_regex.captures_iter(content) {
            if let Some(domain_match) = caps.get(1) {
                let domain = domain_match.as_str().to_lowercase();
                let is_allowed = config.whitelisted_domains.iter().any(|allowed| {
                    let d = allowed.to_lowercase();
                    domain == d || domain.ends_with(&format!(".{d}"))
                });

                if !is_allowed {
                    let weight = *config.strike_weights.get("link").unwrap_or(&1);
                    return Some(AutomodInfraction {
                        rule_type: "link",
                        reason: format!("Unauthorized URL domain: {domain}"),
                        strike_weight: weight,
                    });
                }
            }
        }
    }

    // 6. Blocked Words / Profanity Check
    if !config.blocked_words.is_empty() {
        let content_lower = content.to_lowercase();
        for blocked in &config.blocked_words {
            let b_lower = blocked.trim().to_lowercase();
            if !b_lower.is_empty() && content_lower.contains(&b_lower) {
                let weight = *config.strike_weights.get("bad_word").unwrap_or(&2);
                return Some(AutomodInfraction {
                    rule_type: "bad_word",
                    reason: "Message contained a blocked word or phrase".to_string(),
                    strike_weight: weight,
                });
            }
        }
    }

    // 7. Excessive Caps Check
    if config.anti_caps_enabled {
        let alpha_chars: Vec<char> = content.chars().filter(|c| c.is_alphabetic()).collect();
        if alpha_chars.len() >= config.anti_caps_min_chars as usize {
            let uppercase_count = alpha_chars.iter().filter(|c| c.is_uppercase()).count();
            let uppercase_percent = (uppercase_count.saturating_mul(100)) / alpha_chars.len();

            if uppercase_percent >= config.anti_caps_percent as usize {
                let weight = *config.strike_weights.get("caps").unwrap_or(&1);
                return Some(AutomodInfraction {
                    rule_type: "caps",
                    reason: format!(
                        "Excessive caps lock usage ({uppercase_percent}% uppercase)"
                    ),
                    strike_weight: weight,
                });
            }
        }
    }

    // 8. Mass Mentions Check
    let total_mentions = msg
        .mentions
        .len()
        .saturating_add(msg.mention_roles.len())
        .saturating_add(if msg.mention_everyone { 1 } else { 0 });

    if total_mentions >= config.anti_mention_max as usize {
        let weight = *config.strike_weights.get("mention").unwrap_or(&3);
        return Some(AutomodInfraction {
            rule_type: "mention",
            reason: format!(
                "Mass mentions detected ({total_mentions} mentions >= threshold {})",
                config.anti_mention_max
            ),
            strike_weight: weight,
        });
    }

    None
}

/// Executes punishment ladder escalation and logs the infraction.
pub async fn handle_infraction(
    ctx: &Context,
    msg: &Message,
    state: &Arc<BotState>,
    infraction: AutomodInfraction,
) {
    let Some(guild_id) = msg.guild_id else {
        return;
    };
    let gid = guild_id.get();
    let author_id = msg.author.id.get();
    let author_tag = msg.author.tag();

    debug!(
        guild_id = gid,
        user_id = author_id,
        rule = infraction.rule_type,
        "Automod infraction detected"
    );

    // 1. Delete the offending message
    let _ = msg.delete(&ctx.http).await;

    // 2. Add strike record
    let strike = NewModStrike {
        guild_id: gid,
        user_id: author_id,
        case_id: None,
        strike_count: infraction.strike_weight,
        reason: format!("Automod: {}", infraction.reason),
        created_at: Some(Utc::now().timestamp()),
        expires_at: None,
    };
    let _ = state.storage.add_strike(&strike);

    // 3. Query total active strikes
    let total_strikes = state
        .storage
        .get_active_strikes_count(gid, author_id)
        .unwrap_or(infraction.strike_weight);

    // 4. Evaluate punishment ladder
    let config = match state.storage.get_automod_config(gid) {
        Ok(c) => c,
        Err(_) => return,
    };

    let matched_step = find_escalation_step(&config.punishment_ladder, total_strikes);

    let mut action_taken = "strike";
    let mut duration_str = None;

    if let Some(step) = matched_step {
        match step.action.to_lowercase().as_str() {
            "warn" => {
                action_taken = "warn";
                let _ = send_automod_dm(
                    ctx,
                    &msg.author,
                    "Warning",
                    &infraction.reason,
                    total_strikes,
                )
                .await;
            }
            "timeout" => {
                action_taken = "timeout";
                let dur_secs = step.duration_seconds.unwrap_or(3600);
                let dur_str = format!("{}s", dur_secs);
                duration_str = Some(dur_str);

                let expires_ts = Utc::now().timestamp().saturating_add(dur_secs);
                if let Ok(ts) = Timestamp::from_unix_timestamp(expires_ts) {
                    let edit = EditMember::new().disable_communication_until_datetime(ts);
                    let _ = guild_id.edit_member(&ctx.http, msg.author.id, edit).await;
                }

                let _ = send_automod_dm(
                    ctx,
                    &msg.author,
                    &format!("Timeout ({}m)", dur_secs / 60),
                    &infraction.reason,
                    total_strikes,
                )
                .await;
            }
            "kick" => {
                action_taken = "kick";
                let _ = send_automod_dm(
                    ctx,
                    &msg.author,
                    "Kick",
                    &infraction.reason,
                    total_strikes,
                )
                .await;
                let _ = guild_id
                    .kick_with_reason(
                        &ctx.http,
                        msg.author.id,
                        &format!("Automod: {}", infraction.reason),
                    )
                    .await;
            }
            "ban" => {
                action_taken = "ban";
                let _ = send_automod_dm(
                    ctx,
                    &msg.author,
                    "Ban",
                    &infraction.reason,
                    total_strikes,
                )
                .await;
                let _ = guild_id
                    .ban_with_reason(
                        &ctx.http,
                        msg.author.id,
                        0,
                        &format!("Automod: {}", infraction.reason),
                    )
                    .await;
            }
            _ => {}
        }

        // Create mod case record for the escalated punishment
        let new_case = NewModCase {
            guild_id: gid,
            user_id: author_id,
            moderator_id: 0, // 0 = Silas Automod
            action_type: format!("automod_{action_taken}"),
            reason: infraction.reason.clone(),
            duration_seconds: step.duration_seconds,
            created_at: Some(Utc::now().timestamp()),
            expires_at: step
                .duration_seconds
                .map(|d| Utc::now().timestamp().saturating_add(d)),
        };

        if let Ok(case) = state.storage.create_mod_case(&new_case) {
            log_mod_action(
                &ctx.http,
                &state.storage,
                gid,
                Some(case.case_number),
                &format!("Automod: {}", action_taken.to_uppercase()),
                author_id,
                &author_tag,
                0,
                "Silas Automod",
                &infraction.reason,
                duration_str.as_deref(),
            )
            .await;
        }
    } else {
        // Just log the strike
        log_mod_action(
            &ctx.http,
            &state.storage,
            gid,
            None,
            &format!("Automod Strike (+{})", infraction.strike_weight),
            author_id,
            &author_tag,
            0,
            "Silas Automod",
            &infraction.reason,
            None,
        )
        .await;
    }

    // Send temporary notification in channel and delete after 5 seconds
    let notice = format!(
        "🛡️ <@{author_id}>, your message was removed by Automod: **{}** (+{} strikes, total: {})",
        infraction.reason, infraction.strike_weight, total_strikes
    );
    if let Ok(reply) = msg.channel_id.say(&ctx.http, notice).await {
        let http = Arc::clone(&ctx.http);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let _ = reply.delete(&http).await;
        });
    }
}

/// Dispatches an informational direct message to the offending user.
async fn send_automod_dm(
    ctx: &Context,
    user: &serenity::all::User,
    action: &str,
    reason: &str,
    total_strikes: i64,
) -> bool {
    let dm_text = format!(
        "🛡️ **Silas Automod Notice**\nYou received a **{action}** for violating server rules.\n**Reason:** {reason}\n**Active Strikes:** {total_strikes}"
    );

    match user.create_dm_channel(&ctx.http).await {
        Ok(dm) => dm.say(&ctx.http, dm_text).await.is_ok(),
        Err(_) => false,
    }
}

/// Finds the highest applicable punishment step for the current strike count.
fn find_escalation_step(ladder: &[PunishmentStep], current_strikes: i64) -> Option<PunishmentStep> {
    ladder
        .iter()
        .filter(|step| current_strikes >= step.strikes)
        .max_by_key(|step| step.strikes)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spam_rate_tracker() {
        let tracker = SpamRateTracker::new();
        let guild_id = 12345;
        let user_id = 67890;
        let max_msgs = 3;
        let interval = 5;

        assert!(!tracker.record_and_check(guild_id, user_id, max_msgs, interval));
        assert!(!tracker.record_and_check(guild_id, user_id, max_msgs, interval));
        assert!(!tracker.record_and_check(guild_id, user_id, max_msgs, interval));
        // 4th message exceeds limit of 3
        assert!(tracker.record_and_check(guild_id, user_id, max_msgs, interval));

        // Different user in same guild is not blocked
        assert!(!tracker.record_and_check(guild_id, 99999, max_msgs, interval));
    }

    #[test]
    fn test_invite_regex() {
        let regex = get_invite_regex();
        let valid_invites = [
            "Join here: https://discord.gg/test-invite",
            "discord.gg/abc1234",
            "Check out discord.com/invite/my-community now!",
        ];
        for text in valid_invites {
            assert!(regex.is_match(text), "Should match: {text}");
        }

        let non_invites = [
            "Hello world",
            "https://discord.com/channels/123/456",
            "https://google.com",
        ];
        for text in non_invites {
            assert!(!regex.is_match(text), "Should not match: {text}");
        }
    }

    #[test]
    fn test_url_regex() {
        let regex = get_url_regex();
        let text = "Visit https://malicious-site.xyz/path/to/page and http://sub.legit.com";
        let matches: Vec<String> = regex
            .captures_iter(text)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect();

        assert_eq!(matches, vec!["malicious-site.xyz", "sub.legit.com"]);
    }

    #[test]
    fn test_find_escalation_step() {
        let ladder = vec![
            PunishmentStep {
                strikes: 2,
                action: "warn".to_string(),
                duration_seconds: None,
            },
            PunishmentStep {
                strikes: 4,
                action: "timeout".to_string(),
                duration_seconds: Some(3600),
            },
            PunishmentStep {
                strikes: 8,
                action: "kick".to_string(),
                duration_seconds: None,
            },
            PunishmentStep {
                strikes: 10,
                action: "ban".to_string(),
                duration_seconds: None,
            },
        ];

        assert_eq!(find_escalation_step(&ladder, 1), None);
        assert_eq!(
            find_escalation_step(&ladder, 2).map(|s| s.action),
            Some("warn".to_string())
        );
        assert_eq!(
            find_escalation_step(&ladder, 3).map(|s| s.action),
            Some("warn".to_string())
        );
        assert_eq!(
            find_escalation_step(&ladder, 4).map(|s| s.action),
            Some("timeout".to_string())
        );
        assert_eq!(
            find_escalation_step(&ladder, 7).map(|s| s.action),
            Some("timeout".to_string())
        );
        assert_eq!(
            find_escalation_step(&ladder, 8).map(|s| s.action),
            Some("kick".to_string())
        );
        assert_eq!(
            find_escalation_step(&ladder, 15).map(|s| s.action),
            Some("ban".to_string())
        );
    }
}

