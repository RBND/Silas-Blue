"""
AI Providers package for Silas Blue.
Exports AIProvider interface, OllamaProvider, LlamaCppProvider, and provider factory.
"""

from typing import Dict, Any, Optional
from .base import AIProvider
from .ollama_provider import OllamaProvider
from .llamacpp_provider import LlamaCppProvider

def get_ai_provider(config: Optional[Dict[str, Any]] = None) -> AIProvider:
    """
    Factory function returning the configured AIProvider instance.
    """
    if config is None:
        from utils import load_app_config
        config = load_app_config()

    backend_type = config.get("backend_type", "ollama").lower()
    api_key = config.get("api_key", "")

    if backend_type in ("llamacpp", "llama.cpp", "llama_cpp"):
        base_url = config.get("llamacpp_url", "http://localhost:8080")
        binary_path = config.get("llamacpp_binary_path", "")
        model_path = config.get("llamacpp_model_path", "")
        return LlamaCppProvider(
            base_url=base_url,
            api_key=api_key,
            binary_path=binary_path,
            model_path=model_path
        )
    else:
        base_url = config.get("ollama_url", "http://localhost:11434")
        return OllamaProvider(
            base_url=base_url,
            api_key=api_key
        )

__all__ = ["AIProvider", "OllamaProvider", "LlamaCppProvider", "get_ai_provider"]
