# Configuration module for Silas Blue
# This module contains shared configuration to avoid circular imports

import asyncio
import aiohttp
import gc
import os
import pickle

# Version information
VERSION = "1.2.1"

# The model to use - change this to any model you have pulled in Ollama
DEFAULT_MODEL = "llama3"

# Configuration file path
CONFIG_FILE = "bot_config.pkl"
TOKEN_FILE = "token.txt"

# Ollama API endpoint (default for local installation)
OLLAMA_API_URL = "http://localhost:11434/api/generate"
OLLAMA_TAGS_URL = "http://localhost:11434/api/tags"

# Track active commands per user to implement cooldowns
active_commands = {}

# Flag to indicate if the bot should restart
should_restart = False
# Flag to indicate if the bot should shut down
should_shutdown = False

# Global aiohttp session
session = None

# Discord bot reference (will be set by SilasBlue.py)
bot = None


#Force close all aiohttp sessions to fix restart issue
def force_close_sessions():
    """Force close all aiohttp sessions"""
    # Get all ClientSession objects
    for obj in gc.get_objects():
        if isinstance(obj, aiohttp.ClientSession) and not obj.closed:
            try:
                # Create a new event loop for closing the session
                loop = asyncio.new_event_loop()
                asyncio.set_event_loop(loop)
                loop.run_until_complete(obj.close())
                loop.close()
                print("Closed an aiohttp session")
            except Exception as e:
                print(f"Error closing session: {e}")


# Function to change Discord token
def change_discord_token(new_token=None):
    """Change the Discord token"""
    if not new_token:
        print("Change Discord Token")
        new_token = input("Enter new Discord token: ").strip()

    if not new_token:
        print("Token cannot be empty")
        return False

    # Save the new token to file
    with open(TOKEN_FILE, 'w') as f:
        f.write(new_token)

    print("Token updated. Restart the bot for changes to take effect.")
    return True


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


# Function to get Discord token
def get_discord_token():
    """Get Discord token from file or prompt user to enter it"""
    import pathlib
    
    token_path = pathlib.Path(TOKEN_FILE)

    if token_path.exists():
        with open(token_path, 'r') as f:
            token = f.read().strip()
            if token:
                return token

    # If we get here, either the file doesn't exist or it's empty
    print("Discord Token Setup")
    print("No Discord token found. Please enter your Discord bot token:")
    token = input("Token: ").strip()

    # Save the token to file
    with open(token_path, 'w') as f:
        f.write(token)

    print(f"Token saved to {TOKEN_FILE}")
    return token


# Initialize configuration
config = BotConfig.load()