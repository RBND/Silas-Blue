import asyncio
import datetime
import gc
import pathlib
import shlex

from config import bot, config, ServerConfig, force_close_sessions, change_discord_token, TOKEN_FILE, should_restart, should_shutdown, session
from theme import RetroColors


def handle_terminal_servers_command():
    """Handle the 'servers' terminal command to list all servers"""
    if not bot.guilds:
        print(f"{RetroColors.WARNING}Bot is not connected to any servers.")
        return

    print(f"\n{RetroColors.TITLE}Servers the bot is connected to:")
    print(f"{RetroColors.MAGENTA}{'-' * 50}")
    for i, guild in enumerate(bot.guilds, 1):
        print(f"{RetroColors.CYAN}{i}. {RetroColors.BLUE}{guild.name} {RetroColors.PURPLE}(ID: {guild.id})")
        # Check if we have a config for this server
        if str(guild.id) in config.servers:
            print(f"{RetroColors.INFO}   Config: {RetroColors.SUCCESS}Yes")
        else:
            print(f"{RetroColors.INFO}   Config: {RetroColors.WARNING}No (will be created on first interaction)")
    print(f"{RetroColors.MAGENTA}{'-' * 50}")


def handle_terminal_server_command(args):
    """Handle the 'server' terminal command to view a specific server's configuration"""
    if not args:
        print(f"{RetroColors.ERROR}Error: Server ID is required. Usage: server <server_id>")
        return

    server_id = args[0]

    # Check if the server ID is valid
    guild = None
    for g in bot.guilds:
        if str(g.id) == server_id:
            guild = g
            break

    if not guild:
        print(f"{RetroColors.ERROR}Error: Server with ID {server_id} not found or bot is not connected to it.")
        return

    # Get the server config
    if server_id not in config.servers:
        print(f"{RetroColors.WARNING}No configuration exists for server {guild.name} (ID: {server_id}).")
        print(f"{RetroColors.INFO}A default configuration will be created on first interaction.")
        return

    server_config = config.servers[server_id]

    print(
        f"\n{RetroColors.TITLE}Configuration for server: {RetroColors.BLUE}{guild.name} {RetroColors.PURPLE}(ID: {server_id})")
    print(f"{RetroColors.MAGENTA}{'-' * 50}")

    # Display allowed models
    print(
        f"{RetroColors.CYAN}Allowed Models: {RetroColors.BLUE}{'All' if not server_config.allowed_models else ', '.join(server_config.allowed_models)}")

    # Display permissions
    print(f"\n{RetroColors.TITLE}Permissions:")
    for perm_type, role_ids in server_config.permissions.items():
        role_names = []
        for role_id in role_ids:
            role = guild.get_role(role_id)
            if role:
                role_names.append(f"{role.name} (ID: {role.id})")

        if role_names:
            print(f"{RetroColors.CYAN}  {perm_type}: {RetroColors.BLUE}{', '.join(role_names)}")
        else:
            if perm_type in ["set_model", "manage_config"]:
                print(
                    f"{RetroColors.CYAN}  {perm_type}: {RetroColors.PURPLE}No roles assigned (only server owner and administrators)")
            elif perm_type == "reply_to":
                print(f"{RetroColors.CYAN}  {perm_type}: {RetroColors.PURPLE}No roles assigned (replies to everyone)")
            else:
                print(f"{RetroColors.CYAN}  {perm_type}: {RetroColors.PURPLE}No roles assigned")

    # Display bot nickname
    print(
        f"\n{RetroColors.CYAN}Bot Nickname: {RetroColors.BLUE}{server_config.bot_nickname if server_config.bot_nickname else 'Default'}")

    # Display random replies settings
    print(f"\n{RetroColors.TITLE}Random Replies:")
    print(
        f"{RetroColors.CYAN}  Enabled: {RetroColors.BLUE if server_config.random_replies['enabled'] else RetroColors.PURPLE}{server_config.random_replies['enabled']}")
    print(f"{RetroColors.CYAN}  Probability: {RetroColors.BLUE}{server_config.random_replies['probability'] * 100}%")
    print(f"{RetroColors.CYAN}  Cooldown: {RetroColors.BLUE}{server_config.random_replies['cooldown']} seconds")
    print(
        f"{RetroColors.CYAN}  Last Reply: {RetroColors.BLUE}{datetime.datetime.fromtimestamp(server_config.last_random_reply).strftime('%Y-%m-%d %H:%M:%S') if server_config.last_random_reply > 0 else 'Never'}")

    # Display system instructions
    print(f"\n{RetroColors.TITLE}System Instructions:")
    if server_config.system_instructions:
        print(f"{RetroColors.BLUE}  {server_config.system_instructions}")
    else:
        print(f"{RetroColors.PURPLE}  None")

    # Display paginated responses settings
    print(f"\n{RetroColors.TITLE}Paginated Responses:")
    print(
        f"{RetroColors.CYAN}  Enabled: {RetroColors.BLUE if server_config.paginated_responses['enabled'] else RetroColors.PURPLE}{server_config.paginated_responses['enabled']}")
    print(
        f"{RetroColors.CYAN}  Page Size: {RetroColors.BLUE}{server_config.paginated_responses['page_size']} characters")

    print(f"{RetroColors.MAGENTA}{'-' * 50}")


def handle_terminal_edit_command(args):
    """Handle the 'edit' terminal command to edit a server's configuration"""
    if len(args) < 3:
        print(f"{RetroColors.ERROR}Error: Insufficient arguments. Usage: edit <server_id> <setting> <value>")
        print(
            f"{RetroColors.INFO}Available settings: allowed_models, bot_nickname, random_replies, system_instructions, paginated_responses")
        return

    server_id = args[0]
    setting = args[1]
    value = ' '.join(args[2:])

    # Check if the server ID is valid
    guild = None
    for g in bot.guilds:
        if str(g.id) == server_id:
            guild = g
            break

    if not guild:
        print(f"{RetroColors.ERROR}Error: Server with ID {server_id} not found or bot is not connected to it.")
        return

    # Get or create the server config
    server_config = config.get_server_config(guild.id)

    # Edit the specified setting
    if setting == "allowed_models":
        if value.lower() == "all":
            server_config.allowed_models = []
            print(f"{RetroColors.SUCCESS}Set allowed models to: All models")
        else:
            models = [model.strip() for model in value.split(',')]
            server_config.allowed_models = models
            print(f"{RetroColors.SUCCESS}Set allowed models to: {RetroColors.BLUE}{', '.join(models)}")

    elif setting == "bot_nickname":
        if value.lower() == "default" or value.lower() == "none":
            server_config.bot_nickname = None
            print(f"{RetroColors.SUCCESS}Reset bot nickname to default")
            # Update the bot's nickname in the guild
            asyncio.run_coroutine_threadsafe(guild.me.edit(nick=None), bot.loop)
        else:
            server_config.bot_nickname = value
            print(f"{RetroColors.SUCCESS}Set bot nickname to: {RetroColors.BLUE}{value}")
            # Update the bot's nickname in the guild
            asyncio.run_coroutine_threadsafe(guild.me.edit(nick=value), bot.loop)

    elif setting == "random_replies":
        # Parse the value as a JSON object
        try:
            if value.lower() in ["enable", "enabled", "true", "yes", "1"]:
                server_config.random_replies["enabled"] = True
                print(f"{RetroColors.SUCCESS}Enabled random replies")
            elif value.lower() in ["disable", "disabled", "false", "no", "0"]:
                server_config.random_replies["enabled"] = False
                print(f"{RetroColors.SUCCESS}Disabled random replies")
            elif value.lower().startswith("probability="):
                prob_str = value.split('=')[1].strip()
                prob = float(prob_str)
                if 0 <= prob <= 1:
                    server_config.random_replies["probability"] = prob
                    print(f"{RetroColors.SUCCESS}Set random reply probability to: {RetroColors.BLUE}{prob * 100}%")
                else:
                    print(f"{RetroColors.ERROR}Error: Probability must be between 0 and 1")
                    return
            elif value.lower().startswith("cooldown="):
                cooldown_str = value.split('=')[1].strip()
                cooldown = int(cooldown_str)
                if cooldown >= 0:
                    server_config.random_replies["cooldown"] = cooldown
                    print(f"{RetroColors.SUCCESS}Set random reply cooldown to: {RetroColors.BLUE}{cooldown} seconds")
                else:
                    print(f"{RetroColors.ERROR}Error: Cooldown must be a non-negative integer")
                    return
            else:
                print(
                    f"{RetroColors.ERROR}Error: Invalid value for random_replies. Use 'enable', 'disable', 'probability=X', or 'cooldown=X'")
                return
        except Exception as e:
            print(f"{RetroColors.ERROR}Error parsing random_replies value: {e}")
            return

    elif setting == "system_instructions":
        server_config.system_instructions = value
        print(f"{RetroColors.SUCCESS}Set system instructions to: {RetroColors.BLUE}{value}")

    elif setting == "paginated_responses":
        if value.lower() in ["enable", "enabled", "true", "yes", "1"]:
            server_config.paginated_responses["enabled"] = True
            print(f"{RetroColors.SUCCESS}Enabled paginated responses")
        elif value.lower() in ["disable", "disabled", "false", "no", "0"]:
            server_config.paginated_responses["enabled"] = False
            print(f"{RetroColors.SUCCESS}Disabled paginated responses")
        elif value.lower().startswith("pagesize="):
            size_str = value.split('=')[1].strip()
            size = int(size_str)
            if 100 <= size <= 2000:
                server_config.paginated_responses["page_size"] = size
                print(f"{RetroColors.SUCCESS}Set paginated response page size to: {RetroColors.BLUE}{size} characters")
            else:
                print(f"{RetroColors.ERROR}Error: Page size must be between 100 and 2000")
                return
        else:
            print(
                f"{RetroColors.ERROR}Error: Invalid value for paginated_responses. Use 'enable', 'disable', or 'pagesize=X'")
            return

    else:
        print(f"{RetroColors.ERROR}Error: Unknown setting '{setting}'")
        print(
            f"{RetroColors.INFO}Available settings: allowed_models, bot_nickname, random_replies, system_instructions, paginated_responses")
        return

    # Save the updated configuration
    config.save()
    print(
        f"{RetroColors.SUCCESS}Configuration for server '{RetroColors.BLUE}{guild.name}{RetroColors.SUCCESS}' (ID: {server_id}) has been updated and saved.")


def handle_terminal_delete_command(args):
    """Handle the 'delete' terminal command to delete a server's configuration"""
    if not args:
        print(f"{RetroColors.ERROR}Error: Server ID is required. Usage: delete <server_id>")
        return

    server_id = args[0]

    # Check if the server ID is valid
    guild = None
    for g in bot.guilds:
        if str(g.id) == server_id:
            guild = g
            break

    if not guild and server_id != "default":
        print(f"{RetroColors.WARNING}Warning: Server with ID {server_id} not found or bot is not connected to it.")
        confirmation = input(f"{RetroColors.PROMPT}Do you still want to delete this configuration? (y/n): ")
        if confirmation.lower() != 'y':
            print(f"{RetroColors.INFO}Operation cancelled.")
            return

    # Check if the configuration exists
    if server_id not in config.servers:
        print(f"{RetroColors.ERROR}Error: No configuration exists for server ID {server_id}.")
        return

    # Ask for confirmation
    server_name = guild.name if guild else f"ID {server_id}"
    confirmation = input(
        f"{RetroColors.PROMPT}Are you sure you want to delete the configuration for server '{server_name}'? (y/n): ")
    if confirmation.lower() != 'y':
        print(f"{RetroColors.INFO}Operation cancelled.")
        return

    # Delete the configuration
    del config.servers[server_id]
    config.save()
    print(f"{RetroColors.SUCCESS}Configuration for server '{server_name}' has been deleted.")

    if server_id == "default":
        print(
            f"{RetroColors.INFO}Note: The default configuration has been deleted. New servers will now get a fresh default configuration.")


def handle_terminal_reset_command(args):
    """Handle the 'reset' terminal command to reset a server's configuration to default"""
    if not args:
        print(f"{RetroColors.ERROR}Error: Server ID is required. Usage: reset <server_id>")
        return

    server_id = args[0]

    # Check if the server ID is valid
    guild = None
    for g in bot.guilds:
        if str(g.id) == server_id:
            guild = g
            break

    if not guild:
        print(f"{RetroColors.ERROR}Error: Server with ID {server_id} not found or bot is not connected to it.")
        return

    # Ask for confirmation
    confirmation = input(
        f"{RetroColors.PROMPT}Are you sure you want to reset the configuration for server '{guild.name}'? (y/n): ")
    if confirmation.lower() != 'y':
        print(f"{RetroColors.INFO}Operation cancelled.")
        return

    # Reset the configuration
    config.servers[server_id] = ServerConfig()
    config.save()
    print(f"{RetroColors.SUCCESS}Configuration for server '{guild.name}' has been reset to default.")


def handle_terminal_hardrestart_command():
    """Handle the 'hardrestart' terminal command to completely restart the bot script"""
    print(f"{RetroColors.INFO}Preparing for hard restart...")
    confirmation = input(f"{RetroColors.PROMPT}Are you sure you want to completely restart the bot script? (y/n): ")
    if confirmation.lower() != 'y':
        print(f"{RetroColors.INFO}Hard restart cancelled.")
        return

    print(f"{RetroColors.INFO}Closing all resources...")

    # Force close all sessions
    force_close_sessions()

    # Force garbage collection
    gc.collect()

    print(f"{RetroColors.SUCCESS}Restarting script...")

    # Restart the script
    import os
    import sys
    os.execv(sys.executable, ['python'] + sys.argv)


def handle_terminal_permissions_command(args):
    """Handle the 'permissions' terminal command to manage server permissions"""
    if len(args) < 3:
        print(f"{RetroColors.ERROR}Error: Insufficient arguments.")
        print(f"{RetroColors.INFO}Usage: permissions <server_id> <action> <permission_type> [role_id1,role_id2,...]")
        print(f"{RetroColors.INFO}Actions: view, add, remove, reset")
        print(f"{RetroColors.INFO}Permission types: set_model, manage_config, reply_to")
        return

    server_id = args[0]
    action = args[1].lower()
    permission_type = args[2].lower()

    # Check if the server ID is valid
    guild = None
    for g in bot.guilds:
        if str(g.id) == server_id:
            guild = g
            break

    if not guild:
        print(f"{RetroColors.ERROR}Error: Server with ID {server_id} not found or bot is not connected to it.")
        return

    # Get or create the server config
    server_config = config.get_server_config(guild.id)

    # Check if the permission type is valid
    if permission_type not in ["set_model", "manage_config", "reply_to"]:
        print(f"{RetroColors.ERROR}Error: Invalid permission type '{permission_type}'")
        print(f"{RetroColors.INFO}Valid permission types: set_model, manage_config, reply_to")
        return

    # Handle the action
    if action == "view":
        role_ids = server_config.permissions[permission_type]
        if not role_ids:
            if permission_type in ["set_model", "manage_config"]:
                print(
                    f"{RetroColors.INFO}Permission '{permission_type}': No roles assigned (only server owner and administrators)")
            elif permission_type == "reply_to":
                print(f"{RetroColors.INFO}Permission '{permission_type}': No roles assigned (replies to everyone)")
            else:
                print(f"{RetroColors.INFO}Permission '{permission_type}': No roles assigned")
        else:
            role_details = []
            for role_id in role_ids:
                role = guild.get_role(role_id)
                if role:
                    role_details.append(f"{role.name} (ID: {role.id})")
                else:
                    role_details.append(f"Unknown role (ID: {role_id})")
            print(
                f"{RetroColors.INFO}Permission '{permission_type}' roles: {RetroColors.BLUE}{', '.join(role_details)}")

    elif action == "add":
        if len(args) < 4:
            print(f"{RetroColors.ERROR}Error: Role IDs are required for the 'add' action.")
            print(f"{RetroColors.INFO}Usage: permissions <server_id> add <permission_type> <role_id1,role_id2,...>")
            return

        role_ids_str = args[3]
        role_ids = [int(role_id.strip()) for role_id in role_ids_str.split(',')]

        added_roles = []
        for role_id in role_ids:
            role = guild.get_role(role_id)
            if role:
                if role_id not in server_config.permissions[permission_type]:
                    server_config.permissions[permission_type].append(role_id)
                    added_roles.append(f"{role.name} (ID: {role.id})")
                else:
                    print(
                        f"{RetroColors.WARNING}Role '{role.name}' (ID: {role.id}) already has '{permission_type}' permission.")
            else:
                print(f"{RetroColors.WARNING}Warning: Role with ID {role_id} not found in server '{guild.name}'")

        if added_roles:
            config.save()
            print(
                f"{RetroColors.SUCCESS}Added roles to '{permission_type}' permission: {RetroColors.BLUE}{', '.join(added_roles)}")
        else:
            print(f"{RetroColors.INFO}No roles were added.")

    elif action == "remove":
        if len(args) < 4:
            print(f"{RetroColors.ERROR}Error: Role IDs are required for the 'remove' action.")
            print(f"{RetroColors.INFO}Usage: permissions <server_id> remove <permission_type> <role_id1,role_id2,...>")
            return

        role_ids_str = args[3]
        role_ids = [int(role_id.strip()) for role_id in role_ids_str.split(',')]

        removed_roles = []
        for role_id in role_ids:
            role = guild.get_role(role_id)
            role_name = f"{role.name} (ID: {role.id})" if role else f"Unknown role (ID: {role_id})"

            if role_id in server_config.permissions[permission_type]:
                server_config.permissions[permission_type].remove(role_id)
                removed_roles.append(role_name)
            else:
                print(f"{RetroColors.WARNING}Role {role_name} does not have '{permission_type}' permission.")

        if removed_roles:
            config.save()
            print(
                f"{RetroColors.SUCCESS}Removed roles from '{permission_type}' permission: {RetroColors.BLUE}{', '.join(removed_roles)}")
        else:
            print(f"{RetroColors.INFO}No roles were removed.")

    elif action == "reset":
        # Ask for confirmation
        confirmation = input(
            f"{RetroColors.PROMPT}Are you sure you want to reset the '{permission_type}' permission for server '{guild.name}'? (y/n): ")
        if confirmation.lower() != 'y':
            print(f"{RetroColors.INFO}Operation cancelled.")
            return

        server_config.permissions[permission_type] = []
        config.save()

        if permission_type in ["set_model", "manage_config"]:
            print(
                f"{RetroColors.SUCCESS}Reset '{permission_type}' permission to default (only server owner and administrators)")
        elif permission_type == "reply_to":
            print(f"{RetroColors.SUCCESS}Reset '{permission_type}' permission to default (replies to everyone)")
        else:
            print(f"{RetroColors.SUCCESS}Reset '{permission_type}' permission to default")

    else:
        print(f"{RetroColors.ERROR}Error: Invalid action '{action}'")
        print(f"{RetroColors.INFO}Valid actions: view, add, remove, reset")


def handle_terminal_token_command(args):
    """Handle the 'token' terminal command to change the Discord token"""
    if not args:
        # No arguments, prompt for new token
        change_discord_token()
        return

    if args[0].lower() == "show":
        # Show the current token (masked)
        token_path = pathlib.Path(TOKEN_FILE)
        if token_path.exists():
            with open(token_path, 'r') as f:
                token = f.read().strip()
                if token:
                    # Mask the token for security
                    masked_token = token[:4] + '*' * (len(token) - 8) + token[-4:]
                    print(f"{RetroColors.INFO}Current token: {RetroColors.BLUE}{masked_token}")
                else:
                    print(f"{RetroColors.WARNING}Token file exists but is empty.")
        else:
            print(f"{RetroColors.WARNING}No token file found.")
    else:
        # Use the provided token
        new_token = args[0]
        change_discord_token(new_token)


def handle_terminal_restart_command():
    """Handle the 'restart' terminal command to restart the bot"""
    global should_restart

    print(f"{RetroColors.INFO}Preparing to restart the bot...")
    confirmation = input(f"{RetroColors.PROMPT}Are you sure you want to restart the bot? (y/n): ")
    if confirmation.lower() != 'y':
        print(f"{RetroColors.INFO}Restart cancelled.")
        return

    # Set the restart flag
    should_restart = True

    # Close the bot connection - this will trigger on_close event
    print(f"{RetroColors.INFO}Closing bot connection...")

    # Force close any existing sessions
    global session
    if session and not session.closed:
        try:
            # Create a new event loop for closing the session
            import asyncio
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
            loop.run_until_complete(session.close())
            loop.close()
            print(f"{RetroColors.SUCCESS}Closed custom aiohttp session")
        except Exception as e:
            print(f"{RetroColors.WARNING}Error closing session: {e}")

    session = None

    # Close the bot
    asyncio.run_coroutine_threadsafe(bot.close(), bot.loop)
    print(f"{RetroColors.SUCCESS}Bot is restarting...")


def handle_terminal_shutdown_command():
    """Handle the 'shutdown' terminal command to shut down the bot"""
    global should_shutdown

    print(f"{RetroColors.INFO}Preparing to shutdown the bot...")
    confirmation = input(f"{RetroColors.PROMPT}Are you sure you want to shutdown the bot? (y/n): ")
    if confirmation.lower() != 'y':
        print(f"{RetroColors.INFO}Shutdown cancelled.")
        return

    # Set the shutdown flag
    should_shutdown = True

    # Close the bot connection
    asyncio.run_coroutine_threadsafe(bot.close(), bot.loop)
    print(f"{RetroColors.SUCCESS}Bot is shutting down...")


def terminal_input_handler():
    """Handle terminal input while the bot is running"""
    print(f"{RetroColors.TITLE}Terminal commands are now active. Type 'help' to see available commands.")
    while True:
        try:
            command_line = input(f"{RetroColors.PROMPT}Silas Blue: ").strip()
            if not command_line:
                continue

            # Parse the command line with proper handling of quoted arguments
            try:
                args = shlex.split(command_line)
            except ValueError as e:
                print(f"{RetroColors.ERROR}Error parsing command: {e}")
                continue

            command = args[0].lower()
            args = args[1:] if len(args) > 1 else []

            if command == "help":
                print(f"\n{RetroColors.HEADER}Available terminal commands:")
                print(
                    f"{RetroColors.COMMAND}  help                                {RetroColors.INFO}- Display this help message")
                print(
                    f"{RetroColors.COMMAND}  RBND-NC                             {RetroColors.INFO}- Display the RBND-NC license")
                print(
                    f"{RetroColors.COMMAND}  servers                             {RetroColors.INFO}- List all servers the bot is connected to")
                print(
                    f"{RetroColors.COMMAND}  server <server_id>                  {RetroColors.INFO}- View configuration for a specific server")
                print(
                    f"{RetroColors.COMMAND}  edit <server_id> <setting> <value>  {RetroColors.INFO}- Edit a server's configuration")
                print(
                    f"{RetroColors.COMMAND}  delete <server_id>                  {RetroColors.INFO}- Delete a server's configuration")
                print(
                    f"{RetroColors.COMMAND}  reset <server_id>                   {RetroColors.INFO}- Reset a server's configuration to default")
                print(
                    f"{RetroColors.COMMAND}  permissions <server_id> <action> <permission_type> [role_ids] {RetroColors.INFO}- Manage server permissions")
                print(
                    f"{RetroColors.COMMAND}  token [new_token|show]              {RetroColors.INFO}- Change or view the Discord token")
                print(
                    f"{RetroColors.COMMAND}  restart                             {RetroColors.INFO}- Restart the bot")
                print(
                    f"{RetroColors.COMMAND}  hardrestart                         {RetroColors.INFO}- Completely restart the bot script")
                print(
                    f"{RetroColors.COMMAND}  shutdown                            {RetroColors.INFO}- Shutdown the bot")
                print(f"\n{RetroColors.HEADER}Server Configuration Settings:")
                print(
                    f"{RetroColors.COMMAND}  allowed_models       {RetroColors.INFO}- Comma-separated list of allowed models, or 'all'")
                print(
                    f"{RetroColors.COMMAND}  bot_nickname         {RetroColors.INFO}- Custom nickname for the bot, or 'default'")
                print(
                    f"{RetroColors.COMMAND}  random_replies       {RetroColors.INFO}- 'enable', 'disable', 'probability=X', or 'cooldown=X'")
                print(
                    f"{RetroColors.COMMAND}  system_instructions  {RetroColors.INFO}- System instructions for the model")
                print(
                    f"{RetroColors.COMMAND}  paginated_responses  {RetroColors.INFO}- 'enable', 'disable', or 'pagesize=X'")
                print(f"\n{RetroColors.HEADER}Permission Actions:")
                print(
                    f"{RetroColors.COMMAND}  view                 {RetroColors.INFO}- View roles with a specific permission")
                print(f"{RetroColors.COMMAND}  add                  {RetroColors.INFO}- Add roles to a permission")
                print(f"{RetroColors.COMMAND}  remove               {RetroColors.INFO}- Remove roles from a permission")
                print(f"{RetroColors.COMMAND}  reset                {RetroColors.INFO}- Reset a permission to default")
                print(f"\n{RetroColors.HEADER}Permission Types:")
                print(f"{RetroColors.COMMAND}  set_model            {RetroColors.INFO}- Can change the current model")
                print(
                    f"{RetroColors.COMMAND}  manage_config        {RetroColors.INFO}- Can manage allowed models and permissions")
                print(
                    f"{RetroColors.COMMAND}  reply_to             {RetroColors.INFO}- Bot will only reply to users with these roles")

            elif command == "rbnd-nc":
                print(f"\n{RetroColors.HEADER}RBND-NC License\n")
                print(f"{RetroColors.CYAN}Copyright (c) RobotsNeverDie")
                print(f"{RetroColors.CYAN}All rights reserved.")
                print(
                    f"\n{RetroColors.BLUE}Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the \"Software\"), to use, copy, modify, and merge copies of the Software, subject to the following conditions:")
                print(f"\n{RetroColors.MAGENTA}Non-Commercial Use Only")
                print(
                    f"{RetroColors.BLUE}The Software may not be used, either in whole or in part, for any commercial purposes. Commercial purposes include, but are not limited to, selling, licensing, or offering the Software or derivative works for a fee, using it as part of a service for which payment is received, or using it within a product offered commercially.")
                print(f"\n{RetroColors.MAGENTA}Attribution and Source Linking")
                print(
                    f"{RetroColors.BLUE}Any redistribution or sharing of this Software, modified or unmodified, must include clear and visible attribution to the original author(s) and a working link to the original source repository or website: [Your Website or Repository URL].")
                print(f"\n{RetroColors.MAGENTA}No Sublicensing")
                print(
                    f"{RetroColors.BLUE}You may not sublicense the Software. Any person who receives a copy must also comply with this license directly.")
                print(f"\n{RetroColors.MAGENTA}Revocation of Rights")
                print(
                    f"{RetroColors.BLUE}The licensor reserves the right to revoke this license and any associated permissions to use, modify, or distribute the Software at any time, for any reason or no reason, at their sole discretion. Upon revocation, all use, modification, and distribution of the Software must cease immediately.")
                print(f"\n{RetroColors.MAGENTA}No Warranty")
                print(
                    f"{RetroColors.BLUE}The Software is provided \"as is\", without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and noninfringement. In no event shall the authors be liable for any claim, damages, or other liability arising from the use of the Software.")
                print(
                    f"\n{RetroColors.CYAN}By using, modifying, or distributing the Software, you agree to the terms of this license.")

            elif command == "servers":
                handle_terminal_servers_command()

            elif command == "server":
                handle_terminal_server_command(args)

            elif command == "edit":
                handle_terminal_edit_command(args)

            elif command == "delete":
                handle_terminal_delete_command(args)

            elif command == "reset":
                handle_terminal_reset_command(args)

            elif command == "permissions":
                handle_terminal_permissions_command(args)

            elif command == "token":
                handle_terminal_token_command(args)

            elif command == "restart":
                handle_terminal_restart_command()
                return  # Exit the input handler to allow restart

            elif command == "hardrestart":
                handle_terminal_hardrestart_command()
                return  # Exit the input handler to allow restart

            elif command == "shutdown":
                handle_terminal_shutdown_command()
                return  # Exit the input handler to allow shutdown

            elif command:
                print(f"{RetroColors.ERROR}Unknown command: {command}")
                print(f"{RetroColors.INFO}Type 'help' for a list of available commands")

        except EOFError:
            # Handle Ctrl+D
            break
        except KeyboardInterrupt:
            # Handle Ctrl+C
            print(f"\n{RetroColors.WARNING}Terminal input handler stopped.")
            break
        except Exception as e:
            print(f"{RetroColors.ERROR}Error in terminal input: {e}")
