import unittest
import sys
from PySide6.QtWidgets import QApplication
from gui.flow_layout import FlowLayout
from gui.permission_chips import PermissionChip, PermissionGrantCard

app = QApplication.instance()
if app is None:
    app = QApplication(sys.argv)

class TestGuiPermissions(unittest.TestCase):
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
        self.assertEqual(layout.count(), 2)

if __name__ == "__main__":
    unittest.main()
