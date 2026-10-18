import sys
import os
import logging
import concurrent.futures
import json
import traceback
import queue
from datetime import datetime

from PySide6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QVBoxLayout, QHBoxLayout, QLabel,
    QPushButton, QComboBox, QLineEdit, QProgressBar, QCheckBox, QStackedWidget,
    QTextEdit, QSpinBox, QFileDialog, QGroupBox, QRadioButton, QButtonGroup,
    QFrame, QScrollArea, QSizePolicy
)
from PySide6.QtCore import Qt, QTimer, Signal, Slot, QObject, QUrl
from PySide6.QtGui import QFont, QDesktopServices

from .theme_manager import ThemeManager
from .server_config_page import ServerConfigPage
from ai_providers import get_ai_provider, AIProvider, RemoteOpenAIProvider, REMOTE_PRESETS
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
    backend_detected = Signal(dict)
    remote_test_finished = Signal(bool, str)
    remote_models_fetched = Signal(list)
    web_search_tested = Signal(bool, str)


class MainWindow(QMainWindow):
    """
    Modern Discord-grade Control Panel for Silas Blue.
    Features a sidebar navigation rail, KPI dashboard, dynamic AI backend configuration,
    and a dedicated dual-panel logging console.
    """

    def __init__(self, start_bot=None, stop_bot=None, restart_bot=None):
        super().__init__()
        self.setWindowTitle("Silas Blue Control Panel")
        self.setMinimumSize(980, 700)
        self.resize(1020, 740)

        self._start_bot_func = start_bot
        self._stop_bot_func = stop_bot
        self._restart_bot_func = restart_bot

        self._bot_status = "Stopped"
        self._bot_status_error = None

        self.app_cfg = load_app_config()
        self.provider: AIProvider = get_ai_provider(self.app_cfg)

        self.theme_manager = ThemeManager()

        self.executor = concurrent.futures.ThreadPoolExecutor(max_workers=4)

        # Worker signals
        self.signals = WorkerSignals()
        self.signals.models_loaded.connect(self.on_models_loaded)
        self.signals.ai_status_checked.connect(self.on_ai_status_checked)
        self.signals.model_download_progress.connect(self.on_model_download_progress)
        self.signals.model_download_finished.connect(self.on_model_download_finished)
        self.signals.backend_detected.connect(self.on_backend_detected)
        self.signals.remote_test_finished.connect(self.on_remote_test_finished)
        self.signals.remote_models_fetched.connect(self.on_remote_models_fetched)
        self.signals.web_search_tested.connect(self.on_web_search_tested)

        # Compatibility widgets (in case external modules query them)
        self.bot_status_label = QLabel("Bot Status: Stopped")
        self.ai_status_label = QLabel(f"{self.provider.name} Status: Checking...")
        self.servers_list = QComboBox()
        self.servers_list.setVisible(False)

        # Build Main UI Shell
        self.setup_ui_shell()

        # Apply saved or default theme
        saved_theme = self.app_cfg.get("theme", "retrowave").lower()
        theme_path = f"themes/{saved_theme}.json"
        if not os.path.exists(theme_path):
            theme_path = "themes/retrowave.json"
        self.theme_manager.apply_theme(self, theme_path)

        # Setup Logging Handlers
        self.setup_logging_handlers()

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
        QTimer.singleShot(1200, self.auto_detect_on_startup)

    # -------------------------------------------------------------------------
    # UI SHELL & SIDEBAR NAVIGATION
    # -------------------------------------------------------------------------
    def setup_ui_shell(self):
        root_widget = QWidget()
        root_layout = QHBoxLayout(root_widget)
        root_layout.setContentsMargins(0, 0, 0, 0)
        root_layout.setSpacing(0)
        self.setCentralWidget(root_widget)

        # 1. Left Sidebar
        self.sidebar = QFrame()
        self.sidebar.setObjectName("sidebar")
        sidebar_layout = QVBoxLayout(self.sidebar)
        sidebar_layout.setContentsMargins(10, 14, 10, 14)
        sidebar_layout.setSpacing(6)

        # App branding header
        header_frame = QFrame()
        header_frame.setObjectName("sidebarHeader")
        h_layout = QVBoxLayout(header_frame)
        h_layout.setContentsMargins(4, 0, 4, 8)
        h_layout.setSpacing(2)

        app_title = QLabel("SILAS BLUE")
        app_title.setObjectName("appTitle")
        app_sub = QLabel("AI Discord Assistant v3.3.1")
        app_sub.setObjectName("appSubtitle")
        h_layout.addWidget(app_title)
        h_layout.addWidget(app_sub)

        # Sidebar status pill indicator
        self.sidebar_bot_pill = QLabel("● Stopped")
        self.sidebar_bot_pill.setObjectName("statusPillOffline")
        self.sidebar_bot_pill.setAlignment(Qt.AlignCenter)
        h_layout.addWidget(self.sidebar_bot_pill)

        sidebar_layout.addWidget(header_frame)
        sidebar_layout.addSpacing(10)

        # Navigation buttons
        self.nav_buttons = []

        self.btn_nav_dash = QPushButton("  📊  Dashboard")
        self.btn_nav_ai = QPushButton("  🧠  AI & Models")
        self.btn_nav_servers = QPushButton("  🛡️  Server Config")
        self.btn_nav_logs = QPushButton("  📜  Logs & Console")
        self.btn_nav_settings = QPushButton("  ⚙️  Settings")

        for idx, btn in enumerate([self.btn_nav_dash, self.btn_nav_ai, self.btn_nav_servers, self.btn_nav_logs, self.btn_nav_settings]):
            btn.setObjectName("navButton")
            btn.setCheckable(True)
            btn.setCursor(Qt.PointingHandCursor)
            btn.clicked.connect(lambda checked=False, i=idx: self.switch_page(i))
            sidebar_layout.addWidget(btn)
            self.nav_buttons.append(btn)

        sidebar_layout.addStretch(1)

        # Sidebar footer
        footer_card = QFrame()
        footer_card.setObjectName("card")
        f_layout = QVBoxLayout(footer_card)
        f_layout.setContentsMargins(8, 8, 8, 8)
        f_layout.setSpacing(4)

        self.sidebar_model_label = QLabel(f"Model: {self.app_cfg.get('active_model', 'None')[:16]}")
        self.sidebar_model_label.setStyleSheet("font-size: 11px; opacity: 0.8;")
        f_layout.addWidget(self.sidebar_model_label)

        invite_shortcut_btn = QPushButton("➕ Invite Bot")
        invite_shortcut_btn.setCursor(Qt.PointingHandCursor)
        invite_shortcut_btn.clicked.connect(self.open_invite_dialog)
        f_layout.addWidget(invite_shortcut_btn)

        sidebar_layout.addWidget(footer_card)
        root_layout.addWidget(self.sidebar)

        # 2. Central Content Area (QStackedWidget)
        self.content_stack = QStackedWidget()
        root_layout.addWidget(self.content_stack, 1)

        # Build each page
        self.setup_dashboard_page()
        self.setup_ai_models_page()

        self.server_config_tab = ServerConfigPage(self)
        self.content_stack.addWidget(self.server_config_tab)  # Page 2

        self.setup_logs_page()       # Page 3
        self.setup_settings_page()   # Page 4

        # Select initial page (Dashboard)
        self.switch_page(0)

    def switch_page(self, page_index: int):
        self.content_stack.setCurrentIndex(page_index)
        for idx, btn in enumerate(self.nav_buttons):
            is_active = (idx == page_index)
            btn.setChecked(is_active)
            if is_active:
                btn.setStyleSheet("border-left: 3px solid #ff5ecb; font-weight: bold; background-color: rgba(255, 94, 203, 0.15);")
            else:
                btn.setStyleSheet("")

    # -------------------------------------------------------------------------
    # PAGE 0: DASHBOARD
    # -------------------------------------------------------------------------
    def setup_dashboard_page(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(20, 20, 20, 20)
        layout.setSpacing(16)

        # Header Title
        title_row = QHBoxLayout()
        page_title = QLabel("Dashboard")
        page_title.setStyleSheet("font-size: 20px; font-weight: 700;")
        title_row.addWidget(page_title)
        title_row.addStretch(1)

        refresh_all_btn = QPushButton("🔄 Refresh Status")
        refresh_all_btn.clicked.connect(lambda: [self.update_ai_status(), self.update_bot_status(), self.refresh_models_async()])
        title_row.addWidget(refresh_all_btn)
        layout.addLayout(title_row)

        # --- Section 1: KPI Cards Grid (4 Cards) ---
        kpi_row = QHBoxLayout()
        kpi_row.setSpacing(12)

        # KPI 1: Bot Status
        self.kpi_bot_card = QFrame()
        self.kpi_bot_card.setObjectName("kpiCard")
        b_lay = QVBoxLayout(self.kpi_bot_card)
        b_lay.setContentsMargins(12, 10, 12, 10)
        b_title = QLabel("BOT DAEMON")
        b_title.setObjectName("kpiTitle")
        self.kpi_bot_pill = QLabel("STOPPED")
        self.kpi_bot_pill.setObjectName("statusPillOffline")
        self.kpi_bot_pill.setAlignment(Qt.AlignCenter)
        self.kpi_bot_subtext = QLabel("Discord Gateway Offline")
        self.kpi_bot_subtext.setObjectName("kpiSubtext")
        b_lay.addWidget(b_title)
        b_lay.addWidget(self.kpi_bot_pill)
        b_lay.addWidget(self.kpi_bot_subtext)
        kpi_row.addWidget(self.kpi_bot_card)

        # KPI 2: AI Backend
        self.kpi_ai_card = QFrame()
        self.kpi_ai_card.setObjectName("kpiCard")
        ai_lay = QVBoxLayout(self.kpi_ai_card)
        ai_lay.setContentsMargins(12, 10, 12, 10)
        ai_title = QLabel("AI BACKEND")
        ai_title.setObjectName("kpiTitle")
        self.kpi_ai_val = QLabel(self.provider.name)
        self.kpi_ai_val.setObjectName("kpiValue")
        self.kpi_ai_pill = QLabel("CHECKING...")
        self.kpi_ai_pill.setObjectName("statusPillWarning")
        self.kpi_ai_pill.setAlignment(Qt.AlignCenter)
        self.kpi_ai_subtext = QLabel(self.provider.base_url)
        self.kpi_ai_subtext.setObjectName("kpiSubtext")
        ai_lay.addWidget(ai_title)
        ai_lay.addWidget(self.kpi_ai_pill)
        ai_lay.addWidget(self.kpi_ai_subtext)
        kpi_row.addWidget(self.kpi_ai_card)

        # KPI 3: Active Model
        self.kpi_model_card = QFrame()
        self.kpi_model_card.setObjectName("kpiCard")
        m_lay = QVBoxLayout(self.kpi_model_card)
        m_lay.setContentsMargins(12, 10, 12, 10)
        m_title = QLabel("ACTIVE MODEL")
        m_title.setObjectName("kpiTitle")
        self.kpi_model_val = QLabel(self.app_cfg.get("active_model", "None"))
        self.kpi_model_val.setObjectName("kpiValue")
        m_sub = QLabel("Inference Engine")
        m_sub.setObjectName("kpiSubtext")
        m_lay.addWidget(m_title)
        m_lay.addWidget(self.kpi_model_val)
        m_lay.addWidget(m_sub)
        kpi_row.addWidget(self.kpi_model_card)

        # KPI 4: Connected Servers
        self.kpi_servers_card = QFrame()
        self.kpi_servers_card.setObjectName("kpiCard")
        s_lay = QVBoxLayout(self.kpi_servers_card)
        s_lay.setContentsMargins(12, 10, 12, 10)
        s_title = QLabel("SERVERS")
        s_title.setObjectName("kpiTitle")
        self.kpi_servers_val = QLabel("0 Guilds")
        self.kpi_servers_val.setObjectName("kpiValue")
        s_sub = QLabel("Connected Discord Servers")
        s_sub.setObjectName("kpiSubtext")
        s_lay.addWidget(s_title)
        s_lay.addWidget(self.kpi_servers_val)
        s_lay.addWidget(s_sub)
        kpi_row.addWidget(self.kpi_servers_card)

        layout.addLayout(kpi_row)

        # --- Section 2: Quick Control Cards (Two Columns) ---
        controls_row = QHBoxLayout()
        controls_row.setSpacing(14)

        # Column 1: Bot & AI Controls Card
        daemon_card = QFrame()
        daemon_card.setObjectName("card")
        d_layout = QVBoxLayout(daemon_card)
        d_layout.setContentsMargins(14, 12, 14, 12)
        d_layout.setSpacing(10)

        d_title = QLabel("Bot & Service Controls")
        d_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        d_layout.addWidget(d_title)

        # Bot actions row
        bot_btn_row = QHBoxLayout()
        self.start_bot_btn = QPushButton("▶  Start Bot")
        self.start_bot_btn.setObjectName("successBtn")
        self.stop_bot_btn = QPushButton("⏹  Stop Bot")
        self.stop_bot_btn.setObjectName("dangerBtn")
        self.restart_bot_btn = QPushButton("🔄  Restart Bot")

        bot_btn_row.addWidget(self.start_bot_btn)
        bot_btn_row.addWidget(self.stop_bot_btn)
        bot_btn_row.addWidget(self.restart_bot_btn)
        d_layout.addLayout(bot_btn_row)

        # AI Backend actions row
        ai_btn_row = QHBoxLayout()
        self.start_ai_btn = QPushButton(f"Start {self.provider.name}")
        self.stop_ai_btn = QPushButton(f"Stop {self.provider.name}")
        self.check_ai_btn = QPushButton("Check Status")
        ai_btn_row.addWidget(self.start_ai_btn)
        ai_btn_row.addWidget(self.stop_ai_btn)
        ai_btn_row.addWidget(self.check_ai_btn)
        d_layout.addLayout(ai_btn_row)

        controls_row.addWidget(daemon_card, 1)

        # Column 2: Quick Model Switcher Card
        model_card = QFrame()
        model_card.setObjectName("card")
        m_card_layout = QVBoxLayout(model_card)
        m_card_layout.setContentsMargins(14, 12, 14, 12)
        m_card_layout.setSpacing(8)

        m_card_title = QLabel("Active Model & Quick Download")
        m_card_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        m_card_layout.addWidget(m_card_title)

        # Model select row
        m_select_row = QHBoxLayout()
        m_select_row.addWidget(QLabel("Model:"))
        self.model_select = QComboBox()
        self.model_select.setMinimumWidth(180)
        m_select_row.addWidget(self.model_select, 1)
        self.refresh_models_btn = QPushButton("🔄")
        self.refresh_models_btn.setToolTip("Refresh models list")
        self.refresh_models_btn.clicked.connect(self.refresh_models_async)
        m_select_row.addWidget(self.refresh_models_btn)
        m_card_layout.addLayout(m_select_row)

        # Pull model row
        m_pull_row = QHBoxLayout()
        self.model_download_input = QLineEdit()
        self.model_download_input.setPlaceholderText("Download model (e.g. llama3:8b)")
        self.model_download_btn = QPushButton("Pull Model")
        m_pull_row.addWidget(self.model_download_input, 1)
        m_pull_row.addWidget(self.model_download_btn)
        m_card_layout.addLayout(m_pull_row)

        self.model_download_progress = QProgressBar()
        self.model_download_progress.setVisible(False)
        m_card_layout.addWidget(self.model_download_progress)

        controls_row.addWidget(model_card, 1)
        layout.addLayout(controls_row)

        # --- Section 3: Recent Activity Stream Card ---
        activity_card = QFrame()
        activity_card.setObjectName("card")
        act_layout = QVBoxLayout(activity_card)
        act_layout.setContentsMargins(14, 12, 14, 12)
        act_layout.setSpacing(8)

        act_header = QHBoxLayout()
        act_title = QLabel("Live Bot Activity")
        act_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        act_header.addWidget(act_title)
        act_header.addStretch(1)

        view_full_logs_btn = QPushButton("Open Full Console ↗")
        view_full_logs_btn.setCursor(Qt.PointingHandCursor)
        view_full_logs_btn.clicked.connect(lambda: self.switch_page(3))
        act_header.addWidget(view_full_logs_btn)
        act_layout.addLayout(act_header)

        self.recent_activity_output = QTextEdit()
        self.recent_activity_output.setReadOnly(True)
        self.recent_activity_output.setFont(QFont("Consolas", 10))
        self.recent_activity_output.setPlaceholderText("Bot events, user prompts, and AI responses will stream here in real-time...")
        act_layout.addWidget(self.recent_activity_output, 1)

        layout.addWidget(activity_card, 1)

        # Connect actions
        self.start_bot_btn.clicked.connect(self.start_bot)
        self.stop_bot_btn.clicked.connect(self.stop_bot)
        self.restart_bot_btn.clicked.connect(self.restart_bot)

        self.start_ai_btn.clicked.connect(self.start_ai_service)
        self.stop_ai_btn.clicked.connect(self.stop_ai_service)
        self.check_ai_btn.clicked.connect(self.update_ai_status)

        self.model_download_btn.clicked.connect(self.download_model)
        self.model_select.currentTextChanged.connect(self.on_active_model_changed)

        self.content_stack.addWidget(page)  # Page 0

    # -------------------------------------------------------------------------
    # PAGE 1: AI & MODELS CONFIGURATION
    # -------------------------------------------------------------------------
    def setup_ai_models_page(self):
        page = QWidget()
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.NoFrame)

        container = QWidget()
        layout = QVBoxLayout(container)
        layout.setContentsMargins(20, 20, 20, 20)
        layout.setSpacing(14)

        # Header Title
        title_row = QHBoxLayout()
        page_title = QLabel("AI Backend & Model Configuration")
        page_title.setStyleSheet("font-size: 20px; font-weight: 700;")
        title_row.addWidget(page_title)
        title_row.addStretch(1)

        # Auto-Detect Local AI Button in header
        self.autodetect_btn = QPushButton("🔍  Auto-Detect Local AI")
        self.autodetect_btn.setStyleSheet("font-weight: 600; padding: 6px 12px;")
        self.autodetect_btn.setToolTip("Scan local machine for active Llama.cpp or Ollama instances")
        self.autodetect_btn.clicked.connect(self.run_auto_detect_all)
        title_row.addWidget(self.autodetect_btn)
        layout.addLayout(title_row)

        self.autodetect_status = QLabel("")
        self.autodetect_status.setStyleSheet("color: #00ff88; font-weight: 600;")
        layout.addWidget(self.autodetect_status)

        # --- 1. Backend Selector Card ---
        provider_card = QFrame()
        provider_card.setObjectName("card")
        p_layout = QVBoxLayout(provider_card)
        p_layout.setContentsMargins(14, 12, 14, 12)
        p_layout.setSpacing(8)

        p_title = QLabel("Select Active AI Provider")
        p_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        p_layout.addWidget(p_title)

        self.provider_btn_group = QButtonGroup(self)
        self.rb_ollama = QRadioButton("🦙 Ollama (Local Daemon or Remote Server)")
        self.rb_llamacpp = QRadioButton("⚡ Llama.cpp (llama-server / OpenAI-compatible)")
        self.rb_remote = QRadioButton("☁️ Remote OpenAI-Compatible Cloud API (Xiaomi MiMo, Groq, OpenRouter, DeepSeek, etc.)")

        self.provider_btn_group.addButton(self.rb_ollama, 0)
        self.provider_btn_group.addButton(self.rb_llamacpp, 1)
        self.provider_btn_group.addButton(self.rb_remote, 2)

        backend_type = self.app_cfg.get("backend_type", "ollama").lower()
        if backend_type in ("llamacpp", "llama.cpp", "llama_cpp"):
            self.rb_llamacpp.setChecked(True)
        elif backend_type in ("remote", "remote_openai", "openai", "remote_api"):
            self.rb_remote.setChecked(True)
        else:
            self.rb_ollama.setChecked(True)

        p_layout.addWidget(self.rb_ollama)
        p_layout.addWidget(self.rb_llamacpp)
        p_layout.addWidget(self.rb_remote)
        layout.addWidget(provider_card)

        # --- 2. Dynamic Provider Settings Stack ---
        self.provider_stack = QStackedWidget()

        # View 0: Ollama Settings
        self.ollama_group = QFrame()
        self.ollama_group.setObjectName("card")
        o_layout = QVBoxLayout(self.ollama_group)
        o_layout.setContentsMargins(14, 12, 14, 12)
        o_layout.setSpacing(10)

        o_title = QLabel("Ollama Settings")
        o_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        o_layout.addWidget(o_title)

        o_url_row = QHBoxLayout()
        o_url_row.addWidget(QLabel("Server URL:"))
        self.ollama_url_input = QLineEdit(self.app_cfg.get("ollama_url", "http://localhost:11434"))
        self.ollama_url_input.setPlaceholderText("http://localhost:11434 or http://192.168.1.100:11434")
        o_url_row.addWidget(self.ollama_url_input, 1)
        o_layout.addLayout(o_url_row)

        o_key_row = QHBoxLayout()
        o_key_row.addWidget(QLabel("API Key / Bearer Token (Optional):"))
        self.api_key_input = QLineEdit(self.app_cfg.get("api_key", ""))
        self.api_key_input.setEchoMode(QLineEdit.Password)
        o_key_row.addWidget(self.api_key_input, 1)
        o_layout.addLayout(o_key_row)

        self.provider_stack.addWidget(self.ollama_group)

        # View 1: Llama.cpp Settings
        self.llamacpp_group = QFrame()
        self.llamacpp_group.setObjectName("card")
        l_layout = QVBoxLayout(self.llamacpp_group)
        l_layout.setContentsMargins(14, 12, 14, 12)
        l_layout.setSpacing(10)

        l_title = QLabel("Llama.cpp (llama-server) Settings")
        l_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        l_layout.addWidget(l_title)

        l_url_row = QHBoxLayout()
        l_url_row.addWidget(QLabel("Server URL:"))
        self.llamacpp_url_input = QLineEdit(self.app_cfg.get("llamacpp_url", "http://localhost:8080"))
        self.llamacpp_url_input.setPlaceholderText("http://localhost:8080 or http://127.0.0.1:9931")
        self.detect_llama_btn = QPushButton("🔍 Auto-Detect")
        self.detect_llama_btn.clicked.connect(self.run_auto_detect_llama)
        l_url_row.addWidget(self.llamacpp_url_input, 1)
        l_url_row.addWidget(self.detect_llama_btn)
        l_layout.addLayout(l_url_row)

        binary_row = QHBoxLayout()
        binary_row.addWidget(QLabel("Local llama.exe Binary:"))
        self.llamacpp_binary_input = QLineEdit(self.app_cfg.get("llamacpp_binary_path", ""))
        self.locate_binary_btn = QPushButton("Auto-Find")
        self.locate_binary_btn.clicked.connect(self.auto_find_llama_binary)
        self.browse_binary_btn = QPushButton("Browse...")
        self.browse_binary_btn.clicked.connect(self.browse_llama_binary)
        binary_row.addWidget(self.llamacpp_binary_input, 1)
        binary_row.addWidget(self.locate_binary_btn)
        binary_row.addWidget(self.browse_binary_btn)
        l_layout.addLayout(binary_row)

        model_file_row = QHBoxLayout()
        model_file_row.addWidget(QLabel("Local .gguf Model:"))
        self.llamacpp_model_input = QLineEdit(self.app_cfg.get("llamacpp_model_path", ""))
        self.locate_model_btn = QPushButton("Auto-Find")
        self.locate_model_btn.clicked.connect(self.auto_find_llama_model)
        self.browse_model_btn = QPushButton("Browse...")
        self.browse_model_btn.clicked.connect(self.browse_llama_model)
        model_file_row.addWidget(self.llamacpp_model_input, 1)
        model_file_row.addWidget(self.locate_model_btn)
        model_file_row.addWidget(self.browse_model_btn)
        l_layout.addLayout(model_file_row)

        self.provider_stack.addWidget(self.llamacpp_group)

        # View 2: Remote OpenAI-Compatible API Settings
        self.remote_group = QFrame()
        self.remote_group.setObjectName("card")
        r_layout = QVBoxLayout(self.remote_group)
        r_layout.setContentsMargins(14, 12, 14, 12)
        r_layout.setSpacing(10)

        r_title = QLabel("Remote OpenAI-Compatible API Settings")
        r_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        r_layout.addWidget(r_title)

        preset_row = QHBoxLayout()
        preset_row.addWidget(QLabel("Provider Preset:"))
        self.remote_preset_combo = QComboBox()
        for p_name in REMOTE_PRESETS.keys():
            self.remote_preset_combo.addItem(p_name)
        saved_preset = self.app_cfg.get("remote_preset", "Xiaomi MiMo")
        if saved_preset in REMOTE_PRESETS:
            self.remote_preset_combo.setCurrentText(saved_preset)
        self.remote_preset_combo.currentTextChanged.connect(self.on_remote_preset_changed)
        preset_row.addWidget(self.remote_preset_combo, 1)
        r_layout.addLayout(preset_row)

        r_url_row = QHBoxLayout()
        r_url_row.addWidget(QLabel("Endpoint URL:"))
        self.remote_url_input = QLineEdit(self.app_cfg.get("remote_base_url", "https://api.xiaomimimo.com/v1"))
        r_url_row.addWidget(self.remote_url_input, 1)
        r_layout.addLayout(r_url_row)

        r_key_row = QHBoxLayout()
        r_key_row.addWidget(QLabel("API Key:"))
        self.remote_key_input = QLineEdit(self.app_cfg.get("remote_api_key", ""))
        self.remote_key_input.setEchoMode(QLineEdit.Password)
        self.toggle_remote_key_btn = QPushButton("👁 Show")
        self.toggle_remote_key_btn.setFixedWidth(75)
        self.toggle_remote_key_btn.clicked.connect(self.toggle_remote_key_visibility)
        r_key_row.addWidget(self.remote_key_input, 1)
        r_key_row.addWidget(self.toggle_remote_key_btn)
        r_layout.addLayout(r_key_row)

        r_model_row = QHBoxLayout()
        r_model_row.addWidget(QLabel("Default Model:"))
        self.remote_model_input = QLineEdit(self.app_cfg.get("remote_model", "mimo-v2.6-pro"))
        self.fetch_remote_models_btn = QPushButton("📡 Fetch Models")
        self.fetch_remote_models_btn.clicked.connect(self.fetch_remote_models)
        r_model_row.addWidget(self.remote_model_input, 1)
        r_model_row.addWidget(self.fetch_remote_models_btn)
        r_layout.addLayout(r_model_row)

        r_test_row = QHBoxLayout()
        self.test_remote_btn = QPushButton("⚡ Test Connection")
        self.test_remote_btn.setObjectName("primaryBtn")
        self.test_remote_btn.clicked.connect(self.test_remote_connection)
        self.remote_test_status = QLabel("")
        r_test_row.addWidget(self.test_remote_btn)
        r_test_row.addWidget(self.remote_test_status, 1)
        r_layout.addLayout(r_test_row)

        self.provider_stack.addWidget(self.remote_group)
        layout.addWidget(self.provider_stack)

        # Wire radio buttons to dynamic stack switching
        def update_stack_view(btn_id):
            self.provider_stack.setCurrentIndex(btn_id)

        self.provider_btn_group.idClicked.connect(update_stack_view)
        # Set initial stack index
        if self.rb_llamacpp.isChecked():
            self.provider_stack.setCurrentIndex(1)
        elif self.rb_remote.isChecked():
            self.provider_stack.setCurrentIndex(2)
        else:
            self.provider_stack.setCurrentIndex(0)

        # --- 3. Bot Persona Card ---
        persona_card = QFrame()
        persona_card.setObjectName("card")
        per_layout = QVBoxLayout(persona_card)
        per_layout.setContentsMargins(14, 12, 14, 12)
        per_layout.setSpacing(6)

        per_title = QLabel("System Prompt / Bot Persona")
        per_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        per_layout.addWidget(per_title)
        per_layout.addWidget(QLabel("Defines Silas Blue's personality and instructions sent to the AI backend:"))

        self.system_prompt_input = QTextEdit()
        default_persona = self.app_cfg.get("system_prompt", "You are Silas Blue, a helpful and knowledgeable AI Discord assistant.")
        self.system_prompt_input.setPlainText(default_persona)
        self.system_prompt_input.setMaximumHeight(80)
        per_layout.addWidget(self.system_prompt_input)
        layout.addWidget(persona_card)

        # --- 4. Direct Message Policy Card ---
        dm_card = QFrame()
        dm_card.setObjectName("card")
        dm_layout = QVBoxLayout(dm_card)
        dm_layout.setContentsMargins(14, 12, 14, 12)
        dm_layout.setSpacing(8)

        dm_title = QLabel("Direct Message (DM) Policy")
        dm_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        dm_layout.addWidget(dm_title)

        self.allow_dms_checkbox = QCheckBox("Allow bot to respond to Direct Messages (DMs)")
        self.allow_dms_checkbox.setChecked(bool(self.app_cfg.get("allow_dms", True)))
        dm_layout.addWidget(self.allow_dms_checkbox)

        dm_model_row = QHBoxLayout()
        dm_model_row.addWidget(QLabel("DM Dedicated Model:"))
        self.dm_model_input = QLineEdit(self.app_cfg.get("dm_default_model", ""))
        self.dm_model_input.setPlaceholderText("Leave empty to use active model")
        dm_model_row.addWidget(self.dm_model_input, 1)
        dm_layout.addLayout(dm_model_row)

        dm_msg_row = QHBoxLayout()
        dm_msg_row.addWidget(QLabel("Disabled Notice:"))
        self.dm_disabled_msg_input = QLineEdit(self.app_cfg.get("dm_disabled_message", "Direct messages with the bot are disabled."))
        dm_msg_row.addWidget(self.dm_disabled_msg_input, 1)
        dm_layout.addLayout(dm_msg_row)

        layout.addWidget(dm_card)

        # --- 5. Web Search & Auto-Grounding Card ---
        search_card = QFrame()
        search_card.setObjectName("card")
        sc_layout = QVBoxLayout(search_card)
        sc_layout.setContentsMargins(14, 12, 14, 12)
        sc_layout.setSpacing(10)

        sc_title = QLabel("🌐 Web Search & Auto-Grounding (RAG)")
        sc_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        sc_layout.addWidget(sc_title)

        self.web_search_enabled_checkbox = QCheckBox("Enable Web Search (!search, !web, !links)")
        self.web_search_enabled_checkbox.setChecked(bool(self.app_cfg.get("web_search_enabled", True)))
        sc_layout.addWidget(self.web_search_enabled_checkbox)

        self.web_search_auto_checkbox = QCheckBox("Auto-Search Mode (Intelligently search the web for prompts needing live info)")
        self.web_search_auto_checkbox.setChecked(bool(self.app_cfg.get("web_search_auto", True)))
        sc_layout.addWidget(self.web_search_auto_checkbox)

        prov_row = QHBoxLayout()
        prov_row.addWidget(QLabel("Search Provider:"))
        self.web_search_provider_select = QComboBox()
        self.web_search_provider_select.addItem("Bing (Free, Zero-Config)", userData="bing")
        self.web_search_provider_select.addItem("DuckDuckGo (Free, Zero-Config)", userData="duckduckgo")
        self.web_search_provider_select.addItem("Tavily AI Search (API Key)", userData="tavily")
        self.web_search_provider_select.addItem("Brave Search (API Key)", userData="brave")
        current_sp = self.app_cfg.get("web_search_provider", "bing").lower()
        for idx in range(self.web_search_provider_select.count()):
            if self.web_search_provider_select.itemData(idx) == current_sp:
                self.web_search_provider_select.setCurrentIndex(idx)
                break
        prov_row.addWidget(self.web_search_provider_select, 1)

        prov_row.addWidget(QLabel("Max Results:"))
        self.web_search_max_results_input = QSpinBox()
        self.web_search_max_results_input.setRange(1, 10)
        self.web_search_max_results_input.setValue(int(self.app_cfg.get("web_search_max_results", 5)))
        prov_row.addWidget(self.web_search_max_results_input)
        sc_layout.addLayout(prov_row)

        tavily_row = QHBoxLayout()
        tavily_row.addWidget(QLabel("Tavily API Key (Optional):"))
        self.tavily_api_key_input = QLineEdit(self.app_cfg.get("tavily_api_key", ""))
        self.tavily_api_key_input.setEchoMode(QLineEdit.Password)
        self.tavily_api_key_input.setPlaceholderText("tvly-...")
        tavily_row.addWidget(self.tavily_api_key_input, 1)
        sc_layout.addLayout(tavily_row)

        brave_row = QHBoxLayout()
        brave_row.addWidget(QLabel("Brave API Key (Optional):"))
        self.brave_api_key_input = QLineEdit(self.app_cfg.get("brave_api_key", ""))
        self.brave_api_key_input.setEchoMode(QLineEdit.Password)
        self.brave_api_key_input.setPlaceholderText("BSA...")
        brave_row.addWidget(self.brave_api_key_input, 1)
        sc_layout.addLayout(brave_row)

        test_row = QHBoxLayout()
        self.test_search_input = QLineEdit("Python latest version news")
        self.test_search_input.setPlaceholderText("Enter test search query...")
        self.test_search_btn = QPushButton("🔍 Test Web Search")
        self.test_search_btn.clicked.connect(self.test_web_search_async)
        test_row.addWidget(self.test_search_input, 1)
        test_row.addWidget(self.test_search_btn)
        sc_layout.addLayout(test_row)

        self.test_search_status = QLabel("")
        sc_layout.addWidget(self.test_search_status)

        layout.addWidget(search_card)

        # --- Save Settings Button ---
        save_btn_row = QHBoxLayout()
        self.save_settings_btn = QPushButton("Save AI & Bot Settings")
        self.save_settings_btn.setObjectName("primaryBtn")
        self.save_settings_btn.setMinimumHeight(38)
        self.save_settings_btn.clicked.connect(self.save_backend_settings)

        save_btn_row.addStretch(1)
        save_btn_row.addWidget(self.save_settings_btn, 2)
        save_btn_row.addStretch(1)
        layout.addLayout(save_btn_row)

        scroll.setWidget(container)
        page_layout = QVBoxLayout(page)
        page_layout.setContentsMargins(0, 0, 0, 0)
        page_layout.addWidget(scroll)

        self.content_stack.addWidget(page)  # Page 1

    # -------------------------------------------------------------------------
    # PAGE 3: LOGS & CONSOLE
    # -------------------------------------------------------------------------
    def setup_logs_page(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(20, 20, 20, 20)
        layout.setSpacing(12)

        # Page Header
        title_row = QHBoxLayout()
        page_title = QLabel("Logs & Console")
        page_title.setStyleSheet("font-size: 20px; font-weight: 700;")
        title_row.addWidget(page_title)
        title_row.addStretch(1)

        self.clear_system_log_btn = QPushButton("Clear System Log")
        self.clear_bot_log_btn = QPushButton("Clear Discord Log")
        self.clear_system_log_btn.clicked.connect(lambda: self.system_log_output.clear())
        self.clear_bot_log_btn.clicked.connect(lambda: self.bot_log_output.clear())

        title_row.addWidget(self.clear_system_log_btn)
        title_row.addWidget(self.clear_bot_log_btn)
        layout.addLayout(title_row)

        # Search filter and options toolbar card
        tools_card = QFrame()
        tools_card.setObjectName("card")
        tools_layout = QHBoxLayout(tools_card)
        tools_layout.setContentsMargins(12, 8, 12, 8)
        tools_layout.setSpacing(12)

        self.system_log_to_file_checkbox = QCheckBox("Write System Log to File")
        self.bot_log_to_file_checkbox = QCheckBox("Write Bot Log to File")
        self.debug_checkbox = QCheckBox("Debug Mode")
        self.debug_checkbox.setChecked(bool(self.app_cfg.get("debug", False)))
        self.debug_checkbox.stateChanged.connect(self.on_debug_checkbox_changed)

        tools_layout.addWidget(self.system_log_to_file_checkbox)
        tools_layout.addWidget(self.bot_log_to_file_checkbox)
        tools_layout.addWidget(self.debug_checkbox)
        tools_layout.addStretch(1)

        layout.addWidget(tools_card)

        # Dual Terminal Logs Layout
        log_split_layout = QHBoxLayout()
        log_split_layout.setSpacing(12)

        # System Log
        sys_box = QFrame()
        sys_box.setObjectName("card")
        sys_box_layout = QVBoxLayout(sys_box)
        sys_box_layout.setContentsMargins(10, 10, 10, 10)
        sys_lbl = QLabel("System & AI Service Logs")
        sys_lbl.setStyleSheet("font-weight: 600; color: #00f0ff;")
        sys_box_layout.addWidget(sys_lbl)

        self.system_log_output = QTextEdit()
        self.system_log_output.setReadOnly(True)
        self.system_log_output.setFont(QFont("Consolas", 10))
        sys_box_layout.addWidget(self.system_log_output)
        log_split_layout.addWidget(sys_box, 1)

        # Bot Log
        bot_box = QFrame()
        bot_box.setObjectName("card")
        bot_box_layout = QVBoxLayout(bot_box)
        bot_box_layout.setContentsMargins(10, 10, 10, 10)
        bot_lbl = QLabel("Discord Activity & Prompts")
        bot_lbl.setStyleSheet("font-weight: 600; color: #ff5ecb;")
        bot_box_layout.addWidget(bot_lbl)

        self.bot_log_output = QTextEdit()
        self.bot_log_output.setReadOnly(True)
        self.bot_log_output.setFont(QFont("Consolas", 10))
        bot_box_layout.addWidget(self.bot_log_output)
        log_split_layout.addWidget(bot_box, 1)

        layout.addLayout(log_split_layout, 1)

        self.content_stack.addWidget(page)  # Page 3

    # -------------------------------------------------------------------------
    # PAGE 4: APP SETTINGS
    # -------------------------------------------------------------------------
    def setup_settings_page(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(20, 20, 20, 20)
        layout.setSpacing(16)

        # Header Title
        title_row = QHBoxLayout()
        page_title = QLabel("Settings & Diagnostics")
        page_title.setStyleSheet("font-size: 20px; font-weight: 700;")
        title_row.addWidget(page_title)
        layout.addLayout(title_row)

        # --- Appearance Card ---
        theme_card = QFrame()
        theme_card.setObjectName("card")
        t_layout = QVBoxLayout(theme_card)
        t_layout.setContentsMargins(14, 12, 14, 12)
        t_layout.setSpacing(10)

        t_title = QLabel("Theme & Appearance")
        t_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        t_layout.addWidget(t_title)

        theme_row = QHBoxLayout()
        theme_row.addWidget(QLabel("Active Theme:"))
        self.theme_select = QComboBox()
        for t in self.theme_manager.available_themes():
            self.theme_select.addItem(t.capitalize(), userData=t)
        cur_t = self.app_cfg.get("theme", "retrowave").capitalize()
        self.theme_select.setCurrentText(cur_t)
        self.theme_select.currentTextChanged.connect(self.change_theme)
        theme_row.addWidget(self.theme_select, 1)
        t_layout.addLayout(theme_row)
        layout.addWidget(theme_card)

        # --- Auto-Restart & Daemon Management Card ---
        restart_card = QFrame()
        restart_card.setObjectName("card")
        r_layout = QVBoxLayout(restart_card)
        r_layout.setContentsMargins(14, 12, 14, 12)
        r_layout.setSpacing(10)

        r_title = QLabel("Bot Daemon Reliability & Auto-Restart")
        r_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        r_layout.addWidget(r_title)

        restart_row = QHBoxLayout()
        self.auto_restart_checkbox = QCheckBox("Enable Periodic Auto-Restart")
        self.auto_restart_interval = QComboBox()
        interval_labels = ["30 Min", "1 Hour", "2 Hours", "4 Hours", "8 Hours", "12 Hours", "24 Hours"]
        self.auto_restart_interval.addItems(interval_labels)
        restart_row.addWidget(self.auto_restart_checkbox)
        restart_row.addWidget(QLabel("Interval:"))
        restart_row.addWidget(self.auto_restart_interval)
        restart_row.addStretch(1)
        r_layout.addLayout(restart_row)
        layout.addWidget(restart_card)

        # --- File Storage & Diagnostics Card ---
        diag_card = QFrame()
        diag_card.setObjectName("card")
        diag_layout = QVBoxLayout(diag_card)
        diag_layout.setContentsMargins(14, 12, 14, 12)
        diag_layout.setSpacing(10)

        d_title = QLabel("Folders & Diagnostics")
        d_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        diag_layout.addWidget(d_title)

        diag_btns_row = QHBoxLayout()
        open_logs_btn = QPushButton("📂 Open Logs Folder")
        open_logs_btn.clicked.connect(lambda: self.open_folder("logs"))
        open_config_btn = QPushButton("📂 Open Config Folder")
        open_config_btn.clicked.connect(lambda: self.open_folder("config"))

        diag_btns_row.addWidget(open_logs_btn)
        diag_btns_row.addWidget(open_config_btn)
        diag_btns_row.addStretch(1)
        diag_layout.addLayout(diag_btns_row)
        layout.addWidget(diag_card)

        layout.addStretch(1)
        self.content_stack.addWidget(page)  # Page 4

    def open_folder(self, folder_name: str):
        path = os.path.abspath(folder_name)
        os.makedirs(path, exist_ok=True)
        QDesktopServices.openUrl(QUrl.fromLocalFile(path))

    # -------------------------------------------------------------------------
    # LOGGING INITIALIZATION
    # -------------------------------------------------------------------------
    def setup_logging_handlers(self):
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

    def append_system_log(self, text: str):
        self.system_log_output.append(text)

    # -------------------------------------------------------------------------
    # AI BACKEND & MODEL METHODS
    # -------------------------------------------------------------------------
    def browse_llama_binary(self):
        file_path, _ = QFileDialog.getOpenFileName(self, "Select llama executable", "", "Executables (*.exe);;All Files (*)")
        if file_path:
            self.llamacpp_binary_input.setText(file_path)

    def browse_llama_model(self):
        file_path, _ = QFileDialog.getOpenFileName(self, "Select GGUF Model", "", "GGUF Models (*.gguf);;All Files (*)")
        if file_path:
            self.llamacpp_model_input.setText(file_path)

    def on_remote_preset_changed(self, preset_name: str):
        if preset_name in REMOTE_PRESETS:
            preset_data = REMOTE_PRESETS[preset_name]
            if preset_name != "Custom":
                self.remote_url_input.setText(preset_data.get("base_url", ""))
                default_m = preset_data.get("default_model", "")
                if default_m:
                    self.remote_model_input.setText(default_m)

    def toggle_remote_key_visibility(self):
        if self.remote_key_input.echoMode() == QLineEdit.Password:
            self.remote_key_input.setEchoMode(QLineEdit.Normal)
            self.toggle_remote_key_btn.setText("🔒 Hide")
        else:
            self.remote_key_input.setEchoMode(QLineEdit.Password)
            self.toggle_remote_key_btn.setText("👁 Show")

    def test_remote_connection(self):
        self.test_remote_btn.setEnabled(False)
        self.remote_test_status.setText("Testing connection...")
        self.remote_test_status.setStyleSheet("color: #ffaa00; font-weight: bold;")

        base_url = self.remote_url_input.text().strip()
        api_key = self.remote_key_input.text().strip()
        preset = self.remote_preset_combo.currentText()

        def run_test():
            test_provider = RemoteOpenAIProvider(
                base_url=base_url,
                api_key=api_key,
                preset=preset
            )
            return test_provider.test_connection_sync()

        future = self.executor.submit(run_test)

        def poll():
            if future.done():
                self.test_remote_btn.setEnabled(True)
                try:
                    success, msg = future.result()
                except Exception as e:
                    success, msg = False, str(e)
                self.signals.remote_test_finished.emit(success, msg)
            else:
                QTimer.singleShot(100, poll)
        QTimer.singleShot(100, poll)

    @Slot(bool, str)
    def on_remote_test_finished(self, success: bool, msg: str):
        color = "#00ff88" if success else "#ff4466"
        self.remote_test_status.setText(msg)
        self.remote_test_status.setStyleSheet(f"color: {color}; font-weight: bold;")

    def fetch_remote_models(self):
        self.fetch_remote_models_btn.setEnabled(False)
        base_url = self.remote_url_input.text().strip()
        api_key = self.remote_key_input.text().strip()
        preset = self.remote_preset_combo.currentText()

        def run_fetch():
            test_provider = RemoteOpenAIProvider(
                base_url=base_url,
                api_key=api_key,
                preset=preset
            )
            return test_provider.list_models_sync()

        future = self.executor.submit(run_fetch)

        def poll():
            if future.done():
                self.fetch_remote_models_btn.setEnabled(True)
                try:
                    models = future.result()
                except Exception:
                    models = []
                self.signals.remote_models_fetched.emit(models)
            else:
                QTimer.singleShot(100, poll)
        QTimer.singleShot(100, poll)

    @Slot(list)
    def on_remote_models_fetched(self, models: list):
        if models:
            self.remote_model_input.setText(models[0])
            self.system_log_output.append(f"[REMOTE] Retrieved {len(models)} models from API.")
            self.show_feedback(self.fetch_remote_models_btn, f"Found {len(models)} models!")
        else:
            self.show_feedback(self.fetch_remote_models_btn, "No models retrieved")

    def save_backend_settings(self):
        if self.rb_llamacpp.isChecked():
            backend_type = "llamacpp"
        elif self.rb_remote.isChecked():
            backend_type = "remote_openai"
        else:
            backend_type = "ollama"

        self.app_cfg["backend_type"] = backend_type
        self.app_cfg["ollama_url"] = self.ollama_url_input.text().strip()
        self.app_cfg["api_key"] = self.api_key_input.text().strip()
        self.app_cfg["llamacpp_url"] = self.llamacpp_url_input.text().strip()
        self.app_cfg["llamacpp_binary_path"] = self.llamacpp_binary_input.text().strip()
        self.app_cfg["llamacpp_model_path"] = self.llamacpp_model_input.text().strip()

        # Remote LLM settings
        self.app_cfg["remote_preset"] = self.remote_preset_combo.currentText()
        self.app_cfg["remote_base_url"] = self.remote_url_input.text().strip()
        self.app_cfg["remote_api_key"] = self.remote_key_input.text().strip()
        self.app_cfg["remote_model"] = self.remote_model_input.text().strip()
        self.app_cfg["system_prompt"] = self.system_prompt_input.toPlainText().strip()

        self.app_cfg["allow_dms"] = self.allow_dms_checkbox.isChecked()
        self.app_cfg["dm_default_model"] = self.dm_model_input.text().strip()
        self.app_cfg["dm_disabled_message"] = self.dm_disabled_msg_input.text().strip()

        # Web Search settings
        self.app_cfg["web_search_enabled"] = self.web_search_enabled_checkbox.isChecked()
        self.app_cfg["web_search_auto"] = self.web_search_auto_checkbox.isChecked()
        self.app_cfg["web_search_provider"] = self.web_search_provider_select.currentData() or "bing"
        self.app_cfg["web_search_max_results"] = self.web_search_max_results_input.value()
        self.app_cfg["tavily_api_key"] = self.tavily_api_key_input.text().strip()
        self.app_cfg["brave_api_key"] = self.brave_api_key_input.text().strip()

        save_app_config(self.app_cfg)
        self.provider = get_ai_provider(self.app_cfg)
        reload_ai_provider()

        self.start_ai_btn.setText(f"Start {self.provider.name}")
        self.stop_ai_btn.setText(f"Stop {self.provider.name}")

        self.system_log_output.append(f"[SETTINGS] Saved! Switched to {self.provider.name} at {self.provider.base_url}")
        self.show_feedback(self.save_settings_btn, "✓ Settings Saved!")
        self.update_ai_status()
        self.refresh_models_async()

    def test_web_search_async(self):
        query = self.test_search_input.text().strip() or "Python release"
        provider_name = self.web_search_provider_select.currentData() or "bing"
        tavily_key = self.tavily_api_key_input.text().strip()
        brave_key = self.brave_api_key_input.text().strip()
        max_res = self.web_search_max_results_input.value()

        self.test_search_btn.setEnabled(False)
        self.test_search_status.setStyleSheet("color: #00f0ff; font-weight: 600;")
        self.test_search_status.setText(f"Searching via {provider_name}...")

        def run_search():
            import asyncio
            from web_search import SearchService
            test_cfg = {
                "web_search_provider": provider_name,
                "tavily_api_key": tavily_key,
                "brave_api_key": brave_key,
                "web_search_max_results": max_res
            }
            service = SearchService(test_cfg)
            try:
                loop = asyncio.new_event_loop()
                asyncio.set_event_loop(loop)
                results = loop.run_until_complete(service.search(query, max_results=max_res))
                loop.close()
                if results:
                    title_snippet = results[0].title[:45]
                    return True, f"✓ Success: Found {len(results)} results! (Top: '{title_snippet}...')", results
                return False, "0 results returned from search provider.", []
            except Exception as e:
                return False, f"Search error: {e}", []

        def done_callback(fut):
            success, msg, _ = fut.result()
            self.signals.web_search_tested.emit(success, msg)

        future = self.executor.submit(run_search)
        future.add_done_callback(done_callback)

    @Slot(bool, str)
    def on_web_search_tested(self, success: bool, msg: str):
        self.test_search_btn.setEnabled(True)
        if success:
            self.test_search_status.setStyleSheet("color: #00ff88; font-weight: 600;")
        else:
            self.test_search_status.setStyleSheet("color: #ff3366; font-weight: 600;")
        self.test_search_status.setText(msg)
        self.system_log_output.append(f"[WEB SEARCH] {msg}")

    def auto_detect_on_startup(self):
        """If configured AI backend is offline, runs auto-detection in background."""
        if not self.provider.is_local():
            return

        def check_and_detect():
            if not self.provider.status_sync():
                from ai_providers.detector import detect_all_ai_providers
                return detect_all_ai_providers()
            return None

        future = self.executor.submit(check_and_detect)

        def poll():
            if future.done():
                res = future.result()
                if res and (res.get("llamacpp", {}).get("running") or res.get("ollama", {}).get("running")):
                    self.signals.backend_detected.emit(res)
            else:
                QTimer.singleShot(200, poll)
        QTimer.singleShot(200, poll)

    def run_auto_detect_all(self):
        self.autodetect_btn.setEnabled(False)
        self.autodetect_status.setText("Scanning for running local AI services...")

        def detect():
            from ai_providers.detector import detect_all_ai_providers
            return detect_all_ai_providers()

        future = self.executor.submit(detect)

        def poll():
            if future.done():
                self.autodetect_btn.setEnabled(True)
                res = future.result()
                self.signals.backend_detected.emit(res)
            else:
                QTimer.singleShot(100, poll)
        QTimer.singleShot(100, poll)

    def run_auto_detect_llama(self):
        self.detect_llama_btn.setEnabled(False)

        def detect():
            from ai_providers.detector import detect_running_llamacpp, detect_installed_llamacpp, detect_local_models
            running = detect_running_llamacpp()
            binary = detect_installed_llamacpp()
            models = detect_local_models()
            return running, binary, models

        future = self.executor.submit(detect)

        def poll():
            if future.done():
                self.detect_llama_btn.setEnabled(True)
                running, binary, models = future.result()
                if running:
                    self.rb_llamacpp.setChecked(True)
                    self.provider_stack.setCurrentIndex(1)
                    self.llamacpp_url_input.setText(running["url"])
                    if running.get("binary_path"):
                        self.llamacpp_binary_input.setText(running["binary_path"])
                    if running.get("model_path"):
                        self.llamacpp_model_input.setText(running["model_path"])
                    self.show_feedback(self.detect_llama_btn, "Llama.cpp Found!")
                    self.system_log_output.append(f"[AI Backend] Detected active Llama.cpp instance at {running['url']}")
                    self.save_backend_settings()
                else:
                    if binary:
                        self.llamacpp_binary_input.setText(binary)
                    if models:
                        self.llamacpp_model_input.setText(models[0])
                    msg = "No running server" + (f" (Found binary: {os.path.basename(binary)})" if binary else "")
                    self.show_feedback(self.detect_llama_btn, msg)
            else:
                QTimer.singleShot(100, poll)
        QTimer.singleShot(100, poll)

    def auto_find_llama_binary(self):
        from ai_providers.detector import detect_installed_llamacpp
        binary = detect_installed_llamacpp()
        if binary:
            self.llamacpp_binary_input.setText(binary)
            self.show_feedback(self.locate_binary_btn, "Binary Found!")
            self.system_log_output.append(f"[AI Backend] Located Llama executable: {binary}")
        else:
            self.show_feedback(self.locate_binary_btn, "Not Found")

    def auto_find_llama_model(self):
        from ai_providers.detector import detect_local_models
        models = detect_local_models()
        if models:
            self.llamacpp_model_input.setText(models[0])
            self.show_feedback(self.locate_model_btn, f"Found {len(models)} model(s)!")
            self.system_log_output.append(f"[AI Backend] Located GGUF model: {models[0]}")
        else:
            self.show_feedback(self.locate_model_btn, "No GGUF Found")

    @Slot(dict)
    def on_backend_detected(self, result: dict):
        rec = result.get("recommended_backend")
        llama_info = result.get("llamacpp", {})
        ollama_info = result.get("ollama", {})

        if rec == "llamacpp" or llama_info.get("running"):
            self.rb_llamacpp.setChecked(True)
            self.provider_stack.setCurrentIndex(1)
            self.llamacpp_url_input.setText(llama_info.get("url", "http://localhost:8080"))
            if llama_info.get("binary_path"):
                self.llamacpp_binary_input.setText(llama_info["binary_path"])
            if llama_info.get("model_path"):
                self.llamacpp_model_input.setText(llama_info["model_path"])
            self.autodetect_status.setText(f"✓ Detected running Llama.cpp ({llama_info.get('url')})")
            self.system_log_output.append(f"[AI Backend] Auto-detected running Llama.cpp at {llama_info.get('url')}!")
            self.save_backend_settings()
        elif rec == "ollama" or ollama_info.get("running"):
            self.rb_ollama.setChecked(True)
            self.provider_stack.setCurrentIndex(0)
            self.ollama_url_input.setText(ollama_info.get("url", "http://localhost:11434"))
            self.autodetect_status.setText(f"✓ Detected running Ollama ({ollama_info.get('url')})")
            self.system_log_output.append(f"[AI Backend] Auto-detected running Ollama at {ollama_info.get('url')}!")
            self.save_backend_settings()
        else:
            if llama_info.get("binary_path"):
                self.llamacpp_binary_input.setText(llama_info["binary_path"])
            if llama_info.get("model_path"):
                self.llamacpp_model_input.setText(llama_info["model_path"])
            self.autodetect_status.setText("No active AI servers running. Configured found binaries.")

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
        endpoint = getattr(self.provider, 'base_url', '')

        new_pill_id = "statusPillOnline" if running else "statusPillOffline"
        if running:
            self.ai_status_label.setText(f"{name} ({locality}): Online")
            self.kpi_ai_pill.setText("ONLINE")
        else:
            self.ai_status_label.setText(f"{name} ({locality}): Offline / Unreachable")
            self.kpi_ai_pill.setText("OFFLINE")

        if self.kpi_ai_pill.objectName() != new_pill_id:
            self.kpi_ai_pill.setObjectName(new_pill_id)
            self.kpi_ai_pill.style().unpolish(self.kpi_ai_pill)
            self.kpi_ai_pill.style().polish(self.kpi_ai_pill)

        self.kpi_ai_val.setText(f"{name} ({locality})")
        self.kpi_ai_subtext.setText(endpoint[:28])

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
        self.model_select.blockSignals(True)
        self.model_select.clear()
        if not models:
            self.model_select.addItem("No models found")
            self.kpi_model_val.setText("None")
            self.sidebar_model_label.setText("Model: None")
        else:
            for m in models:
                self.model_select.addItem(m)
            saved_active = self.app_cfg.get("active_model", "")
            if saved_active in models:
                self.model_select.setCurrentText(saved_active)
                self.kpi_model_val.setText(saved_active)
                self.sidebar_model_label.setText(f"Model: {saved_active[:14]}")
            else:
                self.model_select.setCurrentIndex(0)
                self.app_cfg["active_model"] = models[0]
                self.kpi_model_val.setText(models[0])
                self.sidebar_model_label.setText(f"Model: {models[0][:14]}")
                save_app_config(self.app_cfg)
        self.model_select.blockSignals(False)

    def on_active_model_changed(self, model_name: str):
        if not model_name or model_name in ("No models found", f"{self.provider.name} not reachable"):
            return
        self.app_cfg["active_model"] = model_name
        self.kpi_model_val.setText(model_name)
        self.sidebar_model_label.setText(f"Model: {model_name[:14]}")
        save_app_config(self.app_cfg)
        self.system_log_output.append(f"[MODEL] Active model set to: {model_name}")

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

    # -------------------------------------------------------------------------
    # BOT DAEMON STATUS & LIFECYCLE
    # -------------------------------------------------------------------------
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

        self.bot_status_label.setText(f"Bot Status: {self._bot_status}")

        new_pill_id = "statusPillOnline" if self._bot_status == "Running" else "statusPillOffline"
        if self._bot_status == "Running":
            self.kpi_bot_pill.setText("ONLINE")
            self.kpi_bot_subtext.setText("Discord Gateway Connected")
            self.sidebar_bot_pill.setText("● Online")
        else:
            self.kpi_bot_pill.setText("STOPPED")
            self.kpi_bot_subtext.setText("Discord Gateway Offline")
            self.sidebar_bot_pill.setText("● Stopped")

        if self.kpi_bot_pill.objectName() != new_pill_id:
            self.kpi_bot_pill.setObjectName(new_pill_id)
            self.sidebar_bot_pill.setObjectName(new_pill_id)
            for pill in [self.kpi_bot_pill, self.sidebar_bot_pill]:
                pill.style().unpolish(pill)
                pill.style().polish(pill)

    def start_bot(self):
        try:
            if self._start_bot_func:
                self._start_bot_func()
                self.system_log_output.append("[INFO] Start Bot requested.")
                self.show_feedback(self.start_bot_btn, "Bot Started!")
                QTimer.singleShot(1500, self.update_bot_status)
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
                QTimer.singleShot(2000, self.update_bot_status)
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

            # Update KPI card
            server_count = len(new_servers)
            suffix = "Guild" if server_count == 1 else "Guilds"
            self.kpi_servers_val.setText(f"{server_count} {suffix}")

            if hasattr(self, "server_config_tab"):
                self.server_config_tab.update_guilds()
        except Exception:
            pass

    def process_gui_events(self):
        """Processes logs and events directly from the thread-safe queue."""
        now_str = datetime.now().strftime("%H:%M:%S")
        while not gui_event_queue.empty():
            try:
                entry = gui_event_queue.get_nowait()
                event = entry.get("event")
                data = entry.get("data", {})
                if event == "config_change":
                    msg = f"[{now_str}] [Config Change] Guild {data.get('guild_id')}: {data.get('field')} -> {data.get('value')}"
                    self.system_log_output.append(msg)
                    self.recent_activity_output.append(msg)
                elif event == "prompt":
                    guild = data.get("guild_name") or data.get("guild_id")
                    msg = f"[{now_str}] [{guild}] {data.get('user')}: {data.get('prompt')}"
                    self.bot_log_output.append(msg)
                    self.recent_activity_output.append(msg)
                elif event == "reply":
                    guild = data.get("guild_name") or data.get("guild_id")
                    reply_snippet = data.get("reply", "")[:180]
                    msg = f"[{now_str}] [{guild}] Silas Blue -> {reply_snippet}..."
                    self.bot_log_output.append(msg)
                    self.recent_activity_output.append(msg)
                elif event == "web_search":
                    guild = data.get("guild_name") or data.get("guild_id") or "Direct Message"
                    query_snippet = data.get("query", "")[:50]
                    res_count = data.get("results_count", 0)
                    msg = f"[{now_str}] [{guild}] 🌐 Web Search: \"{query_snippet}\" ({res_count} results)"
                    self.bot_log_output.append(msg)
                    self.recent_activity_output.append(msg)
                elif event == "server_joined":
                    guild_name = data.get("guild_name", "Unknown Server")
                    guild_id = data.get("guild_id", "")
                    msg = f"[{now_str}] [Discord] Silas Blue joined server: {guild_name} ({guild_id})"
                    self.system_log_output.append(msg)
                    self.recent_activity_output.append(msg)
                    self.update_servers_list()
                elif event == "server_left":
                    guild_name = data.get("guild_name", "Unknown Server")
                    guild_id = data.get("guild_id", "")
                    msg = f"[{now_str}] [Discord] Silas Blue left server: {guild_name} ({guild_id})"
                    self.system_log_output.append(msg)
                    self.recent_activity_output.append(msg)
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
            self.app_cfg["theme"] = theme_name.lower()
            save_app_config(self.app_cfg)

    def show_feedback(self, button, message):
        orig_text = button.text()
        button.setText(message)
        QTimer.singleShot(1200, lambda: button.setText(orig_text))

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