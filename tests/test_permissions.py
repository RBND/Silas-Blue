import unittest
from unittest.mock import MagicMock
from permissions import PermissionManager
from utils import save_app_config, DEFAULT_APP_CONFIG

class TestPermissions(unittest.TestCase):
    def setUp(self):
        self.pm = PermissionManager()

    def test_dm_allowed(self):
        cfg = dict(DEFAULT_APP_CONFIG)
        cfg["allow_dms"] = True
        save_app_config(cfg)

        msg = MagicMock()
        msg.guild = None
        self.assertTrue(self.pm.can_reply(msg, {}))

    def test_dm_disallowed(self):
        cfg = dict(DEFAULT_APP_CONFIG)
        cfg["allow_dms"] = False
        save_app_config(cfg)

        msg = MagicMock()
        msg.guild = None
        self.assertFalse(self.pm.can_reply(msg, {}))

    def test_guild_everyone_allowed(self):
        msg = MagicMock()
        msg.guild = MagicMock()
        msg.author.roles = []
        msg.author.guild_permissions.administrator = False
        self.assertTrue(self.pm.can_reply(msg, {"reply_roles": ["everyone"]}))

    def test_guild_case_insensitive_roles(self):
        msg = MagicMock()
        msg.guild = MagicMock()
        role = MagicMock()
        role.name = "Moderator"
        msg.author.roles = [role]
        msg.author.guild_permissions.administrator = False

        # Config has lowercase "moderator"
        config = {"reply_roles": ["moderator"]}
        self.assertTrue(self.pm.can_reply(msg, config))

        # Config has uppercase "MODERATOR"
        config = {"reply_roles": ["MODERATOR"]}
        self.assertTrue(self.pm.can_reply(msg, config))

    def test_can_change_model_owner(self):
        guild = MagicMock()
        user = MagicMock()
        guild.owner = user
        self.assertTrue(self.pm.can_change_model(user, guild, {}))

    def test_can_change_model_admin(self):
        guild = MagicMock()
        user = MagicMock()
        guild.owner = MagicMock()
        user.guild_permissions.administrator = True
        self.assertTrue(self.pm.can_change_model(user, guild, {}))

if __name__ == "__main__":
    unittest.main()
