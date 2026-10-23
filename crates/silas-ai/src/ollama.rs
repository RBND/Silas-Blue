//! Ollama local/remote LLM provider (`/api/chat`, `/api/tags`, `/api/pull`).

use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    fast_http_client, inference_http_client, validate_endpoint_url, AiError, CompletionRequest,
    CompletionResponse, LlmProvider, PullProgressCallback, TokenUsage,
};

pub struct OllamaProvider {
    base_url: String,
    api_key: String,
}

impl OllamaProvider {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        let clean = base_url.trim().trim_end_matches('/');
        let url = if clean.is_empty() {
            "http://127.0.0.1:11434".to_owned()
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
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModelItem>,
}

#[derive(Deserialize)]
struct OllamaModelItem {
    name: String,
}

#[derive(Serialize)]
struct OllamaMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: Option<OllamaResponseMessage>,
    #[serde(default)]
    prompt_eval_count: i64,
    #[serde(default)]
    eval_count: i64,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: String,
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "Ollama"
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
        let url = format!("{}/api/chat", self.base_url);

        let mut messages: Vec<OllamaMessage<'_>> = Vec::with_capacity(req.history.len().saturating_add(2));
        if let Some(ref sys) = req.system_prompt {
            if !sys.trim().is_empty() {
                messages.push(OllamaMessage {
                    role: "system",
                    content: sys,
                });
            }
        }
        for m in &req.history {
            messages.push(OllamaMessage {
                role: &m.role,
                content: &m.content,
            });
        }
        messages.push(OllamaMessage {
            role: "user",
            content: &req.prompt,
        });

        let mut payload = json!({
            "model": req.model,
            "messages": messages,
            "stream": false
        });

        if let Some(max_tok) = req.max_tokens {
            if max_tok > 0 {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("options".to_owned(), json!({ "num_predict": max_tok }));
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

        let data: OllamaChatResponse = resp.json().await?;
        let content = data.message.map_or_else(String::new, |m| m.content);
        let usage = TokenUsage::record_or_estimate(
            &req.prompt,
            &content,
            data.prompt_eval_count,
            data.eval_count,
            data.prompt_eval_count.saturating_add(data.eval_count),
        );

        Ok(CompletionResponse { content, usage })
    }

    async fn list_models(&self) -> Result<Vec<String>, AiError> {
        validate_endpoint_url(&self.base_url, false, true)?;
        let url = format!("{}/api/tags", self.base_url);
        let resp = fast_http_client()
            .get(&url)
            .headers(self.headers())
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let data: OllamaTagsResponse = resp.json().await?;
        Ok(data.models.into_iter().map(|m| m.name).collect())
    }

    async fn check_status(&self) -> bool {
        if validate_endpoint_url(&self.base_url, false, true).is_err() {
            return false;
        }
        let url = format!("{}/api/tags", self.base_url);
        fast_http_client()
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    async fn test_connection(&self) -> (bool, String) {
        if let Err(e) = validate_endpoint_url(&self.base_url, false, true) {
            return (false, e.to_string());
        }
        let url = format!("{}/api/tags", self.base_url);
        match fast_http_client().get(&url).headers(self.headers()).send().await {
            Ok(resp) if resp.status().is_success() => {
                let count = resp
                    .json::<OllamaTagsResponse>()
                    .await
                    .map_or(0, |d| d.models.len());
                (
                    true,
                    format!("Ollama online at {} ({count} models installed)", self.base_url),
                )
            }
            Ok(resp) => (
                false,
                format!("Ollama returned HTTP {}", resp.status().as_u16()),
            ),
            Err(e) => (false, format!("Cannot reach Ollama at {}: {e}", self.base_url)),
        }
    }

    async fn pull_model(
        &self,
        model_name: &str,
        progress: Option<PullProgressCallback>,
    ) -> Result<String, AiError> {
        validate_endpoint_url(&self.base_url, false, true)?;
        let url = format!("{}/api/pull", self.base_url);
        let payload = json!({
            "name": model_name.trim(),
            "stream": true
        });

        let resp = inference_http_client()
            .post(&url)
            .headers(self.headers())
            .json(&payload)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(AiError::ApiError {
                status,
                message: text,
            });
        }

        let mut stream = resp.bytes_stream();
        let mut last_emit = Instant::now() - Duration::from_secs(1);

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res?;
            if let Some(ref cb) = progress {
                if last_emit.elapsed() >= Duration::from_millis(250) {
                    if let Ok(line) = std::str::from_utf8(&chunk) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                            let status_txt = val
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Downloading...")
                                .to_owned();
                            let completed = val.get("completed").and_then(|v| v.as_u64()).unwrap_or(0);
                            let total = val.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                            let pct = (completed.saturating_mul(100))
                                .checked_div(total)
                                .unwrap_or(0)
                                .min(100) as u8;
                            cb(pct, status_txt);
                            last_emit = Instant::now();
                        }
                    }
                }
            }
        }

        if let Some(ref cb) = progress {
            cb(100, format!("Successfully pulled {model_name}"));
        }
        Ok(format!("Successfully pulled {model_name}"))
    }
}
