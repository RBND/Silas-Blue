"""
Llama.cpp AI Provider for Silas Blue.
Connects to llama-server (OpenAI-compatible HTTP API) or launches local llama-server binary.
Supports local and remote instances.
"""

import aiohttp
import asyncio
import json
import logging
import os
import subprocess
import time
from typing import List, Optional, Callable, Dict, Any
from urllib.parse import urlparse
from .base import AIProvider

logger = logging.getLogger("silasblue")

class LlamaCppProvider(AIProvider):
    def __init__(
        self,
        base_url: Optional[str] = None,
        api_key: Optional[str] = None,
        binary_path: Optional[str] = None,
        model_path: Optional[str] = None
    ):
        self._base_url = (base_url or "http://localhost:8080").rstrip("/")
        self._api_key = api_key or ""
        self._binary_path = binary_path or ""
        self._model_path = model_path or ""
        self._local_process: Optional[subprocess.Popen] = None

    @property
    def name(self) -> str:
        return "Llama.cpp"

    @property
    def base_url(self) -> str:
        return self._base_url

    def _get_headers(self) -> Dict[str, str]:
        headers = {"Content-Type": "application/json"}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        return headers

    async def send_prompt(
        self,
        prompt: str,
        model: str,
        system_prompt: Optional[str] = None,
        history: Optional[List[Dict[str, str]]] = None
    ) -> str:
        """
        Sends prompt to llama-server via /v1/chat/completions (OpenAI compatible).
        Falls back to /completion if /v1/chat/completions is not available.
        """
        chat_url = f"{self._base_url}/v1/chat/completions"
        messages = []
        if system_prompt:
            messages.append({"role": "system", "content": system_prompt})
        if history:
            messages.extend(history)
        messages.append({"role": "user", "content": prompt})

        payload = {
            "model": model or "default",
            "messages": messages,
            "stream": False,
            "temperature": 0.7
        }

        timeout = aiohttp.ClientTimeout(total=300)
        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.post(chat_url, json=payload, headers=self._get_headers()) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        choices = data.get("choices", [])
                        if choices:
                            return choices[0].get("message", {}).get("content", "")
                        return ""
                    elif resp.status == 404:
                        # Fallback to legacy /completion endpoint
                        legacy_url = f"{self._base_url}/completion"
                        full_prompt = f"System: {system_prompt}\nUser: {prompt}\nAssistant:" if system_prompt else prompt
                        legacy_payload = {"prompt": full_prompt, "n_predict": 1024, "stream": False}
                        async with session.post(legacy_url, json=legacy_payload, headers=self._get_headers()) as l_resp:
                            if l_resp.status == 200:
                                l_data = await l_resp.json()
                                return l_data.get("content", "")
                            else:
                                err_text = await l_resp.text()
                                return f"Llama.cpp Error ({l_resp.status}): {err_text}"
                    elif resp.status == 400:
                        text = await resp.text()
                        if "not found" in text.lower():
                            avail = await self.list_models()
                            if avail and avail[0] != model and avail[0] != "default":
                                logger.warning(f"Model '{model}' not found on Llama.cpp. Retrying with active model '{avail[0]}'...")
                                payload["model"] = avail[0]
                                async with session.post(chat_url, json=payload, headers=self._get_headers()) as retry_resp:
                                    if retry_resp.status == 200:
                                        r_data = await retry_resp.json()
                                        choices = r_data.get("choices", [])
                                        if choices:
                                            return choices[0].get("message", {}).get("content", "")
                                        return ""
                        logger.error(f"Llama.cpp error {resp.status}: {text}")
                        return f"Llama.cpp Error ({resp.status}): {text}"
                    else:
                        text = await resp.text()
                        logger.error(f"Llama.cpp error {resp.status}: {text}")
                        return f"Llama.cpp Error ({resp.status}): {text}"
        except asyncio.TimeoutError:
            logger.error("Llama.cpp request timed out after 300 seconds.")
            return "Error: Request to Llama.cpp timed out. The model may still be generating."
        except Exception as e:
            logger.error(f"Error communicating with Llama.cpp: {e}")
            return f"Error connecting to Llama.cpp ({self._base_url}): {e}"

    async def list_models(self) -> List[str]:
        """
        Retrieves models via /v1/models or checks /props.
        """
        url = f"{self._base_url}/v1/models"
        timeout = aiohttp.ClientTimeout(total=5)
        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.get(url, headers=self._get_headers()) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        models = [m.get("id") for m in data.get("data", []) if m.get("id")]
                        if models:
                            return models
        except Exception:
            pass

        # Check /props endpoint for active model file name
        props_url = f"{self._base_url}/props"
        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.get(props_url, headers=self._get_headers()) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        active_model = data.get("default_generation_settings", {}).get("model")
                        if active_model:
                            return [os.path.basename(active_model)]
        except Exception:
            pass

        # Check local detected models if none returned from server yet
        try:
            from .detector import detect_local_models
            local_models = detect_local_models()
            if local_models:
                return [os.path.basename(m) for m in local_models]
        except Exception:
            pass

        return ["default"]

    async def check_status(self) -> bool:
        """
        Checks health of llama-server via /health or /v1/models.
        """
        urls_to_try = [f"{self._base_url}/health", f"{self._base_url}/v1/models"]
        timeout = aiohttp.ClientTimeout(total=5)
        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                for url in urls_to_try:
                    try:
                        async with session.get(url, headers=self._get_headers()) as resp:
                            if resp.status in (200, 503):  # 503 means model is loading but server is up
                                return True
                    except Exception:
                        continue
            return False
        except Exception:
            return False

    async def pull_model(
        self,
        model_name: str,
        progress_callback: Optional[Callable[[int, str], None]] = None
    ) -> bool:
        """
        Llama-server typically loads local GGUF files rather than remote repo pulls.
        Informs the user to load or provide the GGUF file.
        """
        if progress_callback:
            progress_callback(0, "Llama.cpp models must be placed in models folder or loaded via llama-server -m <file.gguf>")
        return True

    def start_local_service(self) -> bool:
        """
        Optionally launches local llama-server if binary_path and model_path are configured.
        Auto-detects llama executable if binary_path is not specified.
        Supports both llama-server and unified llama CLI (llama.exe serve).
        """
        binary = self._binary_path
        if not binary or not os.path.exists(binary):
            from .detector import detect_installed_llamacpp
            detected = detect_installed_llamacpp()
            if detected:
                binary = detected
                self._binary_path = detected
            else:
                logger.warning("llama executable not specified or file not found.")
                return False

        parsed = urlparse(self._base_url)
        port = str(parsed.port or 8080)
        host = parsed.hostname or "127.0.0.1"

        binary_name = os.path.splitext(os.path.basename(binary))[0].lower()
        is_unified_cli = binary_name in ("llama", "llama-cli")

        cmd = [binary]
        if is_unified_cli:
            cmd.append("serve")

        if self._model_path and os.path.exists(self._model_path):
            cmd.extend(["-m", self._model_path])
        elif not is_unified_cli:
            logger.warning("llama.cpp model path not specified or file not found.")
            return False

        cmd.extend(["--host", host, "--port", port])

        try:
            logger.info(f"Starting local llama server: {' '.join(cmd)}")
            self._local_process = subprocess.Popen(
                cmd,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL
            )
            time.sleep(2)
            return True
        except Exception as e:
            logger.error(f"Failed to start local llama server: {e}")
            return False

    def stop_local_service(self) -> bool:
        """Stops the locally spawned llama-server process."""
        if self._local_process:
            try:
                self._local_process.terminate()
                self._local_process.wait(timeout=5)
            except Exception:
                pass
            self._local_process = None
            return True
        return False

    # --- Synchronous compatibility helpers ---
    def status_sync(self) -> bool:
        try:
            return asyncio.run(self.check_status())
        except Exception:
            return False

    def list_models_sync(self) -> List[str]:
        try:
            return asyncio.run(self.list_models())
        except Exception:
            return ["default"]

    def send_prompt_sync(self, prompt: str, model: str) -> str:
        try:
            return asyncio.run(self.send_prompt(prompt, model))
        except Exception as e:
            return f"Error: {e}"
