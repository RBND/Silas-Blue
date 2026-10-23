//! Role-based token quota calculation, saturating overage enforcement,
//! and in-flight reservation guards to prevent concurrent burst bypasses.

use std::sync::Arc;

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveTime, Offset, TimeZone, Utc};
use dashmap::DashMap;

use crate::config::{parse_snowflake, GuildConfig};
use crate::permissions::MemberContext;

/// Maximum completion token cap passed to any LLM provider when clamping overages.
const MAX_CLAMPED_COMPLETION_TOKENS: u32 = 128_000;

/// Default token reservation held while an LLM request is in-flight.
const DEFAULT_IN_FLIGHT_RESERVATION: i64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaResult {
    pub allowed: bool,
    pub limit: i64, // -1 if unlimited
    pub used: i64,
    pub remaining: i64, // -1 if unlimited
    pub role_name: String,
    pub reset_epoch: i64,
    pub effective_max_tokens: Option<u32>,
    pub reason: String,
}

/// Calculates `(window_start_utc, window_end_utc, reset_epoch)` for a server's quota schedule.
///
/// - `frequency`: `"daily"`, `"weekly"`, `"monthly"`, or `"rolling_hours"`
/// - `reset_time`: `"HH:MM"`
/// - `tz_offset`: integer hours offset from UTC (`-12..=14`)
/// - `day_of_week`: `0 = Monday .. 6 = Sunday` (matches Python `datetime.weekday()`)
/// - `day_of_month`: `1..=28`
pub fn calculate_window_period(
    frequency: &str,
    reset_time: &str,
    tz_offset: i32,
    day_of_week: u32,
    day_of_month: u32,
    current_utc: Option<DateTime<Utc>>,
) -> (DateTime<Utc>, DateTime<Utc>, i64) {
    let now_utc = current_utc.unwrap_or_else(Utc::now);
    let clamped_tz = tz_offset.clamp(-12, 14);
    let offset = FixedOffset::east_opt(clamped_tz.saturating_mul(3600))
        .unwrap_or_else(|| Utc.fix());
    let local_now = now_utc.with_timezone(&offset);

    let (reset_h, reset_m) = parse_hh_mm(reset_time);
    let target_time = NaiveTime::from_hms_opt(reset_h, reset_m, 0).unwrap_or(NaiveTime::MIN);

    let freq_low = frequency.trim().to_ascii_lowercase();

    if freq_low == "rolling_hours" {
        let start = now_utc - Duration::hours(24);
        let end = now_utc + Duration::hours(24);
        return (start, end, end.timestamp());
    }

    let (start_local, end_local) = if freq_low == "weekly" {
        let target_dow = (day_of_week % 7) as i64;
        let current_dow = i64::from(local_now.weekday().num_days_from_monday());
        let days_since = (current_dow - target_dow).rem_euclid(7);
        let candidate_date = (local_now - Duration::days(days_since)).date_naive();
        let candidate_reset = offset
            .from_local_datetime(&candidate_date.and_time(target_time))
            .single()
            .unwrap_or(local_now);

        if local_now >= candidate_reset {
            (candidate_reset, candidate_reset + Duration::days(7))
        } else {
            (candidate_reset - Duration::days(7), candidate_reset)
        }
    } else if freq_low == "monthly" {
        let target_day = day_of_month.clamp(1, 28);
        let year = local_now.year();
        let month = local_now.month();
        let candidate_date =
            NaiveDate::from_ymd_opt(year, month, target_day).unwrap_or_else(|| local_now.date_naive());
        let candidate_reset = offset
            .from_local_datetime(&candidate_date.and_time(target_time))
            .single()
            .unwrap_or(local_now);

        if local_now >= candidate_reset {
            let (next_y, next_m) = if month == 12 {
                (year.saturating_add(1), 1)
            } else {
                (year, month.saturating_add(1))
            };
            let next_date = NaiveDate::from_ymd_opt(next_y, next_m, target_day)
                .unwrap_or(candidate_date);
            let next_reset = offset
                .from_local_datetime(&next_date.and_time(target_time))
                .single()
                .unwrap_or_else(|| candidate_reset + Duration::days(30));
            (candidate_reset, next_reset)
        } else {
            let (prev_y, prev_m) = if month == 1 {
                (year.saturating_sub(1), 12)
            } else {
                (year, month.saturating_sub(1))
            };
            let prev_date = NaiveDate::from_ymd_opt(prev_y, prev_m, target_day)
                .unwrap_or(candidate_date);
            let prev_reset = offset
                .from_local_datetime(&prev_date.and_time(target_time))
                .single()
                .unwrap_or_else(|| candidate_reset - Duration::days(30));
            (prev_reset, candidate_reset)
        }
    } else {
        // Default: daily
        let candidate_date = local_now.date_naive();
        let candidate_reset = offset
            .from_local_datetime(&candidate_date.and_time(target_time))
            .single()
            .unwrap_or(local_now);

        if local_now >= candidate_reset {
            (candidate_reset, candidate_reset + Duration::days(1))
        } else {
            (candidate_reset - Duration::days(1), candidate_reset)
        }
    };

    let start_utc = start_local.with_timezone(&Utc);
    let end_utc = end_local.with_timezone(&Utc);
    let reset_epoch = end_utc.timestamp();
    (start_utc, end_utc, reset_epoch)
}

fn parse_hh_mm(s: &str) -> (u32, u32) {
    let mut parts = s.trim().split(':');
    let h = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .map_or(0, |v| v % 24);
    let m = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .map_or(0, |v| v % 60);
    (h, m)
}

/// Determines the effective token limit for a guild member.
/// Returns `(highest_limit, winning_role_name)` where `-1` represents Unlimited.
/// Highest Limit Wins.
pub fn resolve_user_role_limit(
    member: &MemberContext<'_>,
    guild_config: &GuildConfig,
    guild_role_map: &[(u64, String)],
) -> (i64, String) {
    // 1. Administrator & Guild Owner Bypass
    if member.is_admin_or_owner() {
        return (-1, "Administrator (Bypass)".to_owned());
    }

    let role_limits = &guild_config.token_quotas.role_limits;
    if role_limits.is_empty() {
        return (-1, "Unlimited (No Limits Configured)".to_owned());
    }

    let mut max_limit: i64 = -2; // Unset
    let mut winning_role = "everyone".to_owned();

    for (raw_key, &limit_val) in role_limits {
        let key_trimmed = raw_key.trim();
        let key_lower = key_trimmed.to_lowercase();

        let mut matched = false;
        let mut display_name = key_trimmed.to_owned();

        if key_lower == "everyone" || key_lower == "@everyone" {
            matched = true;
            display_name = "everyone".to_owned();
        } else if let Some(role_id) = parse_snowflake(key_trimmed) {
            if member.role_ids.contains(&role_id) {
                matched = true;
                if let Some((_, name)) = guild_role_map.iter().find(|(id, _)| *id == role_id) {
                    display_name = name.clone();
                }
            }
        } else {
            for r_name in member.role_names {
                if r_name.trim().to_lowercase() == key_lower {
                    matched = true;
                    display_name = r_name.clone();
                    break;
                }
            }
        }

        if matched {
            if limit_val == -1 {
                return (-1, display_name);
            }
            if limit_val > max_limit {
                max_limit = limit_val;
                winning_role = display_name;
            }
        }
    }

    if max_limit == -2 {
        let default_limit = role_limits
            .get("everyone")
            .or_else(|| role_limits.get("@everyone"))
            .copied()
            .unwrap_or(10_000);
        return (default_limit.max(-1), "everyone".to_owned());
    }

    (max_limit, winning_role)
}

/// Evaluates whether a prompt is permitted given `used_tokens` (including any in-flight reservations).
pub fn evaluate_quota(
    member: &MemberContext<'_>,
    guild_config: &GuildConfig,
    guild_role_map: &[(u64, String)],
    used_tokens: i64,
    current_utc: Option<DateTime<Utc>>,
) -> QuotaResult {
    let tq = &guild_config.token_quotas;
    if !tq.enabled {
        return QuotaResult {
            allowed: true,
            limit: -1,
            used: 0,
            remaining: -1,
            role_name: "Uncapped".to_owned(),
            reset_epoch: 0,
            effective_max_tokens: None,
            reason: "Quotas disabled".to_owned(),
        };
    }

    let (limit, role_name) = resolve_user_role_limit(member, guild_config, guild_role_map);
    if limit == -1 {
        return QuotaResult {
            allowed: true,
            limit: -1,
            used: 0,
            remaining: -1,
            role_name,
            reset_epoch: 0,
            effective_max_tokens: None,
            reason: "Unlimited role".to_owned(),
        };
    }

    let (_, _, reset_epoch) = calculate_window_period(
        &tq.reset_frequency,
        &tq.reset_time,
        tq.reset_timezone_offset,
        tq.reset_day_of_week,
        tq.reset_day_of_month,
        current_utc,
    );

    let safe_used = used_tokens.max(0);
    let remaining = limit.saturating_sub(safe_used);

    if remaining > 0 {
        let allow_partial = tq.overage_policy.allow_partial_overage;
        let max_overage = tq.overage_policy.max_overage_tokens;

        let effective_max = if allow_partial {
            if max_overage == -1 {
                None
            } else {
                let total_budget = remaining.saturating_add(max_overage.max(0));
                let clamped = total_budget.clamp(1, i64::from(MAX_CLAMPED_COMPLETION_TOKENS)) as u32;
                Some(clamped)
            }
        } else {
            let clamped = remaining.clamp(1, i64::from(MAX_CLAMPED_COMPLETION_TOKENS)) as u32;
            Some(clamped)
        };

        return QuotaResult {
            allowed: true,
            limit,
            used: safe_used,
            remaining,
            role_name,
            reset_epoch,
            effective_max_tokens: effective_max,
            reason: "Within quota".to_owned(),
        };
    }

    QuotaResult {
        allowed: false,
        limit,
        used: safe_used,
        remaining: 0,
        role_name,
        reset_epoch,
        effective_max_tokens: Some(0),
        reason: "Quota exhausted".to_owned(),
    }
}

/// Manages in-flight token reservations per `(guild_id, user_id)` so a user sending
/// multiple concurrent messages cannot bypass `evaluate_quota` before the first LLM response completes.
#[derive(Clone, Default)]
pub struct QuotaReservationManager {
    in_flight: Arc<DashMap<(u64, u64), i64>>,
}

pub struct QuotaReservationGuard {
    map: Arc<DashMap<(u64, u64), i64>>,
    key: (u64, u64),
    reserved: i64,
}

impl Drop for QuotaReservationGuard {
    fn drop(&mut self) {
        if self.reserved <= 0 {
            return;
        }
        let mut should_remove = false;
        if let Some(mut entry) = self.map.get_mut(&self.key) {
            *entry = entry.saturating_sub(self.reserved).max(0);
            if *entry == 0 {
                should_remove = true;
            }
        }
        if should_remove {
            self.map.remove_if(&self.key, |_, v| *v <= 0);
        }
    }
}

impl QuotaReservationManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns currently reserved in-flight tokens for `(guild_id, user_id)`.
    pub fn current_reserved(&self, guild_id: u64, user_id: u64) -> i64 {
        self.in_flight
            .get(&(guild_id, user_id))
            .map_or(0, |v| *v)
    }

    /// Atomically reserves `tokens_to_reserve` (or `DEFAULT_IN_FLIGHT_RESERVATION`) for `(guild_id, user_id)`.
    pub fn reserve(&self, guild_id: u64, user_id: u64, remaining: i64) -> QuotaReservationGuard {
        let reserved = if remaining < 0 {
            0
        } else {
            remaining.clamp(1, DEFAULT_IN_FLIGHT_RESERVATION)
        };
        if reserved > 0 {
            self.in_flight
                .entry((guild_id, user_id))
                .and_modify(|v| *v = v.saturating_add(reserved))
                .or_insert(reserved);
        }
        QuotaReservationGuard {
            map: Arc::clone(&self.in_flight),
            key: (guild_id, user_id),
            reserved,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_quota_window_weekly_and_monthly() {
        let fixed_now = Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).single().expect("valid date");
        let (start, end, epoch) = calculate_window_period("weekly", "00:00", 0, 0, 1, Some(fixed_now));
        assert!(start <= fixed_now);
        assert!(end > fixed_now);
        assert_eq!(epoch, end.timestamp());
        assert_eq!((end - start).num_days(), 7);

        let (m_start, m_end, _) = calculate_window_period("monthly", "06:30", -4, 0, 15, Some(fixed_now));
        assert!(m_start <= fixed_now);
        assert!(m_end > fixed_now);
    }

    #[test]
    fn test_highest_role_wins_and_partial_overage() {
        let mut cfg = GuildConfig::default();
        cfg.token_quotas.enabled = true;
        cfg.token_quotas.role_limits.insert("everyone".to_owned(), 5_000);
        cfg.token_quotas.role_limits.insert("Supporter".to_owned(), 25_000);
        cfg.token_quotas.overage_policy.allow_partial_overage = true;
        cfg.token_quotas.overage_policy.max_overage_tokens = 1_000;
        cfg.sanitize_and_compile();

        let role_names = vec!["Supporter".to_owned()];
        let member = MemberContext {
            user_id: 999,
            is_guild_owner: false,
            is_administrator: false,
            role_ids: &[],
            role_names: &role_names,
        };

        let res = evaluate_quota(&member, &cfg, &[], 24_500, None);
        assert!(res.allowed);
        assert_eq!(res.limit, 25_000);
        assert_eq!(res.remaining, 500);
        assert_eq!(res.effective_max_tokens, Some(1500));

        let exhausted = evaluate_quota(&member, &cfg, &[], 25_000, None);
        assert!(!exhausted.allowed);
        assert_eq!(exhausted.remaining, 0);
    }

    #[test]
    fn test_in_flight_reservation_guard_blocks_concurrent_burst() {
        let mgr = QuotaReservationManager::new();
        assert_eq!(mgr.current_reserved(1, 42), 0);
        {
            let _g1 = mgr.reserve(1, 42, 600);
            assert_eq!(mgr.current_reserved(1, 42), 600);
        }
        assert_eq!(mgr.current_reserved(1, 42), 0);
    }
}
