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
    If configured for local Ollama, checks if Ollama service is active and starts it if needed.
    Does nothing if configured for a remote server or Llama.cpp.
    """
    provider = get_provider()
    if provider.name == "Ollama" and provider.is_local():
        if not provider.status_sync():
            logger.info("Local Ollama is not running. Attempting to start local Ollama service...")
            if provider.start_local_service():
                logger.info("Local Ollama service started successfully.")
            else:
                logger.warning("Failed to start local Ollama. The bot may not function until Ollama is started.")
        else:
            logger.info("Local Ollama service is running.")

if __name__ == "__main__":
    try:
        threading.Thread(target=ensure_ai_service_running, daemon=True).start()
        start_gui_and_bot()
    except Exception as e:
        count = get_crash_counter() + 1
        set_crash_counter(count)
        logger.error(f"Silas Blue encountered an unhandled crash: {e}")
        raise
