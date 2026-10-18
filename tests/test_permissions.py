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
        role.id = 111222333444555666
        msg.author.roles = [role]
        msg.author.guild_permissions.administrator = False

        # Config has lowercase "moderator"
        config = {"reply_roles": ["moderator"]}
        self.assertTrue(self.pm.can_reply(msg, config))

        # Config has uppercase "MODERATOR"
        config = {"reply_roles": ["MODERATOR"]}
        self.assertTrue(self.pm.can_reply(msg, config))

        # Config has role Snowflake ID string
        config = {"reply_roles": ["111222333444555666"]}
        self.assertTrue(self.pm.can_reply(msg, config))

    def test_user_snowflake_authorization(self):
        msg = MagicMock()
        msg.guild = MagicMock()
        msg.author.id = 295767175426473984
        msg.author.roles = []
        msg.author.guild_permissions.administrator = False
        msg.guild.owner = MagicMock()

        # Reply users whitelist
        config = {"reply_roles": [], "reply_users": ["295767175426473984"]}
        self.assertTrue(self.pm.can_reply(msg, config))

        # Unauthorized user
        msg.author.id = 999999999999999999
        self.assertFalse(self.pm.can_reply(msg, config))

    def test_user_privilege_separation(self):
        guild = MagicMock()
        user = MagicMock()
        user.id = 295767175426473984
        user.roles = []
        user.guild_permissions.administrator = False
        guild.owner = MagicMock()

        # User is only granted reply access, NOT model or permission access
        config = {
            "reply_users": ["295767175426473984"],
            "change_model_users": [],
            "change_permission_users": [],
            "change_model_roles": [],
            "change_permission_roles": []
        }
        self.assertFalse(self.pm.can_change_model(user, guild, config))
        self.assertFalse(self.pm.can_change_permissions(user, guild, config))

        # When explicitly granted model switching
        config["change_model_users"] = ["295767175426473984"]
        self.assertTrue(self.pm.can_change_model(user, guild, config))
        self.assertFalse(self.pm.can_change_permissions(user, guild, config))

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
