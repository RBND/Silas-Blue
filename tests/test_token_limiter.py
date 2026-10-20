"""
Tests for Role-based Token Limiter and Quota Management.
"""

import os
import tempfile
import pytest
from datetime import datetime, timezone, timedelta
from unittest.mock import MagicMock

from token_limiter import TokenLimiter, QuotaResult


@pytest.fixture
def temp_limiter():
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    limiter = TokenLimiter(db_path=db_path)
    yield limiter
    if os.path.exists(db_path):
        try:
            os.remove(db_path)
        except OSError:
            pass


def test_db_token_recording_and_aggregation(temp_limiter):
    guild_id = 123456789
    user_id = 987654321
    now = datetime.now(timezone.utc)
    start_window = now - timedelta(hours=1)
    end_window = now + timedelta(hours=1)

    # Initial usage should be 0
    assert temp_limiter.get_user_usage(guild_id, user_id, start_window, end_window) == 0

    # Record some usage
    temp_limiter.record_tokens(guild_id, user_id, tokens=150, prompt_tokens=50, completion_tokens=100, model="test-model")
    temp_limiter.record_tokens(guild_id, user_id, tokens=250, prompt_tokens=70, completion_tokens=180, model="test-model")

    # Usage should sum to 400
    assert temp_limiter.get_user_usage(guild_id, user_id, start_window, end_window) == 400

    # Another user in same guild should have 0
    assert temp_limiter.get_user_usage(guild_id, 111111111, start_window, end_window) == 0


def test_calculate_window_daily():
    # Fixed test time: 2026-10-19 14:30:00 UTC
    fixed_now = datetime(2026, 10, 19, 14, 30, 0, tzinfo=timezone.utc)
    start, end, epoch = TokenLimiter.calculate_window_period(
        frequency="daily",
        reset_time="00:00",
        tz_offset=0,
        current_utc=fixed_now
    )
    # Reset is at midnight UTC, so window should start at 2026-10-19 00:00 and end at 2026-10-20 00:00
    assert start == datetime(2026, 10, 19, 0, 0, 0, tzinfo=timezone.utc)
    assert end == datetime(2026, 10, 20, 0, 0, 0, tzinfo=timezone.utc)
    assert epoch == int(end.timestamp())


def test_calculate_window_daily_with_tz_offset():
    # Fixed test time: 2026-10-19 02:00:00 UTC
    # In EDT (UTC-4), it is 2026-10-18 22:00:00
    fixed_now = datetime(2026, 10, 19, 2, 0, 0, tzinfo=timezone.utc)
    start, end, epoch = TokenLimiter.calculate_window_period(
        frequency="daily",
        reset_time="00:00",
        tz_offset=-4,
        current_utc=fixed_now
    )
    # In UTC-4, current local time is 10-18 22:00.
    # The current window started at 10-18 00:00 local (= 10-18 04:00 UTC)
    # And ends at 10-19 00:00 local (= 10-19 04:00 UTC)
    assert start == datetime(2026, 10, 18, 4, 0, 0, tzinfo=timezone.utc)
    assert end == datetime(2026, 10, 19, 4, 0, 0, tzinfo=timezone.utc)


def test_calculate_window_weekly():
    # 2026-10-19 is a Monday (weekday=0)
    fixed_now = datetime(2026, 10, 19, 12, 0, 0, tzinfo=timezone.utc)
    start, end, epoch = TokenLimiter.calculate_window_period(
        frequency="weekly",
        reset_time="00:00",
        tz_offset=0,
        day_of_week=0,  # Monday
        current_utc=fixed_now
    )
    assert start == datetime(2026, 10, 19, 0, 0, 0, tzinfo=timezone.utc)
    assert end == datetime(2026, 10, 26, 0, 0, 0, tzinfo=timezone.utc)


def test_role_limit_resolution():
    limiter = TokenLimiter()
    guild_config = {
        "token_quotas": {
            "enabled": True,
            "role_limits": {
                "everyone": 5000,
                "Supporter": 25000,
                "VIP": 100000,
                "Staff": -1
            }
        }
    }

    # User with only @everyone
    role_everyone = MagicMock()
    role_everyone.name = "@everyone"
    role_everyone.id = 1

    member_basic = MagicMock()
    member_basic.guild = MagicMock()
    member_basic.guild.owner = None
    member_basic.guild_permissions.administrator = False
    member_basic.roles = [role_everyone]

    limit, role_name = limiter.get_user_role_limit(member_basic, guild_config)
    assert limit == 5000
    assert role_name == "everyone"

    # User with @everyone and Supporter and VIP (Highest Limit Wins)
    role_supporter = MagicMock()
    role_supporter.name = "Supporter"
    role_supporter.id = 2

    role_vip = MagicMock()
    role_vip.name = "VIP"
    role_vip.id = 3

    member_vip = MagicMock()
    member_vip.guild = member_basic.guild
    member_vip.guild_permissions.administrator = False
    member_vip.roles = [role_everyone, role_supporter, role_vip]

    limit, role_name = limiter.get_user_role_limit(member_vip, guild_config)
    assert limit == 100000
    assert role_name == "VIP"

    # User with Staff (-1 unlimited)
    role_staff = MagicMock()
    role_staff.name = "Staff"
    role_staff.id = 4
    member_staff = MagicMock()
    member_staff.guild = member_basic.guild
    member_staff.guild_permissions.administrator = False
    member_staff.roles = [role_everyone, role_staff]

    limit, role_name = limiter.get_user_role_limit(member_staff, guild_config)
    assert limit == -1
    assert role_name == "Staff"

    # Server Administrator bypass
    member_admin = MagicMock()
    member_admin.guild = member_basic.guild
    member_admin.guild_permissions.administrator = True
    member_admin.roles = [role_everyone]

    limit, role_name = limiter.get_user_role_limit(member_admin, guild_config)
    assert limit == -1
    assert "Administrator" in role_name


def test_check_prompt_allowed_and_partial_overage(temp_limiter):
    guild_id = 999
    user_id = 888

    role_member = MagicMock()
    role_member.name = "Member"
    role_member.id = 10

    member = MagicMock()
    member.id = user_id
    member.guild = MagicMock()
    member.guild.owner = None
    member.guild_permissions.administrator = False
    member.roles = [role_member]

    # Config with 1000 limit, partial overage enabled (max 500)
    config = {
        "token_quotas": {
            "enabled": True,
            "role_limits": {
                "Member": 1000
            },
            "reset_frequency": "daily",
            "reset_time": "00:00",
            "reset_timezone_offset": 0,
            "overage_policy": {
                "allow_partial_overage": True,
                "max_overage_tokens": 500
            }
        }
    }

    # 1. Fresh user: 0 used, 1000 remaining
    res = temp_limiter.check_prompt_allowed(guild_id, member, config)
    assert res.allowed is True
    assert res.remaining == 1000
    assert res.effective_max_tokens == 1500  # 1000 remaining + 500 max overage

    # 2. Record 900 tokens (100 remaining)
    temp_limiter.record_tokens(guild_id, user_id, 900)
    res = temp_limiter.check_prompt_allowed(guild_id, member, config)
    assert res.allowed is True
    assert res.remaining == 100
    assert res.effective_max_tokens == 600  # 100 remaining + 500 overage

    # 3. Record 200 more tokens (now 1100 total used, exceeded limit of 1000)
    temp_limiter.record_tokens(guild_id, user_id, 200)
    res = temp_limiter.check_prompt_allowed(guild_id, member, config)
    assert res.allowed is False
    assert res.remaining == 0
    assert res.used == 1100
    assert res.reason == "Quota exhausted"


def test_strict_partial_overage_disabled(temp_limiter):
    guild_id = 999
    user_id = 777

    role_member = MagicMock()
    role_member.name = "Member"
    role_member.id = 10

    member = MagicMock()
    member.id = user_id
    member.guild = MagicMock()
    member.guild.owner = None
    member.guild_permissions.administrator = False
    member.roles = [role_member]

    # Partial overage disabled
    config = {
        "token_quotas": {
            "enabled": True,
            "role_limits": {
                "Member": 1000
            },
            "reset_frequency": "daily",
            "overage_policy": {
                "allow_partial_overage": False,
                "max_overage_tokens": 0
            }
        }
    }

    # Record 800 tokens: 200 remaining
    temp_limiter.record_tokens(guild_id, user_id, 800)
    res = temp_limiter.check_prompt_allowed(guild_id, member, config)
    assert res.allowed is True
    assert res.remaining == 200
    assert res.effective_max_tokens == 200  # Clamped strictly to remaining budget
