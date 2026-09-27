//! Lightweight OpenAI-compatible Chat Completions via `curl`
//! (avoids pulling reqwest into the offline-vendored workspace).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ChatClient {
    api_key: String,
    base_url: String,
    model: String,
}

impl ChatClient {
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
        }
    }

    pub fn deepseek(api_key: impl Into<String>) -> Self {
        Self::new(api_key, "https://api.deepseek.com", "deepseek-chat")
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub async fn chat(&self, system: &str, user: &str) -> Result<String> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = ChatRequest {
            model: self.model.clone(),
            temperature: 0.2,
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: system.into(),
                },
                ChatMessage {
                    role: "user".into(),
                    content: user.into(),
                },
            ],
        };
        let body_json = serde_json::to_string(&body).context("serialize chat request")?;
        let auth = format!("Authorization: Bearer {}", self.api_key);

        let output = tokio::process::Command::new("curl")
            .args([
                "-fsSL",
                "-X",
                "POST",
                &url,
                "-H",
                &auth,
                "-H",
                "Content-Type: application/json",
                "-d",
                &body_json,
            ])
            .output()
            .await
            .context("spawn curl for LLM request")?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            bail!("curl LLM request failed: {err}");
        }

        let text = String::from_utf8(output.stdout).context("LLM response utf8")?;
        let parsed: ChatResponse =
            serde_json::from_str(&text).with_context(|| format!("parse LLM JSON: {text}"))?;
        let content = parsed
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .context("LLM returned empty choices")?;
        Ok(content)
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    temperature: f64,
    messages: Vec<ChatMessage>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageDe,
}

#[derive(Deserialize)]
struct ChatMessageDe {
    content: Option<String>,
}

/// Strip optional ```json fences.
pub fn extract_json_object(raw: &str) -> Result<String> {
    let t = raw.trim();
    if let Some(start) = t.find('{') {
        if let Some(end) = t.rfind('}') {
            return Ok(t[start..=end].to_string());
        }
    }
    bail!("no JSON object found in LLM output: {raw}");
}
