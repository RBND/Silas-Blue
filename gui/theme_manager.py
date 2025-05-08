import json
import os

class ThemeManager:
    """
    Loads and applies themes to the GUI.
    """

    def __init__(self):
        self.current_theme = {}

    def apply_theme(self, window, theme_file):
        """
        Apply a theme from a JSON file to the given window.
        """
        with open(theme_file, "r") as f:
            theme = json.load(f)
        self.current_theme = theme

        # Build a stylesheet string from the theme
        stylesheet = f"""
            QWidget {{
                background-color: {theme['base']};
                color: {theme['text']};
            }}
            QPushButton {{
                background-color: {theme['button']};
                color: {theme['text']};
                border: 1px solid {theme['accent1']};
            }}
            QPushButton:hover {{
                background-color: {theme['button_hover']};
            }}
            QLineEdit, QTextEdit {{
                background-color: {theme['input_bg']};
                color: {theme['input_fg']};
                border: 1px solid {theme['accent2']};
            }}
            QComboBox {{
                background-color: {theme['input_bg']};
                color: {theme['input_fg']};
            }}
            QScrollBar:vertical, QScrollBar:horizontal {{
                background: {theme['scrollbar']};
            }}
            QTabBar::tab:selected {{
                background: {theme['accent3']};
            }}
            QTabBar::tab:!selected {{
                background: {theme['base']};
            }}
        """
        # Add checkbox and checkmark colors if present in theme
        if 'checkbox_bg' in theme:
            stylesheet += f"\nQCheckBox::indicator {{ background-color: {theme['checkbox_bg']}; border: 1px solid {theme.get('checkbox_border', theme['checkbox_bg'])}; }}"
        if 'checkbox_checkmark' in theme:
            stylesheet += f"\nQCheckBox::indicator:checked {{ background-color: {theme.get('checkbox_checked_bg', theme['checkbox_bg'])}; border: 1px solid {theme.get('checkbox_checked_border', theme.get('checkbox_border', theme['checkbox_bg']))}; }}"
            # Qt does not support checkmark color directly, but we can try to use SVG or custom indicator if needed
        if 'checkbox_hover' in theme:
            stylesheet += f"\nQCheckBox::indicator:hover {{ background-color: {theme['checkbox_hover']}; }}"
        if 'checkbox_disabled_bg' in theme:
            stylesheet += f"\nQCheckBox::indicator:disabled {{ background-color: {theme['checkbox_disabled_bg']}; border: 1px solid {theme.get('checkbox_disabled_border', theme['checkbox_disabled_bg'])}; }}"
        # Note: Qt's QCheckBox does not support checkmark color via stylesheet directly, but we provide the keys for custom widget use
        window.setStyleSheet(stylesheet)

    def get_checkbox_colors(self):
        """Return a dict of checkbox colors from the current theme for use in custom widgets."""
        theme = self.current_theme
        return {
            'bg': theme.get('checkbox_bg', '#232a2e'),
            'border': theme.get('checkbox_border', '#2ec27e'),
            'checked_bg': theme.get('checkbox_checked_bg', '#8ff0a4'),
            'checked_border': theme.get('checkbox_checked_border', '#2ec27e'),
            'checkmark': theme.get('checkbox_checkmark', '#26a269'),
            'hover': theme.get('checkbox_hover', '#8ff0a4'),
            'disabled_bg': theme.get('checkbox_disabled_bg', '#273136'),
            'disabled_border': theme.get('checkbox_disabled_border', '#2d353b'),
            'checkmark_disabled': theme.get('checkbox_checkmark_disabled', '#4b5a5e'),
        }

    def available_themes(self):
        """
        Returns a list of available theme names (from the themes directory).
        """
        theme_dir = "themes"
        return [f[:-5] for f in os.listdir(theme_dir) if f.endswith(".json")] 