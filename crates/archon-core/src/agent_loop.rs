use std::io::{self, Write};

use anyhow::Result;
use futures::StreamExt;

use crate::context::{self, ContextConfig};
use crate::permission::{PermissionHandler, PermissionRequest, PermissionVerdict, RiskLevel};
use crate::session::Session;
use crate::tool::ToolRegistry;
use crate::types::{ContentBlock, ContentBlockInfo, Delta, StopReason, StreamEvent, Usage};

/// Provider trait — must yield a stream of `StreamEvent`.
/// Re-exported here so callers don't need archon-llm directly.
#[async_trait::async_trait]
pub trait StreamProvider: Send + Sync {
    async fn stream_message(
        &self,
        system: &str,
        messages: &[crate::types::Message],
        tools: &[crate::types::ToolDefinition],
        model: &str,
        max_tokens: u32,
    ) -> Result<futures::stream::BoxStream<'static, Result<StreamEvent>>>;
}

/// Run the agent loop: stream LLM → execute tools → loop until end_turn.
pub async fn run_agent_loop(
    provider: &dyn StreamProvider,
    tools: &ToolRegistry,
    permissions: &dyn PermissionHandler,
    session: &mut Session,
    model: &str,
    max_tokens: u32,
    context_config: &ContextConfig,
) -> Result<()> {
    loop {
        // Check if context compression is needed before each LLM call
        context::maybe_compress(session, context_config, provider, model, max_tokens).await?;

        let tool_defs = tools.definitions();
        let mut stream = provider
            .stream_message(
                &session.system_prompt,
                &session.messages,
                &tool_defs,
                model,
                max_tokens,
            )
            .await?;

        // Accumulators for the current response
        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut stop_reason: Option<StopReason> = None;

        // Per-block accumulators (indexed by content_block index)
        let mut block_texts: Vec<String> = Vec::new();
        let mut block_tool_ids: Vec<String> = Vec::new();
        let mut block_tool_names: Vec<String> = Vec::new();
        let mut block_json_bufs: Vec<String> = Vec::new();
        let mut block_types: Vec<BlockType> = Vec::new(); // track type per index

        // Usage tracking for this turn
        let mut turn_usage = Usage::default();

        while let Some(event_result) = stream.next().await {
            let event = event_result?;
            match event {
                StreamEvent::MessageStart { usage, .. } => {
                    turn_usage.input_tokens += usage.input_tokens;
                    turn_usage.output_tokens += usage.output_tokens;
                }
                StreamEvent::ContentBlockStart {
                    index,
                    content_block,
                } => {
                    // Ensure vectors are large enough
                    while block_types.len() <= index {
                        block_texts.push(String::new());
                        block_tool_ids.push(String::new());
                        block_tool_names.push(String::new());
                        block_json_bufs.push(String::new());
                        block_types.push(BlockType::Text);
                    }
                    match content_block {
                        ContentBlockInfo::Text { .. } => {
                            block_types[index] = BlockType::Text;
                        }
                        ContentBlockInfo::ToolUse { id, name } => {
                            block_types[index] = BlockType::ToolUse;
                            block_tool_ids[index] = id;
                            block_tool_names[index] = name;
                        }
                    }
                }
                StreamEvent::ContentBlockDelta { index, delta } => match delta {
                    Delta::TextDelta { text } => {
                        print!("{text}");
                        io::stdout().flush().ok();
                        if index < block_texts.len() {
                            block_texts[index].push_str(&text);
                        }
                    }
                    Delta::InputJsonDelta { partial_json } => {
                        if index < block_json_bufs.len() {
                            block_json_bufs[index].push_str(&partial_json);
                        }
                    }
                },
                StreamEvent::ContentBlockStop { index } => {
                    if index < block_types.len() {
                        match block_types[index] {
                            BlockType::Text => {
                                content_blocks.push(ContentBlock::Text {
                                    text: block_texts[index].clone(),
                                });
                            }
                            BlockType::ToolUse => {
                                let input: serde_json::Value =
                                    if block_json_bufs[index].is_empty() {
                                        serde_json::Value::Object(serde_json::Map::new())
                                    } else {
                                        serde_json::from_str(&block_json_bufs[index])
                                            .unwrap_or(serde_json::Value::Object(
                                                serde_json::Map::new(),
                                            ))
                                    };
                                content_blocks.push(ContentBlock::ToolUse {
                                    id: block_tool_ids[index].clone(),
                                    name: block_tool_names[index].clone(),
                                    input,
                                });
                            }
                        }
                    }
                }
                StreamEvent::MessageDelta {
                    stop_reason: sr,
                    usage,
                } => {
                    stop_reason = sr;
                    turn_usage.input_tokens += usage.input_tokens;
                    turn_usage.output_tokens += usage.output_tokens;
                }
                StreamEvent::MessageStop
                | StreamEvent::Ping => {}
                StreamEvent::Error { message } => {
                    anyhow::bail!("API error: {message}");
                }
            }
        }

        // Record usage for this turn
        session.record_usage(&turn_usage);

        // Ensure newline after streamed text
        if content_blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { .. }))
        {
            println!();
        }

        // Push assistant message into session
        session.push_assistant(content_blocks.clone());

        // Check if we need to execute tools
        match stop_reason {
            Some(StopReason::ToolUse) => {
                // Collect all ToolUse blocks with their original index
                let tool_uses: Vec<(usize, &str, &str, &serde_json::Value)> = content_blocks
                    .iter()
                    .enumerate()
                    .filter_map(|(i, block)| {
                        if let ContentBlock::ToolUse { id, name, input } = block {
                            Some((i, id.as_str(), name.as_str(), input))
                        } else {
                            None
                        }
                    })
                    .collect();

                // Phase 1: Serial permission checks (stdin interaction can't be concurrent)
                #[derive(Debug)]
                enum ToolDecision {
                    Approved,
                    Denied,
                }
                let mut decisions: Vec<(usize, ToolDecision)> = Vec::new();

                for &(idx, _id, name, input) in &tool_uses {
                    let risk = permissions.classify(name, input);
                    if risk != RiskLevel::Safe {
                        let request = PermissionRequest {
                            tool_name: name,
                            input,
                            risk_level: risk,
                        };
                        if permissions.check(&request).await == PermissionVerdict::Deny {
                            decisions.push((idx, ToolDecision::Denied));
                            continue;
                        }
                    }
                    decisions.push((idx, ToolDecision::Approved));
                }

                // Phase 2: Parallel execution of approved tools
                let mut approved_futures = Vec::new();
                let mut tool_results: Vec<(usize, ContentBlock)> = Vec::new();

                for (idx, decision) in &decisions {
                    let (_, id, name, input) = tool_uses.iter().find(|(i, ..)| i == idx).unwrap();
                    match decision {
                        ToolDecision::Denied => {
                            tool_results.push((
                                *idx,
                                ContentBlock::ToolResult {
                                    tool_use_id: id.to_string(),
                                    content: "Permission denied by user.".to_string(),
                                    is_error: Some(true),
                                },
                            ));
                        }
                        ToolDecision::Approved => {
                            if let Some(tool) = tools.get(name) {
                                eprintln!("\n[tool: {name}]");
                                let id = id.to_string();
                                let input = (*input).clone();
                                approved_futures.push(async move {
                                    let result = tool.execute(input).await;
                                    (*idx, id, result)
                                });
                            } else {
                                tool_results.push((
                                    *idx,
                                    ContentBlock::ToolResult {
                                        tool_use_id: id.to_string(),
                                        content: format!("Error: Unknown tool: {name}"),
                                        is_error: Some(true),
                                    },
                                ));
                            }
                        }
                    }
                }

                // Execute all approved tools in parallel
                let parallel_results = futures::future::join_all(approved_futures).await;
                for (idx, id, result) in parallel_results {
                    match result {
                        Ok(output) => {
                            tool_results.push((
                                idx,
                                ContentBlock::ToolResult {
                                    tool_use_id: id,
                                    content: output,
                                    is_error: None,
                                },
                            ));
                        }
                        Err(e) => {
                            tool_results.push((
                                idx,
                                ContentBlock::ToolResult {
                                    tool_use_id: id,
                                    content: format!("Error: {e}"),
                                    is_error: Some(true),
                                },
                            ));
                        }
                    }
                }

                // Sort by original index to maintain order
                tool_results.sort_by_key(|(idx, _)| *idx);
                let ordered_results: Vec<ContentBlock> =
                    tool_results.into_iter().map(|(_, block)| block).collect();

                session.push_tool_results(ordered_results);
                // Continue the loop — send tool results back to LLM
            }
            _ => {
                // EndTurn, MaxTokens, or None — stop looping
                break;
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum BlockType {
    Text,
    ToolUse,
}
