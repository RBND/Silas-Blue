import os
import hmac
import hashlib
import time
import json
import base64
import secrets
import logging
from typing import Optional, Dict, Any, List
import aiohttp
from aiohttp import web

logger = logging.getLogger("silasblue.web.auth")

SECRET_KEY_FILE = os.path.join("config", "web_secret.key")
SESSION_COOKIE_NAME = "sb_session"
SESSION_DURATION_SECONDS = 7 * 24 * 3600  # 7 days

class AuthManager:
    """
    Manages Discord OAuth2 authentication, cryptographically signed session tokens,
    and single-use OTP magic links for local LAN administrator access.
    """

    def __init__(self, app_cfg: dict):
        self.app_cfg = app_cfg
        self.secret_key = self._load_or_generate_secret_key()
        self.active_otp_token: Optional[str] = None
        self.otp_expiry: float = 0.0
        self.failed_attempts: Dict[str, List[float]] = {}  # ip -> list of timestamps

    def _load_or_generate_secret_key(self) -> bytes:
        os.makedirs(os.path.dirname(SECRET_KEY_FILE), exist_ok=True)
        if os.path.exists(SECRET_KEY_FILE):
            try:
                with open(SECRET_KEY_FILE, "rb") as f:
                    key = f.read().strip()
                if len(key) >= 32:
                    return key
            except Exception as e:
                logger.warning(f"Could not read web secret key file: {e}")

        new_key = secrets.token_bytes(32)
        try:
            with open(SECRET_KEY_FILE, "wb") as f:
                f.write(new_key)
        except Exception as e:
            logger.warning(f"Could not persist web secret key: {e}")
        return new_key

    def generate_otp_magic_link(self, host: str, port: int) -> str:
        """Generates a secure one-time token valid for 2 hours."""
        self.active_otp_token = secrets.token_urlsafe(32)
        self.otp_expiry = time.time() + 7200  # 2 hours
        display_host = "localhost" if host in ("0.0.0.0", "127.0.0.1") else host
        return f"http://{display_host}:{port}/auth/otp?token={self.active_otp_token}"

    def verify_otp_token(self, token: str) -> bool:
        if not self.active_otp_token or not token:
            return False
        if time.time() > self.otp_expiry:
            self.active_otp_token = None
            return False
        if hmac.compare_digest(self.active_otp_token, token):
            self.active_otp_token = None  # Single use
            return True
        return False

    def sign_session_data(self, data: dict) -> str:
        """Serializes and signs session data into a secure token."""
        data_bytes = json.dumps(data, separators=(',', ':')).encode('utf-8')
        b64_data = base64.urlsafe_b64encode(data_bytes).decode('ascii')
        signature = hmac.new(self.secret_key, b64_data.encode('ascii'), hashlib.sha256).hexdigest()
        return f"{b64_data}.{signature}"

    def verify_session_token(self, token: str) -> Optional[dict]:
        """Verifies session signature and expiration. Returns payload dict or None."""
        if not token or '.' not in token:
            return None
        try:
            b64_data, signature = token.rsplit('.', 1)
            expected_sig = hmac.new(self.secret_key, b64_data.encode('ascii'), hashlib.sha256).hexdigest()
            if not hmac.compare_digest(signature, expected_sig):
                return None
            data_bytes = base64.urlsafe_b64decode(b64_data.encode('ascii'))
            payload = json.loads(data_bytes.decode('utf-8'))

            if payload.get("exp", 0) < time.time():
                return None  # Expired
            return payload
        except Exception:
            return None

    def create_session_cookie(self, response: web.StreamResponse, user_info: dict, is_secure: bool = False):
        payload = {
            "sub": str(user_info.get("id", "admin")),
            "username": user_info.get("username", "Admin"),
            "avatar": user_info.get("avatar", ""),
            "auth_type": user_info.get("auth_type", "otp"),
            "exp": int(time.time() + SESSION_DURATION_SECONDS)
        }
        token = self.sign_session_data(payload)
        response.set_cookie(
            SESSION_COOKIE_NAME,
            token,
            max_age=SESSION_DURATION_SECONDS,
            httponly=True,
            samesite="Lax",
            secure=is_secure
        )

    def is_user_authorized(self, discord_user_id: str) -> bool:
        """Checks if a Discord User ID is on the admin or owner whitelist."""
        # 1. Configured owner or admin IDs
        admin_ids = [str(x) for x in self.app_cfg.get("admin_discord_ids", [])]
        owner_id = self.app_cfg.get("bot_owner_id")
        if owner_id:
            admin_ids.append(str(owner_id))

        if str(discord_user_id) in admin_ids:
            return True

        # 2. Check live bot application owner if bot is connected
        try:
            from bot_core import bot
            if getattr(bot, "application", None):
                app_owner = getattr(bot.application, "owner", None)
                if app_owner:
                    # Single owner
                    if hasattr(app_owner, "id") and str(app_owner.id) == str(discord_user_id):
                        return True
                    # Team owner
                    if hasattr(app_owner, "members"):
                        if any(str(m.id) == str(discord_user_id) for m in app_owner.members):
                            return True
        except Exception:
            pass

        # 3. If no admin IDs are configured yet, first login can be set as owner
        if not admin_ids:
            logger.info(f"No admin IDs configured. Registering Discord User {discord_user_id} as bot owner.")
            self.app_cfg["bot_owner_id"] = str(discord_user_id)
            from utils import save_app_config
            save_app_config(self.app_cfg)
            return True

        return False

    def is_rate_limited(self, ip: str, max_attempts: int = 5, window_seconds: int = 300) -> bool:
        now = time.time()
        attempts = self.failed_attempts.get(ip, [])
        attempts = [t for t in attempts if now - t < window_seconds]
        self.failed_attempts[ip] = attempts
        return len(attempts) >= max_attempts

    def record_failed_attempt(self, ip: str):
        now = time.time()
        if ip not in self.failed_attempts:
            self.failed_attempts[ip] = []
        self.failed_attempts[ip].append(now)

    async def exchange_discord_oauth_code(self, code: str, redirect_uri: str) -> Optional[dict]:
        """Exchanges an OAuth authorization code with Discord API for access token & user profile."""
        from bot_core import get_bot_client_id
        client_id = self.app_cfg.get("discord_oauth_client_id") or get_bot_client_id()
        client_secret = self.app_cfg.get("discord_oauth_client_secret")

        if not client_id or not client_secret:
            logger.error("Discord OAuth credentials not configured in app_config.json")
            return None

        token_url = "https://discord.com/api/oauth2/token"
        headers = {"Content-Type": "application/x-www-form-urlencoded"}
        data = {
            "client_id": str(client_id),
            "client_secret": str(client_secret),
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": redirect_uri,
        }

        async with aiohttp.ClientSession() as session:
            async with session.post(token_url, data=data, headers=headers) as resp:
                if resp.status != 200:
                    err_txt = await resp.text()
                    logger.error(f"Failed to exchange Discord OAuth code: {resp.status} - {err_txt}")
                    return None
                token_data = await resp.json()

            access_token = token_data.get("access_token")
            if not access_token:
                return None

            user_headers = {"Authorization": f"Bearer {access_token}"}
            async with session.get("https://discord.com/api/users/@me", headers=user_headers) as u_resp:
                if u_resp.status != 200:
                    return None
                return await u_resp.json()
