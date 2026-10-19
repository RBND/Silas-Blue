import os
import asyncio
import logging
import threading
from urllib.parse import urlencode

from aiohttp import web
from .auth import AuthManager
from .api import ApiHandler

logger = logging.getLogger("silasblue.web")

# --- Security Headers Middleware ---
@web.middleware
async def security_headers_middleware(request, handler):
    resp = await handler(request)
    resp.headers["X-Content-Type-Options"] = "nosniff"
    resp.headers["X-Frame-Options"] = "DENY"
    resp.headers["Referrer-Policy"] = "strict-origin-when-cross-origin"
    resp.headers["Permissions-Policy"] = "camera=(), microphone=(), geolocation=()"
    return resp

# --- Rate Limit Middleware ---
@web.middleware
async def rate_limit_middleware(request, handler):
    auth_manager = request.app["auth_manager"]
    peername = request.transport.get_extra_info('peername')
    ip = peername[0] if peername else "unknown"

    if request.path.startswith("/auth/"):
        if auth_manager.is_rate_limited(ip):
            logger.warning(f"Rate limited authentication attempt from IP: {ip}")
            return web.Response(
                text="Too many authentication attempts. Please try again in a few minutes.",
                status=429
            )
    return await handler(request)


def create_web_app(app_cfg: dict) -> web.Application:
    """Initializes and routes the aiohttp web application."""
    app = web.Application(middlewares=[security_headers_middleware, rate_limit_middleware])

    auth_manager = AuthManager(app_cfg)
    api_handler = ApiHandler(auth_manager)

    app["auth_manager"] = auth_manager
    app["api_handler"] = api_handler

    # App Lifecycle
    app.on_startup.append(api_handler.start_background_tasks)
    app.on_cleanup.append(api_handler.cleanup_background_tasks)

    # -------------------------------------------------------------------------
    # Authentication Endpoints
    # -------------------------------------------------------------------------
    async def handle_otp_login(request):
        """Processes One-Time Magic Link authentication."""
        token = request.query.get("token")
        peername = request.transport.get_extra_info('peername')
        ip = peername[0] if peername else "unknown"

        if not token or not auth_manager.verify_otp_token(token):
            auth_manager.record_failed_attempt(ip)
            return web.Response(
                text="Invalid or expired magic link. Please check your server console for a fresh link.",
                status=403
            )

        resp = web.HTTPFound(location="/")
        auth_manager.create_session_cookie(
            resp,
            {"id": "local_admin", "username": "LAN Administrator", "auth_type": "otp"}
        )
        logger.info(f"Successful OTP magic link authentication from {ip}")
        raise resp

    async def handle_discord_login(request):
        """Redirects user to Discord's OAuth2 authorization page."""
        from bot_core import get_bot_client_id
        client_id = app_cfg.get("discord_oauth_client_id") or get_bot_client_id()
        if not client_id:
            return web.Response(text="Discord Bot Client ID is not configured.", status=500)

        # Build redirect URI based on current request host
        scheme = request.headers.get("X-Forwarded-Proto", request.scheme)
        host = request.headers.get("X-Forwarded-Host", request.host)
        redirect_uri = f"{scheme}://{host}/auth/discord/callback"

        params = {
            "client_id": str(client_id),
            "redirect_uri": redirect_uri,
            "response_type": "code",
            "scope": "identify",
            "prompt": "consent"
        }
        oauth_url = f"https://discord.com/api/oauth2/authorize?{urlencode(params)}"
        raise web.HTTPFound(location=oauth_url)

    async def handle_discord_callback(request):
        """Handles OAuth2 callback from Discord."""
        code = request.query.get("code")
        peername = request.transport.get_extra_info('peername')
        ip = peername[0] if peername else "unknown"

        if not code:
            auth_manager.record_failed_attempt(ip)
            return web.Response(text="Missing OAuth code from Discord.", status=400)

        scheme = request.headers.get("X-Forwarded-Proto", request.scheme)
        host = request.headers.get("X-Forwarded-Host", request.host)
        redirect_uri = f"{scheme}://{host}/auth/discord/callback"

        user_info = await auth_manager.exchange_discord_oauth_code(code, redirect_uri)
        if not user_info:
            auth_manager.record_failed_attempt(ip)
            return web.Response(text="Failed to authenticate with Discord.", status=401)

        discord_user_id = str(user_info.get("id"))
        if not auth_manager.is_user_authorized(discord_user_id):
            logger.warning(f"Unauthorized login attempt by Discord user: {user_info.get('username')} ({discord_user_id})")
            return web.Response(
                text=f"Access Denied: Discord account '{user_info.get('username')}' ({discord_user_id}) is not an authorized administrator.",
                status=403
            )

        resp = web.HTTPFound(location="/")
        avatar_url = f"https://cdn.discordapp.com/avatars/{discord_user_id}/{user_info.get('avatar')}.png" if user_info.get("avatar") else ""
        auth_manager.create_session_cookie(
            resp,
            {
                "id": discord_user_id,
                "username": user_info.get("username", "Admin"),
                "avatar": avatar_url,
                "auth_type": "discord"
            },
            is_secure=(scheme == "https")
        )
        logger.info(f"Successful Discord OAuth2 login for user {user_info.get('username')} ({discord_user_id})")
        raise resp

    async def handle_logout(request):
        resp = web.HTTPFound(location="/login")
        resp.del_cookie("sb_session")
        raise resp

    # -------------------------------------------------------------------------
    # Frontend SPA Page Routes
    # -------------------------------------------------------------------------
    static_dir = os.path.join(os.path.dirname(__file__), "static")
    os.makedirs(static_dir, exist_ok=True)

    async def handle_index(request):
        """Serves main Single-Page App if authenticated, else redirects to login."""
        session_cookie = request.cookies.get("sb_session")
        if not auth_manager.verify_session_token(session_cookie):
            raise web.HTTPFound(location="/login")
        return web.FileResponse(os.path.join(static_dir, "index.html"))

    async def handle_login_page(request):
        """Serves login portal page."""
        return web.FileResponse(os.path.join(static_dir, "login.html"))

    # Register Auth Routes
    app.router.add_get("/auth/otp", handle_otp_login)
    app.router.add_get("/auth/discord/login", handle_discord_login)
    app.router.add_get("/auth/discord/callback", handle_discord_callback)
    app.router.add_get("/logout", handle_logout)

    # Register API Routes (Wrapped in require_auth)
    app.router.add_get("/api/status", api_handler.require_auth(api_handler.get_status))
    app.router.add_post("/api/bot/action", api_handler.require_auth(api_handler.bot_action))
    app.router.add_get("/api/models", api_handler.require_auth(api_handler.get_models))
    app.router.add_post("/api/models/active", api_handler.require_auth(api_handler.set_active_model))
    app.router.add_post("/api/models/pull", api_handler.require_auth(api_handler.pull_model))
    app.router.add_get("/api/guild/{guild_id}", api_handler.require_auth(api_handler.get_guild_config))
    app.router.add_post("/api/guild/{guild_id}", api_handler.require_auth(api_handler.save_guild_config))
    app.router.add_get("/api/ai/config", api_handler.require_auth(api_handler.get_ai_config))
    app.router.add_post("/api/ai/config", api_handler.require_auth(api_handler.save_ai_config))
    app.router.add_get("/api/update/status", api_handler.require_auth(api_handler.get_update_status))
    app.router.add_get("/api/bot/invite", api_handler.require_auth(api_handler.get_bot_invite))
    app.router.add_post("/api/update/check", api_handler.require_auth(api_handler.check_update))
    app.router.add_post("/api/update/apply", api_handler.require_auth(api_handler.apply_update))
    app.router.add_post("/api/update/config", api_handler.require_auth(api_handler.save_update_config))

    # WebSocket Log Stream
    app.router.add_get("/api/ws/logs", api_handler.websocket_handler)

    # Web Pages
    app.router.add_get("/", handle_index)
    app.router.add_get("/login", handle_login_page)

    # Static Assets (CSS, JS, Images)
    app.router.add_static("/static/", path=static_dir, name="static")

    return app


_server_state = {
    "host": "127.0.0.1",
    "port": 8080,
    "otp_link": None,
    "ready": threading.Event()
}

def get_server_otp_link(timeout: float = 5.0) -> str:
    """Waits for web server to start and returns the auto-login OTP link."""
    _server_state["ready"].wait(timeout=timeout)
    return _server_state.get("otp_link") or f"http://{_server_state['host']}:{_server_state['port']}"


def start_web_server_in_thread(host: str = "127.0.0.1", port: int = 8080, app_cfg: dict = None) -> threading.Thread:
    """Launches the aiohttp web management server in a dedicated daemon thread."""
    if app_cfg is None:
        from utils import load_app_config
        app_cfg = load_app_config()

    _server_state["host"] = host
    _server_state["port"] = port
    _server_state["ready"].clear()

    def run():
        loop = asyncio.new_event_loop()
        asyncio.set_event_loop(loop)
        app = create_web_app(app_cfg)

        runner = web.AppRunner(app)
        loop.run_until_complete(runner.setup())
        site = web.TCPSite(runner, host, port)
        loop.run_until_complete(site.start())

        # Generate and print OTP Magic Link
        auth_mgr = app["auth_manager"]
        otp_link = auth_mgr.generate_otp_magic_link(host, port)
        _server_state["otp_link"] = otp_link
        _server_state["ready"].set()

        print("\n" + "=" * 65)
        print("          SILAS BLUE WEB DASHBOARD ACTIVE")
        print("=" * 65)
        print(f" Web URL:         http://{host}:{port}")
        print(f" One-Time Login:  {otp_link}")
        print(" (Use the One-Time Login link above to access the dashboard)")
        print("=" * 65 + "\n")

        try:
            loop.run_forever()
        finally:
            loop.run_until_complete(runner.cleanup())
            loop.close()

    t = threading.Thread(target=run, daemon=True, name="SilasBlue-Web")
    t.start()
    return t
