from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QLabel, QComboBox, QSpinBox, QTextEdit, QPushButton,
    QTabWidget, QListWidget, QListWidgetItem, QCheckBox, QHBoxLayout, QFrame
)
from PySide6.QtCore import QTimer, Qt
import json
import logging

from utils import load_config, save_config
from bot_core import bot, update_server_config

logger = logging.getLogger("silasblue")

class ServerConfigPage(QWidget):
    """
    GUI page for editing per-server configuration with modern card layouts.
    """

    def __init__(self, main_window):
        super().__init__()
        self.main_window = main_window
        main_layout = QVBoxLayout()
        main_layout.setContentsMargins(16, 16, 16, 16)
        main_layout.setSpacing(14)
        self.setLayout(main_layout)

        # --- Top Header / Server Selector Card ---
        header_card = QFrame()
        header_card.setObjectName("card")
        header_layout = QHBoxLayout(header_card)
        header_layout.setContentsMargins(14, 12, 14, 12)

        server_label = QLabel("Active Discord Server:")
        server_label.setStyleSheet("font-weight: 600; font-size: 13px;")
        header_layout.addWidget(server_label)

        self.guild_select = QComboBox()
        self.guild_select.setMinimumWidth(260)
        header_layout.addWidget(self.guild_select, 3)

        self.invite_btn = QPushButton("➕ Invite Bot to Server")
        self.invite_btn.setToolTip("Invite Silas Blue to a Discord server")
        self.invite_btn.clicked.connect(self.open_invite_dialog)
        header_layout.addWidget(self.invite_btn, 1)

        main_layout.addWidget(header_card)

        # Tab widget for GUI / Raw JSON
        self.tabs = QTabWidget()
        main_layout.addWidget(self.tabs, 1)

        # --- GUI Editor Tab ---
        self.gui_tab = QWidget()
        gui_layout = QVBoxLayout()
        gui_layout.setContentsMargins(8, 12, 8, 8)
        gui_layout.setSpacing(14)
        self.gui_tab.setLayout(gui_layout)

        # --- Permissions Card ---
        perm_card = QFrame()
        perm_card.setObjectName("card")
        perm_card_layout = QVBoxLayout(perm_card)
        perm_card_layout.setContentsMargins(12, 12, 12, 12)

        perm_title = QLabel("Role-Based Access Control")
        perm_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff; margin-bottom: 4px;")
        perm_card_layout.addWidget(perm_title)

        permissions_row = QHBoxLayout()
        permissions_row.setSpacing(12)

        # Reply roles
        reply_col = QVBoxLayout()
        reply_label = QLabel("Who can the bot reply to?")
        reply_label.setStyleSheet("font-weight: 500; font-size: 12px;")
        reply_col.addWidget(reply_label)
        self.reply_roles_list = QListWidget()
        self.reply_roles_list.setSelectionMode(QListWidget.MultiSelection)
        reply_col.addWidget(self.reply_roles_list)
        permissions_row.addLayout(reply_col)

        # Change model roles
        model_col = QVBoxLayout()
        model_label = QLabel("Who can switch AI models?")
        model_label.setStyleSheet("font-weight: 500; font-size: 12px;")
        model_col.addWidget(model_label)
        self.change_model_roles_list = QListWidget()
        self.change_model_roles_list.setSelectionMode(QListWidget.MultiSelection)
        model_col.addWidget(self.change_model_roles_list)
        permissions_row.addLayout(model_col)

        # Change permission roles
        perm_role_col = QVBoxLayout()
        perm_role_label = QLabel("Who can edit server settings?")
        perm_role_label.setStyleSheet("font-weight: 500; font-size: 12px;")
        perm_role_col.addWidget(perm_role_label)
        self.change_permission_roles_list = QListWidget()
        self.change_permission_roles_list.setSelectionMode(QListWidget.MultiSelection)
        perm_role_col.addWidget(self.change_permission_roles_list)
        permissions_row.addLayout(perm_role_col)

        perm_card_layout.addLayout(permissions_row)
        gui_layout.addWidget(perm_card, 2)

        # --- Options & Behaviors Card ---
        options_card = QFrame()
        options_card.setObjectName("card")
        options_layout = QVBoxLayout(options_card)
        options_layout.setContentsMargins(12, 12, 12, 12)

        options_title = QLabel("Server Response Settings")
        options_title.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff; margin-bottom: 4px;")
        options_layout.addWidget(options_title)

        pag_rand_row = QHBoxLayout()
        self.pagination_enabled = QCheckBox("Enable Message Pagination")
        pag_rand_row.addWidget(self.pagination_enabled)

        pag_rand_row.addWidget(QLabel("Max chars/page (≤1900):"))
        self.pagination_max_chars = QSpinBox()
        self.pagination_max_chars.setRange(500, 1900)
        self.pagination_max_chars.setValue(1900)
        pag_rand_row.addWidget(self.pagination_max_chars)

        pag_rand_row.addSpacing(20)

        self.random_prompt_enabled = QCheckBox("Random Prompt Mode")
        pag_rand_row.addWidget(self.random_prompt_enabled)

        pag_rand_row.addWidget(QLabel("Probability:"))
        self.random_prompt_probability = QComboBox()
        self.random_prompt_probability.addItems([f"{x}%" for x in range(0, 101, 5)])
        pag_rand_row.addWidget(self.random_prompt_probability)

        pag_rand_row.addStretch(1)
        options_layout.addLayout(pag_rand_row)
        gui_layout.addWidget(options_card)

        # Save Button Row
        self.save_btn = QPushButton("Save Server Settings")
        self.save_btn.setObjectName("primaryBtn")
        self.save_btn.setMinimumHeight(34)
        self.save_status_label = QLabel("")
        self.save_status_label.setStyleSheet("font-weight: 600; color: #00ff88;")

        save_btn_row = QHBoxLayout()
        save_btn_row.addStretch(2)
        save_btn_row.addWidget(self.save_btn, 2)
        save_btn_row.addWidget(self.save_status_label)
        save_btn_row.addStretch(2)
        gui_layout.addLayout(save_btn_row)

        self.tabs.addTab(self.gui_tab, "GUI Editor")

        # --- Raw JSON Tab ---
        self.raw_tab = QWidget()
        raw_layout = QVBoxLayout()
        raw_layout.setContentsMargins(12, 12, 12, 12)
        self.raw_tab.setLayout(raw_layout)

        raw_header = QLabel("Raw Server JSON Configuration:")
        raw_header.setStyleSheet("font-weight: 600;")
        raw_layout.addWidget(raw_header)

        self.raw_config = QTextEdit()
        raw_layout.addWidget(self.raw_config, 1)

        raw_save_btn = QPushButton("Save Raw JSON")
        raw_save_btn.setObjectName("primaryBtn")
        raw_save_btn.clicked.connect(self.save_config)
        raw_save_row = QHBoxLayout()
        raw_save_row.addStretch(1)
        raw_save_row.addWidget(raw_save_btn)
        raw_layout.addLayout(raw_save_row)

        self.tabs.addTab(self.raw_tab, "Raw JSON")

        # Connect signals
        self.save_btn.clicked.connect(self.save_config)
        self.guild_select.currentIndexChanged.connect(self.load_config)
        self.tabs.currentChanged.connect(self.on_tab_changed)

        self.reply_roles_list.itemChanged.connect(lambda _: self.on_roles_checkbox_changed())
        self.change_model_roles_list.itemChanged.connect(lambda _: self.on_roles_checkbox_changed())
        self.change_permission_roles_list.itemChanged.connect(lambda _: self.on_roles_checkbox_changed())

        # Initial population
        self.update_guilds()
        self.load_config()

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
                self.load_config()
        except Exception as e:
            logger.error(f"Failed to update guilds: {e}")

    def load_config(self):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return
        try:
            guild = next((g for g in getattr(bot, 'guilds', []) if g.id == guild_id), None)
            if not guild:
                return
        except Exception as e:
            logger.error(f"Failed to load config: {e}")
            return
        config = load_config(guild_id)
        self.set_roles_list(self.reply_roles_list, guild, config.get("reply_roles", []))
        self.set_roles_list(self.change_model_roles_list, guild, config.get("change_model_roles", []))
        self.set_roles_list(self.change_permission_roles_list, guild, config.get("change_permission_roles", []))
        self.pagination_enabled.setChecked(config.get("pagination_enabled", True))
        self.pagination_max_chars.setValue(min(config.get("pagination_max_chars", 1900), 1900))
        self.random_prompt_enabled.setChecked(config.get("random_prompt_enabled", False))
        self.random_prompt_probability.setCurrentText(f"{config.get('random_prompt_probability', 0)}%")
        self.raw_config.setPlainText(json.dumps(config, indent=2))

    def set_roles_list(self, list_widget, guild, selected_roles):
        list_widget.blockSignals(True)
        list_widget.clear()
        selected_lower = [r.lower() for r in selected_roles]
        for role in guild.roles:
            if role.is_default():
                continue
            item = QListWidgetItem(role.name)
            item.setFlags(item.flags() | Qt.ItemIsUserCheckable)
            item.setCheckState(Qt.Checked if role.name.lower() in selected_lower else Qt.Unchecked)
            list_widget.addItem(item)
        item = QListWidgetItem("@everyone")
        item.setFlags(item.flags() | Qt.ItemIsUserCheckable)
        item.setCheckState(Qt.Checked if "everyone" in selected_lower else Qt.Unchecked)
        list_widget.addItem(item)
        list_widget.blockSignals(False)

    def get_selected_roles(self, list_widget):
        roles = []
        for i in range(list_widget.count()):
            item = list_widget.item(i)
            if item.checkState() == Qt.Checked:
                if item.text() == "@everyone":
                    roles.append("everyone")
                else:
                    roles.append(item.text())
        return roles

    def get_config_from_widgets(self):
        max_chars = min(self.pagination_max_chars.value(), 1900)
        return {
            "reply_roles": self.get_selected_roles(self.reply_roles_list),
            "change_model_roles": self.get_selected_roles(self.change_model_roles_list),
            "change_permission_roles": self.get_selected_roles(self.change_permission_roles_list),
            "pagination_enabled": self.pagination_enabled.isChecked(),
            "pagination_max_chars": max_chars,
            "random_prompt_enabled": self.random_prompt_enabled.isChecked(),
            "random_prompt_probability": int(self.random_prompt_probability.currentText().replace("%", ""))
        }

    def on_tab_changed(self, idx):
        if idx == 1:
            config = self.get_config_from_widgets()
            self.raw_config.setPlainText(json.dumps(config, indent=2))
        elif idx == 0:
            try:
                config = json.loads(self.raw_config.toPlainText())
                self.set_widgets_from_config(config)
            except Exception:
                pass

    def set_widgets_from_config(self, config):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return
        guild = next((g for g in getattr(bot, 'guilds', []) if g.id == guild_id), None)
        if not guild:
            return
        self.set_roles_list(self.reply_roles_list, guild, config.get("reply_roles", []))
        self.set_roles_list(self.change_model_roles_list, guild, config.get("change_model_roles", []))
        self.set_roles_list(self.change_permission_roles_list, guild, config.get("change_permission_roles", []))
        self.pagination_enabled.setChecked(config.get("pagination_enabled", True))
        self.pagination_max_chars.setValue(min(config.get("pagination_max_chars", 1900), 1900))
        self.random_prompt_enabled.setChecked(config.get("random_prompt_enabled", False))
        self.random_prompt_probability.setCurrentText(f"{config.get('random_prompt_probability', 0)}%")

    def save_config(self):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return
        if self.tabs.currentIndex() == 0:
            config = self.get_config_from_widgets()
        else:
            try:
                config = json.loads(self.raw_config.toPlainText())
            except Exception:
                return

        # Ensure pagination_max_chars stays within Discord safe limits
        if config.get("pagination_max_chars", 1900) > 1900:
            config["pagination_max_chars"] = 1900

        # Save to disk
        save_config(guild_id, config)
        # Update running bot immediately!
        update_server_config(guild_id, config)

        self.raw_config.setPlainText(json.dumps(config, indent=2))
        logger.info(f"Config for server {guild_id} saved and applied.")
        self.show_feedback(self.save_btn, "✓ Saved!")
        self.reload_config_from_disk()

    def show_feedback(self, button, message):
        orig_text = button.text()
        button.setText(message)
        button.setStyleSheet("background-color: #2ea043; color: #ffffff;")
        QTimer.singleShot(1200, lambda: [button.setText(orig_text), button.setStyleSheet("")])
        if hasattr(self.main_window, "append_system_log"):
            self.main_window.append_system_log(f"[Server Config] Saved settings for active server.")

    def reload_config_from_disk(self):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return
        config = load_config(guild_id)
        if not self.raw_config.hasFocus():
            self.raw_config.setPlainText(json.dumps(config, indent=2))
        if self.tabs.currentIndex() == 0:
            self.set_widgets_from_config(config)

    def on_roles_checkbox_changed(self):
        config = self.get_config_from_widgets()
        self.raw_config.setPlainText(json.dumps(config, indent=2))

    def open_invite_dialog(self):
        """Opens the Invite Bot dialog."""
        from .invite_dialog import InviteBotDialog
        dialog = InviteBotDialog(self)
        dialog.exec()