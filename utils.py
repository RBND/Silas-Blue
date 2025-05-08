"""
Utility functions for Silas Blue.
Handles config loading/saving and paths.
"""

import os
import json
from ollama_api import OllamaClient

CONFIG_DIR = "config"

def get_config_path(guild_id):
    return os.path.join(CONFIG_DIR, f"{guild_id}.json")

def load_config(guild_id):
    """
    Loads the config for a given guild/server.
    Ensures default_model is set to an available model if possible.
    """
    os.makedirs(CONFIG_DIR, exist_ok=True)
    path = get_config_path(guild_id)
    ollama = OllamaClient()
    available_models = ollama.list_models()
    config = None
    if not os.path.exists(path):
        # Default config
        config = {
            "default_model": available_models[0] if available_models else "",
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
        config = json.load(f)
    # Ensure default_model is set and valid
    if ("default_model" not in config or not config["default_model"] or config["default_model"] not in available_models) and available_models:
        config["default_model"] = available_models[0]
        save_config(guild_id, config)
    return config

def save_config(guild_id, config):
    """
    Saves the config for a given guild/server.
    """
    os.makedirs(CONFIG_DIR, exist_ok=True)
    path = get_config_path(guild_id)
    with open(path, "w") as f:
        json.dump(config, f, indent=2)

def get_app_config_path():
    return os.path.join(CONFIG_DIR, "app_config.json")

def load_app_config():
    os.makedirs(CONFIG_DIR, exist_ok=True)
    path = get_app_config_path()
    if not os.path.exists(path):
        # Default app config
        config = {"theme": "retrowave"}
        save_app_config(config)
        return config
    with open(path, "r") as f:
        return json.load(f)

def save_app_config(config):
    os.makedirs(CONFIG_DIR, exist_ok=True)
    path = get_app_config_path()
    with open(path, "w") as f:
        json.dump(config, f, indent=2)

# Utility to set the default model for a guild

def set_default_model(guild_id, model_name):
    config = load_config(guild_id)
    config["default_model"] = model_name
    save_config(guild_id, config) 