"""
Abstract base class for Silas Blue AI Providers.
Supports Ollama, llama.cpp (llama-server), and OpenAI-compatible backends.
"""

from abc import ABC, abstractmethod
import asyncio
import logging
from typing import List, Optional, Callable, Dict, Any

logger = logging.getLogger("silasblue")

class AIProvider(ABC):
    @property
    @abstractmethod
    def name(self) -> str:
        """Name of the provider (e.g. 'Ollama', 'Llama.cpp')."""
        pass

    @property
    @abstractmethod
    def base_url(self) -> str:
        """Base endpoint URL."""
        pass

    @property
    def last_usage(self) -> Dict[str, int]:
        """Returns the token usage dict of the last request."""
        return getattr(self, "_last_usage", {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0})

    def _record_or_estimate_usage(
        self,
        prompt: str,
        content: str,
        prompt_tokens: int = 0,
        completion_tokens: int = 0,
        total_tokens: int = 0
    ) -> Dict[str, int]:
        """Records token usage from backend response, falling back to character-based estimation if 0."""
        p_tok = int(prompt_tokens or 0)
        c_tok = int(completion_tokens or 0)
        t_tok = int(total_tokens or (p_tok + c_tok))
        if t_tok == 0:
            p_tok = max(1, len(prompt) // 4)
            c_tok = max(1, len(content) // 4)
            t_tok = p_tok + c_tok
        self._last_usage = {
            "prompt_tokens": p_tok,
            "completion_tokens": c_tok,
            "total_tokens": t_tok
        }
        return self._last_usage

    @abstractmethod
    async def send_prompt(
        self,
        prompt: str,
        model: str,
        system_prompt: Optional[str] = None,
        history: Optional[List[Dict[str, str]]] = None,
        max_tokens: Optional[int] = None
    ) -> str:
        """
        Sends a prompt to the AI backend and returns the text response.
        """
        pass

    @abstractmethod
    async def list_models(self) -> List[str]:
        """
        Returns a list of available model names.
        """
        pass

    @abstractmethod
    async def check_status(self) -> bool:
        """
        Returns True if the backend is reachable and responsive, False otherwise.
        """
        pass

    @abstractmethod
    async def pull_model(
        self,
        model_name: str,
        progress_callback: Optional[Callable[[int, str], None]] = None
    ) -> bool:
        """
        Pulls or prepares a model.
        progress_callback: fn(percent: int, status_or_speed: str)
        """
        pass

    def start_local_service(self) -> bool:
        """Optional: Start a local server process if supported."""
        return False

    def stop_local_service(self) -> bool:
        """Optional: Stop a local server process if started by this provider."""
        return False

    def is_local(self) -> bool:
        """Returns True if the configured base_url points to localhost/127.0.0.1."""
        url = self.base_url.lower()
        return "localhost" in url or "127.0.0.1" in url or "0.0.0.0" in url

    def _fallback_models_sync(self) -> List[str]:
        """Fallback model list if list_models_sync encounters an exception."""
        return []

    # --- Synchronous compatibility helpers for GUI background threads ---
    def status_sync(self) -> bool:
        try:
            return asyncio.run(self.check_status())
        except Exception:
            return False

    def list_models_sync(self) -> List[str]:
        try:
            return asyncio.run(self.list_models())
        except Exception:
            return self._fallback_models_sync()

    def send_prompt_sync(self, prompt: str, model: str, system_prompt: Optional[str] = None) -> str:
        try:
            return asyncio.run(self.send_prompt(prompt, model, system_prompt=system_prompt))
        except Exception as e:
            return f"Error: {e}"

    def pull_model_sync(self, model_name: str, progress_callback: Optional[Callable[[int, str], None]] = None) -> bool:
        try:
            return asyncio.run(self.pull_model(model_name, progress_callback))
        except Exception as e:
            logger.error(f"Sync pull failed: {e}")
            return False
