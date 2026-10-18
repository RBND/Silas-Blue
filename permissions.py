"""
Permission management for Silas Blue.
Handles who can use which features per server and in direct messages (DMs).
"""

from utils import load_app_config

class PermissionManager:
    @staticmethod
    def _is_admin(user, guild) -> bool:
        if guild and user == guild.owner:
            return True
        return bool(getattr(user, "guild_permissions", None) and user.guild_permissions.administrator)

    def can_reply(self, message, config) -> bool:
        """
        Determines if the bot should reply to this user/message.
        Handles both guild messages and direct messages (DMs) safely.
        """
        # Handle Direct Messages (DMs)
        if message.guild is None:
            app_cfg = load_app_config()
            return bool(app_cfg.get("allow_dms", True))

        author = message.author
        guild = message.guild

        # 1. Administrator & Owner Bypass
        if self._is_admin(author, guild):
            return True

        # 2. Explicit User Whitelist (O(1) Set Lookup)
        allowed_users = {str(uid).strip() for uid in config.get("reply_users", [])}
        if str(author.id) in allowed_users:
            return True

        # 3. Role Whitelist
        allowed_roles = {str(r).casefold().strip() for r in config.get("reply_roles", ["everyone"])}
        if "everyone" in allowed_roles or "@everyone" in allowed_roles:
            return True

        user_role_names = {role.name.casefold() for role in getattr(author, "roles", [])}
        user_role_ids = {str(role.id) for role in getattr(author, "roles", [])}
        return bool((allowed_roles & user_role_names) or (allowed_roles & user_role_ids))

    def can_change_model(self, user, guild, config) -> bool:
        """
        Determines if the user can change the current model for the guild.
        """
        if guild is None:
            return False

        if self._is_admin(user, guild):
            return True

        allowed_users = {str(uid).strip() for uid in config.get("change_model_users", [])}
        if str(user.id) in allowed_users:
            return True

        allowed_roles = {str(r).casefold().strip() for r in config.get("change_model_roles", [])}
        if "everyone" in allowed_roles or "@everyone" in allowed_roles:
            return True

        user_role_names = {role.name.casefold() for role in getattr(user, "roles", [])}
        user_role_ids = {str(role.id) for role in getattr(user, "roles", [])}
        return bool((allowed_roles & user_role_names) or (allowed_roles & user_role_ids))

    def can_change_permissions(self, user, guild, config) -> bool:
        """
        Determines if the user can change permission settings.
        """
        if guild is None:
            return False

        if self._is_admin(user, guild):
            return True

        allowed_users = {str(uid).strip() for uid in config.get("change_permission_users", [])}
        if str(user.id) in allowed_users:
            return True

        allowed_roles = {str(r).casefold().strip() for r in config.get("change_permission_roles", [])}
        if "everyone" in allowed_roles or "@everyone" in allowed_roles:
            return True

        user_role_names = {role.name.casefold() for role in getattr(user, "roles", [])}
        user_role_ids = {str(role.id) for role in getattr(user, "roles", [])}
        return bool((allowed_roles & user_role_names) or (allowed_roles & user_role_ids))

    def can_search(self, user, guild, config) -> bool:
        """
        Determines if the user can use web search features.
        """
        if guild is None:
            return True

        if self._is_admin(user, guild):
            return True

        allowed_users = {str(uid).strip() for uid in config.get("search_users", [])}
        if str(user.id) in allowed_users:
            return True

        allowed_roles = {str(r).casefold().strip() for r in config.get("search_roles", ["everyone"])}
        if "everyone" in allowed_roles or "@everyone" in allowed_roles:
            return True

        user_role_names = {role.name.casefold() for role in getattr(user, "roles", [])}
        user_role_ids = {str(role.id) for role in getattr(user, "roles", [])}
        return bool((allowed_roles & user_role_names) or (allowed_roles & user_role_ids))