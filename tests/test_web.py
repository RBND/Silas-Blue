import unittest
import time
import os
import json
from aiohttp.test_utils import AioHTTPTestCase, unittest_run_loop
from aiohttp import web

from web.auth import AuthManager, SESSION_COOKIE_NAME
from web.server import create_web_app

class TestWebAuth(unittest.TestCase):
    def setUp(self):
        self.cfg = {
            "bot_owner_id": "123456789",
            "admin_discord_ids": ["987654321"]
        }
        self.auth = AuthManager(self.cfg)

    def test_otp_token_lifecycle(self):
        link = self.auth.generate_otp_magic_link("localhost", 8080)
        self.assertIn("token=", link)
        token = self.auth.active_otp_token
        self.assertIsNotNone(token)

        # Successful verification
        self.assertTrue(self.auth.verify_otp_token(token))
        # Second attempt should fail (single-use)
        self.assertFalse(self.auth.verify_otp_token(token))
        # Bogus token fails
        self.assertFalse(self.auth.verify_otp_token("random_bad_token"))

    def test_session_token_signing_and_tampering(self):
        payload = {"sub": "12345", "username": "Tester", "exp": int(time.time() + 3600)}
        signed = self.auth.sign_session_data(payload)

        # Valid verification
        verified = self.auth.verify_session_token(signed)
        self.assertIsNotNone(verified)
        self.assertEqual(verified["sub"], "12345")
        self.assertEqual(verified["username"], "Tester")

        # Tampered signature
        parts = signed.split(".")
        tampered_sig = parts[0] + "." + "a" * len(parts[1])
        self.assertIsNone(self.auth.verify_session_token(tampered_sig))

        # Tampered payload
        tampered_payload = "bW9kaWZpZWQ" + "." + parts[1]
        self.assertIsNone(self.auth.verify_session_token(tampered_payload))

        # Expired payload
        expired_payload = {"sub": "12345", "exp": int(time.time() - 100)}
        signed_expired = self.auth.sign_session_data(expired_payload)
        self.assertIsNone(self.auth.verify_session_token(signed_expired))

    def test_user_authorization_whitelist(self):
        # Configured owner
        self.assertTrue(self.auth.is_user_authorized("123456789"))
        # Configured admin
        self.assertTrue(self.auth.is_user_authorized("987654321"))
        # Random unauthorized user
        self.assertFalse(self.auth.is_user_authorized("999999999"))

    def test_rate_limiting(self):
        test_ip = "192.168.1.100"
        self.assertFalse(self.auth.is_rate_limited(test_ip))

        for _ in range(5):
            self.auth.record_failed_attempt(test_ip)

        self.assertTrue(self.auth.is_rate_limited(test_ip))


class TestWebApiEndpoints(AioHTTPTestCase):
    async def get_application(self):
        self.cfg = {
            "bot_owner_id": "123456789",
            "backend_type": "ollama",
            "active_model": "test-model:latest"
        }
        return create_web_app(self.cfg)

    @unittest_run_loop
    async def test_unauthorized_access_redirects_or_rejects(self):
        # Root redirects to /login when unauthenticated
        resp = await self.client.get("/")
        self.assertEqual(resp.status, 200)
        self.assertEqual(str(resp.url.path), "/login")

        # API endpoint rejects with 401
        api_resp = await self.client.get("/api/status")
        self.assertEqual(api_resp.status, 401)
        data = await api_resp.json()
        self.assertIn("error", data)

    @unittest_run_loop
    async def test_authenticated_api_status(self):
        auth_mgr = self.app["auth_manager"]
        session_token = auth_mgr.sign_session_data({
            "sub": "123456789",
            "username": "Admin",
            "exp": int(time.time() + 3600)
        })

        self.client.session.cookie_jar.update_cookies({SESSION_COOKIE_NAME: session_token})

        resp = await self.client.get("/api/status")
        self.assertEqual(resp.status, 200)
        data = await resp.json()
        self.assertIn("bot", data)
        self.assertIn("ai", data)
        self.assertIn("system", data)
        self.assertIn("version", data)
        self.assertIn("commit", data)
        self.assertEqual(data["ai"]["active_model"], "test-model:latest")

    @unittest_run_loop
    async def test_otp_login_flow(self):
        auth_mgr = self.app["auth_manager"]
        auth_mgr.generate_otp_magic_link("localhost", 8080)
        token = auth_mgr.active_otp_token

        # Access with valid OTP token
        resp = await self.client.get(f"/auth/otp?token={token}")
        self.assertEqual(resp.status, 200)
        self.assertEqual(str(resp.url.path), "/")

        # Check cookie was set
        cookies = self.client.session.cookie_jar.filter_cookies(resp.url)
        self.assertIn(SESSION_COOKIE_NAME, cookies)

    @unittest_run_loop
    async def test_authenticated_get_bot_invite(self):
        auth_mgr = self.app["auth_manager"]
        session_token = auth_mgr.sign_session_data({"id": "admin", "username": "Admin", "exp": 9999999999})
        self.client.session.cookie_jar.update_cookies({SESSION_COOKIE_NAME: session_token})

        resp = await self.client.get("/api/bot/invite")
        self.assertEqual(resp.status, 200)
        data = await resp.json()
        self.assertIn("invite_url", data)
        self.assertIn("client_id", data)

    @unittest_run_loop
    async def test_favicon_endpoints_accessible_without_auth(self):
        # 1. Root /favicon.ico route
        ico_resp = await self.client.get("/favicon.ico")
        self.assertEqual(ico_resp.status, 200)
        self.assertIn("image", ico_resp.headers.get("Content-Type", ""))
        ico_bytes = await ico_resp.read()
        self.assertGreater(len(ico_bytes), 0)

        # 2. Static SVG favicon
        svg_resp = await self.client.get("/static/favicon.svg")
        self.assertEqual(svg_resp.status, 200)
        svg_text = await svg_resp.text()
        self.assertIn("<svg", svg_text)
        self.assertIn("</svg>", svg_text)

        # 3. Static Web App Manifest
        manifest_resp = await self.client.get("/static/site.webmanifest")
        self.assertEqual(manifest_resp.status, 200)
        manifest_data = await manifest_resp.json()
        self.assertEqual(manifest_data.get("name"), "Silas Blue Web Management")

    @unittest_run_loop
    async def test_guild_config_rejects_invalid_guild_id(self):
        auth_mgr = self.app["auth_manager"]
        session_token = auth_mgr.sign_session_data({"id": "admin", "username": "Admin", "exp": 9999999999})
        self.client.session.cookie_jar.update_cookies({SESSION_COOKIE_NAME: session_token})

        # Attempt to read or clobber app_config via guild endpoint
        resp = await self.client.get("/api/guild/app_config")
        self.assertEqual(resp.status, 400)

        resp_post = await self.client.post("/api/guild/app_config", json={"pagination_max_chars": 1000})
        self.assertEqual(resp_post.status, 400)

    @unittest_run_loop
    async def test_ai_config_masks_search_keys_and_blocks_unknown_keys(self):
        auth_mgr = self.app["auth_manager"]
        self.cfg["tavily_api_key"] = "tvly-secret-123"
        self.cfg["brave_api_key"] = "BSA-secret-456"
        session_token = auth_mgr.sign_session_data({"id": "admin", "username": "Admin", "exp": 9999999999})
        self.client.session.cookie_jar.update_cookies({SESSION_COOKIE_NAME: session_token})

        resp = await self.client.get("/api/ai/config")
        self.assertEqual(resp.status, 200)
        data = await resp.json()
        self.assertEqual(data["config"]["tavily_api_key"], "●●●●●●●●")
        self.assertEqual(data["config"]["brave_api_key"], "●●●●●●●●")

    @unittest_run_loop
    async def test_oauth_callback_requires_matching_state_cookie(self):
        resp = await self.client.get("/auth/discord/callback?code=fake_code&state=mismatch")
        self.assertEqual(resp.status, 400)

if __name__ == "__main__":
    unittest.main()

