"""
GUI page for editing per-server configuration with modern Discord-grade UX/UI.
Features:
- Sticky server selector header and sticky action footer (server selection unchanged across sub-tabs).
- Sub-pages split into: Permissions, Token Settings, Server Prompt & Pagination, Forum Auto-Reply, and Raw JSON.
- System prompt text box auto-expands to use all empty vertical space and pushes bottom controls to the bottom without a scroll bar.
- Scalable 4-domain Access Control supporting Roles and specific User Snowflake IDs via FlowLayout chips.
- Role-based token quotas editor with schedule and overage controls.
- Dedicated Discord Forum Auto-Reply settings card.
- Dirty-state tracking with unsaved changes confirmation modal on server switch.
"""

from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QGridLayout, QLabel, QComboBox,
    QSpinBox, QTextEdit, QLineEdit, QPushButton, QStackedWidget, QCheckBox, QFrame,
    QScrollArea, QMessageBox, QSizePolicy
)
from PySide6.QtCore import QTimer, Qt, Signal
import json
import logging

from utils import load_config, save_config, validate_and_sanitize_guild_config
from bot_core import bot, update_server_config
from .permission_chips import PermissionGrantCard, ChannelSelectCard

logger = logging.getLogger("silasblue")


class ServerConfigPage(QWidget):
    """
    Overhauled per-server configuration page designed for scalability and high-DPI responsiveness.
    Preserves active server selection while allowing modular sub-page navigation.
    """

    def __init__(self, main_window):
        super().__init__()
        self.main_window = main_window
        self.is_dirty = False
        self._prev_guild_index = -1
        self._current_subpage_index = 0

        # Root Layout: Sticky Header + Subpage Stack + Sticky Footer
        root_layout = QVBoxLayout(self)
        root_layout.setContentsMargins(16, 14, 16, 12)
        root_layout.setSpacing(10)

        # ---------------------------------------------------------------------
        # 1. TOP STICKY HEADER: Server Selector & Quick Actions
        # ---------------------------------------------------------------------
        header_card = QFrame()
        header_card.setObjectName("card")
        header_layout = QHBoxLayout(header_card)
        header_layout.setContentsMargins(14, 10, 14, 10)
        header_layout.setSpacing(12)

        server_label = QLabel("Active Discord Server:")
        server_label.setStyleSheet("font-weight: 600; font-size: 13px;")
        header_layout.addWidget(server_label)

        self.guild_select = QComboBox()
        self.guild_select.setMinimumWidth(280)
        self.guild_select.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        header_layout.addWidget(self.guild_select, 3)

        self.refresh_guilds_btn = QPushButton("🔄 Refresh")
        self.refresh_guilds_btn.setToolTip("Reload server list from Discord Gateway")
        self.refresh_guilds_btn.clicked.connect(self.update_guilds)
        header_layout.addWidget(self.refresh_guilds_btn)

        self.invite_btn = QPushButton("➕ Invite Bot")
        self.invite_btn.setToolTip("Invite Silas Blue to another Discord server")
        self.invite_btn.clicked.connect(self.open_invite_dialog)
        header_layout.addWidget(self.invite_btn)

        root_layout.addWidget(header_card)

        # ---------------------------------------------------------------------
        # 2. MAIN CONTENT STACK (Modular Sub-Pages)
        # ---------------------------------------------------------------------
        self.subpage_stack = QStackedWidget()
        root_layout.addWidget(self.subpage_stack, 1)

        # Subpage 0: Permissions
        self.page_permissions = self._setup_permissions_subpage()
        self.subpage_stack.addWidget(self.page_permissions)

        # Subpage 1: Token Settings
        self.page_tokens = self._setup_token_settings_subpage()
        self.subpage_stack.addWidget(self.page_tokens)

        # Subpage 2: Server Prompt & Output / Pagination (Auto-expanding, No scrollbar)
        self.page_prompt_pagination = self._setup_prompt_pagination_subpage()
        self.subpage_stack.addWidget(self.page_prompt_pagination)

        # Subpage 3: Forum Auto-Reply
        self.page_forum = self._setup_forum_subpage()
        self.subpage_stack.addWidget(self.page_forum)

        # Subpage 4: Raw JSON
        self.page_raw_json = self._setup_raw_json_subpage()
        self.subpage_stack.addWidget(self.page_raw_json)

        # ---------------------------------------------------------------------
        # 3. BOTTOM STICKY ACTION FOOTER
        # ---------------------------------------------------------------------
        footer_card = QFrame()
        footer_card.setObjectName("card")
        footer_layout = QHBoxLayout(footer_card)
        footer_layout.setContentsMargins(14, 8, 14, 8)
        footer_layout.setSpacing(12)

        # Unsaved Changes Warning Banner
        self.dirty_banner = QLabel("● You have unsaved changes")
        self.dirty_banner.setStyleSheet("font-weight: 700; color: #ffb142; font-size: 12px;")
        self.dirty_banner.setVisible(False)
        footer_layout.addWidget(self.dirty_banner)

        footer_layout.addStretch(1)

        self.discard_btn = QPushButton("Discard Changes")
        self.discard_btn.setCursor(Qt.PointingHandCursor)
        self.discard_btn.setToolTip("Revert edits back to the saved server configuration")
        self.discard_btn.clicked.connect(self.reload_config_from_disk)
        self.discard_btn.setEnabled(False)
        footer_layout.addWidget(self.discard_btn)

        self.save_btn = QPushButton("Save Server Settings")
        self.save_btn.setObjectName("primaryBtn")
        self.save_btn.setMinimumHeight(32)
        self.save_btn.setCursor(Qt.PointingHandCursor)
        self.save_btn.clicked.connect(self.save_config)
        footer_layout.addWidget(self.save_btn)

        self.save_status_label = QLabel("")
        self.save_status_label.setStyleSheet("font-weight: 600; color: #00ff88; font-size: 12px;")
        footer_layout.addWidget(self.save_status_label)

        root_layout.addWidget(footer_card)

        # ---------------------------------------------------------------------
        # CONNECTIONS & INITIALIZATION
        # ---------------------------------------------------------------------
        self.guild_select.currentIndexChanged.connect(self._on_guild_select_changed)

        self.update_guilds()
        self.load_config()

    # -------------------------------------------------------------------------
    # COMPATIBILITY & NAVIGATION
    # -------------------------------------------------------------------------
    @property
    def tabs(self):
        """Backwards compatibility shim for external references."""
        class _TabsProxy:
            def __init__(proxy_self, page):
                proxy_self.page = page
            def currentIndex(proxy_self):
                return proxy_self.page._current_subpage_index
            def setCurrentIndex(proxy_self, idx):
                proxy_self.page.switch_subpage(idx)
        return _TabsProxy(self)

    @property
    def forum_monitored_channels(self):
        """Backwards compatibility shim for external references expecting QLineEdit."""
        class _ChannelsShim:
            def __init__(proxy_self, card):
                proxy_self.card = card
            def text(proxy_self):
                return ", ".join(proxy_self.card.get_channels())
            def setText(proxy_self, val):
                channels = [x.strip() for x in str(val).split(",") if x.strip()]
                proxy_self.card.set_channels(channels)
        return _ChannelsShim(self.forum_channel_card)

    def switch_subpage(self, index: int):
        """Switches the active modular subpage while keeping server selection unchanged."""
        prev_idx = self._current_subpage_index
        # If leaving Raw JSON, sync changes back to widgets
        if prev_idx == 4 and index != 4:
            try:
                cfg = json.loads(self.raw_config.toPlainText())
                self.set_widgets_from_config(cfg)
            except Exception:
                pass
        # If entering Raw JSON, sync widgets into JSON view
        elif index == 4 and prev_idx != 4:
            cfg = self.get_config_from_widgets()
            self.raw_config.blockSignals(True)
            self.raw_config.setPlainText(json.dumps(cfg, indent=2))
            self.raw_config.blockSignals(False)

        self._current_subpage_index = index
        self.subpage_stack.setCurrentIndex(index)
        if hasattr(self.main_window, "update_subnav_buttons"):
            self.main_window.update_subnav_buttons(index)

    def _create_subpage_header(self, title: str, subtitle: str) -> QFrame:
        """Helper to create a crisp section header for each sub-page."""
        header_card = QFrame()
        header_card.setObjectName("card")
        layout = QVBoxLayout(header_card)
        layout.setContentsMargins(14, 10, 14, 10)
        layout.setSpacing(3)

        t_lbl = QLabel(title)
        t_lbl.setStyleSheet("font-weight: 700; font-size: 14px; color: #00f0ff;")
        layout.addWidget(t_lbl)

        s_lbl = QLabel(subtitle)
        s_lbl.setStyleSheet("font-size: 11px; opacity: 0.8;")
        layout.addWidget(s_lbl)
        return header_card

    # -------------------------------------------------------------------------
    # SUBPAGE 0: PERMISSIONS
    # -------------------------------------------------------------------------
    def _setup_permissions_subpage(self) -> QWidget:
        page = QWidget()
        page_layout = QVBoxLayout(page)
        page_layout.setContentsMargins(0, 0, 0, 0)
        page_layout.setSpacing(10)

        header = self._create_subpage_header(
            "🔑 Role-Based & User Access Control",
            "Configure which Discord roles or specific user Snowflake IDs are authorized for each bot capability."
        )
        page_layout.addWidget(header)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.NoFrame)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)

        container = QWidget()
        container_layout = QVBoxLayout(container)
        container_layout.setContentsMargins(0, 0, 0, 0)
        container_layout.setSpacing(12)

        perm_grid = QGridLayout()
        perm_grid.setHorizontalSpacing(14)
        perm_grid.setVerticalSpacing(14)

        # 1. Bot Chat & Replies
        self.perm_reply = PermissionGrantCard(
            title="💬 Bot Chat & Replies",
            description="Who can prompt Silas Blue in server channels and threads to receive AI responses.",
            default_everyone=True
        )
        self.perm_reply.changed.connect(self._on_field_edited)
        perm_grid.addWidget(self.perm_reply, 0, 0)

        # 2. Change AI Model
        self.perm_model = PermissionGrantCard(
            title="🧠 Change Active AI Model",
            description="Who can switch the server's AI model via Discord commands (e.g. !model).",
            default_everyone=False
        )
        self.perm_model.changed.connect(self._on_field_edited)
        perm_grid.addWidget(self.perm_model, 0, 1)

        # 3. Change Server Permissions
        self.perm_admin = PermissionGrantCard(
            title="⚙️ Configure Server Settings",
            description="Who can modify server personas, memory turns, token limits, and forum settings.",
            default_everyone=False
        )
        self.perm_admin.changed.connect(self._on_field_edited)
        perm_grid.addWidget(self.perm_admin, 1, 0)

        # 4. Web Search Features
        self.perm_search = PermissionGrantCard(
            title="🌐 Web Search & RAG Grounding",
            description="Who can use live web queries (!search, !links) and trigger auto-grounded responses.",
            default_everyone=True
        )
        self.perm_search.changed.connect(self._on_field_edited)
        perm_grid.addWidget(self.perm_search, 1, 1)

        container_layout.addLayout(perm_grid)
        container_layout.addStretch(1)

        scroll.setWidget(container)
        page_layout.addWidget(scroll, 1)
        return page

    # -------------------------------------------------------------------------
    # SUBPAGE 1: TOKEN SETTINGS
    # -------------------------------------------------------------------------
    def _setup_token_settings_subpage(self) -> QWidget:
        page = QWidget()
        page_layout = QVBoxLayout(page)
        page_layout.setContentsMargins(0, 0, 0, 0)
        page_layout.setSpacing(10)

        header = self._create_subpage_header(
            "🪙 Role-Based Token Quotas & Reset Schedules",
            "Set token usage quotas per role, automated reset intervals, timezone offsets, and partial overage policies."
        )
        page_layout.addWidget(header)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.NoFrame)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)

        container = QWidget()
        c_layout = QVBoxLayout(container)
        c_layout.setContentsMargins(0, 0, 0, 0)
        c_layout.setSpacing(12)

        # Card 1: Activation & Reset Schedule
        sched_card = QFrame()
        sched_card.setObjectName("card")
        sc_layout = QVBoxLayout(sched_card)
        sc_layout.setContentsMargins(14, 12, 14, 12)
        sc_layout.setSpacing(10)

        self.quotas_enabled = QCheckBox("Enable Role-Based Token Quotas")
        self.quotas_enabled.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        self.quotas_enabled.toggled.connect(self._on_field_edited)
        sc_layout.addWidget(self.quotas_enabled)

        grid = QGridLayout()
        grid.setHorizontalSpacing(15)
        grid.setVerticalSpacing(8)

        grid.addWidget(QLabel("Reset Frequency:"), 0, 0)
        self.quotas_frequency = QComboBox()
        self.quotas_frequency.addItems(["daily", "weekly", "monthly", "rolling_hours"])
        self.quotas_frequency.currentTextChanged.connect(self._on_field_edited)
        grid.addWidget(self.quotas_frequency, 0, 1)

        grid.addWidget(QLabel("Reset Time (24h HH:MM):"), 0, 2)
        self.quotas_reset_time = QLineEdit("00:00")
        self.quotas_reset_time.textChanged.connect(self._on_field_edited)
        grid.addWidget(self.quotas_reset_time, 0, 3)

        grid.addWidget(QLabel("Timezone Offset:"), 1, 0)
        self.quotas_tz_offset = QSpinBox()
        self.quotas_tz_offset.setRange(-12, 14)
        self.quotas_tz_offset.setValue(0)
        self.quotas_tz_offset.setSuffix(" hrs UTC")
        self.quotas_tz_offset.valueChanged.connect(self._on_field_edited)
        grid.addWidget(self.quotas_tz_offset, 1, 1)

        grid.addWidget(QLabel("Day of Week (for Weekly):"), 1, 2)
        self.quotas_day_of_week = QComboBox()
        self.quotas_day_of_week.addItems([
            "Monday (0)", "Tuesday (1)", "Wednesday (2)", "Thursday (3)",
            "Friday (4)", "Saturday (5)", "Sunday (6)"
        ])
        self.quotas_day_of_week.currentIndexChanged.connect(self._on_field_edited)
        grid.addWidget(self.quotas_day_of_week, 1, 3)

        grid.addWidget(QLabel("Day of Month (for Monthly):"), 2, 0)
        self.quotas_day_of_month = QSpinBox()
        self.quotas_day_of_month.setRange(1, 31)
        self.quotas_day_of_month.setValue(1)
        self.quotas_day_of_month.valueChanged.connect(self._on_field_edited)
        grid.addWidget(self.quotas_day_of_month, 2, 1)

        sc_layout.addLayout(grid)
        c_layout.addWidget(sched_card)

        # Card 2: Overage Policy
        ov_card = QFrame()
        ov_card.setObjectName("card")
        ov_layout = QVBoxLayout(ov_card)
        ov_layout.setContentsMargins(14, 12, 14, 12)
        ov_layout.setSpacing(8)

        ov_title = QLabel("Partial Overage Policy")
        ov_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #ffb142;")
        ov_layout.addWidget(ov_title)

        ov_row = QHBoxLayout()
        self.quotas_allow_overage = QCheckBox("Allow Partial Overage on Final Turn")
        self.quotas_allow_overage.setChecked(True)
        self.quotas_allow_overage.toggled.connect(self._on_field_edited)
        ov_row.addWidget(self.quotas_allow_overage)

        ov_row.addWidget(QLabel("Max Overage Buffer (Tokens):"))
        self.quotas_max_overage = QSpinBox()
        self.quotas_max_overage.setRange(-1, 500000)
        self.quotas_max_overage.setValue(1000)
        self.quotas_max_overage.setSpecialValueText("Unlimited (-1)")
        self.quotas_max_overage.valueChanged.connect(self._on_field_edited)
        ov_row.addWidget(self.quotas_max_overage)
        ov_row.addStretch(1)

        ov_layout.addLayout(ov_row)
        c_layout.addWidget(ov_card)

        # Card 3: Role Limits Editor
        limits_card = QFrame()
        limits_card.setObjectName("card")
        lm_layout = QVBoxLayout(limits_card)
        lm_layout.setContentsMargins(14, 12, 14, 12)
        lm_layout.setSpacing(8)

        lm_title = QLabel("Role Limits & Quota Allowances")
        lm_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00ff88;")
        lm_layout.addWidget(lm_title)

        every_row = QHBoxLayout()
        every_row.addWidget(QLabel("Default @everyone Role Limit (Tokens):"))
        self.quotas_everyone_limit = QSpinBox()
        self.quotas_everyone_limit.setRange(-1, 100000000)
        self.quotas_everyone_limit.setValue(10000)
        self.quotas_everyone_limit.setSpecialValueText("Unlimited (-1)")
        self.quotas_everyone_limit.valueChanged.connect(self._on_field_edited)
        every_row.addWidget(self.quotas_everyone_limit)
        every_row.addStretch(1)
        lm_layout.addLayout(every_row)

        cust_lbl = QLabel("Custom Role Allowances (format: RoleNameOrID = Limit, one per line):")
        cust_lbl.setStyleSheet("font-size: 11px; opacity: 0.8; margin-top: 4px;")
        lm_layout.addWidget(cust_lbl)

        self.quotas_custom_limits = QTextEdit()
        self.quotas_custom_limits.setPlaceholderText("VIP = 50000\nSupporter = 25000\nModerator = -1")
        self.quotas_custom_limits.setMaximumHeight(90)
        self.quotas_custom_limits.textChanged.connect(self._on_field_edited)
        lm_layout.addWidget(self.quotas_custom_limits)

        c_layout.addWidget(limits_card)
        c_layout.addStretch(1)

        scroll.setWidget(container)
        page_layout.addWidget(scroll, 1)
        return page

    # -------------------------------------------------------------------------
    # SUBPAGE 2: SERVER PROMPT & PAGINATION (Auto-expanding, No scrollbar)
    # -------------------------------------------------------------------------
    def _setup_prompt_pagination_subpage(self) -> QWidget:
        page = QWidget()
        page_layout = QVBoxLayout(page)
        page_layout.setContentsMargins(0, 0, 0, 0)
        page_layout.setSpacing(10)

        # SECTION 1: System Prompt Card (Stretch factor 1: Auto-expands to fill all empty space)
        prompt_card = QFrame()
        prompt_card.setObjectName("card")
        prompt_layout = QVBoxLayout(prompt_card)
        prompt_layout.setContentsMargins(14, 12, 14, 12)
        prompt_layout.setSpacing(6)

        p_header = QHBoxLayout()
        p_title = QLabel("Server System Prompt / Persona Override")
        p_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        p_header.addWidget(p_title)
        p_header.addStretch(1)

        self.char_counter_label = QLabel("0 / 4,000 chars")
        self.char_counter_label.setStyleSheet("font-size: 11px; opacity: 0.75;")
        p_header.addWidget(self.char_counter_label)
        prompt_layout.addLayout(p_header)

        p_desc = QLabel(
            "Custom instructions or persona for this Discord server. "
            "Foundational safety guardrails are always preserved automatically."
        )
        p_desc.setStyleSheet("font-size: 11px; opacity: 0.8; margin-bottom: 2px;")
        prompt_layout.addWidget(p_desc)

        self.server_system_prompt_input = QTextEdit()
        self.server_system_prompt_input.setPlaceholderText(
            "e.g. You are an expert Python tutor for the Developers server. Keep replies technical, precise, and use Discord code blocks."
        )
        self.server_system_prompt_input.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)
        self.server_system_prompt_input.setMinimumHeight(140)
        self.server_system_prompt_input.textChanged.connect(self._on_prompt_text_changed)
        # Stretch factor 1 inside prompt_layout makes the text editor use all vertical card space
        prompt_layout.addWidget(self.server_system_prompt_input, 1)

        p_tools_row = QHBoxLayout()
        reset_prompt_btn = QPushButton("↺ Reset to Global Default")
        reset_prompt_btn.clicked.connect(lambda: self.server_system_prompt_input.setPlainText(""))
        p_tools_row.addWidget(reset_prompt_btn)

        template_concise_btn = QPushButton("Template: Concise Tech")
        template_concise_btn.clicked.connect(lambda: self.server_system_prompt_input.setPlainText(
            "Answer technical questions with high precision and brevity. Always format code in fenced markdown blocks."
        ))
        p_tools_row.addWidget(template_concise_btn)

        template_mod_btn = QPushButton("Template: Community Assistant")
        template_mod_btn.clicked.connect(lambda: self.server_system_prompt_input.setPlainText(
            "You are a helpful and polite community assistant. Keep answers friendly, welcoming, and easy to understand."
        ))
        p_tools_row.addWidget(template_mod_btn)

        p_tools_row.addStretch(1)
        prompt_layout.addLayout(p_tools_row)

        # Stretch 1 inside page_layout ensures prompt_card uses all available height
        page_layout.addWidget(prompt_card, 1)

        # SECTION 2: Output Delivery & Conversation Memory Card (Anchored to the bottom)
        options_card = QFrame()
        options_card.setObjectName("card")
        options_card_layout = QHBoxLayout(options_card)
        options_card_layout.setContentsMargins(14, 10, 14, 10)
        options_card_layout.setSpacing(20)

        # Left Column: Output & Web Search
        col_left = QVBoxLayout()
        col_left.setSpacing(8)

        out_title = QLabel("Output Delivery & Web Search")
        out_title.setStyleSheet("font-weight: 700; font-size: 12px; color: #00f0ff;")
        col_left.addWidget(out_title)

        pag_row = QHBoxLayout()
        self.pagination_enabled = QCheckBox("Enable Pagination")
        self.pagination_enabled.toggled.connect(self._on_field_edited)
        pag_row.addWidget(self.pagination_enabled)

        pag_row.addWidget(QLabel("Max Chars/Page:"))
        self.pagination_max_chars = QSpinBox()
        self.pagination_max_chars.setRange(500, 1900)
        self.pagination_max_chars.setValue(1000)
        self.pagination_max_chars.valueChanged.connect(self._on_field_edited)
        pag_row.addWidget(self.pagination_max_chars)
        col_left.addLayout(pag_row)

        search_row = QHBoxLayout()
        self.server_web_search_enabled = QCheckBox("Enable Web Search")
        self.server_web_search_enabled.toggled.connect(self._on_field_edited)
        self.server_web_search_auto = QCheckBox("Auto-Search Mode")
        self.server_web_search_auto.toggled.connect(self._on_field_edited)
        search_row.addWidget(self.server_web_search_enabled)
        search_row.addWidget(self.server_web_search_auto)
        col_left.addLayout(search_row)

        options_card_layout.addLayout(col_left, 1)

        # Right Column: Memory & Engagement
        col_right = QVBoxLayout()
        col_right.setSpacing(8)

        mem_title = QLabel("Conversation Memory & Engagement")
        mem_title.setStyleSheet("font-weight: 700; font-size: 12px; color: #00f0ff;")
        col_right.addWidget(mem_title)

        mem_row = QHBoxLayout()
        mem_row.addWidget(QLabel("Memory Turns (0=off):"))
        self.memory_turns = QSpinBox()
        self.memory_turns.setRange(0, 50)
        self.memory_turns.setValue(6)
        self.memory_turns.valueChanged.connect(self._on_field_edited)
        mem_row.addWidget(self.memory_turns)

        mem_row.addWidget(QLabel("Inactivity TTL (min):"))
        self.memory_ttl_minutes = QSpinBox()
        self.memory_ttl_minutes.setRange(1, 1440)
        self.memory_ttl_minutes.setValue(30)
        self.memory_ttl_minutes.valueChanged.connect(self._on_field_edited)
        mem_row.addWidget(self.memory_ttl_minutes)
        col_right.addLayout(mem_row)

        rand_row = QHBoxLayout()
        self.random_prompt_enabled = QCheckBox("Random Prompt Mode")
        self.random_prompt_enabled.toggled.connect(self._on_field_edited)
        rand_row.addWidget(self.random_prompt_enabled)

        rand_row.addWidget(QLabel("Probability:"))
        self.random_prompt_probability = QComboBox()
        self.random_prompt_probability.addItems([f"{x}%" for x in range(0, 101, 5)])
        self.random_prompt_probability.currentTextChanged.connect(self._on_field_edited)
        rand_row.addWidget(self.random_prompt_probability)
        rand_row.addStretch(1)
        col_right.addLayout(rand_row)

        options_card_layout.addLayout(col_right, 1)

        # Stretch 0 pushes options_card to the bottom of the window without any scrollbar!
        page_layout.addWidget(options_card, 0)
        return page

    # -------------------------------------------------------------------------
    # SUBPAGE 3: FORUM AUTO-REPLY
    # -------------------------------------------------------------------------
    def _setup_forum_subpage(self) -> QWidget:
        page = QWidget()
        page_layout = QVBoxLayout(page)
        page_layout.setContentsMargins(0, 0, 0, 0)
        page_layout.setSpacing(10)

        header = self._create_subpage_header(
            "💬 Discord Forum Auto-Reply",
            "Silas Blue automatically monitors newly created forum posts and generates proactive answers."
        )
        page_layout.addWidget(header)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.NoFrame)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)

        container = QWidget()
        c_layout = QVBoxLayout(container)
        c_layout.setContentsMargins(0, 0, 0, 0)
        c_layout.setSpacing(12)

        forum_card = QFrame()
        forum_card.setObjectName("card")
        f_card_layout = QVBoxLayout(forum_card)
        f_card_layout.setContentsMargins(14, 12, 14, 12)
        f_card_layout.setSpacing(10)

        f_toggles_row = QHBoxLayout()
        self.forum_auto_reply_enabled = QCheckBox("Enable Forum Post Auto-Reply")
        self.forum_auto_reply_enabled.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        self.forum_auto_reply_enabled.toggled.connect(self._on_field_edited)
        f_toggles_row.addWidget(self.forum_auto_reply_enabled)

        self.forum_include_post_title = QCheckBox("Include Post Title in Prompt")
        self.forum_include_post_title.setChecked(True)
        self.forum_include_post_title.toggled.connect(self._on_field_edited)
        f_toggles_row.addWidget(self.forum_include_post_title)
        f_toggles_row.addStretch(1)
        f_card_layout.addLayout(f_toggles_row)

        # Monitored Channels Card (search & chip selector, matches permissions)
        self.forum_channel_card = ChannelSelectCard(
            title="💬 Monitored Forum Channels",
            description="Select specific forum channels Silas Blue should monitor for new posts. If empty, all forum channels in this server are monitored."
        )
        self.forum_channel_card.changed.connect(self._on_field_edited)
        f_card_layout.addWidget(self.forum_channel_card)

        f_fields_grid = QGridLayout()
        f_fields_grid.setHorizontalSpacing(15)
        f_fields_grid.setVerticalSpacing(8)

        f_fields_grid.addWidget(QLabel("Multi-Page Delivery Mode:"), 0, 0)
        self.forum_delivery_mode = QComboBox()
        self.forum_delivery_mode.addItem("Send All Chunks Sequentially ([Part 1/N])", "all_chunks")
        self.forum_delivery_mode.addItem("Interactive Pagination Buttons (Persistent)", "interactive")
        self.forum_delivery_mode.currentIndexChanged.connect(self._on_field_edited)
        f_fields_grid.addWidget(self.forum_delivery_mode, 0, 1)

        f_fields_grid.addWidget(QLabel("Required Tags (Whitelist):"), 1, 0)
        self.forum_auto_reply_tags = QLineEdit()
        self.forum_auto_reply_tags.setPlaceholderText("e.g. help, question (leave empty to reply to all tags)")
        self.forum_auto_reply_tags.textChanged.connect(self._on_field_edited)
        f_fields_grid.addWidget(self.forum_auto_reply_tags, 1, 1)

        f_fields_grid.addWidget(QLabel("Ignored Tags (Blacklist):"), 2, 0)
        self.forum_ignore_tags = QLineEdit()
        self.forum_ignore_tags.setPlaceholderText("e.g. resolved, closed, announcement")
        self.forum_ignore_tags.textChanged.connect(self._on_field_edited)
        f_fields_grid.addWidget(self.forum_ignore_tags, 2, 1)

        f_card_layout.addLayout(f_fields_grid)

        f_prompt_lbl = QLabel("Forum-Specific System Prompt (Optional):")
        f_prompt_lbl.setStyleSheet("font-size: 11px; font-weight: 600; margin-top: 4px;")
        f_card_layout.addWidget(f_prompt_lbl)

        self.forum_system_prompt = QTextEdit()
        self.forum_system_prompt.setPlaceholderText("Leave empty to use server default system prompt, or define specialized persona/instructions for forum answers...")
        self.forum_system_prompt.setMaximumHeight(100)
        self.forum_system_prompt.textChanged.connect(self._on_field_edited)
        f_card_layout.addWidget(self.forum_system_prompt)

        c_layout.addWidget(forum_card)
        c_layout.addStretch(1)

        scroll.setWidget(container)
        page_layout.addWidget(scroll, 1)
        return page

    # -------------------------------------------------------------------------
    # SUBPAGE 4: RAW JSON
    # -------------------------------------------------------------------------
    def _setup_raw_json_subpage(self) -> QWidget:
        page = QWidget()
        raw_layout = QVBoxLayout(page)
        raw_layout.setContentsMargins(0, 0, 0, 0)
        raw_layout.setSpacing(10)

        header = self._create_subpage_header(
            "📄 Raw Server JSON Configuration",
            "Directly view and edit the underlying server JSON schema. Changes are validated on save."
        )
        raw_layout.addWidget(header)

        self.raw_config = QTextEdit()
        self.raw_config.textChanged.connect(self._on_field_edited)
        raw_layout.addWidget(self.raw_config, 1)
        return page

    # -------------------------------------------------------------------------
    # DIRTY TRACKING & FEEDBACK
    # -------------------------------------------------------------------------
    def _on_field_edited(self, *args):
        self.mark_dirty(True)

    def _on_prompt_text_changed(self):
        text = self.server_system_prompt_input.toPlainText()
        length = len(text)
        self.char_counter_label.setText(f"{length:,} / 4,000 chars")
        if length > 4000:
            self.char_counter_label.setStyleSheet("color: #ff5252; font-weight: 700; font-size: 11px;")
        else:
            self.char_counter_label.setStyleSheet("font-size: 11px; opacity: 0.75;")
        self.mark_dirty(True)

    def mark_dirty(self, dirty: bool):
        self.is_dirty = dirty
        self.dirty_banner.setVisible(dirty)
        self.discard_btn.setEnabled(dirty)

    # -------------------------------------------------------------------------
    # SERVER SELECTION & DATA SYNC
    # -------------------------------------------------------------------------
    def _on_guild_select_changed(self, new_index: int):
        if self._prev_guild_index == new_index or new_index < 0:
            return

        if self.is_dirty:
            reply = QMessageBox.question(
                self,
                "Unsaved Changes",
                "You have unsaved configuration changes for this server. Do you want to save them before switching?",
                QMessageBox.Save | QMessageBox.Discard | QMessageBox.Cancel,
                QMessageBox.Save
            )
            if reply == QMessageBox.Save:
                self.save_config()
            elif reply == QMessageBox.Cancel:
                self.guild_select.blockSignals(True)
                self.guild_select.setCurrentIndex(self._prev_guild_index)
                self.guild_select.blockSignals(False)
                return

        self._prev_guild_index = new_index
        self.load_config()

    def update_guilds(self):
        current_data = self.guild_select.currentData()
        self.guild_select.blockSignals(True)
        self.guild_select.clear()
        try:
            guilds = getattr(bot, 'guilds', [])
            for g in guilds:
                self.guild_select.addItem(f"{g.name} ({g.id})", g.id)

            idx = self.guild_select.findData(current_data)
            if idx >= 0:
                self.guild_select.setCurrentIndex(idx)
            elif self.guild_select.count() > 0:
                self.guild_select.setCurrentIndex(0)
            self._prev_guild_index = self.guild_select.currentIndex()
        finally:
            self.guild_select.blockSignals(False)

    def load_config(self):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return

        guild = None
        try:
            guild = next((g for g in getattr(bot, 'guilds', []) if g.id == guild_id), None)
        except Exception as e:
            logger.error(f"Error accessing guild: {e}")

        # Update permission cards with guild roles & members
        self.perm_reply.set_guild(guild)
        self.perm_model.set_guild(guild)
        self.perm_admin.set_guild(guild)
        self.perm_search.set_guild(guild)
        self.forum_channel_card.set_guild(guild)

        config = load_config(guild_id)
        self.set_widgets_from_config(config)
        self.mark_dirty(False)

    def reload_config_from_disk(self):
        self.load_config()
        self.show_feedback(self.discard_btn, "Reverted", color="#ffb142")

    def get_config_from_widgets(self) -> dict:
        reply_roles, reply_users = self.perm_reply.get_data()
        model_roles, model_users = self.perm_model.get_data()
        perm_roles, perm_users = self.perm_admin.get_data()
        search_roles, search_users = self.perm_search.get_data()

        max_chars = min(self.pagination_max_chars.value(), 1900)

        # Parse custom token limits
        role_limits = {"everyone": self.quotas_everyone_limit.value()}
        for line in self.quotas_custom_limits.toPlainText().splitlines():
            line = line.strip()
            if "=" in line:
                rk, rv = line.split("=", 1)
                rk = rk.strip()
                if rk:
                    try:
                        role_limits[rk] = int(rv.strip())
                    except ValueError:
                        pass

        token_quotas = {
            "enabled": self.quotas_enabled.isChecked(),
            "role_limits": role_limits,
            "reset_frequency": self.quotas_frequency.currentText().lower(),
            "reset_time": self.quotas_reset_time.text().strip() or "00:00",
            "reset_timezone_offset": self.quotas_tz_offset.value(),
            "reset_day_of_week": self.quotas_day_of_week.currentIndex(),
            "reset_day_of_month": self.quotas_day_of_month.value(),
            "overage_policy": {
                "allow_partial_overage": self.quotas_allow_overage.isChecked(),
                "max_overage_tokens": self.quotas_max_overage.value()
            }
        }

        cfg = {
            "reply_roles": reply_roles,
            "reply_users": reply_users,
            "change_model_roles": model_roles,
            "change_model_users": model_users,
            "change_permission_roles": perm_roles,
            "change_permission_users": perm_users,
            "search_roles": search_roles,
            "search_users": search_users,
            "pagination_enabled": self.pagination_enabled.isChecked(),
            "pagination_max_chars": max_chars,
            "random_prompt_enabled": self.random_prompt_enabled.isChecked(),
            "random_prompt_probability": int(self.random_prompt_probability.currentText().replace("%", "")),
            "system_prompt": self.server_system_prompt_input.toPlainText().strip(),
            "web_search_enabled": self.server_web_search_enabled.isChecked(),
            "web_search_auto": self.server_web_search_auto.isChecked(),
            "memory_turns": self.memory_turns.value(),
            "memory_ttl_minutes": self.memory_ttl_minutes.value(),
            "forum_auto_reply_enabled": self.forum_auto_reply_enabled.isChecked(),
            "forum_include_post_title": self.forum_include_post_title.isChecked(),
            "forum_monitored_channels": self.forum_channel_card.get_channels(),
            "forum_auto_reply_tags": [x.strip() for x in self.forum_auto_reply_tags.text().split(",") if x.strip()],
            "forum_ignore_tags": [x.strip() for x in self.forum_ignore_tags.text().split(",") if x.strip()],
            "forum_system_prompt": self.forum_system_prompt.toPlainText().strip(),
            "forum_delivery_mode": self.forum_delivery_mode.currentData() or "all_chunks",
            "token_quotas": token_quotas
        }
        return validate_and_sanitize_guild_config(cfg)

    def set_widgets_from_config(self, config: dict):
        self.blockSignals(True)
        try:
            self.perm_reply.set_data(config.get("reply_roles", ["everyone"]), config.get("reply_users", []))
            self.perm_model.set_data(config.get("change_model_roles", []), config.get("change_model_users", []))
            self.perm_admin.set_data(config.get("change_permission_roles", []), config.get("change_permission_users", []))
            self.perm_search.set_data(config.get("search_roles", ["everyone"]), config.get("search_users", []))

            self.pagination_enabled.setChecked(config.get("pagination_enabled", True))
            self.pagination_max_chars.setValue(min(config.get("pagination_max_chars", 1000), 1900))
            self.random_prompt_enabled.setChecked(config.get("random_prompt_enabled", False))
            self.random_prompt_probability.setCurrentText(f"{config.get('random_prompt_probability', 0)}%")
            self.server_system_prompt_input.setPlainText(config.get("system_prompt", ""))
            self.server_web_search_enabled.setChecked(config.get("web_search_enabled", True))
            self.server_web_search_auto.setChecked(config.get("web_search_auto", True))
            self.memory_turns.setValue(int(config.get("memory_turns", 6)))
            self.memory_ttl_minutes.setValue(int(config.get("memory_ttl_minutes", 30)))

            self.forum_auto_reply_enabled.setChecked(config.get("forum_auto_reply_enabled", False))
            self.forum_include_post_title.setChecked(config.get("forum_include_post_title", True))
            self.forum_channel_card.set_channels(config.get("forum_monitored_channels", []))
            self.forum_auto_reply_tags.setText(", ".join(config.get("forum_auto_reply_tags", [])))
            self.forum_ignore_tags.setText(", ".join(config.get("forum_ignore_tags", [])))
            self.forum_system_prompt.setPlainText(config.get("forum_system_prompt", ""))
            mode = config.get("forum_delivery_mode", "all_chunks")
            idx_mode = self.forum_delivery_mode.findData(mode)
            if idx_mode >= 0:
                self.forum_delivery_mode.setCurrentIndex(idx_mode)

            # Token Quotas
            tq = config.get("token_quotas", {})
            self.quotas_enabled.setChecked(bool(tq.get("enabled", False)))
            raw_freq = str(tq.get("reset_frequency", "daily")).lower()
            idx_freq = self.quotas_frequency.findText(raw_freq, Qt.MatchFixedString)
            if idx_freq >= 0:
                self.quotas_frequency.setCurrentIndex(idx_freq)
            self.quotas_reset_time.setText(str(tq.get("reset_time", "00:00")))
            self.quotas_tz_offset.setValue(int(tq.get("reset_timezone_offset", 0)))
            self.quotas_day_of_week.setCurrentIndex(max(0, min(int(tq.get("reset_day_of_week", 0)), 6)))
            self.quotas_day_of_month.setValue(max(1, min(int(tq.get("reset_day_of_month", 1)), 31)))

            op = tq.get("overage_policy", {})
            self.quotas_allow_overage.setChecked(bool(op.get("allow_partial_overage", True)))
            self.quotas_max_overage.setValue(int(op.get("max_overage_tokens", 1000)))

            rl = tq.get("role_limits", {"everyone": 10000})
            self.quotas_everyone_limit.setValue(int(rl.get("everyone", 10000)))
            custom_lines = [f"{k} = {v}" for k, v in rl.items() if str(k).lower() != "everyone"]
            self.quotas_custom_limits.setPlainText("\n".join(custom_lines))

            self.raw_config.setPlainText(json.dumps(config, indent=2))
        finally:
            self.blockSignals(False)

    def save_config(self):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return

        if self._current_subpage_index == 4:
            try:
                raw_text = self.raw_config.toPlainText()
                config = json.loads(raw_text)
                config = validate_and_sanitize_guild_config(config)
            except json.JSONDecodeError as e:
                QMessageBox.critical(self, "Invalid JSON", f"Syntax error in JSON:\n{e}")
                return
        else:
            config = self.get_config_from_widgets()

        # Save to disk atomically
        save_config(guild_id, config)
        # Update running bot immediately
        update_server_config(guild_id, config)

        self.raw_config.blockSignals(True)
        self.raw_config.setPlainText(json.dumps(config, indent=2))
        self.raw_config.blockSignals(False)

        self.mark_dirty(False)
        logger.info(f"Configuration for server {guild_id} saved and applied.")
        self.show_feedback(self.save_btn, "✓ Saved & Synced!", color="#2ea043")

    def show_feedback(self, button: QPushButton, message: str, color: str = "#2ea043"):
        orig_text = button.text()
        button.setText(message)
        button.setStyleSheet(f"background-color: {color}; color: #ffffff; font-weight: 700;")
        QTimer.singleShot(1400, lambda: [button.setText(orig_text), button.setStyleSheet("")])
        if hasattr(self.main_window, "append_system_log"):
            self.main_window.append_system_log("[Server Config] Settings saved and applied to active server.")

    def open_invite_dialog(self):
        """Opens the Invite Bot dialog."""
        from .invite_dialog import InviteBotDialog
        dialog = InviteBotDialog(self)
        dialog.exec()
