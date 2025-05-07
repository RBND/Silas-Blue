from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QLabel, QComboBox, QSpinBox, QTextEdit, QPushButton, QTabWidget, QListWidget, QListWidgetItem, QCheckBox, QHBoxLayout
)
from PySide6.QtCore import QTimer, Qt
import os
import json
import logging

from utils import load_config, save_config
from bot_core import bot

class ServerConfigPage(QWidget):
    """
    GUI page for editing per-server configuration.
    """

    def __init__(self, main_window):
        super().__init__()
        self.main_window = main_window
        main_layout = QVBoxLayout()
        self.setLayout(main_layout)

        self.guild_select = QComboBox()
        main_layout.addWidget(QLabel("Select Server:"))
        main_layout.addWidget(self.guild_select)

        # Tab widget for GUI/Raw JSON
        self.tabs = QTabWidget()
        main_layout.addWidget(self.tabs)

        # --- GUI Editor Tab ---
        self.gui_tab = QWidget()
        gui_layout = QVBoxLayout()
        self.gui_tab.setLayout(gui_layout)

        # Dynamic role selectors
        gui_layout.addWidget(QLabel("Who can the bot reply to?"))
        self.reply_roles_list = QListWidget()
        self.reply_roles_list.setSelectionMode(QListWidget.MultiSelection)
        gui_layout.addWidget(self.reply_roles_list)

        gui_layout.addWidget(QLabel("Who can change the current model?"))
        self.change_model_roles_list = QListWidget()
        self.change_model_roles_list.setSelectionMode(QListWidget.MultiSelection)
        gui_layout.addWidget(self.change_model_roles_list)

        gui_layout.addWidget(QLabel("Who can change permission settings?"))
        self.change_permission_roles_list = QListWidget()
        self.change_permission_roles_list.setSelectionMode(QListWidget.MultiSelection)
        gui_layout.addWidget(self.change_permission_roles_list)

        # Pagination
        self.pagination_enabled = QCheckBox("Enable Pagination")
        self.pagination_max_chars = QSpinBox()
        self.pagination_max_chars.setRange(500, 4000)
        self.pagination_max_chars.setValue(2000)
        gui_layout.addWidget(self.pagination_enabled)
        gui_layout.addWidget(QLabel("Max characters per page:"))
        gui_layout.addWidget(self.pagination_max_chars)

        # Random prompt
        self.random_prompt_enabled = QCheckBox("Enable random prompt mode")
        self.random_prompt_probability = QComboBox()
        self.random_prompt_probability.addItems([f"{x}%" for x in range(0, 101, 5)])
        gui_layout.addWidget(self.random_prompt_enabled)
        gui_layout.addWidget(QLabel("Probability:"))
        gui_layout.addWidget(self.random_prompt_probability)

        self.save_btn = QPushButton("Save Config")
        gui_layout.addWidget(self.save_btn)

        self.tabs.addTab(self.gui_tab, "GUI Editor")

        # --- Raw JSON Tab ---
        self.raw_tab = QWidget()
        raw_layout = QVBoxLayout()
        self.raw_tab.setLayout(raw_layout)
        raw_layout.addWidget(QLabel("Raw Config (JSON):"))
        self.raw_config = QTextEdit()
        raw_layout.addWidget(self.raw_config)
        self.tabs.addTab(self.raw_tab, "Raw JSON")

        # Connect signals
        self.save_btn.clicked.connect(self.save_config)
        self.guild_select.currentIndexChanged.connect(self.load_config)
        self.tabs.currentChanged.connect(self.on_tab_changed)

        # Initial population
        self.update_guilds()
        self.load_config()

        # Timer to periodically reload config from disk
        self.config_reload_timer = QTimer(self)
        self.config_reload_timer.timeout.connect(self.reload_config_from_disk)
        self.config_reload_timer.start(2000)  # Every 2 seconds

    def update_guilds(self):
        """Populate the guild selection box with connected servers."""
        try:
            from bot_core import _bot_instance
            bot = _bot_instance
            if not bot:
                return
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
            logging.getLogger("silasblue").error(f"Failed to update guilds: {e}")

    def load_config(self):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return
        try:
            from bot_core import _bot_instance
            bot = _bot_instance
            if not bot:
                return
            guild = next((g for g in getattr(bot, 'guilds', []) if g.id == guild_id), None)
            if not guild:
                return
        except Exception as e:
            logging.getLogger("silasblue").error(f"Failed to load config: {e}")
            return
        config = load_config(guild_id)
        self.set_roles_list(self.reply_roles_list, guild, config.get("reply_roles", []))
        self.set_roles_list(self.change_model_roles_list, guild, config.get("change_model_roles", []))
        self.set_roles_list(self.change_permission_roles_list, guild, config.get("change_permission_roles", []))
        self.pagination_enabled.setChecked(config.get("pagination_enabled", True))
        self.pagination_max_chars.setValue(config.get("pagination_max_chars", 2000))
        self.random_prompt_enabled.setChecked(config.get("random_prompt_enabled", False))
        self.random_prompt_probability.setCurrentText(f"{config.get('random_prompt_probability', 0)}%")
        self.raw_config.setPlainText(json.dumps(config, indent=2))

    def set_roles_list(self, list_widget, guild, selected_roles):
        list_widget.clear()
        for role in guild.roles:
            if role.is_default():
                continue  # Skip @everyone for selection, handle separately if needed
            item = QListWidgetItem(role.name)
            item.setFlags(item.flags() | Qt.ItemIsUserCheckable)
            item.setCheckState(Qt.Checked if role.name in selected_roles else Qt.Unchecked)
            list_widget.addItem(item)
        # Optionally add @everyone
        item = QListWidgetItem("@everyone")
        item.setFlags(item.flags() | Qt.ItemIsUserCheckable)
        item.setCheckState(Qt.Checked if "everyone" in selected_roles else Qt.Unchecked)
        list_widget.addItem(item)

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
        config = {
            "reply_roles": self.get_selected_roles(self.reply_roles_list),
            "change_model_roles": self.get_selected_roles(self.change_model_roles_list),
            "change_permission_roles": self.get_selected_roles(self.change_permission_roles_list),
            "pagination_enabled": self.pagination_enabled.isChecked(),
            "pagination_max_chars": self.pagination_max_chars.value(),
            "random_prompt_enabled": self.random_prompt_enabled.isChecked(),
            "random_prompt_probability": int(self.random_prompt_probability.currentText().replace("%", ""))
        }
        return config

    def on_tab_changed(self, idx):
        # 0 = GUI, 1 = Raw JSON
        if idx == 1:
            # Switched to Raw JSON: update JSON from widgets
            config = self.get_config_from_widgets()
            self.raw_config.setPlainText(json.dumps(config, indent=2))
        elif idx == 0:
            # Switched to GUI: update widgets from JSON (if valid)
            try:
                config = json.loads(self.raw_config.toPlainText())
                self.set_widgets_from_config(config)
            except Exception:
                pass

    def set_widgets_from_config(self, config):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return
        try:
            from bot_core import _bot_instance
            bot = _bot_instance
            if not bot:
                return
            guild = next((g for g in getattr(bot, 'guilds', []) if g.id == guild_id), None)
            if not guild:
                return
        except Exception as e:
            logging.getLogger("silasblue").error(f"Failed to set widgets from config: {e}")
            return
        self.set_roles_list(self.reply_roles_list, guild, config.get("reply_roles", []))
        self.set_roles_list(self.change_model_roles_list, guild, config.get("change_model_roles", []))
        self.set_roles_list(self.change_permission_roles_list, guild, config.get("change_permission_roles", []))
        self.pagination_enabled.setChecked(config.get("pagination_enabled", True))
        self.pagination_max_chars.setValue(config.get("pagination_max_chars", 2000))
        self.random_prompt_enabled.setChecked(config.get("random_prompt_enabled", False))
        self.random_prompt_probability.setCurrentText(f"{config.get('random_prompt_probability', 0)}%")

    def save_config(self):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return
        # Save from whichever tab is active
        if self.tabs.currentIndex() == 0:
            config = self.get_config_from_widgets()
        else:
            try:
                config = json.loads(self.raw_config.toPlainText())
            except Exception:
                return  # Invalid JSON, do not save
        save_config(guild_id, config)
        self.raw_config.setPlainText(json.dumps(config, indent=2))
        logging.getLogger("silasblue").info(f"Config for server {guild_id} saved via GUI.")
        self.show_feedback(self.save_btn, "Saved!")

    def show_feedback(self, button, message):
        """Show a temporary color change and log message for button feedback."""
        orig_style = button.styleSheet()
        button.setStyleSheet("background-color: #00f0ff; color: #181825;")
        QTimer.singleShot(200, lambda: button.setStyleSheet(orig_style))
        if hasattr(self.main_window, "log_output"):
            self.main_window.log_output.append(message)

    def reload_config_from_disk(self):
        guild_id = self.guild_select.currentData()
        if guild_id is None:
            return
        config = load_config(guild_id)
        # Only update widgets if not currently editing (i.e., not focused)
        if not self.raw_config.hasFocus():
            self.raw_config.setPlainText(json.dumps(config, indent=2))
        if self.tabs.currentIndex() == 0:
            self.set_widgets_from_config(config) 