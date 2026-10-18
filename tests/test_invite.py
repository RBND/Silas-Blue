import unittest
from unittest.mock import MagicMock, patch, PropertyMock
import discord
from bot_core import bot, get_client_id_from_token, get_bot_client_id, get_bot_invite_url


class TestBotInvite(unittest.TestCase):
    def test_get_client_id_from_token_valid(self):
        # MTIzNDU2Nzg5MDEyMzQ1Njc4 in base64 is 123456789012345678
        token = "MTIzNDU2Nzg5MDEyMzQ1Njc4.GzX123.ABC-def_ghijk"
        cid = get_client_id_from_token(token)
        self.assertEqual(cid, 123456789012345678)

    def test_get_client_id_from_token_padding(self):
        # OTk5 in base64 is 999
        token = "OTk5.xxxx.yyyy"
        cid = get_client_id_from_token(token)
        self.assertEqual(cid, 999)

    def test_get_client_id_from_token_invalid(self):
        self.assertIsNone(get_client_id_from_token("not-valid-base64!"))
        self.assertIsNone(get_client_id_from_token(""))

    def test_get_bot_client_id_from_bot_user(self):
        mock_user = MagicMock()
        mock_user.id = 987654321
        with patch.object(type(bot), "user", new_callable=PropertyMock, return_value=mock_user):
            self.assertEqual(get_bot_client_id(), 987654321)

    def test_get_bot_invite_url_standard(self):
        url = get_bot_invite_url(client_id=123456789012345678, administrator=False)
        self.assertIsNotNone(url)
        self.assertIn("client_id=123456789012345678", url)
        self.assertIn("permissions=274878286912", url)
        self.assertIn("bot", url)
        self.assertIn("applications.commands", url)

    def test_get_bot_invite_url_admin(self):
        url = get_bot_invite_url(client_id=123456789012345678, administrator=True)
        self.assertIsNotNone(url)
        self.assertIn("client_id=123456789012345678", url)
        self.assertIn("permissions=8", url)

    def test_get_bot_invite_url_no_id(self):
        with patch("bot_core.get_bot_client_id", return_value=None):
            url = get_bot_invite_url(client_id=None)
            self.assertIsNone(url)

    def test_invite_dialog_gui(self):
        import os
        from PySide6.QtWidgets import QApplication
        from gui.invite_dialog import InviteBotDialog

        os.environ["QT_QPA_PLATFORM"] = "offscreen"
        app = QApplication.instance() or QApplication([])

        dialog = InviteBotDialog()
        self.assertTrue(dialog.rb_standard.isChecked())
        initial_url = dialog.invite_link_input.text()
        self.assertIn("permissions=274878286912", initial_url)

        # Switch to admin
        dialog.rb_admin.setChecked(True)
        admin_url = dialog.invite_link_input.text()
        self.assertIn("permissions=8", admin_url)

        # Change client ID
        dialog.client_id_input.setText("999888777")
        custom_url = dialog.invite_link_input.text()
        self.assertIn("client_id=999888777", custom_url)

        # Test copy link
        dialog.copy_invite_link()
        self.assertEqual(app.clipboard().text(), custom_url)


if __name__ == "__main__":
    unittest.main()
