import sys
from PySide6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QVBoxLayout, QHBoxLayout, QLabel,
    QPushButton, QComboBox, QLineEdit, QProgressBar, QCheckBox, QTabWidget, QTextEdit, QSpinBox
)
from PySide6.QtCore import Qt, QTimer, Signal, Slot
import os
import logging
import concurrent.futures
import json
import traceback

from .theme_manager import ThemeManager
from .server_config_page import ServerConfigPage
from ollama_api import OllamaClient
from bot_core import bot

class QTextEditLogger(logging.Handler):
    """Custom logging handler to write logs to a QTextEdit widget."""
    def __init__(self, widget):
        super().__init__()
        self.widget = widget

    def emit(self, record):
        msg = self.format(record)
        self.widget.append(msg)

class MainWindow(QMainWindow):
    """
    Main GUI window for Silas Blue.
    Provides controls for bot and Ollama status, model management, and configuration.
    """

    def __init__(self, start_bot=None, stop_bot=None, restart_bot=None):
        super().__init__()
        self.setWindowTitle("Silas Blue Control Panel")
        self.setMinimumSize(900, 700)

        # Store bot control functions
        self._start_bot_func = start_bot
        self._stop_bot_func = stop_bot
        self._restart_bot_func = restart_bot

        # Track bot status
        self._bot_status = "Unknown"
        self._bot_status_error = None

        # Theme manager
        self.theme_manager = ThemeManager()
        self.theme_manager.apply_theme(self, "themes/retrowave.json")

        # Ollama client
        self.ollama = OllamaClient()

        # Main layout
        self.tabs = QTabWidget()
        self.setCentralWidget(self.tabs)

        # Thread pool executor for background tasks
        self.executor = concurrent.futures.ThreadPoolExecutor(max_workers=2)

        # --- Main Status Tab ---
        self.status_tab = QWidget()
        self.tabs.addTab(self.status_tab, "Status")

        status_layout = QVBoxLayout()
        self.status_tab.setLayout(status_layout)

        # Bot status indicator and controls
        self.bot_status_label = QLabel("Bot Status: <b>Running</b>")
        status_layout.addWidget(self.bot_status_label)

        bot_btn_layout = QHBoxLayout()
        self.start_bot_btn = QPushButton("Start Bot")
        self.stop_bot_btn = QPushButton("Stop Bot")
        self.restart_bot_btn = QPushButton("Restart Bot")
        bot_btn_layout.addWidget(self.start_bot_btn)
        bot_btn_layout.addWidget(self.stop_bot_btn)
        bot_btn_layout.addWidget(self.restart_bot_btn)
        status_layout.addLayout(bot_btn_layout)

        # Auto-restart controls
        self.auto_restart_checkbox = QCheckBox("Enable Auto-Restart")
        self.auto_restart_interval = QComboBox()
        self.auto_restart_interval.addItems([f"{x/2:.1f}h" if x % 2 else f"{x//2}h" for x in range(1, 49)])
        status_layout.addWidget(self.auto_restart_checkbox)
        status_layout.addWidget(self.auto_restart_interval)

        # --- Ollama Status Section ---
        self.ollama_status_label = QLabel("Ollama Status: <b>Unknown</b>")
        status_layout.addWidget(self.ollama_status_label)

        ollama_btn_layout = QHBoxLayout()
        self.start_ollama_btn = QPushButton("Start Ollama")
        self.stop_ollama_btn = QPushButton("Stop Ollama")
        self.restart_ollama_btn = QPushButton("Restart Ollama")
        ollama_btn_layout.addWidget(self.start_ollama_btn)
        ollama_btn_layout.addWidget(self.stop_ollama_btn)
        ollama_btn_layout.addWidget(self.restart_ollama_btn)
        status_layout.addLayout(ollama_btn_layout)

        # Model selection and download
        self.model_select = QComboBox()
        self.model_select.setEditable(False)
        self.refresh_models_async()
        self.model_download_input = QLineEdit()
        self.model_download_input.setPlaceholderText("Enter model name to download")
        self.model_download_btn = QPushButton("Download Model")
        self.model_download_progress = QProgressBar()
        self.model_download_progress.setVisible(False)

        status_layout.addWidget(QLabel("Default Model:"))
        status_layout.addWidget(self.model_select)
        status_layout.addWidget(self.model_download_input)
        status_layout.addWidget(self.model_download_btn)
        status_layout.addWidget(self.model_download_progress)

        # --- Connected Servers List ---
        self.servers_label = QLabel("Connected Servers:")
        status_layout.addWidget(self.servers_label)
        self.servers_list = QComboBox()
        status_layout.addWidget(self.servers_list)

        # Timer to update server list after bot is ready
        self.server_list_timer = QTimer(self)
        self.server_list_timer.timeout.connect(self.update_servers_list)
        self.server_list_timer.start(2000)  # Check every 2 seconds

        # --- Server Config Tab ---
        self.server_config_tab = ServerConfigPage(self)
        self.tabs.addTab(self.server_config_tab, "Server Config")

        # --- Theme Selection ---
        self.theme_select = QComboBox()
        for theme in self.theme_manager.available_themes():
            self.theme_select.addItem(theme.capitalize())
        self.theme_select.currentTextChanged.connect(self.change_theme)
        status_layout.addWidget(QLabel("Theme:"))
        status_layout.addWidget(self.theme_select)

        # --- Log Output Section ---
        self.log_output = QTextEdit()
        self.log_output.setReadOnly(True)
        self.log_output.setMaximumHeight(200)
        status_layout.addWidget(QLabel("Bot & System Log:"))
        status_layout.addWidget(self.log_output)

        # Redirect logging to the log_output widget
        self.log_handler = QTextEditLogger(self.log_output)
        self.log_handler.setFormatter(logging.Formatter('%(asctime)s [%(levelname)s] %(message)s'))
        logging.getLogger().addHandler(self.log_handler)
        logging.getLogger().setLevel(logging.INFO)

        # Redirect stdout and stderr to the log_output widget
        sys.stdout = self
        sys.stderr = self

        # Track last read position in gui_log.txt
        self._gui_log_pos = 0
        self.gui_log_timer = QTimer(self)
        self.gui_log_timer.timeout.connect(self.read_gui_log)
        self.gui_log_timer.start(1000)  # Check every second

        # Connect button signals to slots
        self.model_download_btn.clicked.connect(self.download_model)
        self.start_ollama_btn.clicked.connect(self.start_ollama)
        self.stop_ollama_btn.clicked.connect(self.stop_ollama)
        self.restart_ollama_btn.clicked.connect(self.restart_ollama)
        self.start_bot_btn.clicked.connect(self.start_bot)
        self.stop_bot_btn.clicked.connect(self.stop_bot)
        self.restart_bot_btn.clicked.connect(self.restart_bot)

        # Periodically update Ollama status
        self.ollama_status_timer = QTimer(self)
        self.ollama_status_timer.timeout.connect(self.update_ollama_status)
        self.ollama_status_timer.start(5000)
        self.update_ollama_status()

        # Timer to update bot status
        self.bot_status_timer = QTimer(self)
        self.bot_status_timer.timeout.connect(self.update_bot_status)
        self.bot_status_timer.start(2000)  # Check every 2 seconds
        self.update_bot_status()

    def refresh_models_async(self):
        """Fetch models in a background thread and update the combo box."""
        self.model_select.clear()
        self.model_select.addItem("Loading...")
        future = self.executor.submit(self.ollama.list_models)
        QTimer.singleShot(100, lambda: self.check_models_future(future))

    def check_models_future(self, future):
        if future.done():
            models = future.result()
            self.model_select.clear()
            if not models:
                self.model_select.addItem("No models found")
            else:
                for model in models:
                    self.model_select.addItem(model)
        else:
            QTimer.singleShot(100, lambda: self.check_models_future(future))

    def change_theme(self, theme_name):
        """
        Change the GUI theme.
        """
        theme_file = f"themes/{theme_name.lower()}.json"
        if os.path.exists(theme_file):
            self.theme_manager.apply_theme(self, theme_file)

    def update_ollama_status(self):
        """
        Update the Ollama status label.
        """
        running = self.ollama.status()
        if running:
            self.ollama_status_label.setText("Ollama Status: <b>Running</b>")
        else:
            self.ollama_status_label.setText("Ollama Status: <b>Not Running</b>")

    def download_model(self):
        """
        Download a model from Ollama and show progress.
        """
        model_name = self.model_download_input.text().strip()
        if not model_name:
            return
        self.model_download_progress.setVisible(True)
        self.model_download_progress.setValue(0)

        def progress_callback(percent, speed):
            if percent == -1:
                self.model_download_progress.setFormat("Error")
                self.log_output.append("Download error.")
            else:
                self.model_download_progress.setValue(percent)
                self.log_output.append(f"Download: {percent}% {speed}")

        import threading
        def do_download():
            self.ollama.download_model(model_name, progress_callback)
            self.model_download_progress.setVisible(False)
            self.refresh_models_async()

        threading.Thread(target=do_download, daemon=True).start()

    def start_ollama(self):
        """
        Start Ollama server.
        """
        success = self.ollama.start()
        self.update_ollama_status()
        self.show_feedback(self.start_ollama_btn, "Ollama start requested.")
        if not success:
            self.ollama_status_label.setText("Ollama Status: <b>Failed to start</b>")

    def stop_ollama(self):
        """
        Stop Ollama server.
        """
        success = self.ollama.stop()
        self.update_ollama_status()
        self.show_feedback(self.stop_ollama_btn, "Ollama stop requested.")
        if not success:
            self.ollama_status_label.setText("Ollama Status: <b>Failed to stop</b>")

    def restart_ollama(self):
        """
        Restart Ollama server.
        """
        success = self.ollama.restart()
        self.update_ollama_status()
        self.show_feedback(self.restart_ollama_btn, "Ollama restart requested.")
        if not success:
            self.ollama_status_label.setText("Ollama Status: <b>Failed to restart</b>")

    def show_feedback(self, button, message):
        """Show a temporary color change and log message for button feedback."""
        orig_style = button.styleSheet()
        button.setStyleSheet("background-color: #00f0ff; color: #181825;")
        QTimer.singleShot(200, lambda: button.setStyleSheet(orig_style))
        self.log_output.append(message)

    def write(self, msg):
        if msg.strip():
            self.log_output.append(msg.strip())

    def flush(self):
        pass

    def update_servers_list(self):
        """Update the list of connected servers."""
        try:
            from bot_core import _bot_instance
            bot = _bot_instance
            if not bot:
                return
            current_servers = [self.servers_list.itemData(i) for i in range(self.servers_list.count())]
            new_servers = [(guild.name, guild.id) for guild in getattr(bot, 'guilds', [])]
            # Only update if changed
            if len(current_servers) != len(new_servers) or any(str(gid) not in [str(x[1]) for x in new_servers] for gid in current_servers):
                self.servers_list.clear()
                for name, gid in new_servers:
                    self.servers_list.addItem(f"{name} ({gid})", userData=gid)
            # Also update the config page's server list if it exists
            if hasattr(self, "server_config_tab"):
                self.server_config_tab.update_guilds()
        except Exception as e:
            self.log_output.append(f"[ERROR] Failed to update server list: {e}")

    def read_gui_log(self):
        """Read new lines from config/gui_log.txt and append to log_output."""
        log_path = os.path.join("config", "gui_log.txt")
        if not os.path.exists(log_path):
            return
        try:
            with open(log_path, "r", encoding="utf-8") as f:
                f.seek(self._gui_log_pos)
                lines = f.readlines()
                self._gui_log_pos = f.tell()
            for line in lines:
                try:
                    entry = json.loads(line)
                    event = entry.get("event")
                    data = entry.get("data", {})
                    ts = entry.get("timestamp")
                    if event == "config_change":
                        msg = f"[Config Change] Guild: {data.get('guild_id')} User: {data.get('user')} Field: {data.get('field')} -> {data.get('value')}"
                    elif event == "prompt":
                        msg = f"[Prompt] Guild: {data.get('guild_id')} User: {data.get('user')} Prompt: {data.get('prompt')}"
                    elif event == "reply":
                        msg = f"[Reply] Guild: {data.get('guild_id')} User: {data.get('user')} Reply: {data.get('reply')[:200]}{'...' if len(data.get('reply',''))>200 else ''}"
                    else:
                        msg = str(entry)
                    self.log_output.append(msg)
                except Exception as e:
                    self.log_output.append(f"[Log Parse Error] {e}")
        except Exception as e:
            self.log_output.append(f"[Log File Error] {e}")

    def update_bot_status(self):
        """
        Update the bot status label based on thread state and errors.
        """
        try:
            from bot_core import _bot_thread, _bot_loop
            if _bot_thread is not None and _bot_thread.is_alive():
                self._bot_status = "Running"
                self._bot_status_error = None
            else:
                self._bot_status = "Stopped"
        except Exception as e:
            self._bot_status = "Error"
            self._bot_status_error = str(e)
        status_text = f"Bot Status: <b>{self._bot_status}</b>"
        if self._bot_status_error:
            status_text += f" <span style='color:red;'>(Error: {self._bot_status_error})</span>"
        self.bot_status_label.setText(status_text)

    def start_bot(self):
        """
        Start the Discord bot.
        """
        try:
            if self._start_bot_func:
                self._start_bot_func()
                self.log_output.append("[INFO] Start Bot requested.")
                self.show_feedback(self.start_bot_btn, "Start Bot requested.")
            else:
                self.log_output.append("[WARN] Start Bot function not available.")
        except Exception as e:
            err = traceback.format_exc()
            self.log_output.append(f"[ERROR] Failed to start bot: {e}\n{err}")
            self._bot_status_error = str(e)
            self.update_bot_status()

    def stop_bot(self):
        """
        Stop the Discord bot and close the GUI.
        """
        try:
            if self._stop_bot_func:
                self._stop_bot_func()
                self.log_output.append("[INFO] Stop Bot requested. Shutting down GUI.")
                self.show_feedback(self.stop_bot_btn, "Stop Bot requested. Shutting down GUI.")
                self.close()  # This will close the GUI
            else:
                self.log_output.append("[WARN] Stop Bot function not available.")
        except Exception as e:
            err = traceback.format_exc()
            self.log_output.append(f"[ERROR] Failed to stop bot: {e}\n{err}")
            self._bot_status_error = str(e)
            self.update_bot_status()

    def restart_bot(self):
        """
        Restart the Discord bot (does not close the GUI).
        """
        try:
            if self._restart_bot_func:
                self._bot_status = "Restarting"
                self.update_bot_status()
                self._restart_bot_func()
                self.log_output.append("[INFO] Restart Bot requested. (GUI will remain open.)")
                self.show_feedback(self.restart_bot_btn, "Restart Bot requested.")
            else:
                self.log_output.append("[WARN] Restart Bot function not available.")
        except Exception as e:
            err = traceback.format_exc()
            self.log_output.append(f"[ERROR] Failed to restart bot: {e}\n{err}")
            self._bot_status_error = str(e)
            self.update_bot_status()

# Entry point for running the GUI standalone (for testing)
if __name__ == "__main__":
    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec()) 