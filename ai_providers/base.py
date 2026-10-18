"""
Abstract base class for Silas Blue AI Providers.
Supports Ollama, llama.cpp (llama-server), and OpenAI-compatible backends.
"""

from abc import ABC, abstractmethod
from typing import List, Optional, Callable, Dict, Any

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

    @abstractmethod
    async def send_prompt(
        self,
        prompt: str,
        model: str,
        system_prompt: Optional[str] = None,
        history: Optional[List[Dict[str, str]]] = None
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
