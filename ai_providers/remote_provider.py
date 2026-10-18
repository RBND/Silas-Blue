"""
Remote OpenAI-Compatible AI Provider for Silas Blue.
Supports cloud and remote LLM APIs such as Xiaomi MiMo, OpenAI, OpenRouter, Groq, DeepSeek, and xAI.
Uses lightweight async HTTP (aiohttp) with Bearer token authentication and standard /v1 endpoints.
"""

import aiohttp
import asyncio
import json
import logging
from typing import List, Optional, Callable, Dict, Any, Tuple
from urllib.parse import urlparse
from .base import AIProvider

logger = logging.getLogger("silasblue")

REMOTE_PRESETS: Dict[str, Dict[str, Any]] = {
    "Xiaomi MiMo": {
        "base_url": "https://api.xiaomimimo.com/v1",
        "default_model": "mimo-v2.6-pro",
        "models": ["mimo-v2.6-pro", "mimo-v2.6-flash"],
    },
    "OpenAI": {
        "base_url": "https://api.openai.com/v1",
        "default_model": "gpt-4o-mini",
        "models": ["gpt-4o-mini", "gpt-4o", "o1-mini", "gpt-3.5-turbo"],
    },
    "OpenRouter": {
        "base_url": "https://openrouter.ai/api/v1",
        "default_model": "anthropic/claude-3.5-sonnet",
        "models": ["anthropic/claude-3.5-sonnet", "openai/gpt-4o-mini", "meta-llama/llama-3.3-70b-instruct"],
    },
    "Groq": {
        "base_url": "https://api.groq.com/openai/v1",
        "default_model": "llama-3.3-70b-versatile",
        "models": ["llama-3.3-70b-versatile", "llama-3.1-8b-instant", "mixtral-8x7b-32768"],
    },
    "DeepSeek": {
        "base_url": "https://api.deepseek.com/v1",
        "default_model": "deepseek-chat",
        "models": ["deepseek-chat", "deepseek-reasoner"],
    },
    "xAI (Grok)": {
        "base_url": "https://api.x.ai/v1",
        "default_model": "grok-2-latest",
        "models": ["grok-2-latest", "grok-2-mini"],
    },
    "Custom": {
        "base_url": "https://api.openai.com/v1",
        "default_model": "",
        "models": [],
    }
}


class RemoteOpenAIProvider(AIProvider):
    def __init__(
        self,
        base_url: Optional[str] = None,
        api_key: Optional[str] = None,
        default_model: Optional[str] = None,
        preset: Optional[str] = None
    ):
        raw_url = (base_url or "https://api.xiaomimimo.com/v1").strip().rstrip("/")
        self._base_url = raw_url
        self._api_key = (api_key or "").strip()
        self._default_model = (default_model or "").strip()
        self._preset = preset or "Remote API"

    @property
    def name(self) -> str:
        if self._preset and self._preset != "Custom":
            return f"Remote ({self._preset})"
        return "Remote OpenAI API"

    @property
    def base_url(self) -> str:
        return self._base_url

    @property
    def preset(self) -> str:
        return self._preset

    @property
    def default_model(self) -> str:
        return self._default_model

    def _get_chat_url(self) -> str:
        base = self._base_url.rstrip("/")
        if base.endswith("/v1"):
            return f"{base}/chat/completions"
        return f"{base}/v1/chat/completions"

    def _get_models_url(self) -> str:
        base = self._base_url.rstrip("/")
        if base.endswith("/v1"):
            return f"{base}/models"
        return f"{base}/v1/models"

    def _get_headers(self) -> Dict[str, str]:
        headers = {
            "Content-Type": "application/json",
            "User-Agent": "SilasBlue/1.0"
        }
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        return headers

    async def send_prompt(
        self,
        prompt: str,
        model: str,
        system_prompt: Optional[str] = None
    ) -> str:
        """
        Sends a prompt to the OpenAI-compatible chat completions endpoint asynchronously.
        """
        target_model = model or self._default_model or "mimo-v2.6-pro"
        messages = []
        if system_prompt:
            messages.append({"role": "system", "content": system_prompt})
        messages.append({"role": "user", "content": prompt})

        payload = {
            "model": target_model,
            "messages": messages,
            "stream": False,
            "temperature": 0.7
        }

        timeout = aiohttp.ClientTimeout(total=300)
        chat_url = self._get_chat_url()

        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.post(chat_url, json=payload, headers=self._get_headers()) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        choices = data.get("choices", [])
                        if choices:
                            message = choices[0].get("message", {})
                            content = message.get("content", "")
                            if content:
                                return content
                        return "No response content received from remote model."
                    else:
                        try:
                            err_data = await resp.json()
                            err_msg = err_data.get("error", {}).get("message", await resp.text())
                        except Exception:
                            err_msg = await resp.text()

                        logger.error(f"Remote API error {resp.status}: {err_msg}")
                        if resp.status == 401:
                            return f"Remote API Error 401: Unauthorized. Please check your API key for {self.name}."
                        elif resp.status == 429:
                            return f"Remote API Error 429: Rate limit or quota exceeded ({err_msg})."
                        return f"Remote API Error ({resp.status}): {err_msg}"
        except asyncio.TimeoutError:
            logger.error(f"Remote API request to {chat_url} timed out after 300 seconds.")
            return "Error: Remote AI request timed out. The provider may be experiencing high load."
        except Exception as e:
            logger.error(f"Error communicating with Remote API at {chat_url}: {e}")
            return f"Error connecting to Remote API ({self._base_url}): {e}"

    async def list_models(self) -> List[str]:
        """
        Retrieves list of models from /v1/models if supported.
        Falls back to configured default model or preset suggested models.
        """
        models_url = self._get_models_url()
        timeout = aiohttp.ClientTimeout(total=10)
        retrieved: List[str] = []

        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.get(models_url, headers=self._get_headers()) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        raw_list = data.get("data", [])
                        for item in raw_list:
                            mid = item.get("id")
                            if mid and isinstance(mid, str):
                                retrieved.append(mid)
        except Exception as e:
            logger.debug(f"Could not fetch models from {models_url}: {e}")

        # If retrieved models exist, return them
        if retrieved:
            # If default_model is specified and not at top, prioritize it
            if self._default_model and self._default_model in retrieved:
                retrieved.remove(self._default_model)
                retrieved.insert(0, self._default_model)
            return retrieved

        # Fallback to configured model or preset defaults
        fallbacks: List[str] = []
        if self._default_model:
            fallbacks.append(self._default_model)

        preset_data = REMOTE_PRESETS.get(self._preset, {})
        for m in preset_data.get("models", []):
            if m not in fallbacks:
                fallbacks.append(m)

        if not fallbacks:
            fallbacks = ["mimo-v2.6-pro", "gpt-4o-mini"]

        return fallbacks

    async def check_status(self) -> bool:
        """
        Checks if the remote endpoint is reachable and responsive.
        """
        success, _ = await self.test_connection()
        return success

    async def test_connection(self) -> Tuple[bool, str]:
        """
        Performs a diagnostic connection test against the remote endpoint.
        Returns (success: bool, status_message: str).
        """
        models_url = self._get_models_url()
        timeout = aiohttp.ClientTimeout(total=10)

        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.get(models_url, headers=self._get_headers()) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        count = len(data.get("data", []))
                        model_info = f" ({count} models available)" if count > 0 else ""
                        return True, f"Connection successful! HTTP 200 OK{model_info}"
                    elif resp.status == 401:
                        return False, "Authentication Failed (HTTP 401): Invalid API Key."
                    elif resp.status == 403:
                        return False, "Access Forbidden (HTTP 403): Check permissions or IP whitelist."
                    elif resp.status == 404:
                        # Some endpoints don't support GET /models but accept POST /chat/completions
                        return True, "Endpoint reachable (HTTP 404 on /models; endpoint is online)."
                    else:
                        text = await resp.text()
                        return False, f"Server responded with HTTP {resp.status}: {text[:120]}"
        except aiohttp.ClientConnectorError as e:
            return False, f"Failed to connect: Cannot reach host ({e})"
        except asyncio.TimeoutError:
            return False, "Connection timed out after 10 seconds."
        except Exception as e:
            return False, f"Connection error: {e}"

    async def pull_model(
        self,
        model_name: str,
        progress_callback: Optional[Callable[[int, str], None]] = None
    ) -> bool:
        """
        Remote cloud models are hosted remotely and do not need local downloading.
        """
        if progress_callback:
            progress_callback(100, "Remote model is hosted in the cloud and ready to use.")
        return True

    def is_local(self) -> bool:
        url = self._base_url.lower()
        return "localhost" in url or "127.0.0.1" in url or "0.0.0.0" in url

    # --- Synchronous compatibility wrappers for PySide6 GUI background threads ---
    def status_sync(self) -> bool:
        try:
            return asyncio.run(self.check_status())
        except Exception:
            return False

    def list_models_sync(self) -> List[str]:
        try:
            return asyncio.run(self.list_models())
        except Exception:
            if self._default_model:
                return [self._default_model]
            return ["mimo-v2.6-pro"]

    def send_prompt_sync(self, prompt: str, model: str, system_prompt: Optional[str] = None) -> str:
        try:
            return asyncio.run(self.send_prompt(prompt, model, system_prompt))
        except Exception as e:
            return f"Error: {e}"

    def test_connection_sync(self) -> Tuple[bool, str]:
        try:
            return asyncio.run(self.test_connection())
        except Exception as e:
            return False, f"Test error: {e}"
