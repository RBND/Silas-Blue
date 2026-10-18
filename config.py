"""
Configuration proxy for Silas Blue.
Provides backwards-compatible access to global settings from config/app_config.json.
"""

from utils import load_app_config, save_app_config

_DEBUG_CACHE = None

def get_debug() -> bool:
    global _DEBUG_CACHE
    if _DEBUG_CACHE is not None:
        return _DEBUG_CACHE
    try:
        _DEBUG_CACHE = load_app_config().get("debug", False)
        return _DEBUG_CACHE
    except Exception:
        return False

def set_debug(val: bool):
    global _DEBUG_CACHE
    try:
        _DEBUG_CACHE = bool(val)
        cfg = load_app_config()
        cfg["debug"] = bool(val)
        save_app_config(cfg)
    except Exception:
        pass

def invalidate_debug_cache():
    global _DEBUG_CACHE
    _DEBUG_CACHE = None

class ConfigModule:
    @property
    def DEBUG(self) -> bool:
        return get_debug()

    @DEBUG.setter
    def DEBUG(self, val: bool):
        set_debug(val)

    def invalidate_cache(self):
        invalidate_debug_cache()

import sys
sys.modules[__name__] = ConfigModule()
