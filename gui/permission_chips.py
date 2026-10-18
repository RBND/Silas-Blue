"""
Permission chips and permission grant card components for Silas Blue.
Provides Discord-grade access control management supporting roles and user Snowflake IDs.
"""

import re
from typing import List, Tuple, Optional, Dict
from PySide6.QtWidgets import (
    QFrame, QVBoxLayout, QHBoxLayout, QLabel, QRadioButton,
    QButtonGroup, QLineEdit, QPushButton, QCompleter, QWidget,
    QSizePolicy
)
from PySide6.QtCore import Signal, Qt
from .flow_layout import FlowLayout

SNOWFLAKE_REGEX = re.compile(r"^[1-9]\d{16,19}$")
MENTION_REGEX = re.compile(r"^<@!?([1-9]\d{16,19})>$")
ROLE_MENTION_REGEX = re.compile(r"^<@&([1-9]\d{16,19})>$")


class PermissionChip(QFrame):
    """
    Interactive badge representing an active Role grant or User Snowflake grant.
    """
    removed = Signal(str, str)  # (entity_type: "role"|"user", entity_value: str)

    def __init__(self, name: str, entity_id: str, is_user: bool = False, parent=None):
        super().__init__(parent)
        self.name = name
        self.entity_id = entity_id
        self.is_user = is_user

        self.setObjectName("permChipUser" if is_user else "permChipRole")
        self.setSizePolicy(QSizePolicy.Preferred, QSizePolicy.Fixed)

        layout = QHBoxLayout(self)
        layout.setContentsMargins(8, 3, 6, 3)
        layout.setSpacing(6)

        prefix = "👤 " if is_user else "🛡️ "
        if is_user:
            display_text = f"{prefix}{name}"
            if entity_id and entity_id != name:
                display_text += f" ({entity_id[-4:]})"
            self.setToolTip(f"Discord User ID: {entity_id}")
        else:
            display_text = f"{prefix}{name}"
            self.setToolTip(f"Discord Role: {name}")

        self.label = QLabel(display_text)
        self.label.setStyleSheet("font-size: 11px; font-weight: 600; border: none; background: transparent;")
        layout.addWidget(self.label)

        self.del_btn = QPushButton("✕")
        self.del_btn.setFixedSize(16, 16)
        self.del_btn.setCursor(Qt.PointingHandCursor)
        self.del_btn.setToolTip("Remove grant")
        self.del_btn.setStyleSheet("""
            QPushButton {
                background: transparent;
                border: none;
                font-size: 11px;
                font-weight: bold;
                color: rgba(255, 255, 255, 0.6);
                padding: 0;
            }
            QPushButton:hover {
                color: #ff5252;
                font-weight: 800;
            }
        """)
        self.del_btn.clicked.connect(self._on_remove)
        layout.addWidget(self.del_btn)

    def _on_remove(self):
        entity_type = "user" if self.is_user else "role"
        self.removed.emit(entity_type, self.entity_id or self.name)


class PermissionGrantCard(QFrame):
    """
    Self-contained card for managing a specific permission scope.
    Features:
    - Mode toggle (@everyone vs Restricted)
    - Active grants chip cloud using FlowLayout
    - Smart search and add input supporting roles and user Snowflake IDs
    """
    changed = Signal()

    def __init__(
        self,
        title: str,
        description: str,
        default_everyone: bool = False,
        parent=None
    ):
        super().__init__(parent)
        self.setObjectName("card")
        self.title = title
        self.description = description
        self.default_everyone = default_everyone

        self.roles: List[str] = []
        self.users: List[str] = []
        self.available_roles: List[str] = []
        self.role_id_map: Dict[str, str] = {}  # id_str -> role_name
        self.current_guild = None

        self._setup_ui()

    def _setup_ui(self):
        layout = QVBoxLayout(self)
        layout.setContentsMargins(14, 12, 14, 12)
        layout.setSpacing(10)

        # Header Title & Description
        header_row = QHBoxLayout()
        self.title_label = QLabel(self.title)
        self.title_label.setStyleSheet("font-weight: 700; font-size: 13px; color: #00f0ff;")
        header_row.addWidget(self.title_label)
        header_row.addStretch(1)
        layout.addLayout(header_row)

        desc_label = QLabel(self.description)
        desc_label.setStyleSheet("font-size: 11px; opacity: 0.8;")
        desc_label.setWordWrap(True)
        layout.addWidget(desc_label)

        # Mode Selection
        mode_row = QHBoxLayout()
        self.mode_group = QButtonGroup(self)
        self.rb_everyone = QRadioButton("Everyone (@everyone)")
        self.rb_custom = QRadioButton("Restricted (Admins & Specific Grants)")
        self.mode_group.addButton(self.rb_everyone, 0)
        self.mode_group.addButton(self.rb_custom, 1)
        mode_row.addWidget(self.rb_everyone)
        mode_row.addWidget(self.rb_custom)
        mode_row.addStretch(1)
        layout.addLayout(mode_row)

        self.rb_everyone.toggled.connect(self._on_mode_toggled)

        # Grants Chip Cloud (FlowLayout)
        self.chips_container = QWidget()
        self.flow_layout = FlowLayout(self.chips_container, margin=2, h_spacing=6, v_spacing=6)
        self.chips_container.setMinimumHeight(38)
        layout.addWidget(self.chips_container)

        # Empty state label for custom mode
        self.empty_label = QLabel("No roles or users granted. (Server Administrators always have access)")
        self.empty_label.setStyleSheet("font-size: 11px; font-style: italic; color: rgba(255,255,255,0.45); padding: 4px;")
        self.empty_label.setVisible(False)
        layout.addWidget(self.empty_label)

        # Search / Add Input Row
        self.input_row = QHBoxLayout()
        self.add_input = QLineEdit()
        self.add_input.setPlaceholderText("Enter role name or paste Discord User ID (Snowflake)...")
        self.add_btn = QPushButton("➕ Add Grant")
        self.add_btn.setObjectName("primaryBtn")
        self.add_btn.setCursor(Qt.PointingHandCursor)
        self.add_btn.clicked.connect(self._handle_add)
        self.add_input.returnPressed.connect(self._handle_add)

        self.input_row.addWidget(self.add_input, 1)
        self.input_row.addWidget(self.add_btn)
        layout.addLayout(self.input_row)

        # Validation status label
        self.status_label = QLabel("")
        self.status_label.setStyleSheet("font-size: 11px; font-weight: 500;")
        self.status_label.setVisible(False)
        layout.addWidget(self.status_label)

        if self.default_everyone:
            self.rb_everyone.setChecked(True)
        else:
            self.rb_custom.setChecked(True)

    def _on_mode_toggled(self, everyone_checked: bool):
        is_restricted = not everyone_checked
        self.chips_container.setVisible(is_restricted)
        self.add_input.setEnabled(is_restricted)
        self.add_btn.setEnabled(is_restricted)
        self._update_empty_state()
        self.changed.emit()

    def set_guild(self, guild):
        self.current_guild = guild
        if guild:
            role_names = []
            id_map = {}
            for r in getattr(guild, "roles", []):
                if not r.is_default():
                    role_names.append(r.name)
                    id_map[str(r.id)] = r.name
            self.set_available_roles(role_names, id_map)

    def set_available_roles(self, roles_list: List[str], id_map: Optional[Dict[str, str]] = None):
        self.available_roles = roles_list
        self.role_id_map = id_map or {}
        completer = QCompleter(roles_list, self)
        completer.setCaseSensitivity(Qt.CaseInsensitive)
        completer.setFilterMode(Qt.MatchContains)
        self.add_input.setCompleter(completer)

    def set_data(self, roles: List[str], users: List[str]):
        self.roles = [str(r).strip() for r in roles if str(r).strip()]
        self.users = [str(u).strip() for u in users if str(u).strip() and SNOWFLAKE_REGEX.match(str(u).strip())]

        is_everyone = any(r.lower() in ("everyone", "@everyone") for r in self.roles)
        self.rb_everyone.blockSignals(True)
        self.rb_custom.blockSignals(True)
        if is_everyone:
            self.rb_everyone.setChecked(True)
        else:
            self.rb_custom.setChecked(True)
        self.rb_everyone.blockSignals(False)
        self.rb_custom.blockSignals(False)

        self._on_mode_toggled(is_everyone)
        self._refresh_chips()

    def get_data(self) -> Tuple[List[str], List[str]]:
        if self.rb_everyone.isChecked():
            return ["everyone"], []
        filtered_roles = [r for r in self.roles if r.lower() not in ("everyone", "@everyone")]
        return filtered_roles, list(self.users)

    def _resolve_user_name(self, uid: str) -> str:
        if self.current_guild:
            try:
                member = self.current_guild.get_member(int(uid))
                if member:
                    return getattr(member, "display_name", str(member))
            except Exception:
                pass
        return f"User {uid[-4:]}"

    def _refresh_chips(self):
        self.chips_container.setUpdatesEnabled(False)
        try:
            while self.flow_layout.count():
                item = self.flow_layout.takeAt(0)
                w = item.widget()
                if w:
                    w.deleteLater()

            # Render role chips
            for role in self.roles:
                if role.lower() in ("everyone", "@everyone"):
                    continue
                display_name = self.role_id_map.get(role, role)
                chip = PermissionChip(name=display_name, entity_id=role, is_user=False)
                chip.removed.connect(self._handle_remove)
                self.flow_layout.addWidget(chip)

            # Render user chips
            for uid in self.users:
                name = self._resolve_user_name(uid)
                chip = PermissionChip(name=name, entity_id=uid, is_user=True)
                chip.removed.connect(self._handle_remove)
                self.flow_layout.addWidget(chip)
        finally:
            self.chips_container.setUpdatesEnabled(True)

        self._update_empty_state()

    def _update_empty_state(self):
        has_grants = bool(
            [r for r in self.roles if r.lower() not in ("everyone", "@everyone")] or self.users
        )
        show_empty = (not self.rb_everyone.isChecked()) and (not has_grants)
        self.empty_label.setVisible(show_empty)

    def _handle_add(self):
        text = self.add_input.text().strip()
        if not text:
            return

        self.status_label.setVisible(False)

        # 1. Check for user mention <@!123456789...> or <@123456789...>
        user_mention_match = MENTION_REGEX.match(text)
        if user_mention_match:
            uid = user_mention_match.group(1)
            self._add_user(uid)
            return

        # 2. Check for role mention <@&123456789...>
        role_mention_match = ROLE_MENTION_REGEX.match(text)
        if role_mention_match:
            rid = role_mention_match.group(1)
            self._add_role(rid)
            return

        # 3. Check if raw Discord Snowflake ID (17-20 digits)
        if SNOWFLAKE_REGEX.match(text):
            # Check if it matches an existing server role ID
            if text in self.role_id_map:
                self._add_role(text)
            else:
                self._add_user(text)
            return

        # 4. Otherwise treat as Role Name
        matched_role = None
        for r_name in self.available_roles:
            if r_name.lower() == text.lower():
                matched_role = r_name
                break

        if matched_role:
            self._add_role(matched_role)
        else:
            # Let the user add custom role name or warn
            if len(text) <= 100:
                self._add_role(text)
            else:
                self._show_error("Role name too long or invalid.")

    def _add_user(self, uid: str):
        if uid in self.users:
            self._show_info(f"User {uid[-4:]} is already granted.")
            return
        self.users.append(uid)
        self._refresh_chips()
        self.add_input.clear()
        self.changed.emit()

    def _add_role(self, role: str):
        if any(r.lower() == role.lower() for r in self.roles):
            self._show_info(f"Role '{role}' is already granted.")
            return
        self.roles.append(role)
        self._refresh_chips()
        self.add_input.clear()
        self.changed.emit()

    def _handle_remove(self, entity_type: str, val: str):
        if entity_type == "user":
            self.users = [u for u in self.users if u != val]
        else:
            self.roles = [r for r in self.roles if r != val]
        self._refresh_chips()
        self.changed.emit()

    def _show_info(self, msg: str):
        self.status_label.setText(msg)
        self.status_label.setStyleSheet("color: #00f0ff; font-size: 11px;")
        self.status_label.setVisible(True)

    def _show_error(self, msg: str):
        self.status_label.setText(msg)
        self.status_label.setStyleSheet("color: #ff5252; font-size: 11px;")
        self.status_label.setVisible(True)
