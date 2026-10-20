import unittest
import os
from utils import load_app_config, save_app_config, load_config, save_config
import config

class TestConfig(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from utils import load_app_config
        cls._original_app_cfg = load_app_config()

    @classmethod
    def tearDownClass(cls):
        from utils import save_app_config
        if hasattr(cls, "_original_app_cfg"):
            save_app_config(cls._original_app_cfg)

    def test_app_config_load_and_save(self):
        cfg = load_app_config()
        self.assertIn("backend_type", cfg)
        self.assertIn("ollama_url", cfg)

        cfg["test_key"] = "test_value"
        save_app_config(cfg)

        reloaded = load_app_config()
        self.assertEqual(reloaded.get("test_key"), "test_value")

    def test_config_debug_proxy(self):
        config.DEBUG = True
        self.assertTrue(config.DEBUG)

        cfg = load_app_config()
        self.assertTrue(cfg.get("debug"))

        config.DEBUG = False
        self.assertFalse(config.DEBUG)

    def test_guild_config_clamp_max_chars(self):
        # Even if saved with 3000 chars, load_config must clamp to 1900
        save_config("test_guild_999", {"pagination_max_chars": 3000})
        loaded = load_config("test_guild_999")
        self.assertLessEqual(loaded.get("pagination_max_chars"), 1900)

        # Cleanup
        try:
            os.remove(os.path.join("config", "test_guild_999.json"))
        except Exception:
            pass

    def test_guild_config_server_system_prompt(self):
        guild_id = "test_guild_prompt_123"
        custom_prompt = "You are a specialized coding bot for this server."

        # Default should have empty string
        default_cfg = load_config(guild_id)
        self.assertIn("system_prompt", default_cfg)
        self.assertEqual(default_cfg["system_prompt"], "")

        # Save custom prompt
        default_cfg["system_prompt"] = custom_prompt
        save_config(guild_id, default_cfg)

        # Reload and verify
        reloaded = load_config(guild_id)
        self.assertEqual(reloaded.get("system_prompt"), custom_prompt)

        # Cleanup
        try:
            os.remove(os.path.join("config", f"{guild_id}.json"))
        except Exception:
            pass

    def test_validate_and_sanitize_guild_config(self):
        from utils import validate_and_sanitize_guild_config

        raw = {
            "reply_users": ["295767175426473984", "not_a_snowflake", "123", "295767175426473984"],
            "pagination_max_chars": 5000,
            "random_prompt_probability": 250,
            "memory_turns": -5,
            "memory_ttl_minutes": 99999
        }
        sanitized = validate_and_sanitize_guild_config(raw)

        # Only the valid 18-digit snowflake should remain, deduplicated
        self.assertEqual(sanitized["reply_users"], ["295767175426473984"])
        # Boundaries clamped
        self.assertEqual(sanitized["pagination_max_chars"], 1900)
        self.assertEqual(sanitized["random_prompt_probability"], 100)
        self.assertEqual(sanitized["memory_turns"], 0)
        self.assertEqual(sanitized["memory_ttl_minutes"], 1440)

    def test_forum_auto_reply_config_sanitization(self):
        from utils import validate_and_sanitize_guild_config

        raw = {
            "forum_auto_reply_enabled": True,
            "forum_include_post_title": False,
            "forum_system_prompt": "You are a specialized support helper for forum posts.",
            "forum_monitored_channels": ["123456789012345678", "bad_id", " 123456789012345678 "],
            "forum_auto_reply_tags": ["help", "question", "  urgent  "],
            "forum_ignore_tags": ["resolved", "closed", ""]
        }
        sanitized = validate_and_sanitize_guild_config(raw)

        self.assertTrue(sanitized["forum_auto_reply_enabled"])
        self.assertFalse(sanitized["forum_include_post_title"])
        self.assertEqual(sanitized["forum_system_prompt"], "You are a specialized support helper for forum posts.")
        # Monitored channels: valid snowflake deduplicated
        self.assertEqual(sanitized["forum_monitored_channels"], ["123456789012345678"])
        # Tags trimmed and empty strings dropped
        self.assertEqual(sanitized["forum_auto_reply_tags"], ["help", "question", "urgent"])
        self.assertEqual(sanitized["forum_ignore_tags"], ["resolved", "closed"])

if __name__ == "__main__":
    unittest.main()
