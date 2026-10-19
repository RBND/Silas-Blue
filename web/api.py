import asyncio
import json
import logging
import os
import time
from datetime import datetime
from aiohttp import web, WSMsgType

from bot_core import (
    bot, start_bot, stop_bot, restart_bot,
    get_provider, reload_ai_provider, gui_event_queue,
    update_server_config, _bot_thread
)
from utils import load_app_config, save_app_config, load_config, save_config
from ai_providers import REMOTE_PRESETS, get_ai_provider
from updater import BotUpdater, get_current_version, get_current_commit, parse_semver
import config

logger = logging.getLogger("silasblue.web.api")

start_time = time.time()

class ApiHandler:
    """
    Handles REST API requests and real-time WebSocket log streaming for the web dashboard.
    """

    def __init__(self, auth_manager):
        self.auth_manager = auth_manager
        self.app_cfg = auth_manager.app_cfg
        self.active_ws_clients = set()
        self.ws_dispatcher_task = None

    def require_auth(self, handler):
        """Decorator ensuring request has valid authenticated session."""
        async def wrapper(request):
            session_cookie = request.cookies.get("sb_session")
            session = self.auth_manager.verify_session_token(session_cookie)
            if not session:
                return web.json_response({"error": "Unauthorized", "login_url": "/login"}, status=401)
            request["user"] = session
            return await handler(request)
        return wrapper

    async def start_background_tasks(self, app):
        """Starts WebSocket queue dispatcher loop."""
        self.ws_dispatcher_task = asyncio.create_task(self._log_dispatcher_loop())

    async def cleanup_background_tasks(self, app):
        if self.ws_dispatcher_task:
            self.ws_dispatcher_task.cancel()
            try:
                await self.ws_dispatcher_task
            except asyncio.CancelledError:
                pass
        for ws in list(self.active_ws_clients):
            await ws.close()

    async def _log_dispatcher_loop(self):
        """Monitors gui_event_queue and broadcasts to active WebSocket connections."""
        while True:
            try:
                now_str = datetime.now().strftime("%H:%M:%S")
                # Read any pending events from the queue
                while not gui_event_queue.empty():
                    entry = gui_event_queue.get_nowait()
                    event = entry.get("event")
                    data = entry.get("data", {})

                    formatted_msg = None
                    event_type = "system"

                    if event == "config_change":
                        event_type = "config"
                        formatted_msg = f"[{now_str}] [Config Change] Guild {data.get('guild_id')}: {data.get('field')} -> {data.get('value')}"
                    elif event == "prompt":
                        event_type = "prompt"
                        guild = data.get("guild_name") or data.get("guild_id")
                        formatted_msg = f"[{now_str}] [{guild}] {data.get('user')}: {data.get('prompt')}"
                    elif event == "reply":
                        event_type = "reply"
                        guild = data.get("guild_name") or data.get("guild_id")
                        reply_snippet = data.get("reply", "")[:180]
                        formatted_msg = f"[{now_str}] [{guild}] Silas Blue -> {reply_snippet}..."
                    elif event == "server_joined":
                        event_type = "server"
                        formatted_msg = f"[{now_str}] [Discord] Joined server: {data.get('guild_name')} ({data.get('guild_id')})"
                    elif event == "server_left":
                        event_type = "server"
                        formatted_msg = f"[{now_str}] [Discord] Left server: {data.get('guild_name')} ({data.get('guild_id')})"
                    elif event == "web_search":
                        event_type = "search"
                        guild = data.get("guild_name") or data.get("guild_id") or "Direct Message"
                        formatted_msg = f"[{now_str}] [{guild}] 🌐 Web Search: \"{data.get('query', '')[:50]}\" ({data.get('results_count', 0)} results)"

                    if formatted_msg and self.active_ws_clients:
                        payload = json.dumps({
                            "type": event_type,
                            "timestamp": now_str,
                            "raw": entry,
                            "formatted": formatted_msg
                        })
                        disconnected = set()
                        for ws in self.active_ws_clients:
                            try:
                                await ws.send_str(payload)
                            except Exception:
                                disconnected.add(ws)
                        self.active_ws_clients.difference_update(disconnected)

                await asyncio.sleep(0.3)
            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error(f"Error in web log dispatcher: {e}")
                await asyncio.sleep(1)

    # -------------------------------------------------------------------------
    # REST API HANDLERS
    # -------------------------------------------------------------------------
    async def get_status(self, request):
        """Returns overall bot, AI backend, and system status."""
        bot_running = (_bot_thread is not None and _bot_thread.is_alive())
        provider = get_provider()

        loop = asyncio.get_running_loop()
        ai_online = await loop.run_in_executor(None, provider.status_sync)

        guilds_list = []
        try:
            for g in getattr(bot, "guilds", []):
                guilds_list.append({
                    "id": str(g.id),
                    "name": g.name,
                    "member_count": getattr(g, "member_count", 0),
                    "icon": str(g.icon.url) if getattr(g, "icon", None) else None
                })
        except Exception:
            pass

        uptime_secs = int(time.time() - start_time)

        return web.json_response({
            "version": get_current_version(),
            "commit": get_current_commit(),
            "bot": {
                "running": bot_running,
                "status": "Running" if bot_running else "Stopped",
                "servers_count": len(guilds_list),
                "user": str(bot.user) if getattr(bot, "user", None) else None
            },
            "ai": {
                "name": provider.name,
                "is_local": provider.is_local(),
                "online": ai_online,
                "base_url": getattr(provider, "base_url", ""),
                "active_model": self.app_cfg.get("active_model", "None")
            },
            "system": {
                "uptime_seconds": uptime_secs,
                "debug": config.DEBUG
            },
            "guilds": guilds_list,
            "user": request.get("user", {})
        })

    async def bot_action(self, request):
        """Controls bot daemon (start, stop, restart)."""
        data = await request.json()
        action = data.get("action")
        loop = asyncio.get_running_loop()

        if action == "start":
            loop.run_in_executor(None, start_bot)
            return web.json_response({"success": True, "message": "Bot startup initiated."})
        elif action == "stop":
            loop.run_in_executor(None, stop_bot)
            return web.json_response({"success": True, "message": "Bot shutdown initiated."})
        elif action == "restart":
            loop.run_in_executor(None, restart_bot)
            return web.json_response({"success": True, "message": "Bot restart initiated."})
        else:
            return web.json_response({"error": f"Unknown action: {action}"}, status=400)

    async def get_models(self, request):
        """Fetches list of available models from the active provider."""
        provider = get_provider()
        loop = asyncio.get_running_loop()
        try:
            models = await loop.run_in_executor(None, provider.list_models_sync)
            return web.json_response({"models": models or [], "active": self.app_cfg.get("active_model", "")})
        except Exception as e:
            return web.json_response({"models": [], "error": str(e)})

    async def set_active_model(self, request):
        """Sets the active model."""
        data = await request.json()
        model_name = data.get("model", "").strip()
        if not model_name:
            return web.json_response({"error": "Model name cannot be empty"}, status=400)

        self.app_cfg["active_model"] = model_name
        save_app_config(self.app_cfg)
        return web.json_response({"success": True, "active_model": model_name})

    async def pull_model(self, request):
        """Requests active provider to pull/download a model."""
        data = await request.json()
        model_name = data.get("model", "").strip()
        if not model_name:
            return web.json_response({"error": "Model name cannot be empty"}, status=400)

        provider = get_provider()
        loop = asyncio.get_running_loop()

        def do_pull():
            provider.pull_model_sync(model_name)

        loop.run_in_executor(None, do_pull)
        return web.json_response({"success": True, "message": f"Pull initiated for {model_name}."})

    async def get_guild_config(self, request):
        """Returns per-server access roles and response configuration."""
        guild_id = request.match_info.get("guild_id")
        if not guild_id:
            return web.json_response({"error": "Guild ID required"}, status=400)

        cfg = load_config(guild_id)
        # Find roles in bot cache if available
        roles = []
        try:
            guild = next((g for g in getattr(bot, "guilds", []) if str(g.id) == str(guild_id)), None)
            if guild:
                for r in guild.roles:
                    if not r.is_default():
                        roles.append({"id": str(r.id), "name": r.name})
                roles.append({"id": "everyone", "name": "@everyone"})
        except Exception:
            pass

        return web.json_response({"config": cfg, "available_roles": roles})

    async def save_guild_config(self, request):
        """Updates and applies per-server configuration."""
        guild_id = request.match_info.get("guild_id")
        if not guild_id:
            return web.json_response({"error": "Guild ID required"}, status=400)

        cfg = await request.json()
        if cfg.get("pagination_max_chars", 1900) > 1900:
            cfg["pagination_max_chars"] = 1900

        save_config(guild_id, cfg)
        update_server_config(guild_id, cfg)
        return web.json_response({"success": True, "message": "Server settings saved & applied."})

    async def get_ai_config(self, request):
        """Returns AI provider settings with sensitive API keys masked."""
        cfg = dict(self.app_cfg)
        # Mask api keys
        for key in ["api_key", "remote_api_key", "discord_token", "discord_oauth_client_secret"]:
            if cfg.get(key):
                cfg[key] = "●●●●●●●●" if len(cfg[key]) > 0 else ""
        return web.json_response({"config": cfg, "presets": list(REMOTE_PRESETS.keys()), "preset_details": REMOTE_PRESETS})

    async def save_ai_config(self, request):
        """Saves AI provider settings and reloads the active provider."""
        data = await request.json()

        for key, val in data.items():
            if val != "●●●●●●●●":  # Don't overwrite with mask
                self.app_cfg[key] = val

        if self.app_cfg.get("backend_type") in ("remote", "remote_openai", "openai", "remote_api"):
            if self.app_cfg.get("remote_model"):
                self.app_cfg["active_model"] = self.app_cfg["remote_model"]

        save_app_config(self.app_cfg)
        reload_ai_provider()
        return web.json_response({"success": True, "message": "AI settings saved and backend reloaded."})

    # -------------------------------------------------------------------------
    # WEBSOCKET STREAMING
    # -------------------------------------------------------------------------
    async def websocket_handler(self, request):
        """Streams live log messages and bot events directly to the browser."""
        session_cookie = request.cookies.get("sb_session")
        session = self.auth_manager.verify_session_token(session_cookie)
        if not session:
            return web.Response(status=401, text="Unauthorized WebSocket connection")

        ws = web.WebSocketResponse(heartbeat=30.0)
        await ws.prepare(request)
        self.active_ws_clients.add(ws)

        try:
            # Send initial welcome packet
            await ws.send_str(json.dumps({
                "type": "system",
                "timestamp": datetime.now().strftime("%H:%M:%S"),
                "formatted": f"[{datetime.now().strftime('%H:%M:%S')}] [Web Console] Connected to Silas Blue real-time event stream."
            }))

            async for msg in ws:
                if msg.type == WSMsgType.TEXT:
                    # Can handle client ping/commands if needed
                    pass
                elif msg.type == WSMsgType.ERROR:
                    break
        finally:
            self.active_ws_clients.discard(ws)

        return ws

    # -------------------------------------------------------------------------
    # UPDATER MANAGEMENT
    # -------------------------------------------------------------------------
    async def get_update_status(self, request):
        """Returns current version, commit, update configuration, and cached status."""
        cfg = load_app_config()
        channel = cfg.get("update_channel", "stable")
        
        status_file = os.path.join("state", "update_status.json")
        cached = {}
        if os.path.exists(status_file):
            try:
                with open(status_file, "r", encoding="utf-8") as f:
                    cached = json.load(f)
            except Exception:
                pass

        curr_ver = get_current_version()
        curr_commit = get_current_commit()

        # If cached status indicates an update, but we are already on that version/commit, clear update_available
        if cached and cached.get("update_available"):
            cached_ver = cached.get("latest_version", "")
            cached_commit = (cached.get("latest_commit", "") or "")[:7]
            curr_c = (curr_commit or "")[:7]

            if curr_c and cached_commit and curr_c == cached_commit:
                cached["update_available"] = False
                cached["commits_behind"] = 0
            elif cached_ver and parse_semver(curr_ver) >= parse_semver(cached_ver) and channel == "stable":
                cached["update_available"] = False
                cached["commits_behind"] = 0

        return web.json_response({
            "current_version": curr_ver,
            "current_commit": curr_commit,
            "config": {
                "update_channel": cfg.get("update_channel", "stable"),
                "auto_update_enabled": cfg.get("auto_update_enabled", False),
                "auto_update_mode": cfg.get("auto_update_mode", "notify"),
                "auto_update_interval_hours": cfg.get("auto_update_interval_hours", 24),
                "update_notify_channel_id": cfg.get("update_notify_channel_id"),
                "update_notify_owner_dm": cfg.get("update_notify_owner_dm", True)
            },
            "cached_status": cached
        })

    async def check_update(self, request):
        """Executes a fresh update check on demand."""
        try:
            body = await request.json()
        except Exception:
            body = {}
        channel = body.get("channel")
        res = await asyncio.to_thread(BotUpdater.check_for_updates, channel)
        return web.json_response(res)

    async def apply_update(self, request):
        """Applies update and schedules restart."""
        try:
            body = await request.json()
        except Exception:
            body = {}
        channel = body.get("channel")
        user = request.get("user", {})
        initiator = user.get("username", "WebAdmin")

        success, message = await asyncio.to_thread(
            BotUpdater.apply_update,
            channel=channel,
            initiator=f"Web: {initiator}"
        )

        if success:
            async def delayed_restart():
                await asyncio.sleep(2)
                BotUpdater.trigger_restart()

            asyncio.create_task(delayed_restart())
            return web.json_response({"success": True, "message": message})
        else:
            return web.json_response({"success": False, "error": message}, status=500)

    async def save_update_config(self, request):
        """Saves updater configuration."""
        data = await request.json()
        cfg = load_app_config()

        for key in [
            "update_channel",
            "auto_update_enabled",
            "auto_update_mode",
            "auto_update_interval_hours",
            "update_notify_channel_id",
            "update_notify_owner_dm"
        ]:
            if key in data:
                cfg[key] = data[key]

        save_app_config(cfg)
        self.app_cfg = cfg
        return web.json_response({"success": True, "message": "Updater settings saved."})

    async def get_bot_invite(self, request):
        """Returns Discord OAuth2 Bot invitation URL and Client ID."""
        from bot_core import get_bot_invite_url, get_bot_client_id
        client_id = get_bot_client_id()
        invite_url = get_bot_invite_url()
        return web.json_response({
            "client_id": str(client_id) if client_id else None,
            "invite_url": invite_url
        })


