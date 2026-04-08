use anyhow::Result;
use archon_core::types::{Message, StreamEvent, ToolDefinition};
use async_trait::async_trait;
use futures::stream::BoxStream;

/// Trait for LLM providers that support streaming message responses.
#[async_trait]
pub trait Provider: Send + Sync {
    async fn stream_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        model: &str,
        max_tokens: u32,
    ) -> Result<BoxStream<'static, Result<StreamEvent>>>;
}
