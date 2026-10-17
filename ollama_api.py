"""
Ollama API integration for Silas Blue (Backwards-compatible wrapper).
Delegates to ai_providers.OllamaProvider with support for remote Ollama servers.
"""

from typing import Optional, Callable, List
from ai_providers.ollama_provider import OllamaProvider
from utils import load_app_config

class OllamaClient:
    def __init__(self, base_url: Optional[str] = None, api_key: Optional[str] = None):
        if base_url is None:
            config = load_app_config()
            base_url = config.get("ollama_url", "http://localhost:11434")
            if not api_key:
                api_key = config.get("api_key", "")
        self.provider = OllamaProvider(base_url=base_url, api_key=api_key)

    @property
    def base_url(self) -> str:
        return self.provider.base_url

    def send_prompt(self, prompt: str, model: str) -> str:
        return self.provider.send_prompt_sync(prompt, model)

    def list_models(self) -> List[str]:
        return self.provider.list_models_sync()

    def download_model(self, model_name: str, progress_callback: Optional[Callable[[int, str], None]] = None) -> bool:
        return self.provider.pull_model_sync(model_name, progress_callback)

    def status(self) -> bool:
        return self.provider.status_sync()

    def start(self) -> bool:
        return self.provider.start_local_service()

    def stop(self) -> bool:
        return self.provider.stop_local_service()

    def restart(self) -> bool:
        self.stop()
        return self.start()