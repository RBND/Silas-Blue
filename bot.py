#Silas-Blue V1.1.0
#Copyright (c) 2025 RobotsNeverDie 
#You should have received a copy of the RBND-NC License along with this program.
#If not, see https://github.com/RBND/Silus-Blue

import discord
import aiohttp
import json
import os
import datetime
import pickle
import asyncio
import random
import re
import threading
import sys
import shlex
from discord.ext import commands
from discord import ui, ButtonStyle

# Discord bot setup
intents = discord.Intents.default()
intents.message_content = True
intents.messages = True
intents.members = True  # Need this for role checking
bot = commands.Bot(command_prefix='!', intents=intents)

# Remove the default help command so we can create our own
bot.remove_command('help')

# Ollama API endpoint (default for local installation)
OLLAMA_API_URL = "http://localhost:11434/api/generate"
OLLAMA_TAGS_URL = "http://localhost:11434/api/tags"

# The model to use - change this to any model you have pulled in Ollama
DEFAULT_MODEL = "llama3"

# Configuration file path
CONFIG_FILE = "bot_config.pkl"

# Track active commands per user to implement cooldowns
active_commands = {}


# Server Configuration class
class ServerConfig:
    def __init__(self):
        # Default configuration for a server
        self.allowed_models = []  # Empty means all models are allowed
        self.permissions = {
            "set_model": [],  # Role IDs that can change the model
            "manage_config": [],  # Role IDs that can manage configuration
            "reply_to": []  # Role IDs the bot will reply to (empty means everyone)
        }
        self.bot_nickname = None  # Custom nickname for the bot
        self.random_replies = {
            "enabled": False,  # Whether random replies are enabled
            "probability": 0.05,  # Probability of replying to a random message (5%)
            "cooldown": 300  # Cooldown between random replies in seconds (5 minutes)
        }
        self.last_random_reply = 0  # Timestamp of last random reply
        self.system_instructions = ""  # System instructions for the model
        self.paginated_responses = {
            "enabled": False,  # Whether paginated responses are enabled
            "page_size": 1500  # Maximum characters per page
        }


# Bot Configuration class
class BotConfig:
    def __init__(self):
        # Dictionary to store server-specific configurations
        self.servers = {}  # Key: server_id (str), Value: ServerConfig

    def get_server_config(self, server_id):
        """Get configuration for a specific server, creating it if it doesn't exist"""
        server_id_str = str(server_id)  # Convert to string to ensure consistent keys
        if server_id_str not in self.servers:
            self.servers[server_id_str] = ServerConfig()
        return self.servers[server_id_str]

    def save(self):
        """Save configuration to file"""
        with open(CONFIG_FILE, 'wb') as f:
            pickle.dump(self, f)

    @staticmethod
    def load():
        """Load configuration from file"""
        if os.path.exists(CONFIG_FILE):
            try:
                with open(CONFIG_FILE, 'rb') as f:
                    config = pickle.load(f)

                    # Check if this is an old-style config (pre-server specific)
                    if not hasattr(config, 'servers'):
                        # Migrate old config to new format
                        old_config = config
                        config = BotConfig()

                        # Create a default server config with the old settings
                        default_server = ServerConfig()

                        # Copy over the old settings if they exist
                        if hasattr(old_config, 'allowed_models'):
                            default_server.allowed_models = old_config.allowed_models

                        if hasattr(old_config, 'permissions'):
                            default_server.permissions = old_config.permissions

                        if hasattr(old_config, 'bot_nickname'):
                            default_server.bot_nickname = old_config.bot_nickname

                        if hasattr(old_config, 'random_replies'):
                            default_server.random_replies = old_config.random_replies

                        if hasattr(old_config, 'last_random_reply'):
                            default_server.last_random_reply = old_config.last_random_reply

                        # Store this as a default config that will be copied for new servers
                        config.servers['default'] = default_server

                        print("Migrated old configuration to new server-specific format")

                    # Add new fields if they don't exist
                    for server_id, server_config in config.servers.items():
                        if not hasattr(server_config, 'system_instructions'):
                            server_config.system_instructions = ""

                        if not hasattr(server_config, 'paginated_responses'):
                            server_config.paginated_responses = {
                                "enabled": False,
                                "page_size": 1500
                            }

                    return config
            except Exception as e:
                print(f"Error loading configuration: {e}")
        return BotConfig()  # Return default config if file doesn't exist or error occurs


# Initialize configuration
config = BotConfig.load()

# Create a global aiohttp session
session = None


# Pagination view for long messages
class PaginationView(ui.View):
    def __init__(self, pages, author_id):
        super().__init__(timeout=300)  # 5 minute timeout
        self.pages = pages
        self.current_page = 0
        self.author_id = author_id
        self.update_buttons()

    def update_buttons(self):
        # Update button states based on current page
        self.previous_button.disabled = self.current_page == 0
        self.next_button.disabled = self.current_page == len(self.pages) - 1

        # Update page counter
        self.page_counter.label = f"Page {self.current_page + 1}/{len(self.pages)}"

    @ui.button(label="Previous", style=ButtonStyle.secondary, disabled=True)
    async def previous_button(self, interaction: discord.Interaction, button: ui.Button):
        # Check if the user who clicked is the one who invoked the command
        if interaction.user.id != self.author_id:
            await interaction.response.send_message("You cannot navigate this message as you didn't request it.",
                                                    ephemeral=True)
            return

        self.current_page = max(0, self.current_page - 1)
        self.update_buttons()

        await interaction.response.edit_message(content=self.pages[self.current_page], view=self)

    @ui.button(label="Page 1/1", style=ButtonStyle.primary, disabled=True)
    async def page_counter(self, interaction: discord.Interaction, button: ui.Button):
        # This button is just a counter, doesn't do anything when clicked
        await interaction.response.defer()

    @ui.button(label="Next", style=ButtonStyle.secondary, disabled=True)
    async def next_button(self, interaction: discord.Interaction, button: ui.Button):
        # Check if the user who clicked is the one who invoked the command
        if interaction.user.id != self.author_id:
            await interaction.response.send_message("You cannot navigate this message as you didn't request it.",
                                                    ephemeral=True)
            return

        self.current_page = min(len(self.pages) - 1, self.current_page + 1)
        self.update_buttons()

        await interaction.response.edit_message(content=self.pages[self.current_page], view=self)

    async def on_timeout(self):
        # Disable all buttons when the view times out
        for item in self.children:
            item.disabled = True

        # Try to update the message with disabled buttons
        try:
            message = self.message
            await message.edit(view=self)
        except:
            pass


# Terminal logging function
def log_activity(activity_type, user, content, response=None, guild_id=None):
    timestamp = datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    guild_info = f"Guild: {guild_id} | " if guild_id else ""
    print(f"[{timestamp}] {guild_info}{activity_type} | User: {user} | Content: {content}")
    if response:
        print(f"[{timestamp}] RESPONSE | {response[:100]}{'...' if len(response) > 100 else ''}")
    print("-" * 50)


# Permission checking function
def has_permission(ctx, permission_type):
    """Check if a user has a specific permission"""
    # Get server-specific config
    server_config = config.get_server_config(ctx.guild.id)

    # Server owner always has all permissions
    if ctx.guild and ctx.author.id == ctx.guild.owner_id:
        return True

    # Check if user is an administrator (for set_model and manage_config)
    if permission_type in ["set_model", "manage_config"]:
        if ctx.guild and ctx.author.guild_permissions.administrator:
            return True

    # Check if user has any of the required roles
    required_roles = server_config.permissions.get(permission_type, [])

    # If no roles are specified, default behavior depends on the permission type
    if not required_roles:
        if permission_type == "manage_config" or permission_type == "set_model":
            # Only server owner and administrators can manage config or set model if no roles are specified
            return False
        else:
            # Other permissions are granted to everyone if no roles are specified
            return True

    # Check if user has any of the required roles
    if ctx.guild:
        user_roles = [role.id for role in ctx.author.roles]
        return any(role_id in user_roles for role_id in required_roles)

    return False


# Check if a user can be replied to
def can_reply_to(member):
    """Check if the bot can reply to this user based on roles"""
    # Get server-specific config
    server_config = config.get_server_config(member.guild.id)

    # If no roles are specified, reply to everyone
    if not server_config.permissions["reply_to"]:
        return True

    # Check if user has any of the required roles
    user_roles = [role.id for role in member.roles]
    return any(role_id in user_roles for role_id in server_config.permissions["reply_to"])


# Check if a model is allowed
def is_model_allowed(model_name, guild_id):
    """Check if a model is in the allowed list"""
    # Get server-specific config
    server_config = config.get_server_config(guild_id)

    # If allowed_models is empty, all models are allowed
    if not server_config.allowed_models:
        return True

    return model_name in server_config.allowed_models


# Check if user has an active command
def has_active_command(user_id):
    """Check if a user has an active command"""
    return user_id in active_commands


# Set active command for user
def set_active_command(user_id):
    """Set active command for user"""
    active_commands[user_id] = True


# Clear active command for user
def clear_active_command(user_id):
    """Clear active command for user"""
    if user_id in active_commands:
        del active_commands[user_id]


# Parse command from message
def parse_command(content):
    """Parse command and arguments from message content"""
    # Remove leading/trailing whitespace
    content = content.strip()

    # Split into words
    words = content.split()

    if not words:
        return None, []

    # First word is the command
    command = words[0].lower()

    # Rest are arguments
    args = words[1:]

    return command, args


# Find role by name with improved matching
def find_role_by_name(guild, role_name):
    """Find a role by name with improved matching"""
    # Remove @ symbol if present
    clean_name = role_name.strip()
    if clean_name.startswith('@'):
        clean_name = clean_name[1:]

    # Try exact match first (case-insensitive)
    for role in guild.roles:
        if role.name.lower() == clean_name.lower():
            return role

    # Try partial match
    matching_roles = [r for r in guild.roles if clean_name.lower() in r.name.lower()]
    if matching_roles:
        return matching_roles[0]  # Return the first match

    return None


# Split text into chunks of max_length
def split_text(text, max_length=1900):
    """Split text into chunks of max_length"""
    if len(text) <= max_length:
        return [text]

    chunks = []
    current_chunk = ""

    for line in text.split('\n'):
        # If adding this line would exceed max_length, start a new chunk
        if len(current_chunk) + len(line) + 1 > max_length:
            if current_chunk:
                chunks.append(current_chunk)
                current_chunk = line
            else:
                # If the line itself is too long, split it
                for i in range(0, len(line), max_length):
                    chunks.append(line[i:i + max_length])
        else:
            if current_chunk:
                current_chunk += '\n' + line
            else:
                current_chunk = line

    if current_chunk:
        chunks.append(current_chunk)

    return chunks


# Send paginated response
async def send_paginated_response(ctx, content, author_id):
    """Send a paginated response with navigation buttons"""
    # Get server-specific config
    server_config = config.get_server_config(ctx.guild.id)

    # Check if pagination is enabled
    if not server_config.paginated_responses["enabled"]:
        # If pagination is disabled, send as multiple messages
        chunks = split_text(content)
        for chunk in chunks:
            await ctx.send(chunk)
        return

    # Split content into pages based on page_size
    page_size = server_config.paginated_responses["page_size"]
    pages = split_text(content, page_size)

    # If only one page, send without pagination
    if len(pages) == 1:
        await ctx.send(pages[0])
        return

    # Create pagination view
    view = PaginationView(pages, author_id)

    # Send first page with pagination view
    message = await ctx.send(pages[0], view=view)

    # Store message reference in the view for timeout handling
    view.message = message


# Terminal command handlers
def handle_terminal_servers_command():
    """Handle the 'servers' terminal command to list all servers"""
    if not bot.guilds:
        print("Bot is not connected to any servers.")
        return

    print("\nServers the bot is connected to:")
    print("-" * 50)
    for i, guild in enumerate(bot.guilds, 1):
        print(f"{i}. {guild.name} (ID: {guild.id})")
        # Check if we have a config for this server
        if str(guild.id) in config.servers:
            print(f"   Config: Yes")
        else:
            print(f"   Config: No (will be created on first interaction)")
    print("-" * 50)


def handle_terminal_server_command(args):
    """Handle the 'server' terminal command to view a specific server's configuration"""
    if not args:
        print("Error: Server ID is required. Usage: server <server_id>")
        return

    server_id = args[0]
    
    # Check if the server ID is valid
    guild = None
    for g in bot.guilds:
        if str(g.id) == server_id:
            guild = g
            break
    
    if not guild:
        print(f"Error: Server with ID {server_id} not found or bot is not connected to it.")
        return
    
    # Get the server config
    if server_id not in config.servers:
        print(f"No configuration exists for server {guild.name} (ID: {server_id}).")
        print("A default configuration will be created on first interaction.")
        return
    
    server_config = config.servers[server_id]
    
    print(f"\nConfiguration for server: {guild.name} (ID: {server_id})")
    print("-" * 50)
    
    # Display allowed models
    print(f"Allowed Models: {'All' if not server_config.allowed_models else ', '.join(server_config.allowed_models)}")
    
    # Display permissions
    print("\nPermissions:")
    for perm_type, role_ids in server_config.permissions.items():
        role_names = []
        for role_id in role_ids:
            role = guild.get_role(role_id)
            if role:
                role_names.append(f"{role.name} (ID: {role.id})")
        
        if role_names:
            print(f"  {perm_type}: {', '.join(role_names)}")
        else:
            if perm_type in ["set_model", "manage_config"]:
                print(f"  {perm_type}: No roles assigned (only server owner and administrators)")
            elif perm_type == "reply_to":
                print(f"  {perm_type}: No roles assigned (replies to everyone)")
            else:
                print(f"  {perm_type}: No roles assigned")
    
    # Display bot nickname
    print(f"\nBot Nickname: {server_config.bot_nickname if server_config.bot_nickname else 'Default'}")
    
    # Display random replies settings
    print("\nRandom Replies:")
    print(f"  Enabled: {server_config.random_replies['enabled']}")
    print(f"  Probability: {server_config.random_replies['probability'] * 100}%")
    print(f"  Cooldown: {server_config.random_replies['cooldown']} seconds")
    print(f"  Last Reply: {datetime.datetime.fromtimestamp(server_config.last_random_reply).strftime('%Y-%m-%d %H:%M:%S') if server_config.last_random_reply > 0 else 'Never'}")
    
    # Display system instructions
    print("\nSystem Instructions:")
    if server_config.system_instructions:
        print(f"  {server_config.system_instructions}")
    else:
        print("  None")
    
    # Display paginated responses settings
    print("\nPaginated Responses:")
    print(f"  Enabled: {server_config.paginated_responses['enabled']}")
    print(f"  Page Size: {server_config.paginated_responses['page_size']} characters")
    
    print("-" * 50)


def handle_terminal_edit_command(args):
    """Handle the 'edit' terminal command to edit a server's configuration"""
    if len(args) < 3:
        print("Error: Insufficient arguments. Usage: edit <server_id> <setting> <value>")
        print("Available settings: allowed_models, bot_nickname, random_replies, system_instructions, paginated_responses")
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
        print(f"Error: Server with ID {server_id} not found or bot is not connected to it.")
        return
    
    # Get or create the server config
    server_config = config.get_server_config(guild.id)
    
    # Edit the specified setting
    if setting == "allowed_models":
        if value.lower() == "all":
            server_config.allowed_models = []
            print(f"Set allowed models to: All models")
        else:
            models = [model.strip() for model in value.split(',')]
            server_config.allowed_models = models
            print(f"Set allowed models to: {', '.join(models)}")
    
    elif setting == "bot_nickname":
        if value.lower() == "default" or value.lower() == "none":
            server_config.bot_nickname = None
            print("Reset bot nickname to default")
            # Update the bot's nickname in the guild
            asyncio.run_coroutine_threadsafe(guild.me.edit(nick=None), bot.loop)
        else:
            server_config.bot_nickname = value
            print(f"Set bot nickname to: {value}")
            # Update the bot's nickname in the guild
            asyncio.run_coroutine_threadsafe(guild.me.edit(nick=value), bot.loop)
    
    elif setting == "random_replies":
        # Parse the value as a JSON object
        try:
            if value.lower() in ["enable", "enabled", "true", "yes", "1"]:
                server_config.random_replies["enabled"] = True
                print("Enabled random replies")
            elif value.lower() in ["disable", "disabled", "false", "no", "0"]:
                server_config.random_replies["enabled"] = False
                print("Disabled random replies")
            elif value.lower().startswith("probability="):
                prob_str = value.split('=')[1].strip()
                prob = float(prob_str)
                if 0 <= prob <= 1:
                    server_config.random_replies["probability"] = prob
                    print(f"Set random reply probability to: {prob * 100}%")
                else:
                    print("Error: Probability must be between 0 and 1")
                    return
            elif value.lower().startswith("cooldown="):
                cooldown_str = value.split('=')[1].strip()
                cooldown = int(cooldown_str)
                if cooldown >= 0:
                    server_config.random_replies["cooldown"] = cooldown
                    print(f"Set random reply cooldown to: {cooldown} seconds")
                else:
                    print("Error: Cooldown must be a non-negative integer")
                    return
            else:
                print("Error: Invalid value for random_replies. Use 'enable', 'disable', 'probability=X', or 'cooldown=X'")
                return
        except Exception as e:
            print(f"Error parsing random_replies value: {e}")
            return
    
    elif setting == "system_instructions":
        server_config.system_instructions = value
        print(f"Set system instructions to: {value}")
    
    elif setting == "paginated_responses":
        if value.lower() in ["enable", "enabled", "true", "yes", "1"]:
            server_config.paginated_responses["enabled"] = True
            print("Enabled paginated responses")
        elif value.lower() in ["disable", "disabled", "false", "no", "0"]:
            server_config.paginated_responses["enabled"] = False
            print("Disabled paginated responses")
        elif value.lower().startswith("pagesize="):
            size_str = value.split('=')[1].strip()
            size = int(size_str)
            if 100 <= size <= 2000:
                server_config.paginated_responses["page_size"] = size
                print(f"Set paginated response page size to: {size} characters")
            else:
                print("Error: Page size must be between 100 and 2000")
                return
        else:
            print("Error: Invalid value for paginated_responses. Use 'enable', 'disable', or 'pagesize=X'")
            return
    
    else:
        print(f"Error: Unknown setting '{setting}'")
        print("Available settings: allowed_models, bot_nickname, random_replies, system_instructions, paginated_responses")
        return
    
    # Save the updated configuration
    config.save()
    print(f"Configuration for server '{guild.name}' (ID: {server_id}) has been updated and saved.")


def handle_terminal_delete_command(args):
    """Handle the 'delete' terminal command to delete a server's configuration"""
    if not args:
        print("Error: Server ID is required. Usage: delete <server_id>")
        return

    server_id = args[0]
    
    # Check if the server ID is valid
    guild = None
    for g in bot.guilds:
        if str(g.id) == server_id:
            guild = g
            break
    
    if not guild and server_id != "default":
        print(f"Warning: Server with ID {server_id} not found or bot is not connected to it.")
        confirmation = input("Do you still want to delete this configuration? (y/n): ")
        if confirmation.lower() != 'y':
            print("Operation cancelled.")
            return
    
    # Check if the configuration exists
    if server_id not in config.servers:
        print(f"Error: No configuration exists for server ID {server_id}.")
        return
    
    # Ask for confirmation
    server_name = guild.name if guild else f"ID {server_id}"
    confirmation = input(f"Are you sure you want to delete the configuration for server '{server_name}'? (y/n): ")
    if confirmation.lower() != 'y':
        print("Operation cancelled.")
        return
    
    # Delete the configuration
    del config.servers[server_id]
    config.save()
    print(f"Configuration for server '{server_name}' has been deleted.")
    
    if server_id == "default":
        print("Note: The default configuration has been deleted. New servers will now get a fresh default configuration.")


def handle_terminal_reset_command(args):
    """Handle the 'reset' terminal command to reset a server's configuration to default"""
    if not args:
        print("Error: Server ID is required. Usage: reset <server_id>")
        return

    server_id = args[0]
    
    # Check if the server ID is valid
    guild = None
    for g in bot.guilds:
        if str(g.id) == server_id:
            guild = g
            break
    
    if not guild:
        print(f"Error: Server with ID {server_id} not found or bot is not connected to it.")
        return
    
    # Ask for confirmation
    confirmation = input(f"Are you sure you want to reset the configuration for server '{guild.name}'? (y/n): ")
    if confirmation.lower() != 'y':
        print("Operation cancelled.")
        return
    
    # Reset the configuration
    config.servers[server_id] = ServerConfig()
    config.save()
    print(f"Configuration for server '{guild.name}' has been reset to default.")


def handle_terminal_permissions_command(args):
    """Handle the 'permissions' terminal command to manage server permissions"""
    if len(args) < 3:
        print("Error: Insufficient arguments.")
        print("Usage: permissions <server_id> <action> <permission_type> [role_id1,role_id2,...]")
        print("Actions: view, add, remove, reset")
        print("Permission types: set_model, manage_config, reply_to")
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
        print(f"Error: Server with ID {server_id} not found or bot is not connected to it.")
        return
    
    # Get or create the server config
    server_config = config.get_server_config(guild.id)
    
    # Check if the permission type is valid
    if permission_type not in ["set_model", "manage_config", "reply_to"]:
        print(f"Error: Invalid permission type '{permission_type}'")
        print("Valid permission types: set_model, manage_config, reply_to")
        return
    
    # Handle the action
    if action == "view":
        role_ids = server_config.permissions[permission_type]
        if not role_ids:
            if permission_type in ["set_model", "manage_config"]:
                print(f"Permission '{permission_type}': No roles assigned (only server owner and administrators)")
            elif permission_type == "reply_to":
                print(f"Permission '{permission_type}': No roles assigned (replies to everyone)")
            else:
                print(f"Permission '{permission_type}': No roles assigned")
        else:
            role_details = []
            for role_id in role_ids:
                role = guild.get_role(role_id)
                if role:
                    role_details.append(f"{role.name} (ID: {role.id})")
                else:
                    role_details.append(f"Unknown role (ID: {role_id})")
            print(f"Permission '{permission_type}' roles: {', '.join(role_details)}")
    
    elif action == "add":
        if len(args) < 4:
            print("Error: Role IDs are required for the 'add' action.")
            print("Usage: permissions <server_id> add <permission_type> <role_id1,role_id2,...>")
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
                    print(f"Role '{role.name}' (ID: {role.id}) already has '{permission_type}' permission.")
            else:
                print(f"Warning: Role with ID {role_id} not found in server '{guild.name}'")
        
        if added_roles:
            config.save()
            print(f"Added roles to '{permission_type}' permission: {', '.join(added_roles)}")
        else:
            print("No roles were added.")
    
    elif action == "remove":
        if len(args) < 4:
            print("Error: Role IDs are required for the 'remove' action.")
            print("Usage: permissions <server_id> remove <permission_type> <role_id1,role_id2,...>")
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
                print(f"Role {role_name} does not have '{permission_type}' permission.")
        
        if removed_roles:
            config.save()
            print(f"Removed roles from '{permission_type}' permission: {', '.join(removed_roles)}")
        else:
            print("No roles were removed.")
    
    elif action == "reset":
        # Ask for confirmation
        confirmation = input(f"Are you sure you want to reset the '{permission_type}' permission for server '{guild.name}'? (y/n): ")
        if confirmation.lower() != 'y':
            print("Operation cancelled.")
            return
        
        server_config.permissions[permission_type] = []
        config.save()
        
        if permission_type in ["set_model", "manage_config"]:
            print(f"Reset '{permission_type}' permission to default (only server owner and administrators)")
        elif permission_type == "reply_to":
            print(f"Reset '{permission_type}' permission to default (replies to everyone)")
        else:
            print(f"Reset '{permission_type}' permission to default")
    
    else:
        print(f"Error: Invalid action '{action}'")
        print("Valid actions: view, add, remove, reset")


def terminal_input_handler():
    """Handle terminal input while the bot is running"""
    print("Terminal commands are now active. Type 'help' to see available commands.")
    while True:
        try:
            command_line = input("Silas Blue: ").strip()
            if not command_line:
                continue
                
            # Parse the command line with proper handling of quoted arguments
            try:
                args = shlex.split(command_line)
            except ValueError as e:
                print(f"Error parsing command: {e}")
                continue
                
            command = args[0].lower()
            args = args[1:] if len(args) > 1 else []
            
            if command == "help":
                print("\nAvailable terminal commands:")
                print("  help                                - Display this help message")
                print("  RBND-NC                             - Display the RBND-NC license")
                print("  servers                             - List all servers the bot is connected to")
                print("  server <server_id>                  - View configuration for a specific server")
                print("  edit <server_id> <setting> <value>  - Edit a server's configuration")
                print("  delete <server_id>                  - Delete a server's configuration")
                print("  reset <server_id>                   - Reset a server's configuration to default")
                print("  permissions <server_id> <action> <permission_type> [role_ids] - Manage server permissions")
                print("\nServer Configuration Settings:")
                print("  allowed_models       - Comma-separated list of allowed models, or 'all'")
                print("  bot_nickname         - Custom nickname for the bot, or 'default'")
                print("  random_replies       - 'enable', 'disable', 'probability=X', or 'cooldown=X'")
                print("  system_instructions  - System instructions for the model")
                print("  paginated_responses  - 'enable', 'disable', or 'pagesize=X'")
                print("\nPermission Actions:")
                print("  view                 - View roles with a specific permission")
                print("  add                  - Add roles to a permission")
                print("  remove               - Remove roles from a permission")
                print("  reset                - Reset a permission to default")
                print("\nPermission Types:")
                print("  set_model            - Can change the current model")
                print("  manage_config        - Can manage allowed models and permissions")
                print("  reply_to             - Bot will only reply to users with these roles")
            
            elif command == "rbnd-nc":
                print("\nRBND-NC License\n")
                print("Copyright (c) RobotsNeverDie")
                print("All rights reserved.")
                print("\nPermission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the \"Software\"), to use, copy, modify, and merge copies of the Software, subject to the following conditions:")
                print("\nNon-Commercial Use Only")
                print("The Software may not be used, either in whole or in part, for any commercial purposes. Commercial purposes include, but are not limited to, selling, licensing, or offering the Software or derivative works for a fee, using it as part of a service for which payment is received, or using it within a product offered commercially.")
                print("\nAttribution and Source Linking")
                print("Any redistribution or sharing of this Software, modified or unmodified, must include clear and visible attribution to the original author(s) and a working link to the original source repository or website: [Your Website or Repository URL].")
                print("\nNo Sublicensing")
                print("You may not sublicense the Software. Any person who receives a copy must also comply with this license directly.")
                print("\nRevocation of Rights")
                print("The licensor reserves the right to revoke this license and any associated permissions to use, modify, or distribute the Software at any time, for any reason or no reason, at their sole discretion. Upon revocation, all use, modification, and distribution of the Software must cease immediately.")
                print("\nNo Warranty")
                print("The Software is provided \"as is\", without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and noninfringement. In no event shall the authors be liable for any claim, damages, or other liability arising from the use of the Software.")
                print("\nBy using, modifying, or distributing the Software, you agree to the terms of this license.")
            
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
            
            elif command:
                print(f"Unknown command: {command}")
                print("Type 'help' for a list of available commands")
        
        except EOFError:
            # Handle Ctrl+D
            break
        except KeyboardInterrupt:
            # Handle Ctrl+C
            print("\nTerminal input handler stopped.")
            break
        except Exception as e:
            print(f"Error in terminal input: {e}")


@bot.event
async def on_ready():
    global session
    # Print startup text
    print("Silas Blue V1.0")
    print("Copyright (C) 2025 RobotsNeverDie")
    print("This program comes with ABSOLUTELY NO WARRANTY")
    print("You may only run this software under the RBND-NC license.")
    print("\n------")
    # Create aiohttp session
    session = aiohttp.ClientSession()

    print(f'Logged in as {bot.user.name} ({bot.user.id})')
    print(f'Connected to {len(bot.guilds)} servers:')

    for guild in bot.guilds:
        # Get or create server config
        server_config = config.get_server_config(guild.id)

        print(f'\nServer: {guild.name} (ID: {guild.id})')
        print(
            f'- Allowed Models: {"All" if not server_config.allowed_models else ", ".join(server_config.allowed_models)}')
        print(f'- Set Model Permission: {len(server_config.permissions["set_model"])} roles')
        print(f'- Manage Config Permission: {len(server_config.permissions["manage_config"])} roles')
        print(
            f'- Reply To Permission: {"Everyone" if not server_config.permissions["reply_to"] else f"{len(server_config.permissions["reply_to"])} roles"}')
        print(f'- Bot Nickname: {server_config.bot_nickname if server_config.bot_nickname else "Default"}')
        print(f'- Random Replies: {"Enabled" if server_config.random_replies["enabled"] else "Disabled"}')
        if server_config.random_replies["enabled"]:
            print(f'  - Probability: {server_config.random_replies["probability"] * 100}%')
            print(f'  - Cooldown: {server_config.random_replies["cooldown"]} seconds')
        print(f'- System Instructions: {"Set" if server_config.system_instructions else "None"}')
        print(f'- Paginated Responses: {"Enabled" if server_config.paginated_responses["enabled"] else "Disabled"}')
        if server_config.paginated_responses["enabled"]:
            print(f'  - Page Size: {server_config.paginated_responses["page_size"]} characters')

        # Set custom nickname in guild if configured
        if server_config.bot_nickname:
            try:
                await guild.me.edit(nick=server_config.bot_nickname)
                print(f"Set nickname to '{server_config.bot_nickname}' in guild: {guild.name}")
            except Exception as e:
                print(f"Failed to set nickname in guild {guild.name}: {e}")

    print('\n------')
    
    # Start terminal input handler in a separate thread
    input_thread = threading.Thread(target=terminal_input_handler, daemon=True)
    input_thread.start()


@bot.event
async def on_guild_join(guild):
    """Handle joining a new server"""
    print(f"Joined new server: {guild.name} (ID: {guild.id})")

    # Get or create server config
    server_config = config.get_server_config(guild.id)

    # If we have a default config, copy its settings
    if 'default' in config.servers:
        default_config = config.servers['default']
        server_config.allowed_models = default_config.allowed_models.copy()
        server_config.permissions = {k: v.copy() for k, v in default_config.permissions.items()}
        server_config.bot_nickname = default_config.bot_nickname
        server_config.random_replies = default_config.random_replies.copy()
        server_config.system_instructions = default_config.system_instructions
        server_config.paginated_responses = default_config.paginated_responses.copy()

    # Save the updated config
    config.save()

    print(f"Created configuration for new server: {guild.name}")


@bot.event
async def on_message(message):
    # Ignore messages from the bot itself
    if message.author == bot.user:
        return

    # Check if the bot is mentioned in the message
    if bot.user.mentioned_in(message) and not message.mention_everyone:
        # Check if the user has permission to be replied to
        if not can_reply_to(message.author):
            log_activity("PERMISSION DENIED", message.author, "mention (no reply permission)",
                         guild_id=message.guild.id)
            return

        # Check if user has an active command
        if has_active_command(message.author.id):
            await message.reply("I'm still processing your previous request. Please wait.")
            return

        # Extract the actual content (remove the mention)
        content = message.content.replace(f'<@{bot.user.id}>', '').strip()

        # Parse command and arguments
        command, args = parse_command(content)

        # Check for commands
        if command:
            # Create a mock context for permission checking
            ctx = await bot.get_context(message)

            # Handle various commands
            if command in ['models', '!models']:
                await handle_models_command(message)
                return
            elif command in ['help', '!help']:
                await handle_help_command(message)
                return
            elif command in ['ask', '!ask']:
                # Join all remaining words as the question
                question = ' '.join(args)
                if question:
                    await handle_ask_command(ctx, question)
                else:
                    await message.reply("Please provide a question after 'ask'.")
                return
            elif command in ['setmodel', '!setmodel']:
                if args:
                    await handle_setmodel_command(ctx, args[0])
                else:
                    await message.reply("Please provide a model name after 'setmodel'.")
                return
            elif command in ['search', '!search']:
                query = ' '.join(args)
                if query:
                    await handle_search_command(ctx, query)
                else:
                    await message.reply("Please provide a search query after 'search'.")
                return
            elif command in ['reference', '!reference']:
                if args:
                    try:
                        message_id = int(args[0])
                        prompt = ' '.join(args[1:]) if len(args) > 1 else None
                        await handle_reference_command(ctx, message_id, prompt)
                    except ValueError:
                        await message.reply("Please provide a valid message ID after 'reference'.")
                else:
                    await message.reply("Please provide a message ID after 'reference'.")
                return
            elif command in ['allowmodel', '!allowmodel']:
                if args:
                    await handle_allowmodel_command(ctx, args)
                else:
                    await message.reply("Please provide at least one model name after 'allowmodel'.")
                return
            elif command in ['disallowmodel', '!disallowmodel']:
                if args:
                    await handle_disallowmodel_command(ctx, args)
                else:
                    await message.reply("Please provide at least one model name after 'disallowmodel'.")
                return
            elif command in ['allowallmodels', '!allowallmodels']:
                await handle_allowallmodels_command(ctx)
                return
            elif command in ['addrole', '!addrole']:
                if len(args) >= 2:
                    permission_type = args[0]
                    role_names = ' '.join(args[1:])
                    await handle_addrole_command(ctx, permission_type, role_names)
                else:
                    await message.reply("Please provide a permission type and at least one role name after 'addrole'.")
                return
            elif command in ['removerole', '!removerole']:
                if len(args) >= 2:
                    permission_type = args[0]
                    role_names = ' '.join(args[1:])
                    await handle_removerole_command(ctx, permission_type, role_names)
                else:
                    await message.reply(
                        "Please provide a permission type and at least one role name after 'removerole'.")
                return
            elif command in ['listroles', '!listroles']:
                permission_type = args[0] if args else None
                await handle_listroles_command(ctx, permission_type)
                return
            elif command in ['resetpermissions', '!resetpermissions']:
                permission_type = args[0] if args else None
                await handle_resetpermissions_command(ctx, permission_type)
                return
            elif command in ['setbotname', '!setbotname']:
                new_name = ' '.join(args)
                if new_name:
                    await handle_setbotname_command(ctx, new_name)
                else:
                    await message.reply("Please provide a name after 'setbotname'.")
                return
            elif command in ['resetbotname', '!resetbotname']:
                await handle_resetbotname_command(ctx)
                return
            elif command in ['randomreplies', '!randomreplies']:
                if args:
                    await handle_randomreplies_command(ctx, args)
                else:
                    await message.reply(
                        "Please provide at least one parameter (enable/disable/status/probability/cooldown) after 'randomreplies'.")
                return
            elif command in ['system', '!system', 'systeminstructions', '!systeminstructions']:
                if args:
                    await handle_system_instructions_command(ctx, ' '.join(args))
                else:
                    await handle_system_instructions_command(ctx, "")
                return
            elif command in ['pagination', '!pagination']:
                if args:
                    await handle_pagination_command(ctx, args)
                else:
                    await message.reply(
                        "Please provide at least one parameter (enable/disable/status/pagesize) after 'pagination'.")
                return
            elif command in ['RBND-NC', '!RBND-NC']:
                await handle_rbnd_nc_command(ctx)
                return

        # If no command was recognized, treat as a regular question
        if content:
            log_activity("MENTION", message.author, content, guild_id=message.guild.id)

            # Set active command
            set_active_command(message.author.id)

            async with message.channel.typing():
                try:
                    # Get server-specific model and system instructions
                    server_config = config.get_server_config(message.guild.id)
                    current_model = DEFAULT_MODEL
                    system_instructions = server_config.system_instructions

                    # Prepare the request to Ollama
                    payload = {
                        "model": current_model,
                        "prompt": content,
                        "stream": False
                    }

                    # Add system instructions if set
                    if system_instructions:
                        payload["system"] = system_instructions

                    # Send request to Ollama using aiohttp
                    async with session.post(OLLAMA_API_URL, json=payload) as response:
                        if response.status == 200:
                            result = await response.json()
                            answer = result.get('response', 'No response generated')

                            # Send paginated response
                            await send_paginated_response(message.channel, answer, message.author.id)

                            log_activity("MENTION RESPONSE", bot.user, content, answer, guild_id=message.guild.id)
                        else:
                            error_msg = f"Error: Received status code {response.status} from Ollama API"
                            await message.reply(error_msg)
                            log_activity("ERROR", bot.user, content, error_msg, guild_id=message.guild.id)

                except Exception as e:
                    error_msg = f"Error: {str(e)}"
                    await message.reply(error_msg)
                    log_activity("ERROR", bot.user, content, error_msg, guild_id=message.guild.id)

                finally:
                    # Clear active command
                    clear_active_command(message.author.id)
        else:
            await message.reply("You mentioned me, but didn't ask anything. How can I help?")

    # Check for random replies if enabled
    elif message.guild and not message.content.startswith('!'):
        # Get server-specific config
        server_config = config.get_server_config(message.guild.id)

        if server_config.random_replies["enabled"]:
            # Check cooldown
            current_time = datetime.datetime.now().timestamp()
            if current_time - server_config.last_random_reply >= server_config.random_replies["cooldown"]:
                # Check probability
                if random.random() < server_config.random_replies["probability"]:
                    # Check if the user can be replied to
                    if can_reply_to(message.author):
                        # Don't reply to very short messages
                        if len(message.content) > 10:
                            # Set last random reply time
                            server_config.last_random_reply = current_time
                            config.save()

                            log_activity("RANDOM REPLY", message.author, message.content, guild_id=message.guild.id)

                            # Set active command
                            set_active_command(message.author.id)

                            async with message.channel.typing():
                                try:
                                    # Get server-specific model and system instructions
                                    current_model = DEFAULT_MODEL
                                    system_instructions = server_config.system_instructions

                                    # Prepare the request to Ollama
                                    payload = {
                                        "model": current_model,
                                        "prompt": f"The following is a message in a Discord chat. Please respond to it naturally and briefly (1-2 sentences max):\n\n{message.content}",
                                        "stream": False
                                    }

                                    # Add system instructions if set
                                    if system_instructions:
                                        payload["system"] = system_instructions

                                    # Send request to Ollama using aiohttp
                                    async with session.post(OLLAMA_API_URL, json=payload) as response:
                                        if response.status == 200:
                                            result = await response.json()
                                            answer = result.get('response', 'No response generated')

                                            # Keep random replies short
                                            if len(answer) > 500:
                                                answer = answer[:500] + "..."

                                            await message.reply(answer)
                                            log_activity("RANDOM REPLY RESPONSE", bot.user, message.content, answer,
                                                         guild_id=message.guild.id)
                                        else:
                                            log_activity("ERROR", bot.user, "Random reply",
                                                         f"Received status code {response.status} from Ollama API",
                                                         guild_id=message.guild.id)

                                except Exception as e:
                                    log_activity("ERROR", bot.user, "Random reply", str(e), guild_id=message.guild.id)

                                finally:
                                    # Clear active command
                                    clear_active_command(message.author.id)

    # Process commands
    await bot.process_commands(message)


# Helper function to handle models command
async def handle_models_command(message):
    log_activity("COMMAND", message.author, "models (via mention)", guild_id=message.guild.id)

    # Set active command
    set_active_command(message.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(message.guild.id)

        # Send request to Ollama using aiohttp
        async with session.get(OLLAMA_TAGS_URL) as response:
            if response.status == 200:
                data = await response.json()
                all_models = data.get('models', [])

                if all_models:
                    # Format the model list
                    model_list = []
                    for model in all_models:
                        model_name = model['name']
                        # Mark if model is allowed or not
                        if server_config.allowed_models and model_name not in server_config.allowed_models:
                            model_list.append(f"- {model_name} (⛔ not allowed)")
                        else:
                            # Mark the current model
                            if model_name == DEFAULT_MODEL:
                                model_list.append(f"- {model_name} (✅ current)")
                            else:
                                model_list.append(f"- {model_name}")

                    # Add header with allowed models info
                    if server_config.allowed_models:
                        header = f"Available models (only {len(server_config.allowed_models)} allowed):\n"
                    else:
                        header = "Available models (all allowed):\n"

                    await message.reply(header + "\n".join(model_list))
                    log_activity("COMMAND RESPONSE", bot.user, "models (via mention)", "\n".join(model_list),
                                 guild_id=message.guild.id)
                else:
                    msg = "No models found. Pull models using 'ollama pull model_name'"
                    await message.reply(msg)
                    log_activity("COMMAND RESPONSE", bot.user, "models (via mention)", msg, guild_id=message.guild.id)
            else:
                error_msg = f"Error: Received status code {response.status} from Ollama API"
                await message.reply(error_msg)
                log_activity("ERROR", bot.user, "models (via mention)", error_msg, guild_id=message.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await message.reply(error_msg)
        log_activity("ERROR", bot.user, "models (via mention)", error_msg, guild_id=message.guild.id)

    finally:
        # Clear active command
        clear_active_command(message.author.id)


# Helper function to handle help command
async def handle_help_command(message):
    log_activity("COMMAND", message.author, "help (via mention)", guild_id=message.guild.id)

    # Split help text into sections to avoid Discord's 2000 character limit
    basic_help = """
**Discord Ollama Bot Commands**

**Basic Commands:**
`!ask <question>` - Ask a question to the AI model
`!search <query>` - Search for messages containing the query
`!reference <message_id> [prompt]` - Reference a message and ask AI about it
`!models` - List available models in Ollama
`!setmodel <model_name>` - Change the current model (requires permission)
"""

    model_help = """
**Model Management:**
`!allowmodel <model_name> [model_name2] [...]` - Add model(s) to allowed list (requires permission)
`!disallowmodel <model_name> [model_name2] [...]` - Remove model(s) from allowed list (requires permission)
`!allowallmodels` - Allow all models to be used (requires permission)
`!system <instructions>` - Set system instructions for the model (requires permission)
`!system show` - Show current system instructions
`!system reset` - Reset system instructions to default (empty)
"""

    permission_help = """
**Permission Management:**
`!addrole <permission_type> <role_name> [role_name2] [...]` - Add role(s) to a permission type (requires permission)
`!removerole <permission_type> <role_name> [role_name2] [...]` - Remove role(s) from a permission type (requires permission)
`!listroles [permission_type]` - List roles for a permission type or all types
`!resetpermissions [permission_type]` - Reset permissions to default (server owner only)

**Permission Types:**
- `set_model` - Can change the current model (server owner and administrators by default)
- `manage_config` - Can manage allowed models and permissions (server owner and administrators by default)
- `reply_to` - Bot will only reply to users with these roles (if empty, replies to everyone)
"""

    customization_help = """
**Bot Customization:**
`!setbotname <name>` - Change the bot's nickname (requires manage_config permission)
`!resetbotname` - Reset the bot's nickname to default (requires manage_config permission)
`!randomreplies <parameter> <value>` - Configure random replies (requires manage_config permission)
  - `enable` - Enable random replies
  - `disable` - Disable random replies
  - `status` - Show current random reply settings
  - `probability <0.0-1.0>` - Set probability of random replies (0.05 = 5%)
  - `cooldown <seconds>` - Set cooldown between random replies

`!pagination <parameter> <value>` - Configure paginated responses (requires manage_config permission)
  - `enable` - Enable paginated responses
  - `disable` - Disable paginated responses
  - `status` - Show current pagination settings
  - `pagesize <characters>` - Set maximum characters per page

**Notes:**
- You can mention the bot directly to use any command without the ! prefix
- Role names can be specified with or without the @ symbol
- Each server has its own separate configuration
"""

    # Send each section as a separate message
    await message.reply(basic_help)
    await message.reply(model_help)
    await message.reply(permission_help)
    await message.reply(customization_help)

    log_activity("COMMAND RESPONSE", bot.user, "help (via mention)", "Help text sent (multiple messages)",
                 guild_id=message.guild.id)


# Helper function to handle ask command
async def handle_ask_command(ctx, question):
    # Check if the user has permission to be replied to
    if not can_reply_to(ctx.author):
        log_activity("PERMISSION DENIED", ctx.author, f"ask {question}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to use this command.")
        return

    log_activity("COMMAND", ctx.author, f"ask {question}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        async with ctx.typing():
            try:
                # Get server-specific model and system instructions
                server_config = config.get_server_config(ctx.guild.id)
                current_model = DEFAULT_MODEL
                system_instructions = server_config.system_instructions

                # Prepare the request to Ollama
                payload = {
                    "model": current_model,
                    "prompt": question,
                    "stream": False
                }

                # Add system instructions if set
                if system_instructions:
                    payload["system"] = system_instructions

                # Send request to Ollama using aiohttp
                async with session.post(OLLAMA_API_URL, json=payload) as response:
                    if response.status == 200:
                        result = await response.json()
                        answer = result.get('response', 'No response generated')

                        # Send paginated response
                        await send_paginated_response(ctx, answer, ctx.author.id)

                        log_activity("COMMAND RESPONSE", bot.user, f"ask {question}", answer, guild_id=ctx.guild.id)
                    else:
                        error_msg = f"Error: Received status code {response.status} from Ollama API"
                        await ctx.send(error_msg)
                        log_activity("ERROR", bot.user, f"ask {question}", error_msg, guild_id=ctx.guild.id)

            except Exception as e:
                error_msg = f"Error: {str(e)}"
                await ctx.send(error_msg)
                log_activity("ERROR", bot.user, f"ask {question}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle setmodel command
async def handle_setmodel_command(ctx, model_name):
    # Check if user has permission to set model
    if not has_permission(ctx, "set_model"):
        log_activity("PERMISSION DENIED", ctx.author, f"setmodel {model_name}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to change the model.")
        return

    log_activity("COMMAND", ctx.author, f"setmodel {model_name}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        global DEFAULT_MODEL
        # Check if the model exists
        async with session.get(OLLAMA_TAGS_URL) as response:
            if response.status == 200:
                data = await response.json()
                models = data.get('models', [])
                model_names = [model['name'] for model in models]

                if model_name in model_names:
                    # Check if model is in allowed list
                    if not is_model_allowed(model_name, ctx.guild.id):
                        msg = f"Model '{model_name}' is not in the allowed models list."
                        await ctx.send(msg)
                        log_activity("COMMAND RESPONSE", bot.user, f"setmodel {model_name}", msg, guild_id=ctx.guild.id)
                        return

                    DEFAULT_MODEL = model_name
                    msg = f"Default model set to {model_name}"
                    await ctx.send(msg)
                    log_activity("COMMAND RESPONSE", bot.user, f"setmodel {model_name}", msg, guild_id=ctx.guild.id)
                else:
                    msg = f"Model '{model_name}' not found. Available models: {', '.join(model_names)}"
                    await ctx.send(msg)
                    log_activity("COMMAND RESPONSE", bot.user, f"setmodel {model_name}", msg, guild_id=ctx.guild.id)
            else:
                error_msg = f"Error: Received status code {response.status} from Ollama API"
                await ctx.send(error_msg)
                log_activity("ERROR", bot.user, f"setmodel {model_name}", error_msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"setmodel {model_name}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle search command
async def handle_search_command(ctx, query):
    # Check if the user has permission to be replied to
    if not can_reply_to(ctx.author):
        log_activity("PERMISSION DENIED", ctx.author, f"search {query}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to use this command.")
        return

    log_activity("COMMAND", ctx.author, f"search {query}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get the last 100 messages in the channel
        messages = []
        async for message in ctx.channel.history(limit=100):
            if query.lower() in message.content.lower() and message.id != ctx.message.id:
                messages.append(message)

        if messages:
            # Format the search results
            results = []
            for i, msg in enumerate(messages[:5], 1):  # Limit to 5 results
                timestamp = msg.created_at.strftime("%Y-%m-%d %H:%M")
                content = msg.content if len(msg.content) <= 100 else f"{msg.content[:100]}..."
                results.append(f"{i}. [{timestamp}] {msg.author.name}: {content}")
                results.append(f"   Message ID: {msg.id}")

            response = f"Found {len(messages)} messages containing '{query}':\n\n" + "\n".join(results)

            if len(messages) > 5:
                response += f"\n\n(Showing 5 of {len(messages)} results)"

            # Send paginated response
            await send_paginated_response(ctx, response, ctx.author.id)

            log_activity("COMMAND RESPONSE", bot.user, f"search {query}", response, guild_id=ctx.guild.id)
        else:
            msg = f"No messages found containing '{query}'"
            await ctx.send(msg)
            log_activity("COMMAND RESPONSE", bot.user, f"search {query}", msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"search {query}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle reference command
async def handle_reference_command(ctx, message_id, prompt=None):
    # Check if the user has permission to be replied to
    if not can_reply_to(ctx.author):
        log_activity("PERMISSION DENIED", ctx.author, f"reference {message_id}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to use this command.")
        return

    log_activity("COMMAND", ctx.author, f"reference {message_id} {prompt if prompt else ''}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Try to fetch the message
        message = None
        try:
            message = await ctx.channel.fetch_message(message_id)
        except discord.NotFound:
            await ctx.send(f"Message with ID {message_id} not found in this channel.")
            return

        # Format the referenced message
        timestamp = message.created_at.strftime("%Y-%m-%d %H:%M")
        referenced_content = f"Referenced message from {message.author.name} at {timestamp}:\n\"{message.content}\""

        # Prepare the prompt for Ollama
        if prompt:
            full_prompt = f"{referenced_content}\n\nUser question: {prompt}"
        else:
            full_prompt = f"{referenced_content}\n\nPlease analyze or summarize this message."

        async with ctx.typing():
            # Get server-specific model and system instructions
            server_config = config.get_server_config(ctx.guild.id)
            current_model = DEFAULT_MODEL
            system_instructions = server_config.system_instructions

            # Prepare the request to Ollama
            payload = {
                "model": current_model,
                "prompt": full_prompt,
                "stream": False
            }

            # Add system instructions if set
            if system_instructions:
                payload["system"] = system_instructions

            # Send request to Ollama using aiohttp
            async with session.post(OLLAMA_API_URL, json=payload) as response:
                if response.status == 200:
                    result = await response.json()
                    answer = result.get('response', 'No response generated')

                    # Create a response that includes the reference
                    full_response = f"**Referencing message from {message.author.name}:**\n> {message.content}\n\n{answer}"

                    # Send paginated response
                    await send_paginated_response(ctx, full_response, ctx.author.id)

                    log_activity("COMMAND RESPONSE", bot.user, f"reference {message_id}", answer, guild_id=ctx.guild.id)
                else:
                    error_msg = f"Error: Received status code {response.status} from Ollama API"
                    await ctx.send(error_msg)
                    log_activity("ERROR", bot.user, f"reference {message_id}", error_msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"reference {message_id}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle allowmodel command with multiple models
async def handle_allowmodel_command(ctx, models):
    # Check if user has permission to manage config
    if not has_permission(ctx, "manage_config"):
        log_activity("PERMISSION DENIED", ctx.author, f"allowmodel {' '.join(models)}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to manage allowed models.")
        return

    log_activity("COMMAND", ctx.author, f"allowmodel {' '.join(models)}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        # Check if the models exist
        async with session.get(OLLAMA_TAGS_URL) as response:
            if response.status == 200:
                data = await response.json()
                available_models = data.get('models', [])
                available_model_names = [model['name'] for model in available_models]

                added_models = []
                not_found_models = []
                already_allowed_models = []

                for model_name in models:
                    if model_name in available_model_names:
                        # Add to allowed models if not already there
                        if model_name not in server_config.allowed_models:
                            server_config.allowed_models.append(model_name)
                            added_models.append(model_name)
                        else:
                            already_allowed_models.append(model_name)
                    else:
                        not_found_models.append(model_name)

                # Save config if any models were added
                if added_models:
                    config.save()

                # Prepare response message
                response_parts = []

                if added_models:
                    if len(added_models) == 1:
                        response_parts.append(f"Model '{added_models[0]}' added to allowed models list.")
                    else:
                        response_parts.append(f"Models added to allowed list: {', '.join(added_models)}")

                if already_allowed_models:
                    if len(already_allowed_models) == 1:
                        response_parts.append(
                            f"Model '{already_allowed_models[0]}' was already in the allowed models list.")
                    else:
                        response_parts.append(f"Models already in allowed list: {', '.join(already_allowed_models)}")

                if not_found_models:
                    if len(not_found_models) == 1:
                        response_parts.append(
                            f"Model '{not_found_models[0]}' not found. Available models: {', '.join(available_model_names)}")
                    else:
                        response_parts.append(
                            f"Models not found: {', '.join(not_found_models)}. Available models: {', '.join(available_model_names)}")

                msg = "\n".join(response_parts)
                await ctx.send(msg)
                log_activity("COMMAND RESPONSE", bot.user, f"allowmodel {' '.join(models)}", msg, guild_id=ctx.guild.id)
            else:
                error_msg = f"Error: Received status code {response.status} from Ollama API"
                await ctx.send(error_msg)
                log_activity("ERROR", bot.user, f"allowmodel {' '.join(models)}", error_msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"allowmodel {' '.join(models)}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle disallowmodel command with multiple models
async def handle_disallowmodel_command(ctx, models):
    # Check if user has permission to manage config
    if not has_permission(ctx, "manage_config"):
        log_activity("PERMISSION DENIED", ctx.author, f"disallowmodel {' '.join(models)}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to manage allowed models.")
        return

    log_activity("COMMAND", ctx.author, f"disallowmodel {' '.join(models)}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        removed_models = []
        not_allowed_models = []

        for model_name in models:
            # Remove from allowed models if present
            if model_name in server_config.allowed_models:
                server_config.allowed_models.remove(model_name)
                removed_models.append(model_name)
            else:
                not_allowed_models.append(model_name)

        # Save config if any models were removed
        if removed_models:
            config.save()

        # Prepare response message
        response_parts = []

        if removed_models:
            if len(removed_models) == 1:
                response_parts.append(f"Model '{removed_models[0]}' removed from allowed models list.")
            else:
                response_parts.append(f"Models removed from allowed list: {', '.join(removed_models)}")

        if not_allowed_models:
            if len(not_allowed_models) == 1:
                response_parts.append(f"Model '{not_allowed_models[0]}' was not in the allowed models list.")
            else:
                response_parts.append(f"Models not in allowed list: {', '.join(not_allowed_models)}")

        msg = "\n".join(response_parts)
        await ctx.send(msg)
        log_activity("COMMAND RESPONSE", bot.user, f"disallowmodel {' '.join(models)}", msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"disallowmodel {' '.join(models)}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle allowallmodels command
async def handle_allowallmodels_command(ctx):
    # Check if user has permission to manage config
    if not has_permission(ctx, "manage_config"):
        log_activity("PERMISSION DENIED", ctx.author, "allowallmodels", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to manage allowed models.")
        return

    log_activity("COMMAND", ctx.author, "allowallmodels", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        server_config.allowed_models = []  # Empty list means all models are allowed
        config.save()
        msg = "All models are now allowed."
        await ctx.send(msg)
        log_activity("COMMAND RESPONSE", bot.user, "allowallmodels", msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, "allowallmodels", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle addrole command with multiple roles
async def handle_addrole_command(ctx, permission_type, role_names):
    # Check if user has permission to manage config
    if not has_permission(ctx, "manage_config"):
        log_activity("PERMISSION DENIED", ctx.author, f"addrole {permission_type} {role_names}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to manage role permissions.")
        return

    log_activity("COMMAND", ctx.author, f"addrole {permission_type} {role_names}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        # Check if permission type is valid
        if permission_type not in server_config.permissions:
            await ctx.send(f"Invalid permission type. Valid types: {', '.join(server_config.permissions.keys())}")
            return

        # Parse role names (they might contain spaces)
        # First try to find exact matches, then try partial matches
        role_name_list = [name.strip() for name in role_names.split(',') if name.strip()]
        if len(role_name_list) == 0:
            # If no commas, try to match roles by name
            role_name_list = [role_names]

        added_roles = []
        already_added_roles = []
        not_found_roles = []

        for role_name in role_name_list:
            # Try to find the role using the improved function
            role = find_role_by_name(ctx.guild, role_name)

            if role:
                # Add role to permission if not already there
                if role.id not in server_config.permissions[permission_type]:
                    server_config.permissions[permission_type].append(role.id)
                    added_roles.append(role.name)
                else:
                    already_added_roles.append(role.name)
            else:
                not_found_roles.append(role_name)

        # Save config if any roles were added
        if added_roles:
            config.save()

        # Prepare response message
        response_parts = []

        if added_roles:
            if len(added_roles) == 1:
                response_parts.append(f"Role '{added_roles[0]}' added to {permission_type} permission.")
            else:
                response_parts.append(f"Roles added to {permission_type} permission: {', '.join(added_roles)}")

        if already_added_roles:
            if len(already_added_roles) == 1:
                response_parts.append(f"Role '{already_added_roles[0]}' already had {permission_type} permission.")
            else:
                response_parts.append(
                    f"Roles that already had {permission_type} permission: {', '.join(already_added_roles)}")

        if not_found_roles:
            if len(not_found_roles) == 1:
                response_parts.append(f"Role '{not_found_roles[0]}' not found.")
            else:
                response_parts.append(f"Roles not found: {', '.join(not_found_roles)}")

        msg = "\n".join(response_parts)
        await ctx.send(msg)
        log_activity("COMMAND RESPONSE", bot.user, f"addrole {permission_type} {role_names}", msg,
                     guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"addrole {permission_type} {role_names}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle removerole command with multiple roles
async def handle_removerole_command(ctx, permission_type, role_names):
    # Check if user has permission to manage config
    if not has_permission(ctx, "manage_config"):
        log_activity("PERMISSION DENIED", ctx.author, f"removerole {permission_type} {role_names}",
                     guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to manage role permissions.")
        return

    log_activity("COMMAND", ctx.author, f"removerole {permission_type} {role_names}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        # Check if permission type is valid
        if permission_type not in server_config.permissions:
            await ctx.send(f"Invalid permission type. Valid types: {', '.join(server_config.permissions.keys())}")
            return

        # Parse role names (they might contain spaces)
        # First try to find exact matches, then try partial matches
        role_name_list = [name.strip() for name in role_names.split(',') if name.strip()]
        if len(role_name_list) == 0:
            # If no commas, try to match roles by name
            role_name_list = [role_names]

        removed_roles = []
        not_found_roles = []
        not_had_permission_roles = []

        for role_name in role_name_list:
            # Try to find the role using the improved function
            role = find_role_by_name(ctx.guild, role_name)

            if role:
                # Remove role from permission if present
                if role.id in server_config.permissions[permission_type]:
                    server_config.permissions[permission_type].remove(role.id)
                    removed_roles.append(role.name)
                else:
                    not_had_permission_roles.append(role.name)
            else:
                not_found_roles.append(role_name)

        # Save config if any roles were removed
        if removed_roles:
            config.save()

        # Prepare response message
        response_parts = []

        if removed_roles:
            if len(removed_roles) == 1:
                response_parts.append(f"Role '{removed_roles[0]}' removed from {permission_type} permission.")
            else:
                response_parts.append(f"Roles removed from {permission_type} permission: {', '.join(removed_roles)}")

        if not_had_permission_roles:
            if len(not_had_permission_roles) == 1:
                response_parts.append(
                    f"Role '{not_had_permission_roles[0]}' did not have {permission_type} permission.")
            else:
                response_parts.append(
                    f"Roles that did not have {permission_type} permission: {', '.join(not_had_permission_roles)}")

        if not_found_roles:
            if len(not_found_roles) == 1:
                response_parts.append(f"Role '{not_found_roles[0]}' not found.")
            else:
                response_parts.append(f"Roles not found: {', '.join(not_found_roles)}")

        msg = "\n".join(response_parts)
        await ctx.send(msg)
        log_activity("COMMAND RESPONSE", bot.user, f"removerole {permission_type} {role_names}", msg,
                     guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"removerole {permission_type} {role_names}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle listroles command
async def handle_listroles_command(ctx, permission_type=None):
    log_activity("COMMAND", ctx.author, f"listroles {permission_type if permission_type else 'all'}",
                 guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        if permission_type and permission_type not in server_config.permissions:
            await ctx.send(f"Invalid permission type. Valid types: {', '.join(server_config.permissions.keys())}")
            return

        response = []

        if permission_type:
            # List roles for specific permission type
            role_ids = server_config.permissions[permission_type]
            roles = []
            for role_id in role_ids:
                role = ctx.guild.get_role(role_id)
                if role:
                    roles.append(role.name)
                else:
                    # Role no longer exists, remove it from config
                    server_config.permissions[permission_type].remove(role_id)
                    config.save()

            if roles:
                response.append(f"**{permission_type}** permission roles: {', '.join(roles)}")
            else:
                if permission_type in ["set_model", "manage_config"]:
                    response.append(
                        f"**{permission_type}** permission: No roles assigned (only server owner and administrators)")
                elif permission_type == "reply_to":
                    response.append(f"**{permission_type}** permission: No roles assigned (replies to everyone)")
                else:
                    response.append(f"**{permission_type}** permission: No roles assigned")
        else:
            # List roles for all permission types
            for perm_type, role_ids in server_config.permissions.items():
                roles = []
                for role_id in role_ids:
                    role = ctx.guild.get_role(role_id)
                    if role:
                        roles.append(role.name)
                    else:
                        # Role no longer exists, remove it from config
                        server_config.permissions[perm_type].remove(role_id)
                        config.save()

                if roles:
                    response.append(f"**{perm_type}** permission roles: {', '.join(roles)}")
                else:
                    if perm_type in ["set_model", "manage_config"]:
                        response.append(
                            f"**{perm_type}** permission: No roles assigned (only server owner and administrators)")
                    elif perm_type == "reply_to":
                        response.append(f"**{perm_type}** permission: No roles assigned (replies to everyone)")
                    else:
                        response.append(f"**{perm_type}** permission: No roles assigned")

        # Send paginated response
        await send_paginated_response(ctx, "\n".join(response), ctx.author.id)

        log_activity("COMMAND RESPONSE", bot.user, f"listroles {permission_type if permission_type else 'all'}",
                     "\n".join(response), guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"listroles {permission_type if permission_type else 'all'}", error_msg,
                     guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle resetpermissions command
async def handle_resetpermissions_command(ctx, permission_type=None):
    # Only server owner can reset permissions
    if ctx.guild and ctx.author.id != ctx.guild.owner_id:
        log_activity("PERMISSION DENIED", ctx.author,
                     f"resetpermissions {permission_type if permission_type else 'all'}", guild_id=ctx.guild.id)
        await ctx.send("Only the server owner can reset permissions.")
        return

    log_activity("COMMAND", ctx.author, f"resetpermissions {permission_type if permission_type else 'all'}",
                 guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        if permission_type:
            if permission_type not in server_config.permissions:
                await ctx.send(f"Invalid permission type. Valid types: {', '.join(server_config.permissions.keys())}")
                return

            # Reset specific permission type
            server_config.permissions[permission_type] = []
            config.save()

            if permission_type in ["set_model", "manage_config"]:
                msg = f"Reset {permission_type} permission to default (only server owner and administrators)."
            elif permission_type == "reply_to":
                msg = f"Reset {permission_type} permission to default (replies to everyone)."
            else:
                msg = f"Reset {permission_type} permission to default."
        else:
            # Reset all permissions
            server_config.permissions = {
                "set_model": [],  # Role IDs that can change the model
                "manage_config": [],  # Role IDs that can manage configuration
                "reply_to": []  # Role IDs the bot will reply to (empty means everyone)
            }
            config.save()
            msg = "Reset all permissions to default."

        await ctx.send(msg)
        log_activity("COMMAND RESPONSE", bot.user, f"resetpermissions {permission_type if permission_type else 'all'}",
                     msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"resetpermissions {permission_type if permission_type else 'all'}", error_msg,
                     guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle setbotname command
async def handle_setbotname_command(ctx, new_name):
    # Check if user has permission to manage config
    if not has_permission(ctx, "manage_config"):
        log_activity("PERMISSION DENIED", ctx.author, f"setbotname {new_name}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to change the bot's name.")
        return

    log_activity("COMMAND", ctx.author, f"setbotname {new_name}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        # Update the bot's nickname in the current guild
        await ctx.guild.me.edit(nick=new_name)

        # Save the nickname to config
        server_config.bot_nickname = new_name
        config.save()

        msg = f"Bot nickname changed to '{new_name}'"
        await ctx.send(msg)
        log_activity("COMMAND RESPONSE", bot.user, f"setbotname {new_name}", msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"setbotname {new_name}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle resetbotname command
async def handle_resetbotname_command(ctx):
    # Check if user has permission to manage config
    if not has_permission(ctx, "manage_config"):
        log_activity("PERMISSION DENIED", ctx.author, "resetbotname", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to change the bot's name.")
        return

    log_activity("COMMAND", ctx.author, "resetbotname", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        # Reset the bot's nickname in the current guild
        await ctx.guild.me.edit(nick=None)

        # Save the nickname to config
        server_config.bot_nickname = None
        config.save()

        msg = "Bot nickname reset to default"
        await ctx.send(msg)
        log_activity("COMMAND RESPONSE", bot.user, "resetbotname", msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, "resetbotname", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle randomreplies command
async def handle_randomreplies_command(ctx, args):
    # Check if user has permission to manage config
    if not has_permission(ctx, "manage_config"):
        log_activity("PERMISSION DENIED", ctx.author, f"randomreplies {' '.join(args)}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to manage random replies.")
        return

    log_activity("COMMAND", ctx.author, f"randomreplies {' '.join(args)}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        if not args:
            await ctx.send("Please provide a parameter (enable/disable/status/probability/cooldown).")
            return

        param = args[0].lower()

        if param == "enable":
            server_config.random_replies["enabled"] = True
            config.save()
            msg = "Random replies enabled."

        elif param == "disable":
            server_config.random_replies["enabled"] = False
            config.save()
            msg = "Random replies disabled."

        elif param == "status":
            status = "enabled" if server_config.random_replies["enabled"] else "disabled"
            probability = server_config.random_replies["probability"] * 100
            cooldown = server_config.random_replies["cooldown"]

            msg = f"Random replies are currently {status}.\n"
            msg += f"Probability: {probability}%\n"
            msg += f"Cooldown: {cooldown} seconds"

        elif param == "probability":
            if len(args) < 2:
                await ctx.send("Please provide a probability value between 0.0 and 1.0.")
                return

            try:
                prob = float(args[1])
                if 0.0 <= prob <= 1.0:
                    server_config.random_replies["probability"] = prob
                    config.save()
                    msg = f"Random reply probability set to {prob * 100}%."
                else:
                    msg = "Probability must be between 0.0 and 1.0."
            except ValueError:
                msg = "Invalid probability value. Please provide a number between 0.0 and 1.0."

        elif param == "cooldown":
            if len(args) < 2:
                await ctx.send("Please provide a cooldown value in seconds.")
                return

            try:
                cooldown = int(args[1])
                if cooldown >= 0:
                    server_config.random_replies["cooldown"] = cooldown
                    config.save()
                    msg = f"Random reply cooldown set to {cooldown} seconds."
                else:
                    msg = "Cooldown must be a positive number."
            except ValueError:
                msg = "Invalid cooldown value. Please provide a positive number."

        else:
            msg = "Invalid parameter. Valid parameters: enable, disable, status, probability, cooldown."

        await ctx.send(msg)
        log_activity("COMMAND RESPONSE", bot.user, f"randomreplies {' '.join(args)}", msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"randomreplies {' '.join(args)}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle system instructions command
async def handle_system_instructions_command(ctx, instructions):
    # Check if user has permission to manage config
    if not has_permission(ctx, "manage_config"):
        log_activity("PERMISSION DENIED", ctx.author, f"system {instructions}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to manage system instructions.")
        return

    log_activity("COMMAND", ctx.author, f"system {instructions}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        # Check for special commands
        if instructions.lower() == "show":
            if server_config.system_instructions:
                msg = f"Current system instructions:\n\n\`\`\`\n{server_config.system_instructions}\n\`\`\`"
            else:
                msg = "No system instructions are currently set."

            await ctx.send(msg)
            log_activity("COMMAND RESPONSE", bot.user, "system show", msg, guild_id=ctx.guild.id)
            return

        elif instructions.lower() == "reset":
            server_config.system_instructions = ""
            config.save()
            msg = "System instructions have been reset."

            await ctx.send(msg)
            log_activity("COMMAND RESPONSE", bot.user, "system reset", msg, guild_id=ctx.guild.id)
            return

        # If no special command, set the instructions
        if instructions:
            server_config.system_instructions = instructions
            config.save()
            msg = "System instructions have been set."
        else:
            msg = "Please provide instructions or use 'show' to view current instructions or 'reset' to clear them."

        await ctx.send(msg)
        log_activity("COMMAND RESPONSE", bot.user, f"system {instructions}", msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"system {instructions}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle pagination command
async def handle_pagination_command(ctx, args):
    # Check if user has permission to manage config
    if not has_permission(ctx, "manage_config"):
        log_activity("PERMISSION DENIED", ctx.author, f"pagination {' '.join(args)}", guild_id=ctx.guild.id)
        await ctx.send("You don't have permission to manage pagination settings.")
        return

    log_activity("COMMAND", ctx.author, f"pagination {' '.join(args)}", guild_id=ctx.guild.id)

    # Set active command
    set_active_command(ctx.author.id)

    try:
        # Get server-specific config
        server_config = config.get_server_config(ctx.guild.id)

        if not args:
            await ctx.send("Please provide a parameter (enable/disable/status/pagesize).")
            return

        param = args[0].lower()

        if param == "enable":
            server_config.paginated_responses["enabled"] = True
            config.save()
            msg = "Paginated responses enabled."

        elif param == "disable":
            server_config.paginated_responses["enabled"] = False
            config.save()
            msg = "Paginated responses disabled."

        elif param == "status":
            status = "enabled" if server_config.paginated_responses["enabled"] else "disabled"
            page_size = server_config.paginated_responses["page_size"]

            msg = f"Paginated responses are currently {status}.\n"
            msg += f"Page size: {page_size} characters"

        elif param == "pagesize":
            if len(args) < 2:
                await ctx.send("Please provide a page size value in characters.")
                return

            try:
                page_size = int(args[1])
                if 100 <= page_size <= 2000:
                    server_config.paginated_responses["page_size"] = page_size
                    config.save()
                    msg = f"Page size set to {page_size} characters."
                else:
                    msg = "Page size must be between 100 and 2000 characters."
            except ValueError:
                msg = "Invalid page size value. Please provide a number between 100 and 2000."

        else:
            msg = "Invalid parameter. Valid parameters: enable, disable, status, pagesize."

        await ctx.send(msg)
        log_activity("COMMAND RESPONSE", bot.user, f"pagination {' '.join(args)}", msg, guild_id=ctx.guild.id)

    except Exception as e:
        error_msg = f"Error: {str(e)}"
        await ctx.send(error_msg)
        log_activity("ERROR", bot.user, f"pagination {' '.join(args)}", error_msg, guild_id=ctx.guild.id)

    finally:
        # Clear active command
        clear_active_command(ctx.author.id)


# Helper function to handle RBND-NC license command
async def handle_rbnd_nc_command(ctx):
    """Display the RBND-NC license text"""
    license_text = """RBND-NC License

Copyright (c) RobotsNeverDie
All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to use, copy, modify, and merge copies of the Software, subject to the following conditions:

Non-Commercial Use Only
The Software may not be used, either in whole or in part, for any commercial purposes. Commercial purposes include, but are not limited to, selling, licensing, or offering the Software or derivative works for a fee, using it as part of a service for which payment is received, or using it within a product offered commercially.

Attribution and Source Linking
Any redistribution or sharing of this Software, modified or unmodified, must include clear and visible attribution to the original author(s) and a working link to the original source repository or website: [Your Website or Repository URL].

No Sublicensing
You may not sublicense the Software. Any person who receives a copy must also comply with this license directly.

Revocation of Rights
The licensor reserves the right to revoke this license and any associated permissions to use, modify, or distribute the Software at any time, for any reason or no reason, at their sole discretion. Upon revocation, all use, modification, and distribution of the Software must cease immediately.

No Warranty
The Software is provided "as is", without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and noninfringement. In no event shall the authors be liable for any claim, damages, or other liability arising from the use of the Software.

By using, modifying, or distributing the Software, you agree to the terms of this license."""
    
    # Send paginated response
    await send_paginated_response(ctx, license_text, ctx.author.id)
    log_activity("COMMAND RESPONSE", bot.user, "RBND-NC", "License text displayed", guild_id=ctx.guild.id)

# Command decorator to check for active commands
def cooldown():
    async def predicate(ctx):
        if has_active_command(ctx.author.id):
            await ctx.send("I'm still processing your previous request. Please wait.")
            return False
        set_active_command(ctx.author.id)
        return True

    return commands.check(predicate)


@bot.command(name="ask")
@cooldown()
async def ask(ctx, *, question):
    """Ask a question to the Ollama model"""
    await handle_ask_command(ctx, question)


@bot.command(name="models")
@cooldown()
async def list_models(ctx):
    """List available models in Ollama"""
    await handle_models_command(ctx.message)


@bot.command(name="setmodel")
@cooldown()
async def set_model(ctx, model_name):
    """Set the default model to use"""
    await handle_setmodel_command(ctx, model_name)


@bot.command(name="search")
@cooldown()
async def search_messages(ctx, *, query):
    """Search for messages in the channel containing the query"""
    await handle_search_command(ctx, query)


@bot.command(name="reference")
@cooldown()
async def reference_message(ctx, message_id: int, *, prompt=None):
    """Reference a specific message and ask the AI about it"""
    await handle_reference_command(ctx, message_id, prompt)


@bot.command(name="allowmodel")
@cooldown()
async def allow_model(ctx, *models):
    """Add models to the allowed models list"""
    if not models:
        await ctx.send("Please provide at least one model name.")
        clear_active_command(ctx.author.id)
        return

    await handle_allowmodel_command(ctx, models)


@bot.command(name="disallowmodel")
@cooldown()
async def disallow_model(ctx, *models):
    """Remove models from the allowed models list"""
    if not models:
        await ctx.send("Please provide at least one model name.")
        clear_active_command(ctx.author.id)
        return

    await handle_disallowmodel_command(ctx, models)


@bot.command(name="allowallmodels")
@cooldown()
async def allow_all_models(ctx):
    """Allow all models to be used"""
    await handle_allowallmodels_command(ctx)


@bot.command(name="addrole")
@cooldown()
async def add_role(ctx, permission_type, *, role_names):
    """Add roles to a permission type"""
    await handle_addrole_command(ctx, permission_type, role_names)


@bot.command(name="removerole")
@cooldown()
async def remove_role(ctx, permission_type, *, role_names):
    """Remove roles from a permission type"""
    await handle_removerole_command(ctx, permission_type, role_names)


@bot.command(name="listroles")
@cooldown()
async def list_roles(ctx, permission_type=None):
    """List roles for a permission type or all permission types"""
    await handle_listroles_command(ctx, permission_type)


@bot.command(name="resetpermissions")
@cooldown()
async def reset_permissions(ctx, permission_type=None):
    """Reset permissions to default (only server owner)"""
    await handle_resetpermissions_command(ctx, permission_type)


@bot.command(name="setbotname")
@cooldown()
async def set_bot_name(ctx, *, new_name):
    """Set a custom nickname for the bot"""
    await handle_setbotname_command(ctx, new_name)


@bot.command(name="resetbotname")
@cooldown()
async def reset_bot_name(ctx):
    """Reset the bot's nickname to default"""
    await handle_resetbotname_command(ctx)


@bot.command(name="randomreplies")
@cooldown()
async def random_replies(ctx, *args):
    """Configure random replies"""
    await handle_randomreplies_command(ctx, args)


@bot.command(name="system", aliases=["systeminstructions"])
@cooldown()
async def system_instructions(ctx, *, instructions=""):
    """Set system instructions for the model"""
    await handle_system_instructions_command(ctx, instructions)


@bot.command(name="pagination")
@cooldown()
async def pagination(ctx, *args):
    """Configure paginated responses"""
    await handle_pagination_command(ctx, args)


@bot.command(name="help")
@cooldown()
async def help_command(ctx):
    """Show help information for all commands"""
    await handle_help_command(ctx.message)

@bot.command(name="RBND-NC")
@cooldown()
async def rbnd_nc_license(ctx):
    """Display the RBND-NC license"""
    await handle_rbnd_nc_command(ctx)

# Error handler for commands
@bot.event
async def on_command_error(ctx, error):
    if isinstance(error, commands.CheckFailure):
        # This is triggered by our cooldown check
        pass  # We already sent a message in the check
    elif isinstance(error, commands.CommandNotFound):
        pass  # Ignore command not found errors
    else:
        # Log other errors
        log_activity("ERROR", ctx.author, f"{ctx.message.content}", str(error),
                     guild_id=ctx.guild.id if ctx.guild else None)
        await ctx.send(f"An error occurred: {str(error)}")

        # Make sure to clear active command in case of error
        if ctx.author.id in active_commands:
            clear_active_command(ctx.author.id)


# Clean up aiohttp session on bot shutdown
@bot.event
async def on_close():
    if session:
        await session.close()
        print("Closed aiohttp session")


# Replace 'YOUR_DISCORD_BOT_TOKEN' with your actual bot token
bot.run(os.environ.get('DISCORD_TOKEN', 'INSERT_YOUR_PRIVATE_DISCORD_TOKEN_HERE'))