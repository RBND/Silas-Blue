"""
Silas Blue - Unified Desktop WebView Shell
Provides a lightweight desktop window embedding the Silas Blue Web Console via Edge WebView2.
Eliminates redundant UI code while sharing 100% of features with the web console and headless mode.
"""

import os
import sys
import time
import logging
import threading

logger = logging.getLogger("silasblue.desktop")

def is_webview_available() -> bool:
    """Checks if pywebview runtime is installed and usable."""
    try:
        import webview
        return True
    except ImportError:
        return False

def start_desktop_app(web_host: str = "127.0.0.1", web_port: int = 8080):
    """
    Launches Silas Blue bot, starts local web server, and opens the unified desktop WebView window.
    """
    if not is_webview_available():
        logger.error(
            "pywebview is not installed. Please install it with: pip install pywebview\n"
            "Falling back to browser-based dashboard..."
        )
        import webbrowser
        from web.server import start_web_server_in_thread, get_server_otp_link
        from bot_core import start_bot, stop_bot

        threading.Thread(target=start_bot, daemon=True, name="SilasBlue-DiscordBot").start()
        start_web_server_in_thread(host=web_host, port=web_port)
        url = get_server_otp_link(timeout=6.0)
        webbrowser.open(url)
        try:
            while True:
                time.sleep(1)
        except KeyboardInterrupt:
            stop_bot()
            sys.exit(0)

    import webview
    from bot_core import start_bot, stop_bot
    from web.server import start_web_server_in_thread, get_server_otp_link

    logger.info("Initializing Silas Blue Unified Desktop App...")

    # 1. Start Discord bot core in background
    threading.Thread(target=start_bot, daemon=True, name="SilasBlue-DiscordBot").start()

    # 2. Start local web server
    start_web_server_in_thread(host=web_host, port=web_port)

    # 3. Retrieve one-time auto-login link for immediate admin session
    login_url = get_server_otp_link(timeout=6.0)
    logger.info(f"Connecting Desktop WebView to {login_url}...")

    # 4. Create desktop window
    from SilasBlue import __version__
    window = webview.create_window(
        title=f"Silas Blue v{__version__}",
        url=login_url,
        width=1280,
        height=840,
        min_size=(960, 600),
        background_color="#0f111a",
        text_select=True,
        zoomable=True
    )

    def on_closing():
        logger.info("Desktop window closing. Shutting down Silas Blue services...")
        try:
            stop_bot()
        except Exception as e:
            logger.debug(f"Error during bot shutdown: {e}")

    window.events.closing += on_closing

    # 5. Start webview GUI loop on main thread
    webview.start(debug=False)
    logger.info("Desktop WebView terminated cleanly.")
