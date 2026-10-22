"""
Role-based Token Limiter and Quota Management for Silas Blue.
Provides configurable token limits per Discord role, customizable reset schedules,
and fine-grained partial overage policies.
"""

import os
import sqlite3
import logging
from dataclasses import dataclass
from datetime import datetime, timezone, timedelta
from typing import Optional, Tuple, Dict, Any

from permissions import PermissionManager

logger = logging.getLogger("silasblue.token_limiter")

DB_DIR = "state"
DB_PATH = os.path.join(DB_DIR, "usage.db")


@dataclass
class QuotaResult:
    allowed: bool
    limit: int  # -1 if unlimited
    used: int
    remaining: int  # -1 if unlimited
    role_name: str
    reset_epoch: int
    effective_max_tokens: Optional[int] = None
    reason: str = ""


class TokenLimiter:
    """
    Manages Discord role token limits, usage persistence via SQLite,
    reset schedules, and partial overage enforcement.
    """

    def __init__(self, db_path: str = DB_PATH):
        self.db_path = db_path
        self._last_prune_ts = 0.0
        self._ensure_db()

    def _ensure_db(self):
        """Creates the SQLite database and indices if they do not exist."""
        os.makedirs(os.path.dirname(self.db_path), exist_ok=True)
        try:
            with sqlite3.connect(self.db_path) as conn:
                cursor = conn.cursor()
                cursor.execute("""
                    CREATE TABLE IF NOT EXISTS token_records (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        guild_id INTEGER NOT NULL,
                        user_id INTEGER NOT NULL,
                        tokens INTEGER NOT NULL,
                        prompt_tokens INTEGER DEFAULT 0,
                        completion_tokens INTEGER DEFAULT 0,
                        model TEXT DEFAULT '',
                        timestamp REAL NOT NULL
                    )
                """)
                cursor.execute("""
                    CREATE INDEX IF NOT EXISTS idx_token_records_lookup 
                    ON token_records (guild_id, user_id, timestamp)
                """)
                conn.commit()
        except Exception as e:
            logger.error(f"Error initializing usage database at {self.db_path}: {e}")

    def record_tokens(
        self,
        guild_id: int,
        user_id: int,
        tokens: int,
        prompt_tokens: int = 0,
        completion_tokens: int = 0,
        model: str = ""
    ):
        """Records token consumption for a user prompt."""
        try:
            tokens_int = int(tokens)
            if tokens_int <= 0:
                return
            p_tok = int(prompt_tokens) if prompt_tokens else 0
            c_tok = int(completion_tokens) if completion_tokens else 0
        except (ValueError, TypeError):
            return

        now_ts = datetime.now(timezone.utc).timestamp()
        try:
            with sqlite3.connect(self.db_path) as conn:
                cursor = conn.cursor()
                cursor.execute(
                    """
                    INSERT INTO token_records 
                    (guild_id, user_id, tokens, prompt_tokens, completion_tokens, model, timestamp)
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    """,
                    (int(guild_id), int(user_id), tokens_int, p_tok, c_tok, str(model), now_ts)
                )
                # Periodically prune records older than 35 days (once per 24h) to bound SQLite growth
                if now_ts - self._last_prune_ts > 86400:
                    cutoff_ts = now_ts - (35 * 86400)
                    cursor.execute("DELETE FROM token_records WHERE timestamp < ?", (cutoff_ts,))
                    self._last_prune_ts = now_ts
                conn.commit()
        except Exception as e:
            logger.error(f"Failed to record token usage in {self.db_path}: {e}")

    def get_user_usage(
        self,
        guild_id: int,
        user_id: int,
        start_time: datetime,
        end_time: datetime
    ) -> int:
        """Sums tokens consumed by user in the specified time window."""
        start_ts = start_time.timestamp()
        end_ts = end_time.timestamp()
        try:
            with sqlite3.connect(self.db_path) as conn:
                cursor = conn.cursor()
                cursor.execute(
                    """
                    SELECT COALESCE(SUM(tokens), 0)
                    FROM token_records
                    WHERE guild_id = ? AND user_id = ? AND timestamp >= ? AND timestamp < ?
                    """,
                    (int(guild_id), int(user_id), start_ts, end_ts)
                )
                row = cursor.fetchone()
                return int(row[0]) if row else 0
        except Exception as e:
            logger.error(f"Error reading token usage: {e}")
            return 0

    @staticmethod
    def calculate_window_period(
        frequency: str = "daily",
        reset_time: str = "00:00",
        tz_offset: int = 0,
        day_of_week: int = 0,
        day_of_month: int = 1,
        current_utc: Optional[datetime] = None
    ) -> Tuple[datetime, datetime, int]:
        """
        Calculates (window_start_utc, window_end_utc, reset_epoch).
        
        frequency: 'daily', 'weekly', 'monthly', or 'rolling_hours'
        reset_time: 'HH:MM'
        tz_offset: integer hours offset from UTC (e.g. -5, 0, +2)
        day_of_week: 0=Monday .. 6=Sunday (for weekly)
        day_of_month: 1..28 (for monthly)
        """
        now_utc = current_utc or datetime.now(timezone.utc)
        tz = timezone(timedelta(hours=tz_offset))
        local_now = now_utc.astimezone(tz)

        try:
            parts = reset_time.strip().split(":")
            reset_h = int(parts[0]) % 24
            reset_m = int(parts[1]) % 60
        except Exception:
            reset_h, reset_m = 0, 0

        freq = (frequency or "daily").lower()

        if freq == "weekly":
            target_dow = int(day_of_week) % 7
            days_since = (local_now.weekday() - target_dow) % 7
            candidate_reset = (local_now - timedelta(days=days_since)).replace(
                hour=reset_h, minute=reset_m, second=0, microsecond=0
            )
            if local_now >= candidate_reset:
                window_start_local = candidate_reset
                window_end_local = candidate_reset + timedelta(days=7)
            else:
                window_start_local = candidate_reset - timedelta(days=7)
                window_end_local = candidate_reset

        elif freq == "monthly":
            target_day = max(1, min(int(day_of_month), 28))
            candidate_reset = local_now.replace(
                day=target_day, hour=reset_h, minute=reset_m, second=0, microsecond=0
            )
            if local_now >= candidate_reset:
                window_start_local = candidate_reset
                # Next month
                if candidate_reset.month == 12:
                    window_end_local = candidate_reset.replace(year=candidate_reset.year + 1, month=1)
                else:
                    window_end_local = candidate_reset.replace(month=candidate_reset.month + 1)
            else:
                window_end_local = candidate_reset
                # Previous month
                if candidate_reset.month == 1:
                    window_start_local = candidate_reset.replace(year=candidate_reset.year - 1, month=12)
                else:
                    window_start_local = candidate_reset.replace(month=candidate_reset.month - 1)

        elif freq == "rolling_hours":
            # 24-hour sliding window
            window_start_utc = now_utc - timedelta(hours=24)
            window_end_utc = now_utc + timedelta(hours=24)
            return window_start_utc, window_end_utc, int(window_end_utc.timestamp())

        else:
            # Default: daily
            candidate_reset = local_now.replace(
                hour=reset_h, minute=reset_m, second=0, microsecond=0
            )
            if local_now >= candidate_reset:
                window_start_local = candidate_reset
                window_end_local = candidate_reset + timedelta(days=1)
            else:
                window_start_local = candidate_reset - timedelta(days=1)
                window_end_local = candidate_reset

        window_start_utc = window_start_local.astimezone(timezone.utc)
        window_end_utc = window_end_local.astimezone(timezone.utc)
        reset_epoch = int(window_end_utc.timestamp())
        return window_start_utc, window_end_utc, reset_epoch

    def get_user_role_limit(self, member: Any, guild_config: Dict[str, Any]) -> Tuple[int, str]:
        """
        Determines the effective token limit for a member.
        Returns (highest_limit, winning_role_name).
        -1 represents unlimited.
        Highest Limit Wins.
        """
        # 1. Administrator & Server Owner Bypass
        guild = getattr(member, "guild", None)
        if PermissionManager._is_admin(member, guild):
            return -1, "Administrator (Bypass)"

        quotas_cfg = guild_config.get("token_quotas", {})
        role_limits = quotas_cfg.get("role_limits", {})
        if not role_limits:
            return -1, "Unlimited (No Limits Configured)"

        member_roles = getattr(member, "roles", [])
        user_role_names = {r.name.casefold(): r.name for r in member_roles}
        user_role_ids = {str(r.id): r.name for r in member_roles}

        max_limit = -2  # Unset
        winning_role = "everyone"

        # Check matched roles
        for raw_key, raw_val in role_limits.items():
            key_clean = str(raw_key).strip().casefold()
            try:
                limit_val = int(raw_val)
            except (ValueError, TypeError):
                continue

            matched = False
            role_display_name = str(raw_key)

            if key_clean in ("everyone", "@everyone"):
                matched = True
                role_display_name = "everyone"
            elif key_clean in user_role_names:
                matched = True
                role_display_name = user_role_names[key_clean]
            elif str(raw_key).strip() in user_role_ids:
                matched = True
                role_display_name = user_role_ids[str(raw_key).strip()]

            if matched:
                if limit_val == -1:
                    # Unlimited role always trumps all numerical caps
                    return -1, role_display_name
                if limit_val > max_limit:
                    max_limit = limit_val
                    winning_role = role_display_name

        if max_limit == -2:
            # Fallback to default or unlimited
            default_limit = role_limits.get("everyone", 10000)
            try:
                return int(default_limit), "everyone"
            except (ValueError, TypeError):
                return 10000, "everyone"

        return max_limit, winning_role

    def check_prompt_allowed(
        self,
        guild_id: int,
        member: Any,
        guild_config: Dict[str, Any],
        current_utc: Optional[datetime] = None
    ) -> QuotaResult:
        """
        Evaluates whether a user prompt is permitted under role token quotas.
        Calculates remaining budget, applies partial overage rules, and returns effective_max_tokens.
        """
        quotas_cfg = guild_config.get("token_quotas", {})
        if not quotas_cfg.get("enabled", False):
            return QuotaResult(
                allowed=True,
                limit=-1,
                used=0,
                remaining=-1,
                role_name="Uncapped",
                reset_epoch=0,
                effective_max_tokens=None,
                reason="Quotas disabled"
            )

        limit, role_name = self.get_user_role_limit(member, guild_config)
        if limit == -1:
            return QuotaResult(
                allowed=True,
                limit=-1,
                used=0,
                remaining=-1,
                role_name=role_name,
                reset_epoch=0,
                effective_max_tokens=None,
                reason="Unlimited role"
            )

        # Calculate active window
        freq = quotas_cfg.get("reset_frequency", "daily")
        reset_time = quotas_cfg.get("reset_time", "00:00")
        tz_offset = int(quotas_cfg.get("reset_timezone_offset", 0))
        day_of_week = int(quotas_cfg.get("reset_day_of_week", 0))
        day_of_month = int(quotas_cfg.get("reset_day_of_month", 1))

        start_time, end_time, reset_epoch = self.calculate_window_period(
            frequency=freq,
            reset_time=reset_time,
            tz_offset=tz_offset,
            day_of_week=day_of_week,
            day_of_month=day_of_month,
            current_utc=current_utc
        )

        user_id = getattr(member, "id", 0)
        used_tokens = self.get_user_usage(guild_id, user_id, start_time, end_time)
        remaining = limit - used_tokens

        overage_cfg = quotas_cfg.get("overage_policy", {})
        allow_partial_overage = bool(overage_cfg.get("allow_partial_overage", True))
        try:
            max_overage_tokens = int(overage_cfg.get("max_overage_tokens", 1000))
        except (ValueError, TypeError):
            max_overage_tokens = 1000

        # Scenario 1: Remaining tokens are positive
        if remaining > 0:
            if allow_partial_overage:
                if max_overage_tokens == -1:
                    effective_max = None
                else:
                    effective_max = remaining + max(0, max_overage_tokens)
            else:
                effective_max = max(1, remaining)

            return QuotaResult(
                allowed=True,
                limit=limit,
                used=used_tokens,
                remaining=remaining,
                role_name=role_name,
                reset_epoch=reset_epoch,
                effective_max_tokens=effective_max,
                reason="Within quota"
            )

        # Scenario 2: Remaining tokens exhausted (<= 0)
        return QuotaResult(
            allowed=False,
            limit=limit,
            used=used_tokens,
            remaining=max(0, remaining),
            role_name=role_name,
            reset_epoch=reset_epoch,
            effective_max_tokens=0,
            reason="Quota exhausted"
        )


# Global singleton instance
token_limiter = TokenLimiter()
