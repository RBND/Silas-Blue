"""
Utility functions for Silas Blue.
Handles config loading/saving and paths.
"""

import os
import json
from ollama_api import OllamaClient
import psutil
import logging
try:
    import pynvml
    pynvml.nvmlInit()
    _NVML_AVAILABLE = True
except Exception:
    _NVML_AVAILABLE = False

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

# --- System Usage Utilities ---
def get_cpu_usage():
    """Returns CPU usage as a percentage (float)."""
    return psutil.cpu_percent(interval=0.2)

def get_memory_usage():
    """Returns (used_MB, total_MB, percent) for system memory."""
    mem = psutil.virtual_memory()
    used = mem.used / (1024 * 1024)
    total = mem.total / (1024 * 1024)
    percent = mem.percent
    return used, total, percent

def get_gpu_list():
    """Returns a list of GPU names if available, else empty list."""
    if not _NVML_AVAILABLE:
        return []
    try:
        count = pynvml.nvmlDeviceGetCount()
        return [pynvml.nvmlDeviceGetName(pynvml.nvmlDeviceGetHandleByIndex(i)).decode('utf-8') for i in range(count)]
    except Exception as e:
        logging.error(f"Error getting GPU list: {e}")
        return []

def get_gpu_usage():
    """Returns (gpu_percent, vram_used_MB, vram_total_MB, vram_percent, gpu_name) for the most used NVIDIA GPU, or None if unavailable."""
    if not _NVML_AVAILABLE:
        return None
    try:
        count = pynvml.nvmlDeviceGetCount()
        best = None
        for i in range(count):
            handle = pynvml.nvmlDeviceGetHandleByIndex(i)
            util = pynvml.nvmlDeviceGetUtilizationRates(handle)
            mem = pynvml.nvmlDeviceGetMemoryInfo(handle)
            gpu_percent = util.gpu
            vram_used = mem.used / (1024 * 1024)
            vram_total = mem.total / (1024 * 1024)
            vram_percent = (vram_used / vram_total) * 100 if vram_total else 0
            name = pynvml.nvmlDeviceGetName(handle).decode('utf-8')
            if best is None or gpu_percent > best[0]:
                best = (gpu_percent, vram_used, vram_total, vram_percent, name)
        return best if best else None
    except Exception as e:
        logging.error(f"Error getting GPU usage: {e}")
        return None 