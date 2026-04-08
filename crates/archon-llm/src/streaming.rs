use anyhow::{anyhow, Result};
use archon_core::types::{
    ContentBlockInfo, Delta, StopReason, StreamEvent, Usage,
};

/// Parse a single SSE `data:` JSON payload into a `StreamEvent`.
pub fn parse_stream_event(event_type: &str, data: &str) -> Result<StreamEvent> {
    match event_type {
        "message_start" => {
            let v: serde_json::Value = serde_json::from_str(data)?;
            let msg = &v["message"];
            let id = msg["id"].as_str().unwrap_or_default().to_string();
            let usage = Usage {
                input_tokens: msg["usage"]["input_tokens"].as_u64().unwrap_or(0),
                output_tokens: msg["usage"]["output_tokens"].as_u64().unwrap_or(0),
            };
            Ok(StreamEvent::MessageStart { id, usage })
        }
        "content_block_start" => {
            let v: serde_json::Value = serde_json::from_str(data)?;
            let index = v["index"].as_u64().unwrap_or(0) as usize;
            let cb = &v["content_block"];
            let block_type = cb["type"].as_str().unwrap_or("text");
            let content_block = match block_type {
                "tool_use" => ContentBlockInfo::ToolUse {
                    id: cb["id"].as_str().unwrap_or_default().to_string(),
                    name: cb["name"].as_str().unwrap_or_default().to_string(),
                },
                _ => ContentBlockInfo::Text {
                    text: cb["text"].as_str().unwrap_or_default().to_string(),
                },
            };
            Ok(StreamEvent::ContentBlockStart {
                index,
                content_block,
            })
        }
        "content_block_delta" => {
            let v: serde_json::Value = serde_json::from_str(data)?;
            let index = v["index"].as_u64().unwrap_or(0) as usize;
            let d = &v["delta"];
            let delta_type = d["type"].as_str().unwrap_or("");
            let delta = match delta_type {
                "input_json_delta" => Delta::InputJsonDelta {
                    partial_json: d["partial_json"].as_str().unwrap_or("").to_string(),
                },
                _ => Delta::TextDelta {
                    text: d["text"].as_str().unwrap_or("").to_string(),
                },
            };
            Ok(StreamEvent::ContentBlockDelta { index, delta })
        }
        "content_block_stop" => {
            let v: serde_json::Value = serde_json::from_str(data)?;
            let index = v["index"].as_u64().unwrap_or(0) as usize;
            Ok(StreamEvent::ContentBlockStop { index })
        }
        "message_delta" => {
            let v: serde_json::Value = serde_json::from_str(data)?;
            let d = &v["delta"];
            let stop_reason = d["stop_reason"].as_str().and_then(|s| match s {
                "end_turn" => Some(StopReason::EndTurn),
                "tool_use" => Some(StopReason::ToolUse),
                "max_tokens" => Some(StopReason::MaxTokens),
                "stop_sequence" => Some(StopReason::StopSequence),
                _ => None,
            });
            let usage = Usage {
                input_tokens: v["usage"]["input_tokens"].as_u64().unwrap_or(0),
                output_tokens: v["usage"]["output_tokens"].as_u64().unwrap_or(0),
            };
            Ok(StreamEvent::MessageDelta { stop_reason, usage })
        }
        "message_stop" => Ok(StreamEvent::MessageStop),
        "ping" => Ok(StreamEvent::Ping),
        "error" => {
            let v: serde_json::Value = serde_json::from_str(data)?;
            let message = v["error"]["message"]
                .as_str()
                .unwrap_or("unknown error")
                .to_string();
            Ok(StreamEvent::Error { message })
        }
        _ => Err(anyhow!("Unknown SSE event type: {event_type}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::types::StreamEvent;

    #[test]
    fn test_parse_message_start() {
        let data = r#"{"type":"message_start","message":{"id":"msg_123","usage":{"input_tokens":100,"output_tokens":5}}}"#;
        let event = parse_stream_event("message_start", data).unwrap();
        match event {
            StreamEvent::MessageStart { id, usage } => {
                assert_eq!(id, "msg_123");
                assert_eq!(usage.input_tokens, 100);
                assert_eq!(usage.output_tokens, 5);
            }
            _ => panic!("expected MessageStart"),
        }
    }

    #[test]
    fn test_parse_content_block_start_text() {
        let data = r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
        let event = parse_stream_event("content_block_start", data).unwrap();
        match event {
            StreamEvent::ContentBlockStart { index, content_block } => {
                assert_eq!(index, 0);
                match content_block {
                    ContentBlockInfo::Text { text } => assert_eq!(text, ""),
                    _ => panic!("expected Text"),
                }
            }
            _ => panic!("expected ContentBlockStart"),
        }
    }

    #[test]
    fn test_parse_content_block_start_tool_use() {
        let data = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"read"}}"#;
        let event = parse_stream_event("content_block_start", data).unwrap();
        match event {
            StreamEvent::ContentBlockStart { index, content_block } => {
                assert_eq!(index, 1);
                match content_block {
                    ContentBlockInfo::ToolUse { id, name } => {
                        assert_eq!(id, "toolu_01");
                        assert_eq!(name, "read");
                    }
                    _ => panic!("expected ToolUse"),
                }
            }
            _ => panic!("expected ContentBlockStart"),
        }
    }

    #[test]
    fn test_parse_content_block_delta_text() {
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let event = parse_stream_event("content_block_delta", data).unwrap();
        match event {
            StreamEvent::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 0);
                match delta {
                    Delta::TextDelta { text } => assert_eq!(text, "Hello"),
                    _ => panic!("expected TextDelta"),
                }
            }
            _ => panic!("expected ContentBlockDelta"),
        }
    }

    #[test]
    fn test_parse_content_block_delta_json() {
        let data = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"file"}}"#;
        let event = parse_stream_event("content_block_delta", data).unwrap();
        match event {
            StreamEvent::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 1);
                match delta {
                    Delta::InputJsonDelta { partial_json } => {
                        assert_eq!(partial_json, r#"{"file"#);
                    }
                    _ => panic!("expected InputJsonDelta"),
                }
            }
            _ => panic!("expected ContentBlockDelta"),
        }
    }

    #[test]
    fn test_parse_content_block_stop() {
        let data = r#"{"type":"content_block_stop","index":0}"#;
        let event = parse_stream_event("content_block_stop", data).unwrap();
        match event {
            StreamEvent::ContentBlockStop { index } => assert_eq!(index, 0),
            _ => panic!("expected ContentBlockStop"),
        }
    }

    #[test]
    fn test_parse_message_delta_end_turn() {
        let data = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":0,"output_tokens":42}}"#;
        let event = parse_stream_event("message_delta", data).unwrap();
        match event {
            StreamEvent::MessageDelta { stop_reason, usage } => {
                assert_eq!(stop_reason, Some(StopReason::EndTurn));
                assert_eq!(usage.output_tokens, 42);
            }
            _ => panic!("expected MessageDelta"),
        }
    }

    #[test]
    fn test_parse_message_stop() {
        let event = parse_stream_event("message_stop", "{}").unwrap();
        match event {
            StreamEvent::MessageStop => {}
            _ => panic!("expected MessageStop"),
        }
    }

    #[test]
    fn test_parse_ping() {
        let event = parse_stream_event("ping", "{}").unwrap();
        match event {
            StreamEvent::Ping => {}
            _ => panic!("expected Ping"),
        }
    }

    #[test]
    fn test_parse_error() {
        let data = r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        let event = parse_stream_event("error", data).unwrap();
        match event {
            StreamEvent::Error { message } => assert_eq!(message, "Overloaded"),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn test_parse_unknown_event_type() {
        let result = parse_stream_event("unknown_type", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown SSE event type"));
    }
}
