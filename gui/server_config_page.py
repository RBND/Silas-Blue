"""
GUI page for editing per-server configuration with modern Discord-grade UX/UI.
Features:
- Sticky server selector header and sticky action footer.
- Root QScrollArea preventing layout crushing and 1-line boxes across all screen resolutions and DPI scalings.
- Scalable 4-domain Access Control supporting Roles and specific User Snowflake IDs via FlowLayout chips.
- Expanded System Prompt editor with live character counter and templates.
- Dirty-state tracking with unsaved changes confirmation modal on server switch.
- Robust JSON syntax validation.
"""

from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QGridLayout, QLabel, QComboBox,
    QSpinBox, QTextEdit, QPushButton, QTabWidget, QCheckBox, QFrame,
    QScrollArea, QMessageBox, QSizePolicy
)
from PySide6.QtCore import QTimer, Qt, Signal
import json
import logging

from utils import load_config, save_config, validate_and_sanitize_guild_config
from bot_core import bot, update_server_config
from .permission_chips import PermissionGrantCard

logger = logging.getLogger("silasblue")


class ServerConfigPage(QWidget):
    """
    Overhauled per-server configuration page designed for scalability and high-DPI responsiveness.
    """

    def __init__(self, main_window):
        super().__init__()
        self.main_window = main_window
        self.is_dirty = False
        self._prev_guild_index = -1

        # Root Layout: Sticky Header + Tabs + Sticky Footer
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
        # 2. MAIN CONTENT TABS
        # ---------------------------------------------------------------------
        self.tabs = QTabWidget()
        root_layout.addWidget(self.tabs, 1)

        # Build GUI Editor Tab (Wrapped in QScrollArea)
        self._setup_gui_editor_tab()

        # Build Raw JSON Tab
        self._setup_raw_json_tab()

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
        self.tabs.currentChanged.connect(self.on_tab_changed)

        self.update_guilds()
        self.load_config()

    # -------------------------------------------------------------------------
    # TAB 1: GUI EDITOR
    # -------------------------------------------------------------------------
    def _setup_gui_editor_tab(self):
        self.gui_tab_wrapper = QWidget()
        wrapper_layout = QVBoxLayout(self.gui_tab_wrapper)
        wrapper_layout.setContentsMargins(0, 0, 0, 0)

        # Top-level QScrollArea guarantees no vertical crushing
        self.scroll_area = QScrollArea()
        self.scroll_area.setWidgetResizable(True)
        self.scroll_area.setFrameShape(QFrame.NoFrame)
        self.scroll_area.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)

        container = QWidget()
        container_layout = QVBoxLayout(container)
        container_layout.setContentsMargins(8, 10, 8, 12)
        container_layout.setSpacing(14)

        # SECTION 1: System Prompt / Persona Override Card
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
        self.server_system_prompt_input.setMinimumHeight(105)
        self.server_system_prompt_input.textChanged.connect(self._on_prompt_text_changed)
        prompt_layout.addWidget(self.server_system_prompt_input)

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

        container_layout.addWidget(prompt_card)

        # SECTION 2: Access Control & Permissions (2x2 Grid)
        access_header = QLabel("Role-Based & User Access Control")
        access_header.setStyleSheet("font-weight: 700; font-size: 14px; margin-top: 4px;")
        container_layout.addWidget(access_header)

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

        # 2. AI Model Switching
        self.perm_model = PermissionGrantCard(
            title="⚡ AI Model Switching",
            description="Who can use the !model command to change the active LLM for this server.",
            default_everyone=False
        )
        self.perm_model.changed.connect(self._on_field_edited)
        perm_grid.addWidget(self.perm_model, 0, 1)

        # 3. Server Bot Settings
        self.perm_admin = PermissionGrantCard(
            title="🛡️ Server Bot Administration",
            description="Who can configure prompts, memory, and permissions via bot commands (!setprompt, !setmemory, !setttl).",
            default_everyone=False
        )
        self.perm_admin.changed.connect(self._on_field_edited)
        perm_grid.addWidget(self.perm_admin, 1, 0)

        # 4. Web Search & Grounding
        self.perm_search = PermissionGrantCard(
            title="🌐 Web Search & Grounding",
            description="Who can use !search or trigger real-time AI web search grounding in this server.",
            default_everyone=True
        )
        self.perm_search.changed.connect(self._on_field_edited)
        perm_grid.addWidget(self.perm_search, 1, 1)

        container_layout.addLayout(perm_grid)

        # SECTION 3: Response & Memory Tuning (2 Columns)
        options_header = QLabel("Server Response & Memory Behaviors")
        options_header.setStyleSheet("font-weight: 700; font-size: 14px; margin-top: 4px;")
        container_layout.addWidget(options_header)

        options_card = QFrame()
        options_card.setObjectName("card")
        options_card_layout = QHBoxLayout(options_card)
        options_card_layout.setContentsMargins(14, 12, 14, 12)
        options_card_layout.setSpacing(20)

        # Left Column: Output & Search
        col_left = QVBoxLayout()
        col_left.setSpacing(10)

        out_title = QLabel("Output Delivery & Web Search")
        out_title.setStyleSheet("font-weight: 700; font-size: 12px; color: #00f0ff;")
        col_left.addWidget(out_title)

        pag_row = QHBoxLayout()
        self.pagination_enabled = QCheckBox("Enable Message Pagination")
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
        col_left.addStretch(1)

        options_card_layout.addLayout(col_left, 1)

        # Right Column: Memory & Engagement
        col_right = QVBoxLayout()
        col_right.setSpacing(10)

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
        col_right.addStretch(1)

        options_card_layout.addLayout(col_right, 1)

        container_layout.addWidget(options_card)

        self.scroll_area.setWidget(container)
        wrapper_layout.addWidget(self.scroll_area)
        self.tabs.addTab(self.gui_tab_wrapper, "GUI Editor")

    # -------------------------------------------------------------------------
    # TAB 2: RAW JSON
    # -------------------------------------------------------------------------
    def _setup_raw_json_tab(self):
        self.raw_tab = QWidget()
        raw_layout = QVBoxLayout(self.raw_tab)
        raw_layout.setContentsMargins(12, 12, 12, 12)
        raw_layout.setSpacing(10)

        raw_header = QLabel("Raw Server JSON Configuration:")
        raw_header.setStyleSheet("font-weight: 600; font-size: 13px;")
        raw_layout.addWidget(raw_header)

        self.raw_config = QTextEdit()
        self.raw_config.textChanged.connect(self._on_field_edited)
        raw_layout.addWidget(self.raw_config, 1)

        raw_save_row = QHBoxLayout()
        raw_save_row.addStretch(1)
        raw_save_btn = QPushButton("Save Raw JSON")
        raw_save_btn.setObjectName("primaryBtn")
        raw_save_btn.clicked.connect(self.save_config)
        raw_save_row.addWidget(raw_save_btn)
        raw_layout.addLayout(raw_save_row)

        self.tabs.addTab(self.raw_tab, "Raw JSON")

    # -------------------------------------------------------------------------
    # DIRTY STATE & CHANGE DETECTION
    # -------------------------------------------------------------------------
    def _on_field_edited(self, *args):
        self.mark_dirty(True)

    def _on_prompt_text_changed(self):
        text = self.server_system_prompt_input.toPlainText()
        self.char_counter_label.setText(f"{len(text):,} / 4,000 chars")
        if len(text) > 4000:
            self.char_counter_label.setStyleSheet("font-size: 11px; color: #ff5252; font-weight: 700;")
        else:
            self.char_counter_label.setStyleSheet("font-size: 11px; opacity: 0.75;")
        self.mark_dirty(True)

    def mark_dirty(self, is_dirty: bool):
        self.is_dirty = is_dirty
        self.dirty_banner.setVisible(is_dirty)
        self.discard_btn.setEnabled(is_dirty)
        if is_dirty:
            self.save_btn.setStyleSheet("background-color: #5865f2; color: #ffffff; font-weight: 700;")
        else:
            self.save_btn.setStyleSheet("")

    def _on_guild_select_changed(self, new_idx: int):
        if self._prev_guild_index == new_idx or new_idx == -1:
            return

        if self.is_dirty:
            reply = QMessageBox.question(
                self,
                "Unsaved Changes",
                "You have unsaved changes for the active server. Would you like to save them before switching?",
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

        self._prev_guild_index = new_idx
        self.load_config()

    # -------------------------------------------------------------------------
    # CONFIG LOAD & SAVE
    # -------------------------------------------------------------------------
    def update_guilds(self):
        """Populate the guild selection box with connected servers."""
        try:
            current_ids = [self.guild_select.itemData(i) for i in range(self.guild_select.count())]
            new_guilds = [(guild.name, guild.id) for guild in getattr(bot, 'guilds', [])]
            if len(current_ids) != len(new_guilds) or any(str(gid) not in [str(x[1]) for x in new_guilds] for gid in current_ids):
                self.guild_select.blockSignals(True)
                self.guild_select.clear()
                for name, gid in new_guilds:
                    self.guild_select.addItem(f"{name} ({gid})", userData=gid)
                self.guild_select.blockSignals(False)
                self._prev_guild_index = self.guild_select.currentIndex()
                self.load_config()
        except Exception as e:
            logger.error(f"Failed to update guilds: {e}")

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
            "token_quotas": getattr(self, "_current_token_quotas", {})
        }
        return validate_and_sanitize_guild_config(cfg)

    def set_widgets_from_config(self, config: dict):
        self.blockSignals(True)
        try:
            self._current_token_quotas = config.get("token_quotas", {})
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

            self.raw_config.setPlainText(json.dumps(config, indent=2))
        finally:
            self.blockSignals(False)

    def on_tab_changed(self, idx: int):
        if idx == 1:
            config = self.get_config_from_widgets()
            self.raw_config.blockSignals(True)
            self.raw_config.setPlainText(json.dumps(config, indent=2))
            self.raw_config.blockSignals(False)
        elif idx == 0:
            try:
                config = json.loads(self.raw_config.toPlainText())
                self.set_widgets_from_config(config)
            except Exception:
                pass

    def save_config(self):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return

        if self.tabs.currentIndex() == 0:
            config = self.get_config_from_widgets()
        else:
            try:
                raw_text = self.raw_config.toPlainText()
                config = json.loads(raw_text)
                config = validate_and_sanitize_guild_config(config)
            except json.JSONDecodeError as e:
                QMessageBox.critical(self, "Invalid JSON", f"Syntax error in JSON:\n{e}")
                return

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