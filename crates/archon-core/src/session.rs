use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::types::{ContentBlock, Message, Role, Usage};

/// Holds the conversation history for one agent session.
#[derive(Serialize, Deserialize)]
pub struct Session {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

impl Session {
    pub fn new(system_prompt: String) -> Self {
        Self {
            system_prompt,
            messages: Vec::new(),
            total_input_tokens: 0,
            total_output_tokens: 0,
        }
    }

    /// Record token usage from one LLM turn.
    pub fn record_usage(&mut self, usage: &Usage) {
        self.total_input_tokens += usage.input_tokens;
        self.total_output_tokens += usage.output_tokens;
    }

    /// Append a user message with a single text block.
    pub fn push_user(&mut self, text: &str) {
        self.messages.push(Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        });
    }

    /// Append an assistant message composed of the given content blocks.
    pub fn push_assistant(&mut self, blocks: Vec<ContentBlock>) {
        self.messages.push(Message {
            role: Role::Assistant,
            content: blocks,
        });
    }

    /// Append a user message containing tool results.
    pub fn push_tool_results(&mut self, results: Vec<ContentBlock>) {
        self.messages.push(Message {
            role: Role::User,
            content: results,
        });
    }

    /// Serialize and save the session to a JSON file.
    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load a session from a JSON file.
    pub fn load_from_file(path: &Path) -> Result<Session> {
        let data = std::fs::read_to_string(path)?;
        let session: Session = serde_json::from_str(&data)?;
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_session() {
        let session = Session::new("test prompt".to_string());
        assert_eq!(session.system_prompt, "test prompt");
        assert!(session.messages.is_empty());
        assert_eq!(session.total_input_tokens, 0);
        assert_eq!(session.total_output_tokens, 0);
    }

    #[test]
    fn test_push_user_message() {
        let mut session = Session::new("sys".to_string());
        session.push_user("hello");
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, Role::User);
        match &session.messages[0].content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_push_assistant() {
        let mut session = Session::new("sys".to_string());
        session.push_assistant(vec![ContentBlock::Text {
            text: "hi".to_string(),
        }]);
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, Role::Assistant);
    }

    #[test]
    fn test_push_tool_results() {
        let mut session = Session::new("sys".to_string());
        session.push_tool_results(vec![ContentBlock::ToolResult {
            tool_use_id: "id1".to_string(),
            content: "result".to_string(),
            is_error: None,
        }]);
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, Role::User);
        match &session.messages[0].content[0] {
            ContentBlock::ToolResult { tool_use_id, .. } => assert_eq!(tool_use_id, "id1"),
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn test_record_usage() {
        let mut session = Session::new("sys".to_string());
        let usage1 = Usage {
            input_tokens: 100,
            output_tokens: 50,
        };
        let usage2 = Usage {
            input_tokens: 200,
            output_tokens: 75,
        };
        session.record_usage(&usage1);
        session.record_usage(&usage2);
        assert_eq!(session.total_input_tokens, 300);
        assert_eq!(session.total_output_tokens, 125);
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");

        let mut session = Session::new("my prompt".to_string());
        session.push_user("hello world");
        session.record_usage(&Usage {
            input_tokens: 42,
            output_tokens: 13,
        });
        session.save_to_file(&path).unwrap();

        let loaded = Session::load_from_file(&path).unwrap();
        assert_eq!(loaded.system_prompt, "my prompt");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.total_input_tokens, 42);
        assert_eq!(loaded.total_output_tokens, 13);
    }
}
