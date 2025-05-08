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

logging.basicConfig(level=logging.INFO)

# --- Crash counter logic for auto-debug ---
CRASH_COUNTER_FILE = os.path.join('config', 'crash_counter.txt')
def get_crash_counter():
    try:
        with open(CRASH_COUNTER_FILE, 'r') as f:
            return int(f.read().strip())
    except Exception:
        return 0

def set_crash_counter(val):
    try:
        with open(CRASH_COUNTER_FILE, 'w') as f:
            f.write(str(val))
    except Exception:
        pass

# Increment crash counter and auto-enable debug if needed
count = get_crash_counter()
count += 1
if count >= 2:
    # Set DEBUG=True in config.py
    try:
        with open('config.py', 'w') as f:
            f.write('DEBUG = True\n')
    except Exception:
        pass
    count = 0  # Reset after enabling debug
set_crash_counter(count)

def start_gui_and_bot():
    """
    Starts the PySide6 GUI in the main thread and starts the bot after the event loop starts.
    """
    print("[DEBUG] Creating QApplication...")
    app = QApplication(sys.argv)
    print("[DEBUG] QApplication created.")
    # --- Splash Screen ---
    splash_pix = QPixmap("gui/splash.png")
    splash = QSplashScreen(splash_pix)
    splash.show()
    app.processEvents()  # Ensure splash screen is shown

    def show_main_window():
        print("[DEBUG] Creating MainWindow...")
        window = MainWindow(start_bot=start_bot, stop_bot=stop_bot, restart_bot=restart_bot)
        print("[DEBUG] MainWindow created.")
        window.show()
        splash.finish(window)  # Hide splash when main window is ready
        print("[DEBUG] MainWindow shown.")
        window.redirect_output_to_log()
        # Start the bot after the event loop starts
        QTimer.singleShot(100, lambda: print("[DEBUG] Starting bot...") or start_bot())

    # Show the main window after 3 seconds (3000 ms)
    QTimer.singleShot(3000, show_main_window)
    print("[DEBUG] Entering Qt event loop...")
    sys.exit(app.exec())

def ensure_ollama_running():
    """
    Checks if Ollama is running, and starts it if not. Runs in a background thread.
    """
    ollama_client = OllamaClient()
    if not ollama_client.status():
        print("Ollama is not running. Attempting to start Ollama service...")
        started = ollama_client.start()
        if started:
            print("Ollama service started successfully.")
        else:
            print("Failed to start Ollama service. The bot may not function correctly.")
    else:
        print("Ollama service is already running.")

if __name__ == "__main__":
    print("[DEBUG] Starting Ollama ensure thread...")
    try:
        threading.Thread(target=ensure_ollama_running, daemon=True).start()
        print("[DEBUG] Starting GUI and bot...")
        start_gui_and_bot()
        set_crash_counter(0)  # Reset on clean exit
    except Exception as e:
        print(f"[ERROR] Unhandled exception: {e}")
        # Do not reset crash counter so next launch can auto-enable debug
        raise



