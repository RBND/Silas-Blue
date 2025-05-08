"""
Utility functions for Silas Blue.
Handles config loading/saving and paths.
"""

import os
import json

CONFIG_DIR = "config"

def get_config_path(guild_id):
    return os.path.join(CONFIG_DIR, f"{guild_id}.json")

def load_config(guild_id):
    """
    Loads the config for a given guild/server.
    """
    os.makedirs(CONFIG_DIR, exist_ok=True)
    path = get_config_path(guild_id)
    if not os.path.exists(path):
        # Default config
        config = {
            "default_model": "llama2",
            "reply_roles": ["everyone"],
            "change_model_roles": ["admin", "owner"],
            "change_permission_roles": ["admin", "owner"],
            "pagination_enabled": True,
            "pagination_max_chars": 2000,
            "random_prompt_enabled": False,
            "random_prompt_probability": 0
        }
        save_config(guild_id, config)
        return config
    with open(path, "r") as f:
        return json.load(f)

def save_config(guild_id, config):
    """
    Saves the config for a given guild/server.
    """
    os.makedirs(CONFIG_DIR, exist_ok=True)
    path = get_config_path(guild_id)
    with open(path, "w") as f:
        json.dump(config, f, indent=2) 