import unittest
import asyncio
from unittest.mock import patch, MagicMock, AsyncMock
from ai_providers import get_ai_provider, RemoteOpenAIProvider, REMOTE_PRESETS

def make_mock_session(mock_resp, method="post"):
    mock_ctx = MagicMock()
    mock_ctx.__aenter__ = AsyncMock(return_value=mock_resp)
    mock_ctx.__aexit__ = AsyncMock(return_value=None)

    mock_session = MagicMock()
    mock_session.__aenter__ = AsyncMock(return_value=mock_session)
    mock_session.__aexit__ = AsyncMock(return_value=None)
    if method == "post":
        mock_session.post = MagicMock(return_value=mock_ctx)
    else:
        mock_session.get = MagicMock(return_value=mock_ctx)
    return mock_session

class TestRemoteProvider(unittest.TestCase):
    def test_presets_exist(self):
        self.assertIn("Xiaomi MiMo", REMOTE_PRESETS)
        self.assertIn("OpenAI", REMOTE_PRESETS)
        self.assertIn("OpenRouter", REMOTE_PRESETS)
        self.assertIn("Groq", REMOTE_PRESETS)
        self.assertIn("DeepSeek", REMOTE_PRESETS)
        self.assertIn("xAI (Grok)", REMOTE_PRESETS)
        self.assertEqual(REMOTE_PRESETS["Xiaomi MiMo"]["base_url"], "https://api.xiaomimimo.com/v1")
        self.assertEqual(REMOTE_PRESETS["Xiaomi MiMo"]["default_model"], "mimo-v2.6-pro")

    def test_provider_initialization_urls(self):
        # Base with /v1
        p1 = RemoteOpenAIProvider("https://api.xiaomimimo.com/v1", api_key="test-key", preset="Xiaomi MiMo")
        self.assertEqual(p1._get_chat_url(), "https://api.xiaomimimo.com/v1/chat/completions")
        self.assertEqual(p1._get_models_url(), "https://api.xiaomimimo.com/v1/models")
        self.assertEqual(p1.name, "Remote (Xiaomi MiMo)")
        self.assertFalse(p1.is_local())

        # Base without /v1
        p2 = RemoteOpenAIProvider("https://api.custom.com", api_key="custom-key", preset="Custom")
        self.assertEqual(p2._get_chat_url(), "https://api.custom.com/v1/chat/completions")
        self.assertEqual(p2._get_models_url(), "https://api.custom.com/v1/models")
        self.assertEqual(p2.name, "Remote OpenAI API")

    def test_auth_headers(self):
        p = RemoteOpenAIProvider("https://api.xiaomimimo.com/v1", api_key="miMo-sk-12345")
        headers = p._get_headers()
        self.assertEqual(headers["Authorization"], "Bearer miMo-sk-12345")
        self.assertEqual(headers["Content-Type"], "application/json")

    def test_factory_remote_provider(self):
        config = {
            "backend_type": "remote_openai",
            "remote_preset": "Xiaomi MiMo",
            "remote_base_url": "https://api.xiaomimimo.com/v1",
            "remote_api_key": "test-key-abc",
            "remote_model": "mimo-v2.6-flash"
        }
        provider = get_ai_provider(config)
        self.assertIsInstance(provider, RemoteOpenAIProvider)
        self.assertEqual(provider.base_url, "https://api.xiaomimimo.com/v1")
        self.assertEqual(provider.name, "Remote (Xiaomi MiMo)")
        self.assertEqual(provider.default_model, "mimo-v2.6-flash")

    def test_send_prompt_success(self):
        p = RemoteOpenAIProvider("https://api.xiaomimimo.com/v1", api_key="key", default_model="mimo-v2.6-pro")
        mock_response_data = {
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "Hello from Xiaomi MiMo!"
                    }
                }
            ]
        }

        async def run_test():
            mock_resp = MagicMock()
            mock_resp.status = 200
            mock_resp.json = AsyncMock(return_value=mock_response_data)
            mock_session = make_mock_session(mock_resp, method="post")

            with patch("aiohttp.ClientSession", return_value=mock_session):
                result = await p.send_prompt("Hello", "mimo-v2.6-pro", system_prompt="Be concise.")
                self.assertEqual(result, "Hello from Xiaomi MiMo!")

        asyncio.run(run_test())

    def test_send_prompt_unauthorized(self):
        p = RemoteOpenAIProvider("https://api.xiaomimimo.com/v1", api_key="bad-key")

        async def run_test():
            mock_resp = MagicMock()
            mock_resp.status = 401
            mock_resp.json = AsyncMock(return_value={"error": {"message": "Invalid API key"}})
            mock_resp.text = AsyncMock(return_value="Invalid API key")
            mock_session = make_mock_session(mock_resp, method="post")

            with patch("aiohttp.ClientSession", return_value=mock_session):
                result = await p.send_prompt("Hello", "mimo-v2.6-pro")
                self.assertIn("401", result)
                self.assertIn("Unauthorized", result)

        asyncio.run(run_test())

    def test_list_models_fallback(self):
        p = RemoteOpenAIProvider("https://api.xiaomimimo.com/v1", api_key="key", default_model="mimo-v2.6-pro", preset="Xiaomi MiMo")

        async def run_test():
            mock_resp = MagicMock()
            mock_resp.status = 404
            mock_session = make_mock_session(mock_resp, method="get")

            with patch("aiohttp.ClientSession", return_value=mock_session):
                models = await p.list_models()
                self.assertIn("mimo-v2.6-pro", models)
                self.assertIn("mimo-v2.6-flash", models)

        asyncio.run(run_test())

    def test_test_connection_success(self):
        p = RemoteOpenAIProvider("https://api.xiaomimimo.com/v1", api_key="valid-key")

        async def run_test():
            mock_resp = MagicMock()
            mock_resp.status = 200
            mock_resp.json = AsyncMock(return_value={"data": [{"id": "mimo-v2.6-pro"}]})
            mock_session = make_mock_session(mock_resp, method="get")

            with patch("aiohttp.ClientSession", return_value=mock_session):
                success, msg = await p.test_connection()
                self.assertTrue(success)
                self.assertIn("200 OK", msg)

        asyncio.run(run_test())

    def test_test_connection_auth_failure(self):
        p = RemoteOpenAIProvider("https://api.xiaomimimo.com/v1", api_key="invalid-key")

        async def run_test():
            mock_resp = MagicMock()
            mock_resp.status = 401
            mock_session = make_mock_session(mock_resp, method="get")

            with patch("aiohttp.ClientSession", return_value=mock_session):
                success, msg = await p.test_connection()
                self.assertFalse(success)
                self.assertIn("401", msg)

        asyncio.run(run_test())

if __name__ == "__main__":
    unittest.main()
