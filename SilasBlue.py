"""
Silas Blue - Discord Bot powered by local AI models via Ollama
Main entry point: launches both the Discord bot and the GUI.
"""

import sys
import threading
import os
import logging

from gui.main_window import MainWindow
from PySide6.QtWidgets import QApplication

from bot_core import start_bot, stop_bot, restart_bot

logging.basicConfig(level=logging.INFO)

def start_gui():
    """
    Starts the PySide6 GUI in the main thread.
    """
    app = QApplication(sys.argv)
    # Pass bot control functions to the MainWindow
    window = MainWindow(start_bot=start_bot, stop_bot=stop_bot, restart_bot=restart_bot)
    window.show()
    sys.exit(app.exec())

if __name__ == "__main__":
    # Start the Discord bot using the new control logic
    start_bot()
    # Start the GUI (main thread)
    start_gui()



