"""
Silas Blue - Discord Bot powered by local AI models via Ollama
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

from bot_core import start_bot, stop_bot, restart_bot
from ollama_api import OllamaClient
import config  # Changed from 'from config import DEBUG'

# Ensure logs directory exists
os.makedirs('logs', exist_ok=True)

# Set up logging to file
logging.basicConfig(
    filename='logs/silasblue.log',
    filemode='a',
    format='%(asctime)s %(levelname)s:%(message)s',
    level=logging.DEBUG  # or INFO, as needed
)

# Redirect stdout and stderr to log files
sys.stdout = open('logs/stdout.log', 'a')
sys.stderr = open('logs/stderr.log', 'a')

# Force the root logger's level based on config.DEBUG
logging.getLogger().setLevel(logging.DEBUG if getattr(config, 'DEBUG', False) else logging.INFO)

# --- Crash counter logic for auto-debug ---
CRASH_COUNTER_FILE = os.path.join('config', 'crash_counter.txt')
def get_crash_counter():
    try:
        with open(CRASH_COUNTER_FILE, 'r', encoding='utf-8') as f:
            return int(f.read().strip())
    except Exception:
        return 0

def set_crash_counter(val):
    try:
        with open(CRASH_COUNTER_FILE, 'w', encoding='utf-8') as f:
            f.write(str(val))
    except Exception:
        pass

# Only check and set debug mode if crash counter is high, but do not increment here
count = get_crash_counter()
if count >= 2:
    # Set DEBUG=True in config.py
    try:
        with open('config.py', 'w', encoding='utf-8') as f:
            f.write('DEBUG = True\n')
    except Exception:
        pass
    set_crash_counter(0)  # Reset after enabling debug

def start_gui_and_bot():
    """
    Starts the PySide6 GUI in the main thread and starts the bot after the event loop starts.
    """
    print("[DEBUG] Creating QApplication...")
    logging.debug("Creating QApplication...")
    app = QApplication(sys.argv)
    print("[DEBUG] QApplication created.")
    logging.debug("QApplication created.")
    # --- Splash Screen ---
    print("[DEBUG] Loading splash image...")
    splash_pix = QPixmap("gui/splash.png")
    splash = QSplashScreen(splash_pix)
    print("[DEBUG] Showing splash screen...")
    splash.show()
    app.processEvents()  # Ensure splash screen is shown
    print("[DEBUG] Splash screen shown.")

    def show_main_window():
        print("[DEBUG] Creating MainWindow...")
        logging.debug("Creating MainWindow...")
        window = MainWindow(start_bot=start_bot, stop_bot=stop_bot, restart_bot=restart_bot)
        print("[DEBUG] MainWindow created.")
        logging.debug("MainWindow created.")
        window.show()
        print("[DEBUG] MainWindow shown.")
        splash.finish(window)  # Hide splash when main window is ready
        print("[DEBUG] Splash screen finished.")
        logging.debug("MainWindow shown.")
        window.redirect_output_to_log()
        # Start the bot after the event loop starts
        QTimer.singleShot(100, lambda: logging.debug("Starting bot...") or start_bot())

    # Show the main window after 2 seconds (2000 ms)
    print("[DEBUG] Scheduling main window to show in 2 seconds...")
    QTimer.singleShot(2000, show_main_window)
    print("[DEBUG] Entering Qt event loop...")
    logging.debug("Entering Qt event loop...")
    sys.exit(app.exec())

def ensure_ollama_running():
    """
    Checks if Ollama is running, and starts it if not. Runs in a background thread.
    """
    ollama_client = OllamaClient()
    if not ollama_client.status():
        logging.info("Ollama is not running. Attempting to start Ollama service...")
        started = ollama_client.start()
        if started:
            logging.info("Ollama service started successfully.")
        else:
            logging.error("Failed to start Ollama service. The bot may not function correctly.")
    else:
        logging.info("Ollama service is already running.")

if __name__ == "__main__":
    logging.debug("Starting Ollama ensure thread...")
    try:
        threading.Thread(target=ensure_ollama_running, daemon=True).start()
        logging.debug("Starting GUI and bot...")
        start_gui_and_bot()
        set_crash_counter(0)  # Reset on clean exit
    except Exception as e:
        logging.error(f"Unhandled exception: {e}")
        # Increment crash counter so next launch can auto-enable debug
        count = get_crash_counter()
        set_crash_counter(count + 1)
        raise



