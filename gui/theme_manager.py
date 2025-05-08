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

        tab_text_selected = theme.get('tab_text_selected', theme.get('tab_text', theme['text']))
        tab_text_unselected = theme.get('tab_text_unselected', theme.get('tab_text', theme['text']))
        label_text = theme.get('label_text', theme['text'])
        checkbox_text = theme.get('checkbox_text', theme['text'])
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
                color: {tab_text_selected};
            }}
            QTabBar::tab:!selected {{
                background: {theme['base']};
                color: {tab_text_unselected};
            }}
            QTabWidget::pane {{
                color: {tab_text_selected};
            }}
            QLabel {{
                color: {label_text};
            }}
            QCheckBox {{
                color: {checkbox_text};
            }}
        """
        window.setStyleSheet(stylesheet)

    def available_themes(self):
        """
        Returns a list of available theme names (from the themes directory).
        """
        theme_dir = "themes"
        return [f[:-5] for f in os.listdir(theme_dir) if f.endswith(".json")] 