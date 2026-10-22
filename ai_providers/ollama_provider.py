"""
Ollama AI Provider for Silas Blue.
Provides async and sync interaction with local and remote Ollama instances.
Supports remote model pulling via the Ollama REST API.
"""

import aiohttp
import asyncio
import json
import logging
import os
import subprocess
import time
from typing import List, Optional, Callable, Dict, Any
from .base import AIProvider

logger = logging.getLogger("silasblue")

class OllamaProvider(AIProvider):
    def __init__(self, base_url: Optional[str] = None, api_key: Optional[str] = None):
        self._base_url = (base_url or "http://localhost:11434").rstrip("/")
        self._api_key = api_key or ""
        self._local_process: Optional[subprocess.Popen] = None

    @property
    def name(self) -> str:
        return "Ollama"

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
        history: Optional[List[Dict[str, str]]] = None,
        max_tokens: Optional[int] = None
    ) -> str:
        """
        Sends a prompt to Ollama asynchronously.
        Uses /api/chat if history is provided, otherwise /api/generate.
        """
        if history:
            url = f"{self._base_url}/api/chat"
            messages = []
            if system_prompt:
                messages.append({"role": "system", "content": system_prompt})
            messages.extend(history)
            messages.append({"role": "user", "content": prompt})
            payload = {
                "model": model,
                "messages": messages,
                "stream": False
            }
        else:
            url = f"{self._base_url}/api/generate"
            payload = {
                "model": model,
                "prompt": prompt,
                "stream": False
            }
            if system_prompt:
                payload["system"] = system_prompt

        if max_tokens is not None and max_tokens > 0:
            payload["options"] = {"num_predict": int(max_tokens)}

        timeout = aiohttp.ClientTimeout(total=300)
        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.post(url, json=payload, headers=self._get_headers()) as response:
                    if response.status == 200:
                        data = await response.json()
                        p_tok = int(data.get("prompt_eval_count", 0))
                        c_tok = int(data.get("eval_count", 0))
                        if history:
                            content = data.get("message", {}).get("content", "")
                        else:
                            content = data.get("response", "")
                        self._record_or_estimate_usage(prompt, content, p_tok, c_tok)
                        return content
                    else:
                        text = await response.text()
                        logger.error(f"Ollama error {response.status}: {text}")
                        return f"Ollama Error ({response.status}): {text}"
        except asyncio.TimeoutError:
            logger.error("Ollama request timed out after 300 seconds.")
            return "Error: AI request timed out. The model may still be loading or generating."
        except Exception as e:
            logger.error(f"Error communicating with Ollama: {e}")
            return f"Error connecting to Ollama ({self._base_url}): {e}"

    async def list_models(self) -> List[str]:
        """
        Retrieves list of available models from /api/tags asynchronously.
        """
        url = f"{self._base_url}/api/tags"
        timeout = aiohttp.ClientTimeout(total=10)
        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.get(url, headers=self._get_headers()) as response:
                    if response.status == 200:
                        data = await response.json()
                        return [m["name"] for m in data.get("models", [])]
                    return []
        except Exception as e:
            logger.debug(f"Failed to list Ollama models: {e}")
            return []

    async def check_status(self) -> bool:
        """
        Checks if Ollama is running and responsive via /api/tags.
        """
        url = f"{self._base_url}/api/tags"
        timeout = aiohttp.ClientTimeout(total=5)
        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.get(url, headers=self._get_headers()) as response:
                    return response.status == 200
        except Exception:
            return False

    async def pull_model(
        self,
        model_name: str,
        progress_callback: Optional[Callable[[int, str], None]] = None
    ) -> bool:
        """
        Downloads a model using Ollama's REST API (/api/pull).
        Works seamlessly for both local and remote Ollama servers.
        Streams progress updates back to progress_callback(percent, speed/status).
        """
        url = f"{self._base_url}/api/pull"
        payload = {"model": model_name, "stream": True}
        timeout = aiohttp.ClientTimeout(total=3600)  # Models can take long to download
        start_time = time.time()
        logger.info(f"Starting model pull for '{model_name}' from {self._base_url}")

        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.post(url, json=payload, headers=self._get_headers()) as response:
                    if response.status != 200:
                        err_text = await response.text()
                        logger.error(f"Failed to initiate pull for {model_name}: {err_text}")
                        if progress_callback:
                            progress_callback(-1, f"HTTP {response.status}: {err_text}")
                        return False

                    async for line in response.content:
                        if not line:
                            continue
                        try:
                            msg = json.loads(line.decode("utf-8"))
                            status = msg.get("status", "")
                            total = msg.get("total", 0)
                            completed = msg.get("completed", 0)

                            if total > 0 and progress_callback:
                                percent = int((completed / total) * 100)
                                elapsed = time.time() - start_time
                                speed_mb = (completed / (1024 * 1024)) / elapsed if elapsed > 0 else 0
                                progress_callback(percent, f"{percent}% ({speed_mb:.1f} MB/s) - {status}")
                            elif progress_callback:
                                progress_callback(0, status)

                            if status == "success":
                                logger.info(f"Successfully downloaded model: {model_name}")
                                if progress_callback:
                                    progress_callback(100, "Done")
                                return True
                        except Exception:
                            continue

            if progress_callback:
                progress_callback(100, "Done")
            return True
        except Exception as e:
            logger.error(f"Exception during model pull '{model_name}': {e}")
            if progress_callback:
                progress_callback(-1, str(e))
            return False

    def start_local_service(self) -> bool:
        """Starts Ollama locally if base_url is localhost."""
        if not self.is_local():
            logger.info("Cannot start local service: configured for remote server.")
            return False
        if self.status_sync():
            return True
        try:
            self._local_process = subprocess.Popen(
                ["ollama", "serve"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL
            )
            time.sleep(1)
            return True
        except Exception as e:
            logger.error(f"Failed to start local Ollama: {e}")
            return False

    def stop_local_service(self) -> bool:
        """Stops the locally spawned Ollama subprocess if running."""
        if self._local_process:
            try:
                self._local_process.terminate()
                self._local_process.wait(timeout=5)
            except Exception:
                pass
            self._local_process = None
            return True
        return False
