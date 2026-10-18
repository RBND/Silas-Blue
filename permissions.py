"""
Permission management for Silas Blue.
Handles who can use which features per server and in direct messages (DMs).
"""

from utils import load_app_config

class PermissionManager:
    def can_reply(self, message, config) -> bool:
        """
        Determines if the bot should reply to this user/message.
        Handles both guild messages and direct messages (DMs) safely.
        """
        # Handle Direct Messages (DMs)
        if message.guild is None:
            app_cfg = load_app_config()
            return bool(app_cfg.get("allow_dms", True))

        # Handle Guild messages
        allowed_roles = [r.casefold() for r in config.get("reply_roles", ["everyone"])]
        if "everyone" in allowed_roles:
            return True

        if hasattr(message.author, "guild_permissions") and message.author.guild_permissions.administrator:
            return True

        user_roles = [role.name.casefold() for role in getattr(message.author, "roles", [])]
        return any(role in allowed_roles for role in user_roles)

    def can_change_model(self, user, guild, config) -> bool:
        """
        Determines if the user can change the current model for the guild.
        """
        if guild is None:
            return False

        if user == guild.owner:
            return True

        if hasattr(user, "guild_permissions") and user.guild_permissions.administrator:
            return True

        allowed_roles = [r.casefold() for r in config.get("change_model_roles", ["admin", "owner"])]
        user_roles = [role.name.casefold() for role in getattr(user, "roles", [])]
        return any(role in allowed_roles for role in user_roles)

    def can_change_permissions(self, user, guild, config) -> bool:
        """
        Determines if the user can change permission settings.
        """
        if guild is None:
            return False

        if user == guild.owner:
            return True

        if hasattr(user, "guild_permissions") and user.guild_permissions.administrator:
            return True

        allowed_roles = [r.casefold() for r in config.get("change_permission_roles", ["admin", "owner"])]
        user_roles = [role.name.casefold() for role in getattr(user, "roles", [])]
        return any(role in allowed_roles for role in user_roles)

    def can_search(self, user, guild, config) -> bool:
        """
        Determines if the user can use web search features.
        """
        if guild is None:
            return True

        if hasattr(user, "guild_permissions") and user.guild_permissions.administrator:
            return True

        allowed_roles = [r.casefold() for r in config.get("search_roles", ["everyone"])]
        if "everyone" in allowed_roles:
            return True

        user_roles = [role.name.casefold() for role in getattr(user, "roles", [])]
        return any(role in allowed_roles for role in user_roles)