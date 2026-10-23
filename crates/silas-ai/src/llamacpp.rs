//! Llama.cpp (`llama-server`) OpenAI-compatible local/LAN inference provider.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;

use crate::{
    fast_http_client, inference_http_client, validate_endpoint_url, AiError, CompletionRequest,
    CompletionResponse, LlmProvider, PullProgressCallback, TokenUsage,
};

pub struct LlamaCppProvider {
    base_url: String,
    api_key: String,
}

impl LlamaCppProvider {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        let clean = base_url.trim().trim_end_matches('/');
        let url = if clean.is_empty() {
            "http://127.0.0.1:8081".to_owned()
        } else {
            clean.to_owned()
        };
        Self {
            base_url: url,
            api_key: api_key.trim().to_owned(),
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !self.api_key.is_empty() {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", self.api_key)) {
                h.insert(AUTHORIZATION, val);
            }
        }
        h
    }

    fn chat_url(&self) -> String {
        if self.base_url.ends_with("/v1") {
            format!("{}/chat/completions", self.base_url)
        } else {
            format!("{}/v1/chat/completions", self.base_url)
        }
    }

    fn models_url(&self) -> String {
        if self.base_url.ends_with("/v1") {
            format!("{}/models", self.base_url)
        } else {
            format!("{}/v1/models", self.base_url)
        }
    }
}

#[derive(Deserialize)]
struct OpenAiModelsResponse {
    #[serde(default)]
    data: Vec<OpenAiModelEntry>,
}

#[derive(Deserialize)]
struct OpenAiModelEntry {
    id: String,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    #[serde(default)]
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: Option<OpenAiMessageContent>,
}

#[derive(Deserialize)]
struct OpenAiMessageContent {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
}

#[async_trait]
impl LlmProvider for LlamaCppProvider {
    fn name(&self) -> &str {
        "Llama.cpp"
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn is_local(&self) -> bool {
        let low = self.base_url.to_ascii_lowercase();
        low.contains("127.0.0.1") || low.contains("localhost") || low.contains("[::1]")
    }

    async fn send_prompt(&self, req: CompletionRequest) -> Result<CompletionResponse, AiError> {
        validate_endpoint_url(&self.base_url, false, true)?;
        let url = self.chat_url();

        let mut messages = Vec::with_capacity(req.history.len().saturating_add(2));
        if let Some(ref sys) = req.system_prompt {
            if !sys.trim().is_empty() {
                messages.push(json!({"role": "system", "content": sys}));
            }
        }
        for m in &req.history {
            messages.push(json!({"role": m.role, "content": m.content}));
        }
        messages.push(json!({"role": "user", "content": req.prompt}));

        let mut payload = json!({
            "model": req.model,
            "messages": messages,
            "stream": false,
            "temperature": 0.7
        });
        if let Some(max_tok) = req.max_tokens {
            if max_tok > 0 {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("max_tokens".to_owned(), json!(max_tok));
                }
            }
        }

        let resp = inference_http_client()
            .post(&url)
            .headers(self.headers())
            .json(&payload)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status != 200 {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(AiError::ApiError {
                status,
                message: err_text,
            });
        }

        let data: OpenAiChatResponse = resp.json().await?;
        let content = data
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .map_or_else(String::new, |m| m.content.clone());

        let (p_tok, c_tok, t_tok) = data.usage.map_or((0, 0, 0), |u| {
            (u.prompt_tokens, u.completion_tokens, u.total_tokens)
        });
        let usage = TokenUsage::record_or_estimate(&req.prompt, &content, p_tok, c_tok, t_tok);

        Ok(CompletionResponse { content, usage })
    }

    async fn list_models(&self) -> Result<Vec<String>, AiError> {
        validate_endpoint_url(&self.base_url, false, true)?;
        let url = self.models_url();
        let resp = fast_http_client()
            .get(&url)
            .headers(self.headers())
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(vec!["default".to_owned()]);
        }
        let data: OpenAiModelsResponse = resp.json().await?;
        let list: Vec<String> = data.data.into_iter().map(|m| m.id).collect();
        if list.is_empty() {
            Ok(vec!["default".to_owned()])
        } else {
            Ok(list)
        }
    }

    async fn check_status(&self) -> bool {
        if validate_endpoint_url(&self.base_url, false, true).is_err() {
            return false;
        }
        let health_url = format!("{}/health", self.base_url.trim_end_matches("/v1"));
        if let Ok(resp) = fast_http_client()
            .get(&health_url)
            .headers(self.headers())
            .send()
            .await
        {
            if resp.status().is_success() {
                return true;
            }
        }
        fast_http_client()
            .get(self.models_url())
            .headers(self.headers())
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    async fn test_connection(&self) -> (bool, String) {
        if let Err(e) = validate_endpoint_url(&self.base_url, false, true) {
            return (false, e.to_string());
        }
        if self.check_status().await {
            let models = self.list_models().await.unwrap_or_default();
            (
                true,
                format!(
                    "Llama.cpp server online at {} ({} models active)",
                    self.base_url,
                    models.len()
                ),
            )
        } else {
            (
                false,
                format!("Cannot reach Llama.cpp server at {}", self.base_url),
            )
        }
    }

    async fn pull_model(
        &self,
        model_name: &str,
        progress: Option<PullProgressCallback>,
    ) -> Result<String, AiError> {
        if let Some(ref cb) = progress {
            cb(
                100,
                format!("Llama.cpp model '{model_name}' configured for llama-server"),
            );
        }
        Ok(format!(
            "Llama.cpp uses GGUF model files loaded by llama-server ({model_name})"
        ))
    }
}
