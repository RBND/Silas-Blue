"""
Configuration proxy for Silas Blue.
Provides backwards-compatible access to global settings from config/app_config.json.
"""

from utils import load_app_config, save_app_config

def get_debug() -> bool:
    try:
        return load_app_config().get("debug", False)
    except Exception:
        return False

def set_debug(val: bool):
    try:
        cfg = load_app_config()
        cfg["debug"] = bool(val)
        save_app_config(cfg)
    except Exception:
        pass

class ConfigModule:
    @property
    def DEBUG(self) -> bool:
        return get_debug()

    @DEBUG.setter
    def DEBUG(self, val: bool):
        set_debug(val)

import sys
sys.modules[__name__] = ConfigModule()
