import unittest
import sys
from PySide6.QtWidgets import QApplication
from gui.flow_layout import FlowLayout
from gui.permission_chips import PermissionChip, PermissionGrantCard, ChannelChip, ChannelSelectCard

app = QApplication.instance()
if app is None:
    app = QApplication(sys.argv)

class TestGuiPermissions(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from utils import load_app_config
        cls._original_app_cfg = load_app_config()

    @classmethod
    def tearDownClass(cls):
        from utils import save_app_config
        if hasattr(cls, "_original_app_cfg"):
            save_app_config(cls._original_app_cfg)

    def test_permission_chip_signals(self):
        chip = PermissionChip(name="Moderator", entity_id="123456", is_user=False)
        self.assertEqual(chip.name, "Moderator")
        self.assertEqual(chip.entity_id, "123456")
        self.assertFalse(chip.is_user)

        removed_events = []
        chip.removed.connect(lambda t, v: removed_events.append((t, v)))
        chip._on_remove()
        self.assertEqual(removed_events, [("role", "123456")])

    def test_permission_grant_card_everyone_mode(self):
        card = PermissionGrantCard("Test", "Desc", default_everyone=True)
        self.assertTrue(card.rb_everyone.isChecked())
        roles, users = card.get_data()
        self.assertEqual(roles, ["everyone"])
        self.assertEqual(users, [])

    def test_permission_grant_card_custom_mode_and_add(self):
        card = PermissionGrantCard("Test", "Desc", default_everyone=False)
        card.set_available_roles(["Admin", "Moderator"])

        # Add a role by name
        card.add_input.setText("Admin")
        card._handle_add()
        roles, users = card.get_data()
        self.assertIn("Admin", roles)

        # Add a user by 18-digit Snowflake ID
        card.add_input.setText("295767175426473984")
        card._handle_add()
        roles, users = card.get_data()
        self.assertIn("295767175426473984", users)

        # Add user by Discord mention syntax
        card.add_input.setText("<@!111222333444555666>")
        card._handle_add()
        roles, users = card.get_data()
        self.assertIn("111222333444555666", users)

        # Remove grant
        card._handle_remove("user", "111222333444555666")
        roles, users = card.get_data()
        self.assertNotIn("111222333444555666", users)

    def test_flow_layout_addition(self):
        from PySide6.QtWidgets import QWidget, QLabel
        parent = QWidget()
        layout = FlowLayout(parent)
        lbl1 = QLabel("Chip 1")
        lbl2 = QLabel("Chip 2")
        layout.addWidget(lbl1)
        layout.addWidget(lbl2)
    def test_all_themes_apply_cleanly(self):
        import glob
        from PySide6.QtWidgets import QWidget
        from gui.theme_manager import ThemeManager
        tm = ThemeManager()
        w = QWidget()
        themes = glob.glob("themes/*.json")
        self.assertGreater(len(themes), 0)
        for th in themes:
            tm.apply_theme(w, th)
            self.assertTrue(bool(w.styleSheet()))

    def test_remote_models_combobox_integration(self):
        from gui.main_window import MainWindow
        from PySide6.QtWidgets import QComboBox
        from unittest.mock import MagicMock
        win = MainWindow(start_bot=MagicMock(), stop_bot=MagicMock(), restart_bot=MagicMock())
        try:
            self.assertIsInstance(win.remote_model_combo, QComboBox)
            self.assertTrue(win.remote_model_combo.isEditable())

            # Test preset change populates models in dropdown
            win.remote_preset_combo.setCurrentText("Groq")
            items = [win.remote_model_combo.itemText(i) for i in range(win.remote_model_combo.count())]
            self.assertIn("llama-3.3-70b-versatile", items)
            self.assertEqual(win.remote_model_combo.currentText(), "llama-3.3-70b-versatile")

            # Test fetched models callback loads into dropdown
            fetched = ["mimo-v2.6-pro", "mimo-v2.6-flash", "mimo-custom-experimental"]
            win.on_remote_models_fetched(fetched)
            combo_items = [win.remote_model_combo.itemText(i) for i in range(win.remote_model_combo.count())]
            self.assertEqual(combo_items, fetched)
            self.assertEqual(win.remote_model_combo.currentText(), "mimo-v2.6-pro")

            # Test user can select another model from the dropdown
            win.remote_model_combo.setCurrentText("mimo-v2.6-flash")
            self.assertEqual(win.app_cfg["remote_model"], "mimo-v2.6-flash")
        finally:
            win.close()

    def test_server_config_subpages_and_navigation(self):
        from gui.main_window import MainWindow
        from PySide6.QtWidgets import QSizePolicy
        from unittest.mock import MagicMock
        win = MainWindow(start_bot=MagicMock(), stop_bot=MagicMock(), restart_bot=MagicMock())
        try:
            # 1. Verify 5 sub-nav buttons exist
            self.assertEqual(len(win.server_sub_buttons), 5)
            scp = win.server_config_tab
            self.assertEqual(scp.subpage_stack.count(), 5)

            # Mock some guilds in guild_select
            scp.guild_select.addItem("Guild Alpha", 111111)
            scp.guild_select.addItem("Guild Beta", 222222)
            scp.guild_select.setCurrentIndex(1)
            initial_guild = scp.guild_select.currentIndex()

            # 2. Navigate to Subpage 2 (Prompt & Output) via main window subnav
            win.switch_to_server_config(2)
            self.assertEqual(win.content_stack.currentIndex(), 2)
            self.assertEqual(scp.subpage_stack.currentIndex(), 2)
            self.assertEqual(scp._current_subpage_index, 2)
            # Guild selection must remain unchanged!
            self.assertEqual(scp.guild_select.currentIndex(), initial_guild)

            # 3. Navigate to Subpage 1 (Token Settings)
            win.switch_to_server_config(1)
            self.assertEqual(scp.subpage_stack.currentIndex(), 1)
            self.assertEqual(scp.guild_select.currentIndex(), initial_guild)

            # 4. Navigate to Subpage 3 (Forum Auto-Reply)
            win.switch_to_server_config(3)
            self.assertEqual(scp.subpage_stack.currentIndex(), 3)
            self.assertEqual(scp.guild_select.currentIndex(), initial_guild)

            # 5. Check Prompt page layout properties (expanding textbox, no scroll area)
            self.assertEqual(scp.server_system_prompt_input.sizePolicy().verticalPolicy(), QSizePolicy.Expanding)
            # The prompt page itself is a direct QWidget layout, not a QScrollArea
            from PySide6.QtWidgets import QScrollArea
            self.assertFalse(isinstance(scp.page_prompt_pagination, QScrollArea))
        finally:
            win.close()

    def test_server_config_multi_subpage_save_integration(self):
        from gui.main_window import MainWindow
        from unittest.mock import MagicMock
        win = MainWindow(start_bot=MagicMock(), stop_bot=MagicMock(), restart_bot=MagicMock())
        try:
            scp = win.server_config_tab
            # Populate fields on different subpages:
            # Subpage 2: Prompt
            scp.server_system_prompt_input.setPlainText("Custom system persona test")
            scp.pagination_enabled.setChecked(True)
            scp.pagination_max_chars.setValue(1250)

            # Subpage 1: Token Quotas
            scp.quotas_enabled.setChecked(True)
            scp.quotas_frequency.setCurrentText("weekly")
            scp.quotas_everyone_limit.setValue(25000)
            scp.quotas_custom_limits.setPlainText("VIP = 80000\nMod = -1")

            # Subpage 3: Forum
            scp.forum_auto_reply_enabled.setChecked(True)
            scp.forum_include_post_title.setChecked(True)
            scp.forum_channel_card.set_channels(["111222333444555666", "777888999000111222"])
            scp.forum_auto_reply_tags.setText("help, questions")
            scp.forum_system_prompt.setPlainText("Forum persona test")

            # Verify get_config_from_widgets gathers everything together
            cfg = scp.get_config_from_widgets()
            self.assertEqual(cfg["system_prompt"], "Custom system persona test")
            self.assertEqual(cfg["pagination_max_chars"], 1250)
            self.assertTrue(cfg["token_quotas"]["enabled"])
            self.assertEqual(cfg["token_quotas"]["reset_frequency"], "weekly")
            self.assertEqual(cfg["token_quotas"]["role_limits"]["everyone"], 25000)
            self.assertEqual(cfg["token_quotas"]["role_limits"]["VIP"], 80000)
            self.assertEqual(cfg["token_quotas"]["role_limits"]["Mod"], -1)
            self.assertTrue(cfg["forum_auto_reply_enabled"])
            self.assertEqual(cfg["forum_monitored_channels"], ["111222333444555666", "777888999000111222"])
            self.assertEqual(cfg["forum_auto_reply_tags"], ["help", "questions"])
            self.assertEqual(cfg["forum_system_prompt"], "Forum persona test")
        finally:
            win.close()

    def test_channel_select_card_functionality(self):
        card = ChannelSelectCard()
        # Initial empty state
        self.assertFalse(card.empty_label.isHidden())
        self.assertEqual(card.get_channels(), [])

        # Set fake available channels
        card.available_channels = [("support-forum", "123456789012345678"), ("general-chat", "987654321098765432")]
        card.channel_id_to_name = {"123456789012345678": "support-forum", "987654321098765432": "general-chat"}

        # 1. Add by channel name with hash
        card.add_input.setText("#support-forum")
        card._on_add()
        self.assertEqual(card.get_channels(), ["123456789012345678"])
        self.assertTrue(card.empty_label.isHidden())

        # 2. Add by snowflake ID
        card.add_input.setText("987654321098765432")
        card._on_add()
        self.assertEqual(card.get_channels(), ["123456789012345678", "987654321098765432"])

        # 3. Add by channel mention
        card.add_input.setText("<#555666777888999000>")
        card._on_add()
        self.assertIn("555666777888999000", card.get_channels())

        # 4. Remove channel
        card._on_remove("123456789012345678")
        self.assertNotIn("123456789012345678", card.get_channels())

    def test_channel_chip_signal(self):
        chip = ChannelChip("bugs-forum", "123456789012345678")
        removed = []
        chip.removed.connect(lambda cid: removed.append(cid))
        chip.del_btn.click()
        self.assertEqual(removed, ["123456789012345678"])

if __name__ == "__main__":
    unittest.main()
