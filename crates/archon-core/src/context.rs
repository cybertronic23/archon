use anyhow::Result;
use futures::StreamExt;

use crate::session::Session;
use crate::types::{ContentBlock, Message, Role, StreamEvent};
use crate::StreamProvider;

/// Configuration for context window management.
#[derive(Debug, Clone)]
pub struct ContextConfig {
    /// Maximum context tokens before compression triggers.
    pub max_context_tokens: u64,
    /// Fraction of max_context_tokens at which compression triggers (0.0–1.0).
    pub compression_threshold: f64,
    /// Number of recent messages to keep uncompressed.
    pub keep_recent: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 200_000,
            compression_threshold: 0.80,
            keep_recent: 10,
        }
    }
}

/// Check if context compression is needed and perform it if so.
///
/// Compresses old messages into a summary using the LLM provider,
/// preserving the most recent `keep_recent` messages.
pub async fn maybe_compress(
    session: &mut Session,
    config: &ContextConfig,
    provider: &dyn StreamProvider,
    model: &str,
    max_tokens: u32,
) -> Result<()> {
    let threshold = (config.max_context_tokens as f64 * config.compression_threshold) as u64;
    if session.total_input_tokens < threshold {
        return Ok(());
    }

    let msg_count = session.messages.len();
    if msg_count <= config.keep_recent + 2 {
        // Not enough messages to compress
        return Ok(());
    }

    let split_at = msg_count - config.keep_recent;
    let old_messages = &session.messages[..split_at];

    // Serialize old messages into a text representation
    let mut conversation_text = String::new();
    for msg in old_messages {
        let role_label = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        for block in &msg.content {
            match block {
                ContentBlock::Text { text } => {
                    conversation_text.push_str(&format!("{role_label}: {text}\n\n"));
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    let input_str = serde_json::to_string(input).unwrap_or_default();
                    // Truncate tool inputs to 500 chars
                    let truncated = if input_str.len() > 500 {
                        format!("{}...", &input_str[..500])
                    } else {
                        input_str
                    };
                    conversation_text
                        .push_str(&format!("{role_label} [tool: {name}]: {truncated}\n\n"));
                }
                ContentBlock::ToolResult { content, .. } => {
                    // Truncate tool results to 500 chars
                    let truncated = if content.len() > 500 {
                        format!("{}...", &content[..500])
                    } else {
                        content.clone()
                    };
                    conversation_text
                        .push_str(&format!("{role_label} [tool result]: {truncated}\n\n"));
                }
            }
        }
    }

    // Call LLM to generate a summary
    let summary_system = "You are a conversation summarizer. Summarize the following conversation concisely, preserving key decisions, file paths, code changes, and important context. Be thorough but brief.";
    let summary_messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: format!("Summarize this conversation:\n\n{conversation_text}"),
        }],
    }];

    let summary_max_tokens = max_tokens.min(4096);
    let mut stream = provider
        .stream_message(
            summary_system,
            &summary_messages,
            &[], // no tools for summarization
            model,
            summary_max_tokens,
        )
        .await?;

    let mut summary_text = String::new();
    while let Some(event_result) = stream.next().await {
        let event = event_result?;
        if let StreamEvent::ContentBlockDelta {
            delta: crate::types::Delta::TextDelta { text },
            ..
        } = event
        {
            summary_text.push_str(&text);
        }
    }

    if summary_text.is_empty() {
        // Summarization failed — skip compression this round
        return Ok(());
    }

    // Build compressed messages: summary User+Assistant pair + recent messages
    let recent_messages = session.messages[split_at..].to_vec();

    let mut new_messages = vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: format!("[Conversation summary]\n\n{summary_text}"),
            }],
        },
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: "Understood. I have the conversation context from the summary above. Let me continue helping you.".to_string(),
            }],
        },
    ];
    new_messages.extend(recent_messages);

    eprintln!(
        "[context] Compressed {} messages into summary ({} → {} messages)",
        split_at,
        msg_count,
        new_messages.len()
    );

    session.messages = new_messages;
    // Reset token counters — the next API call will report fresh values
    session.total_input_tokens = 0;
    session.total_output_tokens = 0;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = ContextConfig::default();
        assert_eq!(config.max_context_tokens, 200_000);
        assert!((config.compression_threshold - 0.80).abs() < f64::EPSILON);
        assert_eq!(config.keep_recent, 10);
    }

    #[test]
    fn test_no_compression_under_threshold() {
        // Session with tokens well under threshold should not trigger compression.
        // We can't easily call maybe_compress without a provider, but we can verify
        // the threshold math: threshold = 200_000 * 0.80 = 160_000.
        let config = ContextConfig::default();
        let threshold = (config.max_context_tokens as f64 * config.compression_threshold) as u64;
        assert_eq!(threshold, 160_000);

        let mut session = Session::new("test".to_string());
        session.total_input_tokens = 100_000; // under 160_000
        assert!(session.total_input_tokens < threshold);
    }
}
