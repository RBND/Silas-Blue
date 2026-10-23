//! Unified LLM inference providers (`Ollama`, `Llama.cpp`, and `OpenAI-Compatible Remote APIs`),
//! shared connection-pooled HTTP clients (`redirect::Policy::none()`), SSRF validation,
//! and 30-second TTL model list caching.

pub mod detector;
pub mod llamacpp;
pub mod ollama;
pub mod remote;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use async_trait::async_trait;
use parking_lot::RwLock;
use reqwest::{redirect, Client};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use silas_core::config::AppSettings;
use silas_core::memory::ChatMessage;
use silas_core::text::char_count;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("Invalid or blocked endpoint URL: {0}")]
    InvalidUrl(String),
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Provider returned error ({status}): {message}")]
    ApiError { status: u16, message: String },
    #[error("Provider offline or unreachable: {0}")]
    Offline(String),
}

/// Shared fast HTTP client (12s total timeout, `redirect::Policy::none()`, pooled keep-alive)
/// used for `/api/tags`, `/v1/models`, `/health`, OAuth2, and web search.
#[allow(clippy::expect_used)]
static FAST_HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .redirect(redirect::Policy::none())
        .pool_max_idle_per_host(8)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(60))
        .user_agent("SilasBlue/4.0")
        .build()
        .expect("build fast_http_client")
});

/// Shared inference HTTP client (10s connect timeout, 180s per-chunk read timeout, `redirect::Policy::none()`)
/// used for LLM completions and streaming model pulls.
#[allow(clippy::expect_used)]
static INFERENCE_HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(180))
        .redirect(redirect::Policy::none())
        .pool_max_idle_per_host(4)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(60))
        .user_agent("SilasBlue/4.0")
        .build()
        .expect("build inference_http_client")
});

pub fn fast_http_client() -> &'static Client {
    &FAST_HTTP_CLIENT
}

pub fn inference_http_client() -> &'static Client {
    &INFERENCE_HTTP_CLIENT
}

/// Validates an AI provider endpoint URL against SSRF rules (`HIGH-02`):
/// - Must parse as a valid `http://` or `https://` URL with no embedded credentials.
/// - Always blocks cloud instance metadata / link-local (`169.254.0.0/16`, `fe80::/10`, `fd00:ec2::254`),
///   broadcast, unspecified (`0.0.0.0` as target client host is normalized to `127.0.0.1` only if local), and multicast IPs.
/// - If `is_remote_cloud` is true, also rejects loopback (`127.0.0.0/8`, `::1`, `localhost`) unless `allow_local_remote` is true.
pub fn validate_endpoint_url(
    raw_url: &str,
    is_remote_cloud: bool,
    allow_local_remote: bool,
) -> Result<Url, AiError> {
    let trimmed = raw_url.trim();
    let parsed = Url::parse(trimmed).map_err(|e| AiError::InvalidUrl(e.to_string()))?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AiError::InvalidUrl(format!(
            "Unsupported URL scheme '{}' (only http and https are permitted)",
            parsed.scheme()
        )));
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AiError::InvalidUrl(
            "Embedded credentials in endpoint URLs are forbidden".to_owned(),
        ));
    }

    let Some(host_str) = parsed.host_str() else {
        return Err(AiError::InvalidUrl("URL is missing a host".to_owned()));
    };

    let host_lower = host_str.trim_matches(&['[', ']'][..]).to_ascii_lowercase();
    if host_lower == "metadata.google.internal" || host_lower == "instance-data" {
        return Err(AiError::InvalidUrl(
            "Cloud metadata hostnames are blocked by SSRF policy".to_owned(),
        ));
    }

    if is_remote_cloud && !allow_local_remote && host_lower == "localhost" {
        return Err(AiError::InvalidUrl(
            "Remote cloud API URL cannot target localhost (use Ollama or Llama.cpp backend for local models)"
                .to_owned(),
        ));
    }

    if let Ok(ip) = host_lower.parse::<IpAddr>() {
        if is_blocked_metadata_or_special_ip(ip) {
            return Err(AiError::InvalidUrl(format!(
                "IP address {ip} is blocked by SSRF security policy"
            )));
        }
        if is_remote_cloud && !allow_local_remote && ip.is_loopback() {
            return Err(AiError::InvalidUrl(
                "Remote cloud API URL cannot target loopback IP".to_owned(),
            ));
        }
    }

    Ok(parsed)
}

fn is_blocked_metadata_or_special_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_link_local() // 169.254.0.0/16 (AWS/GCP/Azure IMDS)
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4 == Ipv4Addr::new(100, 100, 100, 200) // Alibaba Cloud metadata
        }
        IpAddr::V6(v6) => {
            v6.is_multicast()
                || v6.is_unspecified()
                || (v6.segments().first().copied().unwrap_or(0) & 0xffc0 == 0xfe80) // fe80::/10 link-local
                || v6 == Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254) // AWS IPv6 IMDS
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

impl TokenUsage {
    /// Returns exact token counts from the backend, or estimates from character counts (`chars / 4`) if zero.
    pub fn record_or_estimate(
        prompt: &str,
        completion: &str,
        prompt_tokens: i64,
        completion_tokens: i64,
        total_tokens: i64,
    ) -> Self {
        let mut p = prompt_tokens.max(0);
        let mut c = completion_tokens.max(0);
        let mut t = total_tokens.max(p.saturating_add(c));

        if t == 0 {
            p = ((char_count(prompt) / 4).max(1)) as i64;
            c = ((char_count(completion) / 4).max(1)) as i64;
            t = p.saturating_add(c);
        }
        Self {
            prompt_tokens: p,
            completion_tokens: c,
            total_tokens: t,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prompt: String,
    pub model: String,
    pub system_prompt: Option<String>,
    pub history: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: String,
    pub usage: TokenUsage,
}

pub type PullProgressCallback = Arc<dyn Fn(u8, String) + Send + Sync>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    fn base_url(&self) -> &str;
    fn is_local(&self) -> bool;

    async fn send_prompt(&self, req: CompletionRequest) -> Result<CompletionResponse, AiError>;
    async fn list_models(&self) -> Result<Vec<String>, AiError>;
    async fn check_status(&self) -> bool;
    async fn test_connection(&self) -> (bool, String);
    async fn pull_model(
        &self,
        model_name: &str,
        progress: Option<PullProgressCallback>,
    ) -> Result<String, AiError>;
}

/// Built-in cloud provider preset definition.
#[derive(Debug, Clone, Serialize)]
pub struct RemotePresetInfo {
    pub name: &'static str,
    pub base_url: &'static str,
    pub default_model: &'static str,
    pub models: &'static [&'static str],
}

pub const REMOTE_PRESETS: &[RemotePresetInfo] = &[
    RemotePresetInfo {
        name: "Xiaomi MiMo",
        base_url: "https://api.xiaomimimo.com/v1",
        default_model: "mimo-v2.6-pro",
        models: &["mimo-v2.6-pro", "mimo-v2.6-flash"],
    },
    RemotePresetInfo {
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-4o-mini",
        models: &["gpt-4o-mini", "gpt-4o", "o3-mini", "gpt-3.5-turbo"],
    },
    RemotePresetInfo {
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        default_model: "anthropic/claude-3.5-sonnet",
        models: &[
            "anthropic/claude-3.5-sonnet",
            "openai/gpt-4o-mini",
            "meta-llama/llama-3.3-70b-instruct",
        ],
    },
    RemotePresetInfo {
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        default_model: "llama-3.3-70b-versatile",
        models: &[
            "llama-3.3-70b-versatile",
            "llama-3.1-8b-instant",
            "mixtral-8x7b-32768",
        ],
    },
    RemotePresetInfo {
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        default_model: "deepseek-chat",
        models: &["deepseek-chat", "deepseek-reasoner"],
    },
    RemotePresetInfo {
        name: "xAI (Grok)",
        base_url: "https://api.x.ai/v1",
        default_model: "grok-2-latest",
        models: &["grok-2-latest", "grok-2-mini"],
    },
    RemotePresetInfo {
        name: "Custom",
        base_url: "https://api.openai.com/v1",
        default_model: "",
        models: &[],
    },
];

pub fn build_provider_from_settings(settings: &AppSettings) -> Arc<dyn LlmProvider> {
    let bt = settings.backend_type.trim().to_ascii_lowercase();
    match bt.as_str() {
        "llamacpp" | "llama.cpp" | "llama_cpp" => Arc::new(llamacpp::LlamaCppProvider::new(
            &settings.llamacpp_url,
            &settings.local_api_key,
        )),
        "remote" | "remote_openai" | "openai" | "remote_api" => {
            Arc::new(remote::RemoteOpenAiProvider::new(
                &settings.remote_base_url,
                &settings.remote_api_key,
                &settings.remote_model,
                &settings.remote_preset,
            ))
        }
        _ => Arc::new(ollama::OllamaProvider::new(
            &settings.ollama_url,
            &settings.local_api_key,
        )),
    }
}

struct CachedModelList {
    fetched_at: Instant,
    models: Vec<String>,
}

/// Manages the active `LlmProvider` instance and a 30-second TTL model list cache
/// so chat turns never make redundant `/api/tags` or `/v1/models` round-trips.
#[derive(Clone)]
pub struct ProviderManager {
    active: Arc<ArcSwap<Arc<dyn LlmProvider>>>,
    model_cache: Arc<RwLock<Option<CachedModelList>>>,
}

impl ProviderManager {
    pub fn new(settings: &AppSettings) -> Self {
        let prov = build_provider_from_settings(settings);
        Self {
            active: Arc::new(ArcSwap::from_pointee(prov)),
            model_cache: Arc::new(RwLock::new(None)),
        }
    }

    #[inline]
    pub fn get(&self) -> Arc<dyn LlmProvider> {
        self.active.load_full().as_ref().clone()
    }

    pub fn reload(&self, settings: &AppSettings) {
        let prov = build_provider_from_settings(settings);
        self.active.store(Arc::new(prov));
        *self.model_cache.write() = None;
    }

    /// Returns cached models if fetched within the last 30 seconds, or queries the provider.
    pub async fn list_models_cached(&self, force_refresh: bool) -> Vec<String> {
        if !force_refresh {
            if let Some(ref cached) = *self.model_cache.read() {
                if cached.fetched_at.elapsed() < Duration::from_secs(30) && !cached.models.is_empty()
                {
                    return cached.models.clone();
                }
            }
        }
        let prov = self.get();
        match prov.list_models().await {
            Ok(models) => {
                if !models.is_empty() {
                    *self.model_cache.write() = Some(CachedModelList {
                        fetched_at: Instant::now(),
                        models: models.clone(),
                    });
                }
                models
            }
            Err(_) => Vec::new(),
        }
    }

    /// Deterministically resolves the target model for a guild or DM turn without ever
    /// mutating `guild_configs.default_model` in the database.
    pub async fn resolve_model_for_turn(
        &self,
        override_model: &str,
        app_settings: &AppSettings,
    ) -> String {
        let global_effective = app_settings.effective_global_model();
        let requested = override_model.trim();
        if !requested.is_empty() {
            return requested.to_owned();
        }
        if !global_effective.is_empty() {
            return global_effective;
        }
        let models = self.list_models_cached(false).await;
        models
            .first()
            .cloned()
            .unwrap_or_else(|| "llama3:latest".to_owned())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_ssrf_blocks_cloud_metadata_and_credentials() {
        assert!(validate_endpoint_url("http://169.254.169.254/latest/meta-data/", false, true).is_err());
        assert!(validate_endpoint_url("http://user:pass@127.0.0.1:11434", false, true).is_err());
        assert!(validate_endpoint_url("file:///etc/passwd", false, true).is_err());
        assert!(validate_endpoint_url("http://127.0.0.1:11434", true, false).is_err());
        assert!(validate_endpoint_url("http://127.0.0.1:11434", false, false).is_ok());
        assert!(validate_endpoint_url("https://api.xiaomimimo.com/v1", true, false).is_ok());
    }
}
