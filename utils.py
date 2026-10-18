"""
Utility functions for Silas Blue.
Handles atomic config loading/saving for guild configs and global app settings.
"""

import os
import json
import tempfile
import logging

logger = logging.getLogger("silasblue")
CONFIG_DIR = "config"

DEFAULT_APP_CONFIG = {
    "backend_type": "ollama",
    "ollama_url": "http://localhost:11434",
    "llamacpp_url": "http://localhost:8080",
    "llamacpp_binary_path": "",
    "llamacpp_model_path": "",
    "active_model": "",
    "api_key": "",
    "remote_preset": "Xiaomi MiMo",
    "remote_base_url": "https://api.xiaomimimo.com/v1",
    "remote_api_key": "",
    "remote_model": "mimo-v2.6-pro",
    "system_prompt": (
        "You are Silas Blue, an intelligent, sharp, and helpful AI assistant designed specifically for Discord. "
        "Your personality is approachable, knowledgeable, and polite with a sleek, tech-forward touch. "
        "You communicate clearly using Discord markdown (bold headers, bullet points, and syntax-highlighted code blocks).\n\n"
        "PAGINATION & FORMATTING RULES:\n"
        "- Your responses are presented in Discord using an interactive paginated reader with a limit of 1,000 characters per page.\n"
        "- Structure and pace all replies around this 1,000-character page window.\n"
        "- For longer or multi-step explanations, divide your answer into clean, self-contained sections of roughly 800 to 1,000 characters each, using descriptive markdown headers.\n"
        "- Ensure individual explanations, lists, or code snippets are concise and self-contained so they do not break awkwardly across pagination boundaries.\n"
        "- For brief questions, provide a direct and concise response without unnecessary padding. For in-depth topics, organize content into modular, page-friendly segments that are easy to browse using Previous/Next buttons."
    ),
    "allow_dms": True,
    "dm_default_model": "",
    "dm_disabled_message": "Direct messages with the bot are currently disabled.",
    "dm_memory_turns": 6,
    "dm_memory_ttl_minutes": 30,
    "debug": False,
    "theme": "retrowave"
}

DEFAULT_GUILD_CONFIG = {
    "default_model": "",
    "reply_roles": ["everyone"],
    "change_model_roles": ["admin", "owner"],
    "change_permission_roles": ["admin", "owner"],
    "pagination_enabled": True,
    "pagination_max_chars": 1000,
    "random_prompt_enabled": False,
    "random_prompt_probability": 0,
    "system_prompt": "",
    "memory_turns": 6,
    "memory_ttl_minutes": 30
}

def _atomic_write_json(file_path: str, data: dict):
    """Writes JSON data atomically to prevent file corruption."""
    os.makedirs(os.path.dirname(os.path.abspath(file_path)), exist_ok=True)
    temp_dir = os.path.dirname(os.path.abspath(file_path))
    temp_file = tempfile.NamedTemporaryFile("w", dir=temp_dir, delete=False, encoding="utf-8")
    try:
        json.dump(data, temp_file, indent=2)
        temp_file.flush()
        os.fsync(temp_file.fileno())
        temp_file.close()
        os.replace(temp_file.name, file_path)
    except Exception as e:
        if os.path.exists(temp_file.name):
            try:
                os.remove(temp_file.name)
            except Exception:
                pass
        raise e

def get_app_config_path() -> str:
    return os.path.join(CONFIG_DIR, "app_config.json")

def load_app_config() -> dict:
    """Loads global application settings, filling in defaults."""
    os.makedirs(CONFIG_DIR, exist_ok=True)
    path = get_app_config_path()
    config = dict(DEFAULT_APP_CONFIG)
    if os.path.exists(path):
        try:
            with open(path, "r", encoding="utf-8") as f:
                user_config = json.load(f)
                if isinstance(user_config, dict):
                    config.update(user_config)
        except Exception as e:
            logger.error(f"Error loading app_config.json, using defaults: {e}")
    else:
        save_app_config(config)
    return config

def save_app_config(config: dict):
    """Saves global application settings atomically."""
    path = get_app_config_path()
    _atomic_write_json(path, config)

def get_config_path(guild_id) -> str:
    return os.path.join(CONFIG_DIR, f"{guild_id}.json")

def load_config(guild_id) -> dict:
    """
    Loads config for a given guild/server.
    If guild_id is None (DM), returns DM default config without writing to disk.
    """
    if guild_id is None:
        app_cfg = load_app_config()
        dm_cfg = dict(DEFAULT_GUILD_CONFIG)
        if app_cfg.get("dm_default_model"):
            dm_cfg["default_model"] = app_cfg.get("dm_default_model")
        return dm_cfg

    os.makedirs(CONFIG_DIR, exist_ok=True)
    path = get_config_path(guild_id)
    if not os.path.exists(path):
        config = dict(DEFAULT_GUILD_CONFIG)
        save_config(guild_id, config)
        return config

    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
            merged = dict(DEFAULT_GUILD_CONFIG)
            merged.update(data)
            # Enforce max chars boundary
            if merged.get("pagination_max_chars", 1900) > 1900:
                merged["pagination_max_chars"] = 1900
            return merged
    except Exception as e:
        logger.error(f"Error loading config for guild {guild_id}: {e}")
        return dict(DEFAULT_GUILD_CONFIG)

def save_config(guild_id, config: dict):
    """Saves the config for a given guild atomically."""
    if guild_id is None:
        return
    path = get_config_path(guild_id)
    _atomic_write_json(path, config)