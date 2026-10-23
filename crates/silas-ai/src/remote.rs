//! OpenAI-Compatible Remote Cloud AI Provider (`Xiaomi MiMo`, `OpenAI`, `OpenRouter`, `Groq`, `DeepSeek`, `xAI`).

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;

use crate::{
    fast_http_client, inference_http_client, validate_endpoint_url, AiError, CompletionRequest,
    CompletionResponse, LlmProvider, PullProgressCallback, TokenUsage, REMOTE_PRESETS,
};

pub struct RemoteOpenAiProvider {
    display_name: String,
    base_url: String,
    api_key: String,
    default_model: String,
    preset: String,
}

impl RemoteOpenAiProvider {
    pub fn new(base_url: &str, api_key: &str, default_model: &str, preset: &str) -> Self {
        let clean_url = base_url.trim().trim_end_matches('/');
        let url = if clean_url.is_empty() {
            "https://api.xiaomimimo.com/v1".to_owned()
        } else {
            clean_url.to_owned()
        };
        let clean_preset = preset.trim();
        let display_name = if !clean_preset.is_empty() && clean_preset != "Custom" {
            format!("Remote ({clean_preset})")
        } else {
            "Remote OpenAI API".to_owned()
        };
        Self {
            display_name,
            base_url: url,
            api_key: api_key.trim().to_owned(),
            default_model: default_model.trim().to_owned(),
            preset: clean_preset.to_owned(),
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

    fn fallback_models(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.default_model.is_empty() {
            out.push(self.default_model.clone());
        }
        if let Some(p) = REMOTE_PRESETS
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(&self.preset))
        {
            for &m in p.models {
                if !out.iter().any(|x| x == m) {
                    out.push(m.to_owned());
                }
            }
        }
        if out.is_empty() {
            out.push("mimo-v2.6-pro".to_owned());
            out.push("gpt-4o-mini".to_owned());
        }
        out
    }
}

#[derive(Deserialize)]
struct RemoteModelsResponse {
    #[serde(default)]
    data: Vec<RemoteModelEntry>,
}

#[derive(Deserialize)]
struct RemoteModelEntry {
    id: String,
}

#[derive(Deserialize)]
struct RemoteChatResponse {
    #[serde(default)]
    choices: Vec<RemoteChoice>,
    #[serde(default)]
    usage: Option<RemoteUsage>,
}

#[derive(Deserialize)]
struct RemoteChoice {
    message: Option<RemoteMessage>,
}

#[derive(Deserialize)]
struct RemoteMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct RemoteUsage {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
}

#[async_trait]
impl LlmProvider for RemoteOpenAiProvider {
    fn name(&self) -> &str {
        &self.display_name
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn is_local(&self) -> bool {
        false
    }

    async fn send_prompt(&self, req: CompletionRequest) -> Result<CompletionResponse, AiError> {
        validate_endpoint_url(&self.base_url, true, false)?;
        let target_model = if !req.model.trim().is_empty() {
            req.model.trim()
        } else if !self.default_model.is_empty() {
            &self.default_model
        } else {
            "mimo-v2.6-pro"
        };

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
            "model": target_model,
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
            .post(self.chat_url())
            .headers(self.headers())
            .json(&payload)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status != 200 {
            let raw = resp.text().await.unwrap_or_default();
            let err_msg = serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .map(str::to_owned)
                })
                .unwrap_or(raw);

            return Err(AiError::ApiError {
                status,
                message: err_msg,
            });
        }

        let data: RemoteChatResponse = resp.json().await?;
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
        validate_endpoint_url(&self.base_url, true, false)?;
        let mut retrieved = Vec::new();
        if let Ok(resp) = fast_http_client()
            .get(self.models_url())
            .headers(self.headers())
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(data) = resp.json::<RemoteModelsResponse>().await {
                    for item in data.data {
                        if !item.id.trim().is_empty() {
                            retrieved.push(item.id);
                        }
                    }
                }
            }
        }

        if !retrieved.is_empty() {
            if !self.default_model.is_empty() {
                if let Some(pos) = retrieved.iter().position(|m| m == &self.default_model) {
                    let dm = retrieved.remove(pos);
                    retrieved.insert(0, dm);
                }
            }
            return Ok(retrieved);
        }

        Ok(self.fallback_models())
    }

    async fn check_status(&self) -> bool {
        self.test_connection().await.0
    }

    async fn test_connection(&self) -> (bool, String) {
        if let Err(e) = validate_endpoint_url(&self.base_url, true, false) {
            return (false, e.to_string());
        }
        match fast_http_client()
            .get(self.models_url())
            .headers(self.headers())
            .send()
            .await
        {
            Ok(resp) => {
                let code = resp.status().as_u16();
                match code {
                    200 => {
                        let count = resp
                            .json::<RemoteModelsResponse>()
                            .await
                            .map_or(0, |d| d.data.len());
                        (
                            true,
                            format!("Connection successful! HTTP 200 OK ({count} models available)"),
                        )
                    }
                    401 => (
                        false,
                        "Authentication Failed (HTTP 401): Invalid API Key.".to_owned(),
                    ),
                    403 => (
                        false,
                        "Access Forbidden (HTTP 403): Check API key permissions or IP whitelist."
                            .to_owned(),
                    ),
                    404 => (
                        true,
                        "Endpoint reachable (HTTP 404 on /models; chat completions endpoint online)."
                            .to_owned(),
                    ),
                    _ => {
                        let txt = resp.text().await.unwrap_or_default();
                        let short: String = txt.chars().take(120).collect();
                        (false, format!("Server responded with HTTP {code}: {short}"))
                    }
                }
            }
            Err(e) => (false, format!("Connection error reaching {}: {e}", self.base_url)),
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
                "Remote cloud model is hosted remotely and ready to use.".to_owned(),
            );
        }
        Ok(format!(
            "Remote model '{model_name}' is cloud-hosted and ready."
        ))
    }
}
