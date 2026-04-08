use anyhow::Result;
use archon_core::types::{Message, StreamEvent, ToolDefinition};
use archon_core::StreamProvider;
use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::Client;
use serde_json::json;

use crate::retry::{with_retry, RetryConfig};
use crate::streaming::parse_stream_event;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    retry_config: RetryConfig,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            retry_config: RetryConfig::default(),
        }
    }

    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }
}

#[async_trait]
impl StreamProvider for AnthropicProvider {
    async fn stream_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        model: &str,
        max_tokens: u32,
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        let mut body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "stream": true,
            "system": system,
            "messages": messages,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(tools)?;
        }

        let body_str = serde_json::to_string(&body)?;

        let response = with_retry(&self.retry_config, || {
            let client = self.client.clone();
            let api_key = self.api_key.clone();
            let body_str = body_str.clone();
            async move {
                Ok(client
                    .post(ANTHROPIC_API_URL)
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .body(body_str)
                    .send()
                    .await?)
            }
        })
        .await?;

        // Read the SSE byte stream and parse into StreamEvents
        let byte_stream = response.bytes_stream();

        let event_stream = {
            use futures::stream;
            use tokio::sync::mpsc;

            let (tx, rx) = mpsc::channel::<Result<StreamEvent>>(64);

            tokio::spawn(async move {
                use futures::TryStreamExt;
                let mut byte_stream = byte_stream;
                let mut buffer = String::new();
                let mut current_event_type = String::new();
                let mut current_data = String::new();

                while let Ok(Some(chunk)) = byte_stream.try_next().await {
                    buffer.push_str(&String::from_utf8_lossy(&chunk));

                    // Process complete lines
                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                        buffer = buffer[newline_pos + 1..].to_string();

                        if line.is_empty() {
                            // Empty line = end of event
                            if !current_event_type.is_empty() && !current_data.is_empty() {
                                match parse_stream_event(&current_event_type, &current_data) {
                                    Ok(event) => {
                                        if tx.send(Ok(event)).await.is_err() {
                                            return;
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Err(e)).await;
                                        return;
                                    }
                                }
                            }
                            current_event_type.clear();
                            current_data.clear();
                        } else if let Some(rest) = line.strip_prefix("event: ") {
                            current_event_type = rest.to_string();
                        } else if let Some(rest) = line.strip_prefix("data: ") {
                            if !current_data.is_empty() {
                                current_data.push('\n');
                            }
                            current_data.push_str(rest);
                        }
                    }
                }

                // Process any remaining event
                if !current_event_type.is_empty() && !current_data.is_empty() {
                    if let Ok(event) = parse_stream_event(&current_event_type, &current_data) {
                        let _ = tx.send(Ok(event)).await;
                    }
                }
            });

            stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|item| (item, rx))
            })
        };

        Ok(Box::pin(event_stream))
    }
}

// Also implement the Provider trait from this crate (mirrors StreamProvider)
#[async_trait]
impl crate::provider::Provider for AnthropicProvider {
    async fn stream_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        model: &str,
        max_tokens: u32,
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        <Self as StreamProvider>::stream_message(self, system, messages, tools, model, max_tokens)
            .await
    }
}
