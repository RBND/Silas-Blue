import sys
import os
import logging
import concurrent.futures
import json
import traceback
import queue

from PySide6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QVBoxLayout, QHBoxLayout, QLabel,
    QPushButton, QComboBox, QLineEdit, QProgressBar, QCheckBox, QTabWidget,
    QTextEdit, QSpinBox, QFileDialog, QGroupBox, QRadioButton, QButtonGroup
)
from PySide6.QtCore import Qt, QTimer, Signal, Slot, QObject
from PySide6.QtGui import QFontMetrics

from .theme_manager import ThemeManager
from .server_config_page import ServerConfigPage
from ai_providers import get_ai_provider, AIProvider
from bot_core import bot, reload_ai_provider, gui_event_queue
from utils import load_app_config, save_app_config
import config

logger = logging.getLogger("silasblue")

def debug_print(msg):
    if config.DEBUG:
        print(msg)

# --- File log handlers ---
class FileLogger(logging.Handler):
    def __init__(self, filepath, enabled_func):
        super().__init__()
        self.filepath = filepath
        self.enabled_func = enabled_func
        os.makedirs(os.path.dirname(filepath), exist_ok=True)

    def emit(self, record):
        if self.enabled_func():
            msg = self.format(record)
            try:
                with open(self.filepath, "a", encoding="utf-8") as f:
                    f.write(msg + "\n")
            except Exception:
                pass

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
    ai_status_checked = Signal(bool, str)
    model_download_progress = Signal(int, str)
    model_download_finished = Signal()

class MainWindow(QMainWindow):
    """
    Main GUI window for Silas Blue.
    Provides controls for bot and AI backend status, remote/local configuration, and server management.
    """

    def __init__(self, start_bot=None, stop_bot=None, restart_bot=None):
        super().__init__()
        self.setWindowTitle("Silas Blue Control Panel")
        self.setMinimumSize(700, 650)

        self._start_bot_func = start_bot
        self._stop_bot_func = stop_bot
        self._restart_bot_func = restart_bot

        self._bot_status = "Unknown"
        self._bot_status_error = None

        self.theme_manager = ThemeManager()
        self.theme_manager.apply_theme(self, "themes/retrowave.json")

        self.app_cfg = load_app_config()
        self.provider: AIProvider = get_ai_provider(self.app_cfg)

        self.tabs = QTabWidget()
        self.setCentralWidget(self.tabs)

        self.executor = concurrent.futures.ThreadPoolExecutor(max_workers=4)

        # Worker signals
        self.signals = WorkerSignals()
        self.signals.models_loaded.connect(self.on_models_loaded)
        self.signals.ai_status_checked.connect(self.on_ai_status_checked)
        self.signals.model_download_progress.connect(self.on_model_download_progress)
        self.signals.model_download_finished.connect(self.on_model_download_finished)

        # Build tabs
        self.setup_status_tab()
        self.server_config_tab = ServerConfigPage(self)
        self.tabs.addTab(self.server_config_tab, "Server Config")
        self.setup_backend_settings_tab()

        # Timers
        self.server_list_timer = QTimer(self)
        self.server_list_timer.timeout.connect(self.update_servers_list)
        self.server_list_timer.start(5000)

        self.ai_status_timer = QTimer(self)
        self.ai_status_timer.timeout.connect(self.update_ai_status)
        self.ai_status_timer.start(8000)

        self.bot_status_timer = QTimer(self)
        self.bot_status_timer.timeout.connect(self.update_bot_status)
        self.bot_status_timer.start(3000)

        # In-memory queue reader for GUI events
        self.gui_queue_timer = QTimer(self)
        self.gui_queue_timer.timeout.connect(self.process_gui_events)
        self.gui_queue_timer.start(250)

        # Initial loads
        QTimer.singleShot(200, self.update_ai_status)
        QTimer.singleShot(500, self.refresh_models_async)

    def setup_status_tab(self):
        self.status_tab = QWidget()
        self.tabs.addTab(self.status_tab, "Status & Controls")
        layout = QVBoxLayout()
        self.status_tab.setLayout(layout)

        # Status row
        status_row = QHBoxLayout()

        # Bot Controls
        bot_col = QVBoxLayout()
        self.bot_status_label = QLabel("Bot Status: <b>Stopped</b>")
        bot_col.addWidget(self.bot_status_label)
        bot_btn_layout = QHBoxLayout()
        self.start_bot_btn = QPushButton("Start Bot")
        self.stop_bot_btn = QPushButton("Stop Bot")
        self.restart_bot_btn = QPushButton("Restart Bot")
        bot_btn_layout.addWidget(self.start_bot_btn)
        bot_btn_layout.addWidget(self.stop_bot_btn)
        bot_btn_layout.addWidget(self.restart_bot_btn)
        bot_col.addLayout(bot_btn_layout)
        status_row.addLayout(bot_col, 3)

        # AI Backend Controls
        ai_col = QVBoxLayout()
        self.ai_status_label = QLabel(f"{self.provider.name} Status: <b>Checking...</b>")
        ai_col.addWidget(self.ai_status_label)
        ai_btn_layout = QHBoxLayout()
        self.start_ai_btn = QPushButton(f"Start {self.provider.name}")
        self.stop_ai_btn = QPushButton(f"Stop {self.provider.name}")
        self.check_ai_btn = QPushButton("Check Status")
        ai_btn_layout.addWidget(self.start_ai_btn)
        ai_btn_layout.addWidget(self.stop_ai_btn)
        ai_btn_layout.addWidget(self.check_ai_btn)
        ai_col.addLayout(ai_btn_layout)
        status_row.addLayout(ai_col, 3)

        # Auto-restart controls
        restart_col = QVBoxLayout()
        self.auto_restart_checkbox = QCheckBox("Auto-Restart Bot")
        self.auto_restart_interval = QComboBox()
        interval_labels = ["30 Min", "1 Hour", "2 Hours", "4 Hours", "8 Hours", "12 Hours", "24 Hours"]
        self.auto_restart_interval.addItems(interval_labels)
        restart_col.addWidget(self.auto_restart_checkbox)
        restart_col.addWidget(self.auto_restart_interval)
        status_row.addLayout(restart_col, 1)

        layout.addLayout(status_row)

        # Model row
        model_row = QHBoxLayout()
        self.model_select = QComboBox()
        self.model_select.setEditable(False)
        self.model_download_input = QLineEdit()
        self.model_download_input.setPlaceholderText("Enter model to download (e.g. llama3:8b)")
        self.model_download_btn = QPushButton("Pull Model")
        self.model_download_progress = QProgressBar()
        self.model_download_progress.setVisible(False)

        model_row.addWidget(QLabel("Active Model:"))
        model_row.addWidget(self.model_select, 2)
        model_row.addWidget(self.model_download_input, 2)
        model_row.addWidget(self.model_download_btn, 1)
        layout.addLayout(model_row)
        layout.addWidget(self.model_download_progress)

        # Servers & Theme row
        servers_theme_row = QHBoxLayout()
        self.servers_label = QLabel("Connected Servers:")
        self.servers_list = QComboBox()
        self.invite_bot_btn = QPushButton("➕ Invite Bot")
        self.invite_bot_btn.setToolTip("Invite Silas Blue to a Discord server")
        self.invite_bot_btn.clicked.connect(self.open_invite_dialog)
        servers_theme_row.addWidget(self.servers_label)
        servers_theme_row.addWidget(self.servers_list, 2)
        servers_theme_row.addWidget(self.invite_bot_btn, 1)
        servers_theme_row.addWidget(QLabel("Theme:"))
        self.theme_select = QComboBox()
        for t in self.theme_manager.available_themes():
            self.theme_select.addItem(t.capitalize())
        self.theme_select.currentTextChanged.connect(self.change_theme)
        servers_theme_row.addWidget(self.theme_select, 1)
        layout.addLayout(servers_theme_row)

        # Logs
        log_layout = QHBoxLayout()
        self.system_log_output = QTextEdit()
        self.system_log_output.setReadOnly(True)
        self.bot_log_output = QTextEdit()
        self.bot_log_output.setReadOnly(True)
        log_layout.addWidget(self.system_log_output)
        log_layout.addWidget(self.bot_log_output)

        clear_log_layout = QHBoxLayout()
        self.clear_system_log_btn = QPushButton("Clear System Log")
        self.clear_bot_log_btn = QPushButton("Clear Bot Log")
        clear_log_layout.addWidget(self.clear_system_log_btn)
        clear_log_layout.addWidget(self.clear_bot_log_btn)

        layout.addWidget(QLabel("System Log (Left) & Bot / Discord Activity (Right):"))
        layout.addLayout(log_layout)
        layout.addLayout(clear_log_layout)

        checkboxes_layout = QHBoxLayout()
        self.system_log_to_file_checkbox = QCheckBox("Write System Log to File")
        self.bot_log_to_file_checkbox = QCheckBox("Write Bot Log to File")
        self.debug_checkbox = QCheckBox("Enable Debug Mode")
        self.debug_checkbox.setChecked(bool(self.app_cfg.get("debug", False)))
        checkboxes_layout.addWidget(self.system_log_to_file_checkbox)
        checkboxes_layout.addWidget(self.bot_log_to_file_checkbox)
        checkboxes_layout.addWidget(self.debug_checkbox)
        layout.addLayout(checkboxes_layout)

        # Connect logging
        self.system_log_handler = QTextEditLogger(self.system_log_output, logger_name=None)
        self.bot_log_handler = QTextEditLogger(self.bot_log_output, logger_name="silasblue")
        self.system_file_handler = FileLogger(os.path.join("logs", "system.log"), lambda: self.system_log_to_file_checkbox.isChecked())
        self.bot_file_handler = FileLogger(os.path.join("logs", "bot.log"), lambda: self.bot_log_to_file_checkbox.isChecked())

        root_logger = logging.getLogger()
        bot_logger = logging.getLogger("silasblue")
        for h in [self.system_log_handler, self.system_file_handler]:
            if h not in root_logger.handlers:
                root_logger.addHandler(h)
        for h in [self.bot_log_handler, self.bot_file_handler]:
            if h not in bot_logger.handlers:
                bot_logger.addHandler(h)

        # Connect buttons
        self.start_bot_btn.clicked.connect(self.start_bot)
        self.stop_bot_btn.clicked.connect(self.stop_bot)
        self.restart_bot_btn.clicked.connect(self.restart_bot)

        self.start_ai_btn.clicked.connect(self.start_ai_service)
        self.stop_ai_btn.clicked.connect(self.stop_ai_service)
        self.check_ai_btn.clicked.connect(self.update_ai_status)

        self.model_download_btn.clicked.connect(self.download_model)
        self.clear_system_log_btn.clicked.connect(lambda: self.system_log_output.clear())
        self.clear_bot_log_btn.clicked.connect(lambda: self.bot_log_output.clear())
        self.debug_checkbox.stateChanged.connect(self.on_debug_checkbox_changed)

    def setup_backend_settings_tab(self):
        """Settings tab for configuring Ollama/Llama.cpp, Remote URLs, and DM Policy."""
        self.backend_tab = QWidget()
        self.tabs.addTab(self.backend_tab, "AI & Bot Settings")
        layout = QVBoxLayout()
        self.backend_tab.setLayout(layout)

        # 1. AI Provider Selection
        provider_group = QGroupBox("Active AI Backend")
        provider_layout = QVBoxLayout()
        self.provider_btn_group = QButtonGroup(self)
        self.rb_ollama = QRadioButton("Ollama (Local or Remote Server)")
        self.rb_llamacpp = QRadioButton("Llama.cpp (llama-server / OpenAI-compatible)")
        self.provider_btn_group.addButton(self.rb_ollama, 1)
        self.provider_btn_group.addButton(self.rb_llamacpp, 2)

        if self.app_cfg.get("backend_type", "ollama") == "llamacpp":
            self.rb_llamacpp.setChecked(True)
        else:
            self.rb_ollama.setChecked(True)

        provider_layout.addWidget(self.rb_ollama)
        provider_layout.addWidget(self.rb_llamacpp)
        provider_group.setLayout(provider_layout)
        layout.addWidget(provider_group)

        # 2. Ollama Settings
        self.ollama_group = QGroupBox("Ollama Configuration (Supports Remote Servers - Issue #13)")
        ollama_layout = QVBoxLayout()
        ollama_url_row = QHBoxLayout()
        ollama_url_row.addWidget(QLabel("Server URL:"))
        self.ollama_url_input = QLineEdit(self.app_cfg.get("ollama_url", "http://localhost:11434"))
        self.ollama_url_input.setPlaceholderText("http://localhost:11434 or http://192.168.1.100:11434")
        ollama_url_row.addWidget(self.ollama_url_input)
        ollama_layout.addLayout(ollama_url_row)

        api_key_row = QHBoxLayout()
        api_key_row.addWidget(QLabel("API Key / Bearer Token (Optional for Reverse Proxies):"))
        self.api_key_input = QLineEdit(self.app_cfg.get("api_key", ""))
        self.api_key_input.setEchoMode(QLineEdit.Password)
        api_key_row.addWidget(self.api_key_input)
        ollama_layout.addLayout(api_key_row)
        self.ollama_group.setLayout(ollama_layout)
        layout.addWidget(self.ollama_group)

        # 3. Llama.cpp Settings
        self.llamacpp_group = QGroupBox("Llama.cpp (llama-server) Configuration")
        llamacpp_layout = QVBoxLayout()
        llama_url_row = QHBoxLayout()
        llama_url_row.addWidget(QLabel("Server URL:"))
        self.llamacpp_url_input = QLineEdit(self.app_cfg.get("llamacpp_url", "http://localhost:8080"))
        self.llamacpp_url_input.setPlaceholderText("http://localhost:8080 or http://192.168.1.100:8080")
        llama_url_row.addWidget(self.llamacpp_url_input)
        llamacpp_layout.addLayout(llama_url_row)

        # Optional local launcher
        binary_row = QHBoxLayout()
        binary_row.addWidget(QLabel("Local llama-server.exe (Optional):"))
        self.llamacpp_binary_input = QLineEdit(self.app_cfg.get("llamacpp_binary_path", ""))
        self.browse_binary_btn = QPushButton("Browse...")
        self.browse_binary_btn.clicked.connect(self.browse_llama_binary)
        binary_row.addWidget(self.llamacpp_binary_input)
        binary_row.addWidget(self.browse_binary_btn)
        llamacpp_layout.addLayout(binary_row)

        model_file_row = QHBoxLayout()
        model_file_row.addWidget(QLabel("Local .gguf Model File (Optional):"))
        self.llamacpp_model_input = QLineEdit(self.app_cfg.get("llamacpp_model_path", ""))
        self.browse_model_btn = QPushButton("Browse...")
        self.browse_model_btn.clicked.connect(self.browse_llama_model)
        model_file_row.addWidget(self.llamacpp_model_input)
        model_file_row.addWidget(self.browse_model_btn)
        llamacpp_layout.addLayout(model_file_row)

        self.llamacpp_group.setLayout(llamacpp_layout)
        layout.addWidget(self.llamacpp_group)

        # 4. Direct Message (DM) Policy
        dm_group = QGroupBox("Direct Message (DM) Policy")
        dm_layout = QVBoxLayout()
        self.allow_dms_checkbox = QCheckBox("Allow bot to respond to Direct Messages (DMs)")
        self.allow_dms_checkbox.setChecked(bool(self.app_cfg.get("allow_dms", True)))
        dm_layout.addWidget(self.allow_dms_checkbox)

        dm_model_row = QHBoxLayout()
        dm_model_row.addWidget(QLabel("DM Default Model:"))
        self.dm_model_input = QLineEdit(self.app_cfg.get("dm_default_model", ""))
        self.dm_model_input.setPlaceholderText("Leave empty to use active model")
        dm_model_row.addWidget(self.dm_model_input)
        dm_layout.addLayout(dm_model_row)

        dm_msg_row = QHBoxLayout()
        dm_msg_row.addWidget(QLabel("Disabled Notice Message:"))
        self.dm_disabled_msg_input = QLineEdit(self.app_cfg.get("dm_disabled_message", "Direct messages with the bot are disabled."))
        dm_msg_row.addWidget(self.dm_disabled_msg_input)
        dm_layout.addLayout(dm_msg_row)

        dm_group.setLayout(dm_layout)
        layout.addWidget(dm_group)

        # Save Settings Button
        save_btn_layout = QHBoxLayout()
        self.save_settings_btn = QPushButton("Save AI & Bot Settings")
        self.save_settings_btn.setStyleSheet("font-weight: bold; padding: 8px;")
        self.save_settings_btn.clicked.connect(self.save_backend_settings)
        save_btn_layout.addStretch(1)
        save_btn_layout.addWidget(self.save_settings_btn, 2)
        save_btn_layout.addStretch(1)
        layout.addLayout(save_btn_layout)
        layout.addStretch(1)

    def browse_llama_binary(self):
        file_path, _ = QFileDialog.getOpenFileName(self, "Select llama-server executable", "", "Executables (*.exe);;All Files (*)")
        if file_path:
            self.llamacpp_binary_input.setText(file_path)

    def browse_llama_model(self):
        file_path, _ = QFileDialog.getOpenFileName(self, "Select GGUF Model", "", "GGUF Models (*.gguf);;All Files (*)")
        if file_path:
            self.llamacpp_model_input.setText(file_path)

    def save_backend_settings(self):
        backend_type = "llamacpp" if self.rb_llamacpp.isChecked() else "ollama"
        self.app_cfg["backend_type"] = backend_type
        self.app_cfg["ollama_url"] = self.ollama_url_input.text().strip()
        self.app_cfg["api_key"] = self.api_key_input.text().strip()
        self.app_cfg["llamacpp_url"] = self.llamacpp_url_input.text().strip()
        self.app_cfg["llamacpp_binary_path"] = self.llamacpp_binary_input.text().strip()
        self.app_cfg["llamacpp_model_path"] = self.llamacpp_model_input.text().strip()
        self.app_cfg["allow_dms"] = self.allow_dms_checkbox.isChecked()
        self.app_cfg["dm_default_model"] = self.dm_model_input.text().strip()
        self.app_cfg["dm_disabled_message"] = self.dm_disabled_msg_input.text().strip()

        save_app_config(self.app_cfg)
        self.provider = get_ai_provider(self.app_cfg)
        reload_ai_provider()

        self.start_ai_btn.setText(f"Start {self.provider.name}")
        self.stop_ai_btn.setText(f"Stop {self.provider.name}")

        self.system_log_output.append(f"[SETTINGS] Saved! Switched to {self.provider.name} at {self.provider.base_url}")
        self.show_feedback(self.save_settings_btn, "Settings Saved!")
        self.update_ai_status()
        self.refresh_models_async()

    def update_ai_status(self):
        def check():
            return self.provider.status_sync(), self.provider.name
        future = self.executor.submit(check)

        def poll():
            if future.done():
                running, name = future.result()
                self.signals.ai_status_checked.emit(running, name)
            else:
                QTimer.singleShot(100, poll)
        QTimer.singleShot(100, poll)

    @Slot(bool, str)
    def on_ai_status_checked(self, running: bool, name: str):
        is_local = self.provider.is_local()
        locality = "Local" if is_local else "Remote"
        if running:
            self.ai_status_label.setText(f"{name} ({locality}): <b style='color:#00ff88;'>Online</b>")
        else:
            self.ai_status_label.setText(f"{name} ({locality}): <b style='color:#ff4466;'>Offline / Unreachable</b>")

        # Disable local process controls if remote
        self.start_ai_btn.setEnabled(is_local)
        self.stop_ai_btn.setEnabled(is_local)

    def start_ai_service(self):
        self.system_log_output.append(f"[INFO] Starting {self.provider.name} local service...")
        started = self.provider.start_local_service()
        if started:
            self.system_log_output.append(f"[INFO] {self.provider.name} started.")
        else:
            self.system_log_output.append(f"[WARN] Could not start {self.provider.name} locally.")
        QTimer.singleShot(1500, self.update_ai_status)

    def stop_ai_service(self):
        self.system_log_output.append(f"[INFO] Stopping {self.provider.name} local service...")
        stopped = self.provider.stop_local_service()
        if stopped:
            self.system_log_output.append(f"[INFO] {self.provider.name} stopped.")
        else:
            self.system_log_output.append(f"[WARN] Could not stop {self.provider.name}.")
        QTimer.singleShot(1000, self.update_ai_status)

    def refresh_models_async(self):
        def fetch():
            if self.provider.status_sync():
                return self.provider.list_models_sync()
            return None
        future = self.executor.submit(fetch)

        def poll():
            if future.done():
                models = future.result()
                if models is None:
                    self.model_select.clear()
                    self.model_select.addItem(f"{self.provider.name} not reachable")
                else:
                    self.signals.models_loaded.emit(models)
            else:
                QTimer.singleShot(100, poll)
        QTimer.singleShot(100, poll)

    @Slot(list)
    def on_models_loaded(self, models):
        self.model_select.clear()
        if not models:
            self.model_select.addItem("No models found")
        else:
            for m in models:
                self.model_select.addItem(m)

    def download_model(self):
        model_name = self.model_download_input.text().strip()
        if not model_name:
            return
        self.model_download_progress.setVisible(True)
        self.model_download_progress.setValue(0)
        self.system_log_output.append(f"[INFO] Requesting model '{model_name}' download via REST API...")

        def progress_cb(percent, status):
            self.signals.model_download_progress.emit(percent, status)

        def run_pull():
            self.provider.pull_model_sync(model_name, progress_cb)
            self.signals.model_download_finished.emit()
            self.refresh_models_async()

        self.executor.submit(run_pull)

    @Slot(int, str)
    def on_model_download_progress(self, percent, status):
        if percent < 0:
            self.model_download_progress.setFormat("Error")
            self.system_log_output.append(f"[ERROR] Download failed: {status}")
        else:
            self.model_download_progress.setValue(percent)
            self.system_log_output.append(f"[PULL] {status}")

    @Slot()
    def on_model_download_finished(self):
        self.model_download_progress.setVisible(False)

    def update_bot_status(self):
        try:
            from bot_core import _bot_thread
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
            status_text += f" <span style='color:red;'>({self._bot_status_error})</span>"
        self.bot_status_label.setText(status_text)

    def start_bot(self):
        try:
            if self._start_bot_func:
                self._start_bot_func()
                self.system_log_output.append("[INFO] Start Bot requested.")
                self.show_feedback(self.start_bot_btn, "Bot Started!")
        except Exception as e:
            self.system_log_output.append(f"[ERROR] Failed to start bot: {e}")

    def stop_bot(self):
        try:
            if self._stop_bot_func:
                self._stop_bot_func()
                self.system_log_output.append("[INFO] Stop Bot requested. Closing.")
                self.close()
        except Exception as e:
            self.system_log_output.append(f"[ERROR] Failed to stop bot: {e}")

    def restart_bot(self):
        try:
            if self._restart_bot_func:
                self.system_log_output.append("[INFO] Restarting bot...")
                self._restart_bot_func()
                self.show_feedback(self.restart_bot_btn, "Bot Restarted!")
        except Exception as e:
            self.system_log_output.append(f"[ERROR] Failed to restart bot: {e}")

    def update_servers_list(self):
        try:
            current_servers = [self.servers_list.itemData(i) for i in range(self.servers_list.count())]
            new_servers = [(guild.name, guild.id) for guild in getattr(bot, "guilds", [])]
            if len(current_servers) != len(new_servers) or any(str(gid) not in [str(x[1]) for x in new_servers] for gid in current_servers):
                self.servers_list.clear()
                for name, gid in new_servers:
                    self.servers_list.addItem(f"{name} ({gid})", userData=gid)
            if hasattr(self, "server_config_tab"):
                self.server_config_tab.update_guilds()
        except Exception:
            pass

    def process_gui_events(self):
        """Processes logs and events directly from the thread-safe queue."""
        while not gui_event_queue.empty():
            try:
                entry = gui_event_queue.get_nowait()
                event = entry.get("event")
                data = entry.get("data", {})
                if event == "config_change":
                    self.system_log_output.append(f"[Config Change] Guild: {data.get('guild_id')} Field: {data.get('field')} -> {data.get('value')}")
                elif event == "prompt":
                    guild = data.get("guild_name") or data.get("guild_id")
                    self.bot_log_output.append(f"[{guild}] {data.get('user')}: {data.get('prompt')}")
                elif event == "reply":
                    guild = data.get("guild_name") or data.get("guild_id")
                    reply_snippet = data.get("reply", "")[:180]
                    self.bot_log_output.append(f"[{guild}] Silas Blue -> {reply_snippet}...")
                elif event == "server_joined":
                    guild_name = data.get("guild_name", "Unknown Server")
                    guild_id = data.get("guild_id", "")
                    self.system_log_output.append(f"[Discord] Silas Blue joined new server: {guild_name} ({guild_id})")
                    self.update_servers_list()
                elif event == "server_left":
                    guild_name = data.get("guild_name", "Unknown Server")
                    guild_id = data.get("guild_id", "")
                    self.system_log_output.append(f"[Discord] Silas Blue left server: {guild_name} ({guild_id})")
                    self.update_servers_list()
            except queue.Empty:
                break
            except Exception:
                pass

    def open_invite_dialog(self):
        """Opens the Invite Bot dialog."""
        from .invite_dialog import InviteBotDialog
        dialog = InviteBotDialog(self)
        dialog.exec()

    def change_theme(self, theme_name):
        theme_file = f"themes/{theme_name.lower()}.json"
        if os.path.exists(theme_file):
            self.theme_manager.apply_theme(self, theme_file)

    def show_feedback(self, button, message):
        orig_style = button.styleSheet()
        button.setStyleSheet("background-color: #00f0ff; color: #181825;")
        QTimer.singleShot(250, lambda: button.setStyleSheet(orig_style))

    def on_debug_checkbox_changed(self, state):
        debug_enabled = state == Qt.Checked
        config.DEBUG = debug_enabled
        logging.getLogger().setLevel(logging.DEBUG if debug_enabled else logging.INFO)
        logging.getLogger("silasblue").setLevel(logging.DEBUG if debug_enabled else logging.INFO)

    def write(self, msg):
        if msg.strip():
            self.system_log_output.append(msg.strip())

    def flush(self):
        pass

    def redirect_output_to_log(self):
        sys.stdout = self
        sys.stderr = self

if __name__ == "__main__":
    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())