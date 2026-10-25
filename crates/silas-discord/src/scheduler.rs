//! Background timed scheduler task running every 15 seconds to evaluate expired
//! moderation punishments (tempbans, timeouts) and decay automod strikes.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use parking_lot::RwLock;
use serenity::all::{EditMember, GuildId, Http, UserId};
use silas_storage::Storage;
use tracing::{debug, error, info, warn};

use crate::logging::log_mod_action;

/// Interval for checking expired moderation cases (15 seconds).
pub const CHECK_INTERVAL_SECS: u64 = 15;

/// Number of 15-second cycles before running strike decay (20 cycles = 5 minutes).
pub const STRIKE_DECAY_CYCLE_TICKS: u64 = 20;

/// Spawns the recurring background task that checks expired cases and decays strikes.
pub fn spawn_scheduler_worker(
    storage: Storage,
    http_getter: Arc<RwLock<Option<Arc<Http>>>>,
) {
    tokio::spawn(async move {
        info!("Moderation background scheduler worker started (interval: 15s).");
        let mut tick_counter: u64 = 0;

        loop {
            tokio::time::sleep(Duration::from_secs(CHECK_INTERVAL_SECS)).await;
            tick_counter = tick_counter.wrapping_add(1);

            let http = match http_getter.read().clone() {
                Some(h) => h,
                None => {
                    debug!("Discord HTTP client not connected; skipping scheduler tick.");
                    continue;
                }
            };

            let now = Utc::now().timestamp();

            // 1. Process expired moderation cases
            match storage.get_active_expired_cases(now) {
                Ok(expired_cases) => {
                    for case in expired_cases {
                        process_expired_case(&http, &storage, &case).await;
                    }
                }
                Err(e) => {
                    error!("Scheduler failed to query active expired cases: {e}");
                }
            }

            // 2. Periodically process strike decay and memory cleanup every 5 minutes
            if tick_counter.is_multiple_of(STRIKE_DECAY_CYCLE_TICKS) {
                process_strike_decay(&storage, now);
                crate::triggers::prune_stale_cooldowns(Duration::from_secs(3600));
            }
        }
    });
}

/// Executes unban or removes timeout for an expired moderation case.
async fn process_expired_case(http: &Http, storage: &Storage, case: &silas_storage::ModCase) {
    let guild = GuildId::new(case.guild_id);
    let target_user = UserId::new(case.user_id);

    match case.action_type.to_lowercase().as_str() {
        "tempban" | "ban" => {
            debug!(
                guild_id = case.guild_id,
                user_id = case.user_id,
                case_number = case.case_number,
                "Lifting expired tempban"
            );

            match guild.unban(http, target_user).await {
                Ok(()) => {
                    info!(
                        guild_id = case.guild_id,
                        user_id = case.user_id,
                        "Successfully unbanned user for expired tempban"
                    );
                    log_mod_action(
                        http,
                        storage,
                        case.guild_id,
                        Some(case.case_number),
                        "Tempban Expired",
                        case.user_id,
                        &format!("<@{}>", case.user_id),
                        0,
                        "Silas Scheduler",
                        "Automatic unban after tempban duration expired",
                        None,
                    )
                    .await;
                }
                Err(e) => {
                    warn!(
                        guild_id = case.guild_id,
                        user_id = case.user_id,
                        "Failed to unban user for expired tempban: {e}"
                    );
                }
            }
        }
        "timeout" => {
            debug!(
                guild_id = case.guild_id,
                user_id = case.user_id,
                case_number = case.case_number,
                "Lifting expired timeout"
            );

            let edit = EditMember::new().enable_communication();
            match guild.edit_member(http, target_user, edit).await {
                Ok(_) => {
                    info!(
                        guild_id = case.guild_id,
                        user_id = case.user_id,
                        "Successfully lifted communication timeout"
                    );
                    log_mod_action(
                        http,
                        storage,
                        case.guild_id,
                        Some(case.case_number),
                        "Timeout Expired",
                        case.user_id,
                        &format!("<@{}>", case.user_id),
                        0,
                        "Silas Scheduler",
                        "Automatic timeout removal after duration expired",
                        None,
                    )
                    .await;
                }
                Err(e) => {
                    warn!(
                        guild_id = case.guild_id,
                        user_id = case.user_id,
                        "Failed to remove timeout for user: {e}"
                    );
                }
            }
        }
        _ => {
            // Other actions (e.g. warn, softban) have no active Discord punishment to revert
        }
    }

    // Always deactivate the case in the database once expired
    if let Err(e) = storage.deactivate_case(case.id) {
        error!(
            case_id = case.id,
            "Failed to deactivate expired case in database: {e}"
        );
    }
}

/// Evaluates and decays strikes across all cached guilds according to each guild's configuration.
fn process_strike_decay(storage: &Storage, now: i64) {
    let guild_ids = storage.list_guild_ids();
    for gid in guild_ids {
        let automod_cfg = match storage.get_automod_config(gid) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if automod_cfg.strike_decay_hours <= 0 {
            continue;
        }

        let decay_seconds = automod_cfg.strike_decay_hours.saturating_mul(3600);
        let cutoff = now.saturating_sub(decay_seconds);

        match storage.decay_strikes(gid, cutoff) {
            Ok(decayed) if decayed > 0 => {
                info!(
                    guild_id = gid,
                    decayed_strikes = decayed,
                    "Decayed expired strikes for guild"
                );
            }
            Ok(_) => {}
            Err(e) => {
                warn!(
                    guild_id = gid,
                    "Failed to decay strikes for guild: {e}"
                );
            }
        }
    }
}
