import unittest
from unittest.mock import patch
import os
import json
import shutil
import tempfile

from updater import (
    parse_semver,
    parse_version_from_text,
    get_current_version,
    get_current_commit,
    is_docker,
    is_git_repo,
    BotUpdater,
    PENDING_UPDATE_FILE,
    BACKUP_DIR
)
from utils import load_app_config, save_app_config

class TestUpdater(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._orig_cfg = load_app_config()

    @classmethod
    def tearDownClass(cls):
        save_app_config(cls._orig_cfg)

    def test_semver_parsing(self):
        self.assertEqual(parse_semver("3.3.4"), (3, 3, 4))
        self.assertEqual(parse_semver("v3.3.4"), (3, 3, 4))
        self.assertEqual(parse_semver("V3.4.0"), (3, 4, 0))
        self.assertEqual(parse_semver("2.1.0-alpha"), (2, 1, 0))
        self.assertTrue(parse_semver("3.3.5") > parse_semver("3.3.4"))
        self.assertTrue(parse_semver("4.0.0") > parse_semver("3.9.9"))
        self.assertFalse(parse_semver("3.3.4") > parse_semver("3.3.4"))

    def test_git_repo_and_version(self):
        curr_ver = get_current_version()
        self.assertRegex(curr_ver, r"^\d+\.\d+\.\d+")
        if is_git_repo():
            sha = get_current_commit()
            self.assertNotEqual(sha, "unknown")
            self.assertGreaterEqual(len(sha), 7)

    def test_backup_creation_and_rotation(self):
        backup_path = BotUpdater.create_backup()
        self.assertTrue(os.path.isdir(backup_path))
        
        # Verify backup contains config
        cfg_backup = os.path.join(backup_path, "config")
        self.assertTrue(os.path.isdir(cfg_backup))

        # Cleanup created backup for test cleanliness
        shutil.rmtree(backup_path, ignore_errors=True)

    def test_check_for_updates_latest_channel(self):
        status = BotUpdater.check_for_updates("latest")
        self.assertIn("current_version", status)
        self.assertIn("current_commit", status)
        self.assertIn("update_available", status)
        self.assertIn("channel", status)
        self.assertEqual(status["channel"], "latest")

    def test_check_for_updates_stable_channel(self):
        status = BotUpdater.check_for_updates("stable")
        self.assertIn("current_version", status)
        self.assertIn("latest_version", status)
        self.assertIn("update_available", status)
        self.assertEqual(status["channel"], "stable")

    def test_health_check_without_pending_update(self):
        # Ensure no pending update file exists
        if os.path.exists(PENDING_UPDATE_FILE):
            os.remove(PENDING_UPDATE_FILE)
        res = BotUpdater.verify_boot_health()
        self.assertIsNone(res)

    def test_health_check_with_pending_update(self):
        os.makedirs(os.path.dirname(PENDING_UPDATE_FILE), exist_ok=True)
        test_payload = {
            "previous_commit": "abc1234",
            "previous_version": "3.3.3",
            "target_commit": get_current_commit(),
            "target_version": get_current_version(),
            "channel": "stable",
            "notify_context": {"channel_id": "123456789"}
        }
        with open(PENDING_UPDATE_FILE, "w", encoding="utf-8") as f:
            json.dump(test_payload, f)

        res = BotUpdater.verify_boot_health()
        self.assertIsNotNone(res)
        self.assertEqual(res["previous_version"], "3.3.3")
        self.assertEqual(res["notify_context"]["channel_id"], "123456789")
        # Verify pending update file was cleaned up on success
        self.assertFalse(os.path.exists(PENDING_UPDATE_FILE))

    def test_default_config_updater_keys(self):
        cfg = load_app_config()
        self.assertIn("update_channel", cfg)
        self.assertIn("auto_update_enabled", cfg)
        self.assertIn("auto_update_mode", cfg)
        self.assertIn("auto_update_interval_hours", cfg)

    def test_parse_version_from_text(self):
        self.assertEqual(parse_version_from_text('__version__ = "3.5.11"\n'), "3.5.11")
        self.assertEqual(parse_version_from_text("__version__ = 'v3.6.0'"), "3.6.0")
        self.assertIsNone(parse_version_from_text("version = '1.0.0'"))

    def test_latest_channel_reports_new_remote_version(self):
        with patch("updater.get_current_version", return_value="3.5.9"), \
             patch("updater.get_current_commit", return_value="aaaaaaa"), \
             patch.object(BotUpdater, "_fetch_remote_main", return_value=("bbbbbbb", 2, ["bbbbbbb Bump to 3.5.11"])), \
             patch.object(BotUpdater, "get_remote_version", return_value="3.5.11"):
            status = BotUpdater.check_for_updates("latest")
            self.assertTrue(status["update_available"])
            self.assertEqual(status["current_version"], "3.5.9")
            self.assertEqual(status["latest_version"], "3.5.11")
            self.assertEqual(status["latest_commit"], "bbbbbbb")
            self.assertEqual(status["commits_behind"], 2)

    def test_stable_channel_reports_new_remote_version_from_main(self):
        with patch("updater.get_current_version", return_value="3.5.9"), \
             patch("updater.get_current_commit", return_value="aaaaaaa"), \
             patch.object(BotUpdater, "query_github_api", return_value=[]), \
             patch.object(BotUpdater, "_fetch_remote_main", return_value=("bbbbbbb", 1, ["bbbbbbb v3.5.11"])), \
             patch.object(BotUpdater, "get_remote_version", return_value="3.5.11"):
            status = BotUpdater.check_for_updates("stable")
            self.assertTrue(status["update_available"])
            self.assertEqual(status["current_version"], "3.5.9")
            self.assertEqual(status["latest_version"], "3.5.11")
            self.assertEqual(status["latest_commit"], "bbbbbbb")

if __name__ == "__main__":
    unittest.main()
