use anyhow::Result;
use archon_core::types::{
    ContentBlock, ContentBlockInfo, Delta, Message, Role, StopReason, StreamEvent, ToolDefinition,
    Usage,
};
use archon_core::StreamProvider;
use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::Client;
use serde_json::json;

use crate::retry::{with_retry, RetryConfig};

const DEFAULT_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";

pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    base_url: String,
    retry_config: RetryConfig,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: DEFAULT_BASE_URL.to_string(),
            retry_config: RetryConfig::default(),
        }
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url,
            retry_config: RetryConfig::default(),
        }
    }

    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }
}

/// Convert internal messages to OpenAI chat format.
fn build_openai_messages(system: &str, messages: &[Message]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();

    // System prompt as first message
    if !system.is_empty() {
        out.push(json!({"role": "system", "content": system}));
    }

    for msg in messages {
        match msg.role {
            Role::User => {
                // User messages may contain Text and ToolResult blocks.
                // ToolResult blocks become separate {"role":"tool"} messages.
                // Text blocks are merged into a single user message.
                let mut text_parts = Vec::new();
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => {
                            text_parts.push(text.clone());
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            // Flush any accumulated text first
                            if !text_parts.is_empty() {
                                out.push(json!({
                                    "role": "user",
                                    "content": text_parts.join("\n"),
                                }));
                                text_parts.clear();
                            }
                            out.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content,
                            }));
                        }
                        ContentBlock::ToolUse { .. } => {
                            // Should not appear in user messages, ignore
                        }
                    }
                }
                if !text_parts.is_empty() {
                    out.push(json!({
                        "role": "user",
                        "content": text_parts.join("\n"),
                    }));
                }
            }
            Role::Assistant => {
                // Assistant messages may contain Text and ToolUse blocks.
                let mut content_text = String::new();
                let mut tool_calls = Vec::new();

                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => {
                            content_text.push_str(text);
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(input).unwrap_or_default(),
                                },
                            }));
                        }
                        ContentBlock::ToolResult { .. } => {
                            // Should not appear in assistant messages, ignore
                        }
                    }
                }

                let mut msg_obj = json!({"role": "assistant"});
                if !content_text.is_empty() {
                    msg_obj["content"] = json!(content_text);
                }
                if !tool_calls.is_empty() {
                    msg_obj["tool_calls"] = json!(tool_calls);
                }
                out.push(msg_obj);
            }
        }
    }

    out
}

/// Convert tool definitions to OpenAI function-calling format.
fn build_openai_tools(tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                },
            })
        })
        .collect()
}

/// State machine for translating OpenAI streaming chunks into StreamEvents.
struct OpenAIStreamState {
    /// Whether we've emitted MessageStart yet.
    started: bool,
    /// Current content block index (auto-incremented).
    next_block_index: usize,
    /// Index of the currently open text block, if any.
    open_text_block: Option<usize>,
    /// Map from tool_calls array index → our content block index.
    /// Tracks which tool call indices have been started.
    tool_block_indices: std::collections::HashMap<u64, usize>,
}

impl OpenAIStreamState {
    fn new() -> Self {
        Self {
            started: false,
            next_block_index: 0,
            open_text_block: None,
            tool_block_indices: std::collections::HashMap::new(),
        }
    }

    /// Process a single parsed chunk and return zero or more StreamEvents.
    fn process_chunk(&mut self, chunk: &serde_json::Value) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        // Emit MessageStart on the first chunk
        if !self.started {
            self.started = true;
            let id = chunk["id"].as_str().unwrap_or("").to_string();
            let usage = if let Some(u) = chunk.get("usage") {
                Usage {
                    input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0),
                    output_tokens: u["completion_tokens"].as_u64().unwrap_or(0),
                }
            } else {
                Usage::default()
            };
            events.push(StreamEvent::MessageStart { id, usage });
        }

        let choices = match chunk["choices"].as_array() {
            Some(c) => c,
            None => return events,
        };

        for choice in choices {
            let delta = &choice["delta"];
            let finish_reason = choice["finish_reason"].as_str();

            // Handle text content delta
            if let Some(content) = delta["content"].as_str() {
                if !content.is_empty() {
                    // Open a text block if not already open
                    if self.open_text_block.is_none() {
                        let idx = self.next_block_index;
                        self.next_block_index += 1;
                        self.open_text_block = Some(idx);
                        events.push(StreamEvent::ContentBlockStart {
                            index: idx,
                            content_block: ContentBlockInfo::Text {
                                text: String::new(),
                            },
                        });
                    }
                    let idx = self.open_text_block.unwrap();
                    events.push(StreamEvent::ContentBlockDelta {
                        index: idx,
                        delta: Delta::TextDelta {
                            text: content.to_string(),
                        },
                    });
                }
            }

            // Handle tool_calls deltas
            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                for tc in tool_calls {
                    let tc_index = tc["index"].as_u64().unwrap_or(0);
                    let func = &tc["function"];

                    // A tool call is NEW only if we haven't tracked this index yet
                    // and it has a non-empty name. DashScope may resend id/name as
                    // empty strings on continuation chunks.
                    let is_new = !self.tool_block_indices.contains_key(&tc_index)
                        && func["name"].as_str().map_or(false, |n| !n.is_empty());

                    if is_new {
                        // Close the open text block first, if any
                        if let Some(text_idx) = self.open_text_block.take() {
                            events.push(StreamEvent::ContentBlockStop { index: text_idx });
                        }

                        let idx = self.next_block_index;
                        self.next_block_index += 1;
                        self.tool_block_indices.insert(tc_index, idx);

                        let id = tc["id"].as_str().unwrap_or("").to_string();
                        let name = func["name"].as_str().unwrap_or("").to_string();
                        events.push(StreamEvent::ContentBlockStart {
                            index: idx,
                            content_block: ContentBlockInfo::ToolUse { id, name },
                        });
                    }

                    // Append arguments (works for both new and continuation chunks)
                    if let Some(args) = func["arguments"].as_str() {
                        if !args.is_empty() {
                            if let Some(&idx) = self.tool_block_indices.get(&tc_index) {
                                events.push(StreamEvent::ContentBlockDelta {
                                    index: idx,
                                    delta: Delta::InputJsonDelta {
                                        partial_json: args.to_string(),
                                    },
                                });
                            }
                        }
                    }
                }
            }

            // Handle finish_reason
            if let Some(reason) = finish_reason {
                // Close any open text block
                if let Some(text_idx) = self.open_text_block.take() {
                    events.push(StreamEvent::ContentBlockStop { index: text_idx });
                }
                // Close all open tool blocks
                let mut tool_indices: Vec<usize> =
                    self.tool_block_indices.values().copied().collect();
                tool_indices.sort();
                for idx in tool_indices {
                    events.push(StreamEvent::ContentBlockStop { index: idx });
                }
                self.tool_block_indices.clear();

                let stop_reason = match reason {
                    "tool_calls" => Some(StopReason::ToolUse),
                    "stop" => Some(StopReason::EndTurn),
                    "length" => Some(StopReason::MaxTokens),
                    _ => None,
                };

                // Extract usage from the chunk if present
                let usage = if let Some(u) = chunk.get("usage") {
                    Usage {
                        input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0),
                        output_tokens: u["completion_tokens"].as_u64().unwrap_or(0),
                    }
                } else {
                    Usage::default()
                };

                events.push(StreamEvent::MessageDelta { stop_reason, usage });
            }
        }

        events
    }
}

#[async_trait]
impl StreamProvider for OpenAIProvider {
    async fn stream_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        model: &str,
        max_tokens: u32,
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        let openai_messages = build_openai_messages(system, messages);

        let mut body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "stream": true,
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = json!(build_openai_tools(tools));
        }

        let url = format!("{}/chat/completions", self.base_url);
        let body_str = serde_json::to_string(&body)?;

        let response = with_retry(&self.retry_config, || {
            let client = self.client.clone();
            let api_key = self.api_key.clone();
            let url = url.clone();
            let body_str = body_str.clone();
            async move {
                Ok(client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("Content-Type", "application/json")
                    .body(body_str)
                    .send()
                    .await?)
            }
        })
        .await?;

        let byte_stream = response.bytes_stream();

        let event_stream = {
            use futures::stream;
            use tokio::sync::mpsc;

            let (tx, rx) = mpsc::channel::<Result<StreamEvent>>(64);

            tokio::spawn(async move {
                use futures::TryStreamExt;
                let mut byte_stream = byte_stream;
                let mut buffer = String::new();
                let mut state = OpenAIStreamState::new();

                while let Ok(Some(chunk)) = byte_stream.try_next().await {
                    buffer.push_str(&String::from_utf8_lossy(&chunk));

                    // Process complete lines
                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                        buffer = buffer[newline_pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        if let Some(data) = line.strip_prefix("data: ") {
                            // Check for stream termination
                            if data.trim() == "[DONE]" {
                                let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
                                return;
                            }

                            // Parse JSON chunk
                            match serde_json::from_str::<serde_json::Value>(data) {
                                Ok(parsed) => {
                                    let events = state.process_chunk(&parsed);
                                    for event in events {
                                        if tx.send(Ok(event)).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = tx
                                        .send(Err(anyhow::anyhow!(
                                            "Failed to parse OpenAI chunk: {e}"
                                        )))
                                        .await;
                                    return;
                                }
                            }
                        }
                        // Ignore lines that don't start with "data: " (e.g. comments, event: lines)
                    }
                }

                // If stream ended without [DONE], still emit MessageStop
                let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
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
impl crate::provider::Provider for OpenAIProvider {
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
