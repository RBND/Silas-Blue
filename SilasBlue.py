"""
Silas Blue - Discord Bot powered by local AI models via Ollama or Llama.cpp
Main entry point: launches Discord bot with either desktop GUI or headless web management.
"""

__version__ = "3.5.10"

import sys
import threading
import os
import time
import signal
import logging
import argparse

# Remove conflicting PYTHONHOME in Windows multi-python environments
os.environ.pop("PYTHONHOME", None)

from bot_core import start_bot, stop_bot, restart_bot, get_provider
from utils import load_app_config, save_app_config
import config

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("silasblue")

CRASH_COUNTER_FILE = os.path.join("config", "crash_counter.txt")

def get_crash_counter() -> int:
    try:
        with open(CRASH_COUNTER_FILE, "r", encoding="utf-8") as f:
            return int(f.read().strip())
    except Exception:
        return 0

def set_crash_counter(val: int):
    try:
        os.makedirs("config", exist_ok=True)
        with open(CRASH_COUNTER_FILE, "w", encoding="utf-8") as f:
            f.write(str(val))
    except Exception:
        pass

# Check if repeated crashes previously occurred
if get_crash_counter() >= 2:
    config.DEBUG = True
    logger.info("Debug mode auto-enabled due to previous crashes.")
    # Check if an update caused a boot failure; if so, trigger rollback
    try:
        from updater import BotUpdater
        rolled_back, msg = BotUpdater.rollback()
        if rolled_back:
            logger.warning(f"Repeated crash on boot detected! Automated update rollback executed: {msg}")
    except Exception as e:
        logger.debug(f"Rollback verification check: {e}")
    set_crash_counter(0)

def start_gui_and_bot(enable_web: bool = True, web_host: str = "127.0.0.1", web_port: int = 8080, legacy_qt: bool = False):
    """Starts the desktop GUI (unified WebView by default, or legacy PySide6) and bot services."""
    if legacy_qt:
        logger.info("Starting Silas Blue with legacy PySide6 Qt GUI...")
        from gui.main_window import MainWindow
        from PySide6.QtWidgets import QApplication, QSplashScreen
        from PySide6.QtCore import QTimer, Qt
        from PySide6.QtGui import QPixmap, QGuiApplication

        # Enable fractional High-DPI Pass-Through scaling to prevent layout clipping
        QGuiApplication.setHighDpiScaleFactorRoundingPolicy(
            Qt.HighDpiScaleFactorRoundingPolicy.PassThrough
        )

        app = QApplication(sys.argv)

        # Splash Screen
        splash = None
        splash_path = os.path.join("gui", "splash.png")
        if os.path.exists(splash_path):
            splash_pix = QPixmap(splash_path)
            splash = QSplashScreen(splash_pix)
            splash.show()
            app.processEvents()

        # Optional local web server
        if enable_web:
            try:
                from web.server import start_web_server_in_thread
                start_web_server_in_thread(host=web_host, port=web_port)
            except Exception as e:
                logger.warning(f"Failed to start embedded web server: {e}")

        def show_main_window():
            window = MainWindow(start_bot=start_bot, stop_bot=stop_bot, restart_bot=restart_bot)
            window.show()
            if splash:
                splash.finish(window)
            window.redirect_output_to_log()
            # Start Discord bot in background
            QTimer.singleShot(100, start_bot)

        # Transition from splash to main window
        QTimer.singleShot(1500, show_main_window)
        exit_code = app.exec()
        set_crash_counter(0)  # Clean exit
        sys.exit(exit_code)

    # Unified WebView Desktop App (Single UI codebase)
    from gui.webview_app import start_desktop_app
    start_desktop_app(web_host=web_host, web_port=web_port)
    set_crash_counter(0)
    sys.exit(0)

def start_headless_and_bot(web_host: str = "127.0.0.1", web_port: int = 8080):
    """Starts bot core and web management server in headless mode (no GUI display needed)."""
    logger.info("Starting Silas Blue in HEADLESS mode...")

    # Start Discord Bot
    start_bot()

    # Start Web Management Server
    from web.server import start_web_server_in_thread
    start_web_server_in_thread(host=web_host, port=web_port)

    # Wait for SIGINT / SIGTERM
    stop_event = threading.Event()

    def handle_signal(sig, frame):
        logger.info("Shutdown signal received. Stopping services...")
        stop_bot()
        stop_event.set()

    signal.signal(signal.SIGINT, handle_signal)
    signal.signal(signal.SIGTERM, handle_signal)

    logger.info("Silas Blue headless daemon is running. Press Ctrl+C to terminate.")
    while not stop_event.is_set():
        time.sleep(1)

    set_crash_counter(0)
    sys.exit(0)

def ensure_ai_service_running():
    """
    Checks if active AI provider is running. If not, attempts to start it locally
    or auto-detects another running local AI backend when configured as 'auto'.
    Never overwrites user-configured remote cloud APIs or explicit backend settings.
    """
    from bot_core import reload_ai_provider
    from ai_providers.detector import detect_all_ai_providers

    app_cfg = load_app_config()
    current_backend = app_cfg.get("backend_type", "ollama").lower()
    provider = get_provider()

    # 1. Remote cloud providers: test status, but NEVER hijack or overwrite with local backends
    if current_backend in ("remote", "remote_openai", "openai", "remote_api") or not provider.is_local():
        if provider.status_sync():
            logger.info(f"{provider.name} service is running and responsive.")
        else:
            logger.warning(
                f"Configured remote AI provider '{provider.name}' ({provider.base_url}) is currently not responding. "
                "Retaining remote configuration. Please verify your network connection and API key."
            )
        return

    # 2. Configured local provider is already running and responsive
    if provider.status_sync():
        logger.info(f"{provider.name} service is running and responsive.")
        return

    # 3. Attempt to start configured local service
    if provider.is_local():
        logger.info(f"Local {provider.name} is not running. Attempting to start local {provider.name} service...")
        if provider.start_local_service():
            logger.info(f"Local {provider.name} service started successfully.")
            return
        else:
            logger.warning(f"Failed to start local {provider.name}.")

    # 4. Only if backend_type was explicitly set to 'auto', auto-switch to a detected running backend
    if current_backend == "auto":
        logger.info("Scanning for active local AI backends...")
        detected = detect_all_ai_providers()

        if detected["llamacpp"]["running"]:
            llama_info = detected["llamacpp"]
            logger.info(f"Auto-detected running Llama.cpp instance at {llama_info['url']}! Switching backend...")
            app_cfg["backend_type"] = "llamacpp"
            app_cfg["llamacpp_url"] = llama_info["url"]
            if llama_info.get("binary_path"):
                app_cfg["llamacpp_binary_path"] = llama_info["binary_path"]
            if llama_info.get("model_path") and not app_cfg.get("llamacpp_model_path"):
                app_cfg["llamacpp_model_path"] = llama_info["model_path"]
            save_app_config(app_cfg)
            reload_ai_provider()
            logger.info("Successfully switched to Llama.cpp backend.")
        elif detected["ollama"]["running"]:
            ollama_info = detected["ollama"]
            logger.info(f"Auto-detected running Ollama instance at {ollama_info['url']}! Switching backend...")
            app_cfg["backend_type"] = "ollama"
            app_cfg["ollama_url"] = ollama_info["url"]
            save_app_config(app_cfg)
            reload_ai_provider()
            logger.info("Successfully switched to Ollama backend.")
    else:
        logger.warning(
            f"Configured local backend '{current_backend}' is offline and could not be started automatically. "
            "Retaining user configuration. You can run auto-detection or start the service from the Control Panel."
        )

def is_display_available() -> bool:
    """Checks if a graphical display environment is available."""
    if sys.platform.startswith("win"):
        return True
    return bool(os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY"))

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Silas Blue Discord Bot & AI Assistant")
    parser.add_argument("--headless", "--no-gui", action="store_true", help="Run in headless daemon mode without desktop GUI")
    parser.add_argument("--web", action="store_true", default=True, help="Enable web management server (default: enabled)")
    parser.add_argument("--no-web", action="store_true", help="Disable web management server")
    parser.add_argument("--web-host", default="127.0.0.1", help="Host interface for web dashboard (default: 127.0.0.1; use 0.0.0.0 for LAN/VPS)")
    parser.add_argument("--web-port", type=int, default=8080, help="Port for web dashboard (default: 8080)")

    parser.add_argument("--legacy-gui", "--qt", action="store_true", help="Run with legacy PySide6 Qt GUI instead of unified WebView")

    args = parser.parse_args()

    enable_web = args.web and not args.no_web
    is_headless = args.headless or not is_display_available()

    try:
        threading.Thread(target=ensure_ai_service_running, daemon=True).start()

        if is_headless:
            start_headless_and_bot(web_host=args.web_host, web_port=args.web_port)
        else:
            start_gui_and_bot(
                enable_web=enable_web,
                web_host=args.web_host,
                web_port=args.web_port,
                legacy_qt=args.legacy_gui
            )

    except Exception as e:
        count = get_crash_counter() + 1
        set_crash_counter(count)
        logger.error(f"Silas Blue encountered an unhandled crash: {e}")
        if count >= 2:
            try:
                from updater import BotUpdater
                rolled_back, msg = BotUpdater.rollback()
                if rolled_back:
                    logger.warning(f"Repeated crash on boot detected! Automated update rollback executed: {msg}")
            except Exception:
                pass
        raise
