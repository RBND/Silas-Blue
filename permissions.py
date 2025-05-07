"""
Permission management for Silas Blue.
Handles who can use which features per server.
"""

class PermissionManager:
    def can_reply(self, message, config):
        """
        Determines if the bot should reply to this user/message.
        """
        # By default, reply to everyone
        allowed_roles = config.get("reply_roles", ["everyone"])
        if "everyone" in allowed_roles:
            return True
        if message.author.guild_permissions.administrator:
            return True
        user_roles = [role.name for role in message.author.roles]
        return any(role in allowed_roles for role in user_roles)

    def can_change_model(self, user, guild, config):
        """
        Determines if the user can change the current model.
        """
        allowed_roles = config.get("change_model_roles", ["admin", "owner"])
        if user.guild_permissions.administrator or user == guild.owner:
            return True
        user_roles = [role.name.lower() for role in user.roles]
        return any(role in allowed_roles for role in user_roles)

    def can_change_permissions(self, user, guild, config):
        """
        Determines if the user can change permission settings.
        """
        allowed_roles = config.get("change_permission_roles", ["admin", "owner"])
        if user.guild_permissions.administrator or user == guild.owner:
            return True
        user_roles = [role.name.lower() for role in user.roles]
        return any(role in allowed_roles for role in user_roles) 