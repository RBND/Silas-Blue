//! Startup AI backend verification and local service probing.
//! Strict Remote Guard: Never overwrites user-configured remote cloud APIs or explicit backend settings.

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use silas_core::config::AppSettings;

use crate::{llamacpp::LlamaCppProvider, ollama::OllamaProvider, LlmProvider, ProviderManager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalProviderProbe {
    pub ollama_online: bool,
    pub ollama_url: String,
    pub llamacpp_online: bool,
    pub llamacpp_url: String,
}

/// Probes local Ollama (`127.0.0.1:11434`) and Llama.cpp (`127.0.0.1:8081`) concurrently.
pub async fn probe_local_providers(settings: &AppSettings) -> LocalProviderProbe {
    let ollama = OllamaProvider::new(&settings.ollama_url, &settings.local_api_key);
    let llama = LlamaCppProvider::new(&settings.llamacpp_url, &settings.local_api_key);

    let (ollama_online, llamacpp_online) =
        tokio::join!(ollama.check_status(), llama.check_status());

    LocalProviderProbe {
        ollama_online,
        ollama_url: ollama.base_url().to_owned(),
        llamacpp_online,
        llamacpp_url: llama.base_url().to_owned(),
    }
}

/// Verifies the active AI provider on startup without overwriting configured remote cloud providers.
pub async fn verify_active_provider_on_startup(
    provider_mgr: &ProviderManager,
    settings: &AppSettings,
) {
    let current_backend = settings.backend_type.trim().to_ascii_lowercase();
    let provider = provider_mgr.get();

    // 1. Remote cloud providers: check reachability, NEVER hijack or overwrite with local backends
    if matches!(
        current_backend.as_str(),
        "remote" | "remote_openai" | "openai" | "remote_api"
    ) || !provider.is_local()
    {
        if provider.check_status().await {
            info!(
                provider = provider.name(),
                url = provider.base_url(),
                "Remote AI provider is online and responsive"
            );
        } else {
            warn!(
                provider = provider.name(),
                url = provider.base_url(),
                "Configured remote AI provider is not responding; retaining remote configuration"
            );
        }
        return;
    }

    // 2. Configured local provider
    if provider.check_status().await {
        info!(
            provider = provider.name(),
            url = provider.base_url(),
            "Local AI provider is online and responsive"
        );
    } else {
        warn!(
            provider = provider.name(),
            url = provider.base_url(),
            "Configured local AI provider is currently offline; start Ollama/llama-server or switch backends in the Web Console"
        );
    }
}
