import sys
from PySide6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QVBoxLayout, QHBoxLayout, QLabel,
    QPushButton, QComboBox, QLineEdit, QProgressBar, QCheckBox, QTabWidget, QTextEdit, QSpinBox
)
from PySide6.QtCore import Qt, QTimer, Signal, Slot, QObject
from PySide6.QtGui import QFontMetrics
import os
import logging
import concurrent.futures
import json
import traceback

from .theme_manager import ThemeManager
from .server_config_page import ServerConfigPage
from ollama_api import OllamaClient
from bot_core import bot
import config
import utils  # Add this import

def debug_print(msg):
    if config.DEBUG:
        print(msg)

# --- New: Persistent crash counter for auto-debug ---
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

# --- New: File log handlers ---
class FileLogger(logging.Handler):
    def __init__(self, filepath, enabled_func):
        super().__init__()
        self.filepath = filepath
        self.enabled_func = enabled_func
        os.makedirs(os.path.dirname(filepath), exist_ok=True)
    def emit(self, record):
        if self.enabled_func():
            msg = self.format(record)
            with open(self.filepath, 'a', encoding='utf-8') as f:
                f.write(msg + '\n')

# --- Modified: QTextEditLogger to optionally filter by logger name ---
class QTextEditLogger(logging.Handler):
    def __init__(self, widget, logger_name=None):
        super().__init__()
        self.widget = widget
        self.logger_name = logger_name
    def emit(self, record):
        if self.logger_name is None or record.name == self.logger_name:
            msg = self.format(record)
            self.widget.append(msg)

class WorkerSignals(QObject):
    models_loaded = Signal(list)
    ollama_status_checked = Signal(bool)
    model_download_progress = Signal(int, str)
    model_download_finished = Signal()

class MainWindow(QMainWindow):
    """
    Main GUI window for Silas Blue.
    Provides controls for bot and Ollama status, model management, and configuration.
    """

    def __init__(self, start_bot=None, stop_bot=None, restart_bot=None):
        try:
            debug_print("[DEBUG] MainWindow.__init__ starting...")
            super().__init__()
            debug_print("[DEBUG] QMainWindow super().__init__ done")
            self.setWindowTitle("Silas Blue Control Panel")
            self.setMinimumSize(600, 600)

            debug_print("[DEBUG] Storing bot control functions")
            self._start_bot_func = start_bot
            self._stop_bot_func = stop_bot
            self._restart_bot_func = restart_bot

            debug_print("[DEBUG] Initializing bot status tracking")
            self._bot_status = "Unknown"
            self._bot_status_error = None

            debug_print("[DEBUG] Creating ThemeManager")
            self.theme_manager = ThemeManager()

            # --- Load theme from app config ---
            app_config = utils.load_app_config()
            theme_name = app_config.get("theme", "retrowave")
            theme_file = f"themes/{theme_name.lower()}.json"
            if not os.path.exists(theme_file):
                theme_file = "themes/retrowave.json"
                theme_name = "retrowave"
            debug_print(f"[DEBUG] Applying theme: {theme_file}")
            self.theme_manager.apply_theme(self, theme_file)

            debug_print("[DEBUG] Creating OllamaClient")
            self.ollama = OllamaClient()

            debug_print("[DEBUG] Creating main layout")
            self.tabs = QTabWidget()
            self.setCentralWidget(self.tabs)

            debug_print("[DEBUG] Creating ThreadPoolExecutor")
            self.executor = concurrent.futures.ThreadPoolExecutor(max_workers=2)

            debug_print("[DEBUG] Creating status tab")
            self.status_tab = QWidget()
            self.tabs.addTab(self.status_tab, "Status")

            status_layout = QVBoxLayout()
            self.status_tab.setLayout(status_layout)

            # --- Bot, Ollama, and Auto-Restart in a single row ---
            status_row_layout = QHBoxLayout()

            # Bot status and buttons
            bot_status_col = QVBoxLayout()
            self.bot_status_label = QLabel("Bot Status: <b>Running</b>")
            bot_status_col.addWidget(self.bot_status_label)
            bot_btn_layout = QHBoxLayout()
            self.start_bot_btn = QPushButton("Start Bot")
            self.stop_bot_btn = QPushButton("Stop Bot")
            self.restart_bot_btn = QPushButton("Restart Bot")
            bot_btn_layout.addWidget(self.start_bot_btn)
            bot_btn_layout.addWidget(self.stop_bot_btn)
            bot_btn_layout.addWidget(self.restart_bot_btn)
            bot_status_col.addLayout(bot_btn_layout)
            status_row_layout.addLayout(bot_status_col, 3)

            # Ollama status and buttons
            ollama_status_col = QVBoxLayout()
            self.ollama_status_label = QLabel("Ollama Status: <b>Unknown</b>")
            ollama_status_col.addWidget(self.ollama_status_label)
            ollama_btn_layout = QHBoxLayout()
            self.start_ollama_btn = QPushButton("Start Ollama")
            self.stop_ollama_btn = QPushButton("Stop Ollama")
            self.restart_ollama_btn = QPushButton("Restart Ollama")
            ollama_btn_layout.addWidget(self.start_ollama_btn)
            ollama_btn_layout.addWidget(self.stop_ollama_btn)
            ollama_btn_layout.addWidget(self.restart_ollama_btn)
            ollama_status_col.addLayout(ollama_btn_layout)
            status_row_layout.addLayout(ollama_status_col, 3)

            # Auto-restart controls in the same row
            auto_restart_col = QVBoxLayout()
            self.auto_restart_checkbox = QCheckBox("Enable Auto-Restart")
            self.auto_restart_interval = QComboBox()
            # Use '30 Min' for 0.5h, and 'Hour(s)' for others
            interval_labels = []
            for x in range(1, 49):
                if x == 1:
                    interval_labels.append('30 Min')
                else:
                    hours = x / 2
                    if hours == 1:
                        interval_labels.append('1 Hour')
                    else:
                        interval_labels.append(f'{int(hours) if hours.is_integer() else hours} Hours')
            self.auto_restart_interval.addItems(interval_labels)
            self.auto_restart_interval.setMaximumWidth(100)
            auto_restart_col.addWidget(self.auto_restart_checkbox)
            auto_restart_col.addWidget(self.auto_restart_interval)
            status_row_layout.addLayout(auto_restart_col, 1)

            status_layout.addLayout(status_row_layout)

            debug_print("[DEBUG] Creating model selection and download controls")
            # --- Model selection and download controls in a row ---
            model_row_layout = QHBoxLayout()
            self.model_select = QComboBox()
            self.model_select.setEditable(False)
            self.refresh_models_async()
            self.model_download_input = QLineEdit()
            self.model_download_input.setPlaceholderText("Enter model name to download")
            self.model_download_btn = QPushButton("Download Model")
            self.model_download_progress = QProgressBar()
            self.model_download_progress.setVisible(False)

            model_row_layout.addWidget(QLabel("Default Model:"))
            model_row_layout.addWidget(self.model_select, 2)
            model_row_layout.addWidget(self.model_download_input, 2)
            model_row_layout.addWidget(self.model_download_btn, 1)
            status_layout.addLayout(model_row_layout)
            status_layout.addWidget(self.model_download_progress)

            debug_print("[DEBUG] Creating connected servers list")
            # --- Connected Servers and Theme selection in a row ---
            servers_theme_row = QHBoxLayout()
            self.servers_label = QLabel("Connected Servers:")
            self.servers_list = QComboBox()
            servers_theme_row.addWidget(self.servers_label)
            servers_theme_row.addWidget(self.servers_list, 2)
            theme_label = QLabel("Theme:")
            servers_theme_row.addWidget(theme_label)
            self.theme_select = QComboBox()
            self._theme_name_map = {}  # Map display name to actual theme name
            for theme in self.theme_manager.available_themes():
                display = theme.capitalize()
                self.theme_select.addItem(display)
                self._theme_name_map[display] = theme
            # Set current theme in dropdown
            current_display = theme_name.capitalize()
            idx = self.theme_select.findText(current_display)
            if idx != -1:
                self.theme_select.setCurrentIndex(idx)
            self.theme_select.currentTextChanged.connect(self.change_theme)
            servers_theme_row.addWidget(self.theme_select, 1)
            status_layout.addLayout(servers_theme_row)

            debug_print("[DEBUG] Creating server list timer")
            self.server_list_timer = QTimer(self)
            self.server_list_timer.timeout.connect(self.update_servers_list)
            self.server_list_timer.start(2000)  # Check every 2 seconds

            debug_print("[DEBUG] Creating server config tab")
            self.server_config_tab = ServerConfigPage(self)
            self.tabs.addTab(self.server_config_tab, "Server Config")

            debug_print("[DEBUG] Creating log output section")
            log_layout = QHBoxLayout()
            self.system_log_output = QTextEdit()
            self.system_log_output.setReadOnly(True)
            self.bot_log_output = QTextEdit()
            self.bot_log_output.setReadOnly(True)
            log_layout.addWidget(self.system_log_output)
            log_layout.addWidget(self.bot_log_output)
            # Add clear buttons for each log
            clear_log_layout = QHBoxLayout()
            self.clear_system_log_btn = QPushButton("Clear System Log")
            self.clear_bot_log_btn = QPushButton("Clear Bot Log")
            clear_log_layout.addWidget(self.clear_system_log_btn)
            clear_log_layout.addWidget(self.clear_bot_log_btn)
            status_layout.addWidget(QLabel("System Log (left) & Bot Log (right):"))
            status_layout.addLayout(log_layout)
            status_layout.addLayout(clear_log_layout)
            # Checkboxes for file logging and debug mode (now in a row)
            self.system_log_to_file_checkbox = QCheckBox("Write System Log to File")
            self.bot_log_to_file_checkbox = QCheckBox("Write Bot Log to File")
            self.debug_checkbox = QCheckBox("Enable Debug Mode")
            self.debug_checkbox.setChecked(config.DEBUG)
            checkboxes_layout = QHBoxLayout()
            checkboxes_layout.addWidget(self.system_log_to_file_checkbox)
            checkboxes_layout.addWidget(self.bot_log_to_file_checkbox)
            checkboxes_layout.addWidget(self.debug_checkbox)
            status_layout.addLayout(checkboxes_layout)

            debug_print("[DEBUG] Setting up logging handler")
            self.system_log_handler = QTextEditLogger(self.system_log_output, logger_name=None)  # root logger
            self.bot_log_handler = QTextEditLogger(self.bot_log_output, logger_name="silasblue")
            self.system_file_handler = FileLogger(os.path.join('logs', 'system.log'), lambda: self.system_log_to_file_checkbox.isChecked())
            self.bot_file_handler = FileLogger(os.path.join('logs', 'bot.log'), lambda: self.bot_log_to_file_checkbox.isChecked())
            root_logger = logging.getLogger()
            bot_logger = logging.getLogger("silasblue")
            for h in [self.system_log_handler, self.system_file_handler]:
                if h not in root_logger.handlers:
                    root_logger.addHandler(h)
            for h in [self.bot_log_handler, self.bot_file_handler]:
                if h not in bot_logger.handlers:
                    bot_logger.addHandler(h)
            bot_logger.setLevel(logging.DEBUG if config.DEBUG else logging.INFO)
            bot_logger.propagate = False  # Prevent duplicate logs in root logger
            # --- New: Debug checkbox logic ---
            self.debug_checkbox.stateChanged.connect(self.on_debug_checkbox_changed)
            # --- New: Crash counter logic ---
            self.handle_crash_counter()

            debug_print("[DEBUG] Setting up gui_log.txt tracking")
            self._gui_log_pos = 0
            self.gui_log_timer = QTimer(self)
            self.gui_log_timer.timeout.connect(self.read_gui_log)
            self.gui_log_timer.start(1000)  # Check every second

            debug_print("[DEBUG] Connecting button signals to slots")
            self.model_download_btn.clicked.connect(self.download_model)
            self.start_ollama_btn.clicked.connect(self.start_ollama)
            self.stop_ollama_btn.clicked.connect(self.stop_ollama)
            self.restart_ollama_btn.clicked.connect(self.restart_ollama)
            self.start_bot_btn.clicked.connect(self.start_bot)
            self.stop_bot_btn.clicked.connect(self.stop_bot)
            self.restart_bot_btn.clicked.connect(self.restart_bot)

            debug_print("[DEBUG] Setting up Ollama status timer")
            self.ollama_status_timer = QTimer(self)
            self.ollama_status_timer.timeout.connect(self.update_ollama_status)
            self.ollama_status_timer.start(5000)
            self.update_ollama_status()

            debug_print("[DEBUG] Setting up bot status timer")
            self.bot_status_timer = QTimer(self)
            self.bot_status_timer.timeout.connect(self.update_bot_status)
            self.bot_status_timer.start(2000)  # Check every 2 seconds
            self.update_bot_status()

            debug_print("[DEBUG] Setting up WorkerSignals and lazy loading")
            self.signals = WorkerSignals()
            self.signals.models_loaded.connect(self.on_models_loaded)
            self.signals.ollama_status_checked.connect(self.on_ollama_status_checked)
            self.signals.model_download_progress.connect(self.on_model_download_progress)
            self.signals.model_download_finished.connect(self.on_model_download_finished)

            QTimer.singleShot(100, self.refresh_models_async)
            self.server_list_timer.setInterval(5000)  # 5 seconds
            self.ollama_status_timer.setInterval(10000)  # 10 seconds
            self.bot_status_timer.setInterval(5000)  # 5 seconds

            # Connect clear log buttons
            self.clear_system_log_btn.clicked.connect(lambda: self.system_log_output.clear())
            self.clear_bot_log_btn.clicked.connect(lambda: self.bot_log_output.clear())

            # --- Dynamic QComboBox width adjustment ---
            def adjust_combobox_width(combobox, min_width=100, padding=24):
                font_metrics = QFontMetrics(combobox.font())
                max_width = min_width
                for i in range(combobox.count()):
                    text = combobox.itemText(i)
                    width = font_metrics.horizontalAdvance(text)
                    if width > max_width:
                        max_width = width
                combobox.setMinimumWidth(max_width + padding)
            # Adjust all relevant comboboxes after population
            adjust_combobox_width(self.auto_restart_interval)
            adjust_combobox_width(self.model_select)
            adjust_combobox_width(self.servers_list)
            adjust_combobox_width(self.theme_select)
            # Connect signals to adjust width on changes
            self.auto_restart_interval.currentIndexChanged.connect(lambda: adjust_combobox_width(self.auto_restart_interval))
            self.model_select.currentIndexChanged.connect(lambda: adjust_combobox_width(self.model_select))
            self.servers_list.currentIndexChanged.connect(lambda: adjust_combobox_width(self.servers_list))
            self.theme_select.currentIndexChanged.connect(lambda: adjust_combobox_width(self.theme_select))

            debug_print("[DEBUG] MainWindow.__init__ finished!")
        except Exception as e:
            import traceback
            debug_print(f"[DEBUG] Exception in MainWindow.__init__: {e}\n" + traceback.format_exc())
            raise

    def refresh_models_async(self):
        # Only try to fetch models if Ollama is running
        def fetch_models_if_running():
            if self.ollama.status():
                return self.ollama.list_models()
            else:
                return None  # Ollama not running yet
        future = self.executor.submit(fetch_models_if_running)
        QTimer.singleShot(100, lambda: self.check_models_future(future))

    def check_models_future(self, future):
        if future.done():
            models = future.result()
            if models is None:
                self.model_select.clear()
                self.model_select.addItem("Ollama not running")
            else:
                self.signals.models_loaded.emit(models)
        else:
            QTimer.singleShot(100, lambda: self.check_models_future(future))

    @Slot(list)
    def on_models_loaded(self, models):
        self.model_select.clear()
        if not models:
            self.model_select.addItem("No models found")
        else:
            for model in models:
                self.model_select.addItem(model)

    def update_ollama_status(self):
        future = self.executor.submit(self.ollama.status)
        QTimer.singleShot(100, lambda: self.check_ollama_status_future(future))

    def check_ollama_status_future(self, future):
        if future.done():
            running = future.result()
            self.signals.ollama_status_checked.emit(running)
        else:
            QTimer.singleShot(100, lambda: self.check_ollama_status_future(future))

    @Slot(bool)
    def on_ollama_status_checked(self, running):
        if running:
            self.ollama_status_label.setText("Ollama Status: <b>Running</b>")
        else:
            self.ollama_status_label.setText("Ollama Status: <b>Not Running</b>")

    def download_model(self):
        model_name = self.model_download_input.text().strip()
        if not model_name:
            return
        self.model_download_progress.setVisible(True)
        self.model_download_progress.setValue(0)
        self._last_progress_percent = None  # Track last percent for deduplication
        # NOTE: All Qt object usage must be in the main thread. Worker threads must only use signals/slots to communicate with the GUI.
        def progress_callback(percent, speed):
            if percent != self._last_progress_percent:
                self._last_progress_percent = percent
                self.signals.model_download_progress.emit(percent, speed)
        def do_download():
            self.ollama.download_model(model_name, progress_callback)
            self.signals.model_download_finished.emit()
        self.executor.submit(do_download)

    @Slot(int, str)
    def on_model_download_progress(self, percent, speed):
        if percent == -1:
            self.model_download_progress.setFormat("Error")
            self.system_log_output.append("Download error.")
        else:
            self.model_download_progress.setValue(percent)
            self.system_log_output.append(f"Download: {percent}% {speed}")

    @Slot()
    def on_model_download_finished(self):
        self.model_download_progress.setVisible(False)
        self.refresh_models_async()

    def change_theme(self, display_name):
        """
        Change the GUI theme and save to app config.
        """
        theme_name = self._theme_name_map.get(display_name, "retrowave")
        theme_file = f"themes/{theme_name.lower()}.json"
        if os.path.exists(theme_file):
            self.theme_manager.apply_theme(self, theme_file)
            # Save to app config
            app_config = utils.load_app_config()
            app_config["theme"] = theme_name
            utils.save_app_config(app_config)

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
            self.system_log_output.append(f"[ERROR] Failed to update server list: {e}")

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
                    # Route prompt/reply/discord events to bot log, config_change/errors to system log
                    if event == "config_change":
                        msg = f"[Config Change] Guild: {data.get('guild_id')} User: {data.get('user')} Field: {data.get('field')} -> {data.get('value')}"
                        self.system_log_output.append(msg)
                    elif event == "prompt":
                        msg = f"[Prompt] Guild: {data.get('guild_id')} User: {data.get('user')} Prompt: {data.get('prompt')}"
                        self.bot_log_output.append(msg)
                    elif event == "reply":
                        msg = f"[Reply] Guild: {data.get('guild_id')} User: {data.get('user')} Reply: {data.get('reply')[:200]}{'...' if len(data.get('reply',''))>200 else ''}"
                        self.bot_log_output.append(msg)
                    else:
                        # Assume all other Discord events go to bot log
                        msg = str(entry)
                        self.bot_log_output.append(msg)
                except Exception as e:
                    self.system_log_output.append(f"[Log Parse Error] {e}")
        except Exception as e:
            self.system_log_output.append(f"[Log File Error] {e}")

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
                self.system_log_output.append("[INFO] Start Bot requested.")
                self.show_feedback(self.start_bot_btn, "Start Bot requested.")
            else:
                self.system_log_output.append("[WARN] Start Bot function not available.")
        except Exception as e:
            err = traceback.format_exc()
            self.system_log_output.append(f"[ERROR] Failed to start bot: {e}\n{err}")
            self._bot_status_error = str(e)
            self.update_bot_status()

    def stop_bot(self):
        """
        Stop the Discord bot and close the GUI.
        """
        try:
            if self._stop_bot_func:
                self._stop_bot_func()
                self.system_log_output.append("[INFO] Stop Bot requested. Shutting down GUI.")
                self.show_feedback(self.stop_bot_btn, "Stop Bot requested. Shutting down GUI.")
                self.close()  # This will close the GUI
            else:
                self.system_log_output.append("[WARN] Stop Bot function not available.")
        except Exception as e:
            err = traceback.format_exc()
            self.system_log_output.append(f"[ERROR] Failed to stop bot: {e}\n{err}")
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
                self.system_log_output.append("[INFO] Restart Bot requested. (GUI will remain open.)")
                self.show_feedback(self.restart_bot_btn, "Restart Bot requested.")
            else:
                self.system_log_output.append("[WARN] Restart Bot function not available.")
        except Exception as e:
            err = traceback.format_exc()
            self.system_log_output.append(f"[ERROR] Failed to restart bot: {e}\n{err}")
            self._bot_status_error = str(e)
            self.update_bot_status()

    def show_feedback(self, button, message):
        """Show a temporary color change and log message for button feedback."""
        orig_style = button.styleSheet()
        button.setStyleSheet("background-color: #00f0ff; color: #181825;")
        QTimer.singleShot(200, lambda: button.setStyleSheet(orig_style))
        self.system_log_output.append(message)

    def write(self, msg):
        if msg.strip():
            self.system_log_output.append(msg.strip())

    def flush(self):
        pass

    def redirect_output_to_log(self):
        debug_print("[DEBUG] Redirecting stdout and stderr to log_output (after show)")
        sys.stdout = self
        sys.stderr = self

    def start_ollama(self):
        """Start the Ollama service via OllamaClient."""
        try:
            self.system_log_output.append("[INFO] Starting Ollama service...")
            started = self.ollama.start()
            if started:
                self.system_log_output.append("[INFO] Ollama service started successfully.")
            else:
                self.system_log_output.append("[ERROR] Failed to start Ollama service.")
            self.update_ollama_status()
        except Exception as e:
            import traceback
            err = traceback.format_exc()
            self.system_log_output.append(f"[ERROR] Exception while starting Ollama: {e}\n{err}")
            self.update_ollama_status()

    def stop_ollama(self):
        """Stop the Ollama service via OllamaClient, if supported."""
        try:
            self.system_log_output.append("[INFO] Stopping Ollama service...")
            stopped = getattr(self.ollama, 'stop', lambda: False)()
            if stopped:
                self.system_log_output.append("[INFO] Ollama service stopped successfully.")
            else:
                self.system_log_output.append("[ERROR] Failed to stop Ollama service or not supported.")
            self.update_ollama_status()
        except Exception as e:
            import traceback
            err = traceback.format_exc()
            self.system_log_output.append(f"[ERROR] Exception while stopping Ollama: {e}\n{err}")
            self.update_ollama_status()

    def restart_ollama(self):
        """Restart the Ollama service via OllamaClient, if supported."""
        try:
            self.system_log_output.append("[INFO] Restarting Ollama service...")
            restarted = getattr(self.ollama, 'restart', None)
            if callable(restarted):
                result = restarted()
            else:
                # Fallback: stop then start
                stop = getattr(self.ollama, 'stop', lambda: False)
                start = getattr(self.ollama, 'start', lambda: False)
                result = stop() and start()
            if result:
                self.system_log_output.append("[INFO] Ollama service restarted successfully.")
            else:
                self.system_log_output.append("[ERROR] Failed to restart Ollama service.")
            self.update_ollama_status()
        except Exception as e:
            import traceback
            err = traceback.format_exc()
            self.system_log_output.append(f"[ERROR] Exception while restarting Ollama: {e}\n{err}")
            self.update_ollama_status()

    def on_debug_checkbox_changed(self, state):
        debug_enabled = state == Qt.Checked
        config.DEBUG = debug_enabled
        # Overwrite config.py
        try:
            with open('config.py', 'w', encoding='utf-8') as f:
                f.write(f'DEBUG = {str(debug_enabled)}\n')
        except Exception as e:
            self.system_log_output.append(f"[ERROR] Failed to update config.py: {e}")
        # Update logger levels
        logging.getLogger().setLevel(logging.DEBUG if debug_enabled else logging.INFO)
        logging.getLogger("silasblue").setLevel(logging.DEBUG if debug_enabled else logging.INFO)

    def handle_crash_counter(self):
        count = get_crash_counter()
        if count >= 2:
            # Auto-enable debug
            self.debug_checkbox.setChecked(True)
            self.system_log_output.append("[INFO] Debug mode auto-enabled due to repeated crashes.")
            set_crash_counter(0)  # Reset after enabling

    # --- On successful close, reset crash counter ---
    def closeEvent(self, event):
        set_crash_counter(0)
        super().closeEvent(event)

# Entry point for running the GUI standalone (for testing)
if __name__ == "__main__":
    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    window.redirect_output_to_log()
    sys.exit(app.exec()) 