import json
import os
import re

class ThemeManager:
    """
    Loads and applies modern Discord-grade themes to the Silas Blue GUI.
    """

    def __init__(self):
        self.current_theme = {}

    def _hex_to_rgba(self, hex_color: str, alpha: float = 1.0) -> str:
        """Convert a #RRGGBB or #RGB hex string to rgba(r, g, b, a)."""
        hex_color = hex_color.lstrip('#')
        if len(hex_color) == 3:
            hex_color = ''.join(c * 2 for c in hex_color)
        try:
            r = int(hex_color[0:2], 16)
            g = int(hex_color[2:4], 16)
            b = int(hex_color[4:6], 16)
            return f"rgba({r}, {g}, {b}, {alpha:.2f})"
        except Exception:
            return hex_color

    def _is_light_theme(self, hex_color: str) -> bool:
        """Determines if a hex color has light background luminance (>0.5) for WCAG contrast."""
        hex_color = hex_color.lstrip('#')
        if len(hex_color) == 3:
            hex_color = ''.join(c * 2 for c in hex_color)
        try:
            r = int(hex_color[0:2], 16)
            g = int(hex_color[2:4], 16)
            b = int(hex_color[4:6], 16)
            return ((0.299 * r + 0.587 * g + 0.114 * b) / 255) > 0.5
        except Exception:
            return False

    def apply_theme(self, window, theme_file):
        """
        Apply a theme from a JSON file to the given window with modern cards,
        sidebar styling, badges, and responsive controls.
        """
        with open(theme_file, "r", encoding="utf-8") as f:
            theme = json.load(f)
        self.current_theme = theme

        base = theme.get('base', '#181825')
        accent1 = theme.get('accent1', '#5865f2')
        accent2 = theme.get('accent2', '#00f0ff')
        accent3 = theme.get('accent3', '#a259ff')
        text = theme.get('text', '#f8f8f2')
        button = theme.get('button', '#282a36')
        button_hover = theme.get('button_hover', '#5865f2')
        scrollbar = theme.get('scrollbar', '#282a36')
        input_bg = theme.get('input_bg', '#232136')
        input_fg = theme.get('input_fg', '#f8f8f2')
        card_bg = theme.get('input_bg', '#232136')
        sidebar_bg = theme.get('titlebar', base)

        # Subtle card border using semi-transparent accent or soft line
        card_border = self._hex_to_rgba(accent1, 0.28)
        nav_hover = self._hex_to_rgba(button_hover, 0.18)
        nav_active_bg = self._hex_to_rgba(accent1, 0.22)
        accent_focus = self._hex_to_rgba(accent2, 0.8)

        # Dynamic contrast palette for status pills (WCAG 2.1 AA >= 4.5:1)
        is_light = self._is_light_theme(base)
        if is_light:
            pill_online_fg = "#1a7f37"
            pill_online_bg = "rgba(26, 127, 55, 0.15)"
            pill_online_border = "rgba(26, 127, 55, 0.45)"
            pill_offline_fg = "#cf222e"
            pill_offline_bg = "rgba(207, 34, 46, 0.15)"
            pill_offline_border = "rgba(207, 34, 46, 0.45)"
            pill_warn_fg = "#9a6700"
            pill_warn_bg = "rgba(154, 103, 0, 0.15)"
            pill_warn_border = "rgba(154, 103, 0, 0.45)"
        else:
            pill_online_fg = "#2ecc71"
            pill_online_bg = "rgba(46, 204, 113, 0.20)"
            pill_online_border = "rgba(46, 204, 113, 0.50)"
            pill_offline_fg = "#ff5252"
            pill_offline_bg = "rgba(255, 82, 82, 0.20)"
            pill_offline_border = "rgba(255, 82, 82, 0.50)"
            pill_warn_fg = "#ffb142"
            pill_warn_bg = "rgba(255, 177, 66, 0.20)"
            pill_warn_border = "rgba(255, 177, 66, 0.50)"

        stylesheet = f"""
            /* Global Application Window */
            QWidget {{
                background-color: {base};
                color: {text};
                font-family: 'Segoe UI', -apple-system, BlinkMacSystemFont, Roboto, sans-serif;
                font-size: 13px;
                selection-background-color: {accent1};
                selection-color: #ffffff;
            }}

            QMainWindow {{
                background-color: {base};
            }}

            /* Sidebar Container & Navigation */
            #sidebar {{
                background-color: {sidebar_bg};
                border-right: 1px solid {self._hex_to_rgba(accent1, 0.20)};
                min-width: 210px;
                max-width: 230px;
            }}

            #sidebarHeader {{
                padding: 14px 12px;
                border-bottom: 1px solid {self._hex_to_rgba(accent1, 0.15)};
            }}

            #appTitle {{
                font-size: 17px;
                font-weight: 700;
                color: {text};
                letter-spacing: 0.5px;
            }}

            #appSubtitle {{
                font-size: 11px;
                color: {self._hex_to_rgba(text, 0.65)};
            }}

            #navButton {{
                background-color: transparent;
                color: {self._hex_to_rgba(text, 0.85)};
                border: none;
                border-radius: 8px;
                padding: 10px 14px;
                text-align: left;
                font-size: 13px;
                font-weight: 500;
                margin: 2px 8px;
            }}

            #navButton:hover {{
                background-color: {nav_hover};
                color: {text};
            }}

            #navButton:checked, #navButtonActive {{
                background-color: {nav_active_bg};
                color: #ffffff;
                font-weight: 600;
                border-left: 3px solid {accent1};
            }}

            /* Modern Card Containers */
            QFrame.card, QWidget#card, QFrame#card {{
                background-color: {card_bg};
                border: 1px solid {card_border};
                border-radius: 10px;
                padding: 12px;
            }}

            QFrame.kpiCard, QWidget#kpiCard {{
                background-color: {card_bg};
                border: 1px solid {card_border};
                border-radius: 10px;
                padding: 12px 14px;
            }}

            QFrame.kpiCard:hover {{
                border: 1px solid {accent1};
            }}

            #kpiTitle {{
                font-size: 11px;
                font-weight: 600;
                text-transform: uppercase;
                letter-spacing: 0.8px;
                color: {self._hex_to_rgba(text, 0.65)};
            }}

            #kpiValue {{
                font-size: 18px;
                font-weight: 700;
                color: {text};
                margin-top: 2px;
            }}

            #kpiSubtext {{
                font-size: 11px;
                color: {self._hex_to_rgba(text, 0.55)};
                margin-top: 1px;
            }}

            /* GroupBoxes */
            QGroupBox {{
                background-color: {card_bg};
                border: 1px solid {card_border};
                border-radius: 10px;
                margin-top: 24px;
                padding-top: 16px;
                font-weight: 600;
                font-size: 13px;
                color: {text};
            }}

            QGroupBox::title {{
                subcontrol-origin: margin;
                subcontrol-position: top left;
                left: 14px;
                padding: 0 6px;
                color: {accent2};
            }}

            /* Standard Buttons */
            QPushButton {{
                background-color: {button};
                color: {text};
                border: 1px solid {self._hex_to_rgba(accent1, 0.40)};
                border-radius: 6px;
                padding: 7px 14px;
                font-size: 12px;
                font-weight: 500;
            }}

            QPushButton:hover {{
                background-color: {button_hover};
                color: #ffffff;
                border: 1px solid {accent1};
            }}

            QPushButton:pressed {{
                background-color: {self._hex_to_rgba(accent1, 0.70)};
            }}

            QPushButton:disabled {{
                background-color: {self._hex_to_rgba(button, 0.5)};
                color: {self._hex_to_rgba(text, 0.4)};
                border: 1px solid transparent;
            }}

            QPushButton#primaryBtn {{
                background-color: {accent1};
                color: #ffffff;
                font-weight: 600;
                border: 1px solid {accent1};
            }}

            QPushButton#primaryBtn:hover {{
                background-color: {accent3};
                border: 1px solid {accent3};
            }}

            QPushButton#successBtn {{
                background-color: #238636;
                color: #ffffff;
                border: 1px solid #2ea043;
                font-weight: 600;
            }}

            QPushButton#successBtn:hover {{
                background-color: #2ea043;
            }}

            QPushButton#dangerBtn {{
                background-color: #da3633;
                color: #ffffff;
                border: 1px solid #f85149;
                font-weight: 600;
            }}

            QPushButton#dangerBtn:hover {{
                background-color: #f85149;
            }}

            /* Inputs & Text Fields */
            QLineEdit, QTextEdit, QPlainTextEdit, QSpinBox {{
                background-color: {input_bg};
                color: {input_fg};
                border: 1px solid {self._hex_to_rgba(accent2, 0.35)};
                border-radius: 6px;
                padding: 6px 10px;
                font-size: 12px;
            }}

            QLineEdit:focus, QTextEdit:focus, QPlainTextEdit:focus, QSpinBox:focus {{
                border: 1px solid {accent_focus};
                background-color: {self._hex_to_rgba(input_bg, 0.95)};
            }}

            /* ComboBox */
            QComboBox {{
                background-color: {input_bg};
                color: {input_fg};
                border: 1px solid {self._hex_to_rgba(accent2, 0.35)};
                border-radius: 6px;
                padding: 5px 10px;
                min-height: 20px;
            }}

            QComboBox:focus {{
                border: 1px solid {accent_focus};
            }}

            QComboBox::drop-down {{
                subcontrol-origin: padding;
                subcontrol-position: top right;
                width: 22px;
                border-left-width: 0px;
                border-top-right-radius: 6px;
                border-bottom-right-radius: 6px;
            }}

            QComboBox QAbstractItemView {{
                background-color: {card_bg};
                color: {input_fg};
                border: 1px solid {accent1};
                selection-background-color: {accent1};
                selection-color: #ffffff;
                border-radius: 6px;
                padding: 4px;
            }}

            /* Lists & Views */
            QListWidget {{
                background-color: {input_bg};
                color: {input_fg};
                border: 1px solid {self._hex_to_rgba(accent2, 0.30)};
                border-radius: 6px;
                padding: 4px;
            }}

            QListWidget::item {{
                padding: 6px 8px;
                border-radius: 4px;
                margin: 1px 0px;
            }}

            QListWidget::item:hover {{
                background-color: {nav_hover};
            }}

            QListWidget::item:selected {{
                background-color: {accent1};
                color: #ffffff;
            }}

            /* ScrollBars */
            QScrollBar:vertical {{
                background: transparent;
                width: 8px;
                margin: 0px;
            }}

            QScrollBar::handle:vertical {{
                background: {scrollbar};
                min-height: 24px;
                border-radius: 4px;
            }}

            QScrollBar::handle:vertical:hover {{
                background: {accent1};
            }}

            QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {{
                height: 0px;
            }}

            QScrollBar:horizontal {{
                background: transparent;
                height: 8px;
                margin: 0px;
            }}

            QScrollBar::handle:horizontal {{
                background: {scrollbar};
                min-width: 24px;
                border-radius: 4px;
            }}

            QScrollBar::handle:horizontal:hover {{
                background: {accent1};
            }}

            QScrollBar::add-line:horizontal, QScrollBar::sub-line:horizontal {{
                width: 0px;
            }}

            /* Progress Bar */
            QProgressBar {{
                background-color: {input_bg};
                border: 1px solid {self._hex_to_rgba(accent1, 0.3)};
                border-radius: 5px;
                text-align: center;
                color: {text};
                font-size: 11px;
                font-weight: 600;
                min-height: 14px;
            }}

            QProgressBar::chunk {{
                background-color: {accent2};
                border-radius: 4px;
            }}

            /* Status Pills with Dynamic WCAG AA Contrast */
            QLabel#statusPillOnline {{
                background-color: {pill_online_bg};
                color: {pill_online_fg};
                border: 1px solid {pill_online_border};
                border-radius: 10px;
                padding: 3px 9px;
                font-size: 11px;
                font-weight: 700;
            }}

            QLabel#statusPillOffline {{
                background-color: {pill_offline_bg};
                color: {pill_offline_fg};
                border: 1px solid {pill_offline_border};
                border-radius: 10px;
                padding: 3px 9px;
                font-size: 11px;
                font-weight: 700;
            }}

            QLabel#statusPillWarning {{
                background-color: {pill_warn_bg};
                color: {pill_warn_fg};
                border: 1px solid {pill_warn_border};
                border-radius: 10px;
                padding: 3px 9px;
                font-size: 11px;
                font-weight: 700;
            }}

            /* Permission Grants Chips */
            QFrame#permChipRole {{
                background-color: {self._hex_to_rgba(accent1, 0.22)};
                border: 1px solid {self._hex_to_rgba(accent1, 0.55)};
                border-radius: 6px;
            }}

            QFrame#permChipRole:hover {{
                border-color: {accent1};
            }}

            QFrame#permChipUser {{
                background-color: {self._hex_to_rgba(accent2, 0.20)};
                border: 1px solid {self._hex_to_rgba(accent2, 0.55)};
                border-radius: 6px;
            }}

            QFrame#permChipUser:hover {{
                border-color: {accent2};
            }}

            /* Accessibility Focus Rings */
            QPushButton:focus, QComboBox:focus, QSpinBox:focus {{
                border: 1px solid {accent_focus};
            }}

            /* Tabs (when embedded in child widgets) */
            QTabBar::tab {{
                background: {card_bg};
                color: {self._hex_to_rgba(text, 0.75)};
                padding: 8px 16px;
                margin-right: 4px;
                border-top-left-radius: 6px;
                border-top-right-radius: 6px;
                font-weight: 500;
            }}

            QTabBar::tab:selected {{
                background: {base};
                color: {accent2};
                border-bottom: 2px solid {accent2};
                font-weight: 600;
            }}

            QTabWidget::pane {{
                border: 1px solid {card_border};
                border-radius: 6px;
                top: -1px;
            }}
        """

        # Append checkbox indicators if provided in theme
        if 'checkbox_bg' in theme:
            stylesheet += f"\nQCheckBox::indicator {{ background-color: {theme['checkbox_bg']}; border: 1px solid {theme.get('checkbox_border', theme['checkbox_bg'])}; border-radius: 3px; width: 14px; height: 14px; }}"
        if 'checkbox_checkmark' in theme:
            stylesheet += f"\nQCheckBox::indicator:checked {{ background-color: {theme.get('checkbox_checked_bg', theme['checkbox_bg'])}; border: 1px solid {theme.get('checkbox_checked_border', theme.get('checkbox_border', theme['checkbox_bg']))}; }}"
        if 'checkbox_hover' in theme:
            stylesheet += f"\nQCheckBox::indicator:hover {{ border-color: {theme['checkbox_hover']}; }}"

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
        if not os.path.isdir(theme_dir):
            return []
        return [f[:-5] for f in os.listdir(theme_dir) if f.endswith(".json")]