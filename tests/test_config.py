import unittest
import os
from utils import load_app_config, save_app_config, load_config, save_config
import config

class TestConfig(unittest.TestCase):
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

if __name__ == "__main__":
    unittest.main()
