"""
Silas Blue - Discord Bot powered by local AI models via Ollama or Llama.cpp
Main entry point: launches both the Discord bot and the GUI.
"""

import sys
import threading
import os
import logging

from gui.main_window import MainWindow
from PySide6.QtWidgets import QApplication, QSplashScreen
from PySide6.QtCore import QTimer
from PySide6.QtGui import QPixmap

from bot_core import start_bot, stop_bot, restart_bot, get_provider
from utils import load_app_config
import config

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("silasblue")

CRASH_COUNTER_FILE = os.path.join("config", "crash_counter.txt")

def get_crash_counter() -> int:
    try:
        with open(CRASH_COUNTER_FILE, "r", encoding="utf-8") as f:
            return int(f.read().strip())
    except Exception:
        return 0

def set_crash_counter(val: int):
    try:
        os.makedirs("config", exist_ok=True)
        with open(CRASH_COUNTER_FILE, "w", encoding="utf-8") as f:
            f.write(str(val))
    except Exception:
        pass

# Check if repeated crashes previously occurred
if get_crash_counter() >= 2:
    config.DEBUG = True
    logger.info("Debug mode auto-enabled due to previous crashes.")
    set_crash_counter(0)

def start_gui_and_bot():
    """Starts the PySide6 GUI and triggers bot startup."""
    app = QApplication(sys.argv)

    # Splash Screen
    splash = None
    splash_path = os.path.join("gui", "splash.png")
    if os.path.exists(splash_path):
        splash_pix = QPixmap(splash_path)
        splash = QSplashScreen(splash_pix)
        splash.show()
        app.processEvents()

    def show_main_window():
        window = MainWindow(start_bot=start_bot, stop_bot=stop_bot, restart_bot=restart_bot)
        window.show()
        if splash:
            splash.finish(window)
        window.redirect_output_to_log()
        # Start Discord bot in background
        QTimer.singleShot(100, start_bot)

    # Transition from splash to main window
    QTimer.singleShot(1500, show_main_window)
    exit_code = app.exec()
    set_crash_counter(0)  # Clean exit
    sys.exit(exit_code)

def ensure_ai_service_running():
    """
    Checks if active AI provider is running. If not, attempts to start it locally
    or auto-detects another running local AI backend (e.g. Llama.cpp or Ollama).
    """
    from utils import load_app_config, save_app_config
    from bot_core import reload_ai_provider
    from ai_providers.detector import detect_all_ai_providers

    provider = get_provider()

    # 1. If currently configured provider is already responsive, nothing to do
    if provider.status_sync():
        logger.info(f"{provider.name} service is running and responsive.")
        return

    # 2. Attempt to start configured local service
    if provider.is_local():
        logger.info(f"Local {provider.name} is not running. Attempting to start local {provider.name} service...")
        if provider.start_local_service():
            logger.info(f"Local {provider.name} service started successfully.")
            return
        else:
            logger.warning(f"Failed to start local {provider.name}.")

    # 3. If configured provider is still offline, check if another local AI backend is running
    logger.info("Scanning for active local AI backends...")
    detected = detect_all_ai_providers()

    app_cfg = load_app_config()
    current_backend = app_cfg.get("backend_type", "ollama").lower()

    if detected["llamacpp"]["running"] and current_backend != "llamacpp":
        llama_info = detected["llamacpp"]
        logger.info(f"Auto-detected running Llama.cpp instance at {llama_info['url']}! Switching backend...")
        app_cfg["backend_type"] = "llamacpp"
        app_cfg["llamacpp_url"] = llama_info["url"]
        if llama_info.get("binary_path"):
            app_cfg["llamacpp_binary_path"] = llama_info["binary_path"]
        if llama_info.get("model_path") and not app_cfg.get("llamacpp_model_path"):
            app_cfg["llamacpp_model_path"] = llama_info["model_path"]
        save_app_config(app_cfg)
        reload_ai_provider()
        logger.info("Successfully switched to Llama.cpp backend.")
    elif detected["ollama"]["running"] and current_backend != "ollama":
        ollama_info = detected["ollama"]
        logger.info(f"Auto-detected running Ollama instance at {ollama_info['url']}! Switching backend...")
        app_cfg["backend_type"] = "ollama"
        app_cfg["ollama_url"] = ollama_info["url"]
        save_app_config(app_cfg)
        reload_ai_provider()
        logger.info("Successfully switched to Ollama backend.")
    elif detected["llamacpp"]["running"] and current_backend == "llamacpp":
        # Update URL if port changed (e.g. 9931 vs 8080)
        llama_info = detected["llamacpp"]
        if app_cfg.get("llamacpp_url") != llama_info["url"]:
            logger.info(f"Updating Llama.cpp URL to active endpoint: {llama_info['url']}")
            app_cfg["llamacpp_url"] = llama_info["url"]
            if llama_info.get("binary_path") and not app_cfg.get("llamacpp_binary_path"):
                app_cfg["llamacpp_binary_path"] = llama_info["binary_path"]
            save_app_config(app_cfg)
            reload_ai_provider()

if __name__ == "__main__":
    try:
        threading.Thread(target=ensure_ai_service_running, daemon=True).start()
        start_gui_and_bot()
    except Exception as e:
        count = get_crash_counter() + 1
        set_crash_counter(count)
        logger.error(f"Silas Blue encountered an unhandled crash: {e}")
        raise
