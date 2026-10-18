"""
InviteBotDialog - GUI Dialog for inviting Silas Blue to Discord servers.
Provides one-click browser authorization, clipboard copy, and permission customization.
"""

import logging
import webbrowser
from typing import Optional

from PySide6.QtWidgets import (
    QDialog, QVBoxLayout, QHBoxLayout, QLabel, QLineEdit,
    QPushButton, QRadioButton, QButtonGroup, QGroupBox,
    QApplication
)
from PySide6.QtCore import Qt, QUrl, QTimer
from PySide6.QtGui import QDesktopServices

from bot_core import get_bot_client_id, get_bot_invite_url

logger = logging.getLogger("silasblue")


class InviteBotDialog(QDialog):
    """
    Dialog providing invite URL generation and one-click authorization
    for adding Silas Blue to any Discord server.
    """

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Invite Silas Blue to a Discord Server")
        self.setMinimumWidth(560)
        self.setModal(True)

        layout = QVBoxLayout()
        self.setLayout(layout)

        # Header / Explanatory Banner
        header_title = QLabel("<h2>Invite Silas Blue to Your Discord Server</h2>")
        header_title.setTextFormat(Qt.RichText)
        header_desc = QLabel(
            "Add Silas Blue to any server where you have <b>Manage Server</b> or <b>Administrator</b> permissions.<br>"
            "Click <b>Open in Browser</b> to open Discord's official authorization page and select your server."
        )
        header_desc.setTextFormat(Qt.RichText)
        header_desc.setWordWrap(True)
        layout.addWidget(header_title)
        layout.addWidget(header_desc)
        layout.addSpacing(8)

        # Client ID row
        client_id_group = QGroupBox("Bot Application Details")
        client_id_layout = QVBoxLayout()
        client_id_group.setLayout(client_id_layout)

        cid_row = QHBoxLayout()
        cid_row.addWidget(QLabel("Client ID:"))
        self.client_id_input = QLineEdit()
        self.client_id_input.setPlaceholderText("Discord Bot / Application ID (e.g. 1361883752363720816)")

        detected_cid = get_bot_client_id()
        if detected_cid:
            self.client_id_input.setText(str(detected_cid))
            self.cid_status_label = QLabel("<span style='color: #00ff88;'>✓ Detected from active bot configuration</span>")
        else:
            self.cid_status_label = QLabel("<span style='color: #ffaa00;'>⚠ Enter Application ID manually or configure bot token</span>")

        self.cid_status_label.setTextFormat(Qt.RichText)
        cid_row.addWidget(self.client_id_input, 2)
        client_id_layout.addLayout(cid_row)
        client_id_layout.addWidget(self.cid_status_label)
        layout.addWidget(client_id_group)
        layout.addSpacing(4)

        # Permission Level group
        perm_group = QGroupBox("Bot Permissions")
        perm_layout = QVBoxLayout()
        perm_group.setLayout(perm_layout)

        self.perm_btn_group = QButtonGroup(self)
        self.rb_standard = QRadioButton("Standard Permissions (Recommended)")
        self.rb_standard.setToolTip("View Channels, Send Messages, Threads, Embed Links, Attach Files, Read History, Reactions, Slash Commands")
        self.rb_admin = QRadioButton("Administrator (Full Access)")
        self.rb_admin.setToolTip("Grants all permissions. Ideal for personal / private servers.")

        self.rb_standard.setChecked(True)
        self.perm_btn_group.addButton(self.rb_standard, 1)
        self.perm_btn_group.addButton(self.rb_admin, 2)

        standard_desc = QLabel("<small style='color: #8888aa;'>• Includes: Read & Send Messages, Threads, Embeds, Attachments, Message History, Reactions, Emojis</small>")
        standard_desc.setTextFormat(Qt.RichText)

        perm_layout.addWidget(self.rb_standard)
        perm_layout.addWidget(standard_desc)
        perm_layout.addWidget(self.rb_admin)
        layout.addWidget(perm_group)
        layout.addSpacing(4)

        # Generated Invite Link group
        link_group = QGroupBox("Discord Authorization Link")
        link_layout = QVBoxLayout()
        link_group.setLayout(link_layout)

        self.invite_link_input = QLineEdit()
        self.invite_link_input.setReadOnly(True)
        link_layout.addWidget(self.invite_link_input)

        self.action_status_label = QLabel("")
        self.action_status_label.setTextFormat(Qt.RichText)
        link_layout.addWidget(self.action_status_label)

        layout.addWidget(link_group)
        layout.addSpacing(8)

        # Action Buttons
        btn_layout = QHBoxLayout()
        self.open_browser_btn = QPushButton("🌐 Open in Browser")
        self.open_browser_btn.setStyleSheet("font-weight: bold; padding: 8px 16px;")

        self.copy_link_btn = QPushButton("📋 Copy Invite Link")
        self.copy_link_btn.setStyleSheet("padding: 8px 16px;")

        self.close_btn = QPushButton("Close")
        self.close_btn.setStyleSheet("padding: 8px 16px;")

        btn_layout.addWidget(self.open_browser_btn, 2)
        btn_layout.addWidget(self.copy_link_btn, 2)
        btn_layout.addStretch(1)
        btn_layout.addWidget(self.close_btn, 1)
        layout.addLayout(btn_layout)

        # Signals
        self.client_id_input.textChanged.connect(self.update_invite_url)
        self.rb_standard.toggled.connect(self.update_invite_url)
        self.rb_admin.toggled.connect(self.update_invite_url)
        self.open_browser_btn.clicked.connect(self.open_in_browser)
        self.copy_link_btn.clicked.connect(self.copy_invite_link)
        self.close_btn.clicked.connect(self.accept)

        # Initialize URL
        self.update_invite_url()

    def get_current_client_id(self) -> Optional[int]:
        txt = self.client_id_input.text().strip()
        if txt.isdigit():
            return int(txt)
        return get_bot_client_id()

    def update_invite_url(self):
        cid = self.get_current_client_id()
        is_admin = self.rb_admin.isChecked()

        if cid:
            url = get_bot_invite_url(client_id=cid, administrator=is_admin)
            self.invite_link_input.setText(url or "")
            self.open_browser_btn.setEnabled(True)
            self.copy_link_btn.setEnabled(True)
        else:
            self.invite_link_input.setText("")
            self.open_browser_btn.setEnabled(False)
            self.copy_link_btn.setEnabled(False)
            self.action_status_label.setText("<span style='color: #ff6666;'>Please provide a valid numeric Bot Client ID above.</span>")

    def open_in_browser(self):
        url = self.invite_link_input.text().strip()
        if not url:
            return

        try:
            # Try QDesktopServices first, fallback to webbrowser
            opened = QDesktopServices.openUrl(QUrl(url))
            if not opened:
                webbrowser.open(url)
            self.action_status_label.setText(
                "<span style='color: #00ff88; font-weight: bold;'>✓ Opened in your browser! Select your Discord server and click Authorize.</span>"
            )
            if self.parent() and hasattr(self.parent(), "system_log_output"):
                self.parent().system_log_output.append(f"[INVITE] Opened Discord bot authorization in default browser: {url}")
        except Exception as e:
            logger.error(f"Failed to open browser for invite URL: {e}")
            webbrowser.open(url)
            self.action_status_label.setText("<span style='color: #00ff88;'>✓ Opened in browser via standard handler.</span>")

    def copy_invite_link(self):
        url = self.invite_link_input.text().strip()
        if not url:
            return
        clipboard = QApplication.clipboard()
        clipboard.setText(url)

        orig_text = self.copy_link_btn.text()
        self.copy_link_btn.setText("✓ Copied!")
        self.action_status_label.setText("<span style='color: #00ff88; font-weight: bold;'>✓ Invite link copied to clipboard!</span>")
        QTimer.singleShot(1500, lambda: self.copy_link_btn.setText(orig_text))
