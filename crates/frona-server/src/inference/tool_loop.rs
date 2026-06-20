use std::time::Instant;

use base64::Engine;
use rig_core::completion::message::{
    DocumentSourceKind, ImageMediaType, MimeType, ToolResult, ToolResultContent, UserContent,
};
use rig_core::completion::request::ToolDefinition as RigToolDefinition;
use rig_core::completion::{AssistantContent, Message as RigMessage};
use tokio_util::sync::CancellationToken;

use crate::chat::broadcast::EventSender;
use crate::chat::message::models::{MessageResponse, Reasoning};

use crate::core::error::AppError;
use crate::core::metrics::{self, InferenceMetricsContext};
use crate::tool::registry::AgentToolRegistry;
use crate::tool::{InferenceContext, ToolDefinition};

use super::config::ModelGroup;
use super::registry::ModelProviderRegistry;
use super::retry::StreamResult;
use super::retry::stream_with_retry_and_fallback;

pub struct InferenceEvent {
    pub kind: InferenceEventKind,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum InferenceEventKind {
    // ── Streaming within a turn ──────────────────────────────────────
    Text(String),
    Reasoning(String),
    ToolCall {
        id: String,
        provider_call_id: String,
        name: String,
        arguments: serde_json::Value,
        description: Option<String>,
    },
    ToolResult {
        name: String,
        result: String,
        success: bool,
    },
    EntityUpdated {
        table: String,
        record_id: String,
        fields: serde_json::Value,
    },
    Retry { retry_after_ms: u64, reason: &'static str },

    // ── Turn-lifecycle ───────────────────────────────────────────────
    /// Inference turn is starting (initial submit or resume after HITL).
    /// Channel adapters use this to begin a "thinking/typing" affordance.
    Start,
    /// Inference loop completed normally. `message` is the persisted final
    /// state (content, reasoning, attachments).
    Done { message: MessageResponse },
    /// Inference loop was cancelled (cancellation token fired, e.g. user
    /// submitted a new message while a previous turn was running).
    Cancelled { reason: String },
    /// Inference loop ended in error (provider failure, max turns, etc.).
    Failed { error: String },
    /// Loop is parked, waiting for something external (the human, a sibling
    /// task, a webhook) to resume it. The `reason` carries WHY; the message
    /// is the executing-status message at the point of the pause. Every
    /// pause cause fires this — adapters / FE that just want "loop stopped
    /// streaming" can match on `Paused { .. }` without inspecting reason.
    Paused {
        reason: PauseReason,
        message: MessageResponse,
    },
    /// Human just resolved a HITL — the loop is about to resume. The message
    /// reflects the post-resolution state (resolved tool_call.result set).
    Resume { message: MessageResponse },
}

/// Why the inference loop paused. Each variant gets its own dispatcher
/// branch on the channel and FE sides — adding a new pause cause is one
/// new variant + one new branch.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum PauseReason {
    /// Loop paused on pending HITL prompts. The pending tool_calls are on
    /// `message.tool_calls` (entries where `hitl.status == Pending`).
    Hitl,
}

#[derive(Debug, Clone)]
pub struct ToolCallResult {
    pub provider_call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub result: String,
    pub success: bool,
    pub duration_ms: u64,
    pub hitl: Option<crate::inference::hitl::Hitl>,
    pub task_event: Option<crate::inference::tool_call::TaskEvent>,
    pub system_prompt: Option<String>,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ToolLoopOutcome {
    Completed {
        text: String,
        attachments: Vec<crate::storage::Attachment>,
        lifecycle_event: Option<crate::inference::tool_call::TaskEvent>,
        reasoning: Option<Reasoning>,
    },
    Cancelled(String),
    ExternalToolPending {
        turn_text: String,
        tool_calls: Vec<crate::inference::tool_call::ToolCallResponse>,
        system_prompt: Option<String>,
    },
}


pub fn extract_reasoning(contents: &[AssistantContent]) -> Option<Reasoning> {
    contents.iter().find_map(|c| {
        if let AssistantContent::Reasoning(r) = c {
            Some(Reasoning {
                id: r.id.clone(),
                content: r.display_text(),
                signature: r.first_signature().map(|s| s.to_string()),
            })
        } else {
            None
        }
    })
}

fn to_rig_tool_definitions(defs: &[ToolDefinition], exclude_mcp: bool) -> Vec<RigToolDefinition> {
    defs.iter()
        .filter(|d| !exclude_mcp || !d.id.starts_with("mcp__"))
        .map(|d| {
            let description = if d.provider_id.is_empty() {
                d.description.clone()
            } else {
                format!("{}\n\nTool group: {}", d.description, d.provider_id)
            };
            RigToolDefinition {
                name: d.id.clone(),
                description,
                parameters: d.parameters.clone(),
            }
        })
        .collect()
}

async fn check_cancellation(
    cancel_token: &CancellationToken,
    event_tx: &EventSender,
    turn_text: &str,
) -> Option<ToolLoopOutcome> {
    let _ = event_tx; // no in-loop Cancelled signal — caller emits the lifecycle event
    if cancel_token.is_cancelled() {
        Some(ToolLoopOutcome::Cancelled(turn_text.to_string()))
    } else {
        None
    }
}


async fn process_model_response(
    contents: &[AssistantContent],
    chat_history: &mut Vec<RigMessage>,
) -> bool {
    let mut has_tool_calls = false;
    let mut assistant_content_items: Vec<AssistantContent> = Vec::new();

    for content in contents {
        if let AssistantContent::ToolCall(_) = content {
            has_tool_calls = true;
        }
        assistant_content_items.push(content.clone());
    }

    let assistant_msg = RigMessage::Assistant {
        id: None,
        content: rig_core::OneOrMany::many(assistant_content_items)
            .unwrap_or_else(|_| rig_core::OneOrMany::one(AssistantContent::text(""))),
    };
    chat_history.push(assistant_msg);

    has_tool_calls
}

fn build_tool_result_message(
    tool_call_id: String,
    result: String,
    tool_output: &crate::tool::ToolOutput,
) -> RigMessage {
    let has_images = !tool_output.images().is_empty();
    if has_images {
        let tool_result_content = UserContent::ToolResult(ToolResult {
            id: tool_call_id,
            call_id: None,
            content: rig_core::OneOrMany::one(ToolResultContent::text(&result)),
        });
        let mut user_contents = vec![tool_result_content];
        for img in tool_output.images() {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&img.bytes);
            user_contents.push(UserContent::Image(
                rig_core::completion::message::Image {
                    data: DocumentSourceKind::Base64(b64),
                    media_type: ImageMediaType::from_mime_type(&img.media_type),
                    detail: None,
                    additional_params: None,
                },
            ));
        }
        RigMessage::User {
            content: rig_core::OneOrMany::many(user_contents).unwrap(),
        }
    } else {
        RigMessage::tool_result(tool_call_id, result)
    }
}

struct ToolCallExecutionResult {
    external_tools: Vec<(crate::inference::tool_call::ToolCallResponse, ToolCallResult)>,
    internal_tool_results: Vec<ToolCallResult>,
    accumulated_system_prompts: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
async fn execute_tool_calls(
    chat_service: &crate::chat::service::ChatService,
    tool_registry: &AgentToolRegistry,
    ctx: &InferenceContext,
    event_tx: &EventSender,
    metrics_ctx: &InferenceMetricsContext,
    chat_history: &mut Vec<RigMessage>,
    all_attachments: &mut Vec<crate::storage::Attachment>,
    contents: &[AssistantContent],
    message_id: &str,
    turn: u32,
    turn_text: Option<&str>,
    turn_reasoning: Option<&Reasoning>,
) -> Result<ToolCallExecutionResult, AppError> {
    let mut result = ToolCallExecutionResult {
        external_tools: Vec::new(),
        internal_tool_results: Vec::new(),
        accumulated_system_prompts: Vec::new(),
    };

    let mut turn_metadata_used = false;

    for content in contents {
        if ctx.cancel_token.is_cancelled() {
            break;
        }

        let AssistantContent::ToolCall(tool_call) = content else {
            continue;
        };

        let tool_name = &tool_call.function.name;
        let mut arguments = tool_call.function.arguments.clone();
        let description = arguments
            .as_object_mut()
            .and_then(|obj| obj.remove("description"))
            .and_then(|v| v.as_str().map(String::from));

        let te_id = crate::core::repository::new_id();

        event_tx.send(InferenceEvent {
            kind: InferenceEventKind::ToolCall {
                id: te_id.clone(),
                provider_call_id: tool_call.id.clone(),
                name: tool_name.clone(),
                arguments: arguments.clone(),
                description: description.clone(),
            },
        });

        tracing::debug!(tool = %tool_name, args = %arguments, "Executing tool");

        // Persist record BEFORE execution (crash resilience).
        // Stamp turn-level metadata (text + reasoning) on the FIRST tool_call
        // of the turn — both fields gate on the same `turn_metadata_used`
        // flag so they stay paired even when turn_text is None.
        let (current_turn_text, current_turn_reasoning) = if !turn_metadata_used {
            turn_metadata_used = true;
            (turn_text.map(|s| s.to_string()), turn_reasoning.cloned())
        } else {
            (None, None)
        };
        let mut te_record = chat_service
            .begin_tool_call(
                &te_id,
                &ctx.chat.id,
                message_id,
                turn,
                &tool_call.id,
                tool_name,
                &arguments,
                description.clone(),
                current_turn_text,
                current_turn_reasoning,
            )
            .await?;

        let start = Instant::now();
        let (text, tool_output) = match tool_registry.execute(tool_name, arguments, ctx).await {
            Ok(output) => {
                let text = output.text_content().to_string();
                tracing::debug!(tool = %tool_name, result = %text, "Tool executed");
                let duration = start.elapsed();
                metrics::record_tool_call(
                    tool_name,
                    &metrics_ctx.user_id,
                    &metrics_ctx.agent_id,
                    duration,
                    "success",
                );
                (text, Some(output))
            }
            Err(e) => {
                tracing::warn!(tool = %tool_name, error = %e, "Tool execution failed");
                let duration = start.elapsed();
                metrics::record_tool_call(
                    tool_name,
                    &metrics_ctx.user_id,
                    &metrics_ctx.agent_id,
                    duration,
                    "error",
                );
                (format!("Error: {e}"), None)
            }
        };

        let hitl_emitted = tool_output.as_ref().and_then(|o| o.hitl().cloned());
        let task_event_emitted = tool_output.as_ref().and_then(|o| o.task_event().cloned());
        let sp = tool_output.as_ref().and_then(|o| o.system_prompt().map(str::to_string));

        if let Some(ref output) = tool_output {
            for attachment in output.attachments() {
                all_attachments.push(attachment.clone());
            }
        }

        let success = tool_output.as_ref().is_some_and(|o| o.is_success());
        let duration_ms = start.elapsed().as_millis() as u64;

        // Persist result AFTER execution
        chat_service
            .finish_tool_call(
                &te_record.id,
                text.clone(),
                success,
                duration_ms,
                sp.clone(),
            )
            .await?;

        // Persist the typed HITL / TaskEvent on the tool_call row.
        if let Some(ref h) = hitl_emitted {
            chat_service.set_hitl(&te_record.id, h.clone()).await?;
        }
        if let Some(ref e) = task_event_emitted {
            chat_service.set_task_event(&te_record.id, e.clone()).await?;
        }
        // Update in-memory record with finished fields so the SSE response is complete
        te_record.result = text.clone();
        te_record.success = success;
        te_record.duration_ms = duration_ms;
        te_record.hitl = hitl_emitted.clone();
        te_record.task_event = task_event_emitted.clone();
        te_record.system_prompt = sp.clone();

        let te_response: crate::inference::tool_call::ToolCallResponse = te_record.into();

        let tool_call_result = ToolCallResult {
            provider_call_id: tool_call.id.clone(),
            tool_name: tool_name.clone(),
            arguments: te_response.arguments.clone(),
            result: text.clone(),
            success,
            duration_ms,
            hitl: hitl_emitted.clone(),
            task_event: task_event_emitted.clone(),
            system_prompt: sp.clone(),
        };

        // A tool pauses the loop when it emits a Pending HITL OR explicitly
        // sets `as_pending_external` (voice tools, etc., resolve via external
        // system callback).
        let is_pending_external = hitl_emitted
            .as_ref()
            .is_some_and(|h| h.status == crate::inference::tool_call::ToolStatus::Pending)
            || tool_output.as_ref().is_some_and(|o| o.is_pending_external());

        if is_pending_external {
            result.external_tools.push((te_response, tool_call_result));
        } else {
            event_tx.send(InferenceEvent {
                kind: InferenceEventKind::ToolResult {
                    name: tool_name.clone(),
                    result: text.clone(),
                    success,
                },
            });
            if let Some(sp_value) = sp {
                result.accumulated_system_prompts.push(sp_value);
            }
            result.internal_tool_results.push(tool_call_result);
            if let Some(output) = tool_output {
                let msg = build_tool_result_message(tool_call.id.clone(), text, &output);
                chat_history.push(msg);
            } else {
                chat_history.push(RigMessage::tool_result(tool_call.id.clone(), text));
            }
        }
    }

    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_tool_loop(
    registry: &ModelProviderRegistry,
    model_group: &ModelGroup,
    system_prompt: &str,
    mut chat_history: Vec<RigMessage>,
    tool_registry: &AgentToolRegistry,
    event_tx: EventSender,
    cancel_token: CancellationToken,
    ctx: &InferenceContext,
    metrics_ctx: &InferenceMetricsContext,
    chat_service: &crate::chat::service::ChatService,
    message_id: &str,
) -> Result<ToolLoopOutcome, AppError> {
    let tool_defs = tool_registry.definitions();
    let rig_tools = to_rig_tool_definitions(tool_defs, tool_registry.mcp_bridge_mode());

    let mut all_attachments: Vec<crate::storage::Attachment> = Vec::new();
    let mut current_system_prompt = system_prompt.to_string();
    let mut last_reasoning: Option<Reasoning> = None;
    let mut final_text = String::new();

    let max_tool_turns = model_group.inference.max_tool_turns;
    for turn in 0..max_tool_turns {
        if let Some(outcome) = check_cancellation(&cancel_token, &event_tx, &final_text).await {
            return Ok(outcome);
        }

        tracing::debug!(turn, "Tool loop turn");

        let max_output = model_group.max_tokens.unwrap_or(model_group.inference.default_max_tokens) as usize;
        chat_history = crate::inference::context::truncate_history(
            chat_history,
            &current_system_prompt,
            &model_group.main.model_id,
            model_group.context_window,
            max_output,
            model_group.inference.history_truncation_pct,
        );

        // Drop leading orphaned tool_results whose tool_use was truncated away
        while let Some(RigMessage::User { content }) = chat_history.first() {
            if content.iter().any(|c| matches!(c, UserContent::ToolResult(_))) {
                chat_history.remove(0);
            } else {
                break;
            }
        }

        let mut turn_text = String::new();
        let contents = match stream_with_retry_and_fallback(
            registry,
            model_group,
            &current_system_prompt,
            &chat_history,
            &rig_tools,
            &event_tx,
            &cancel_token,
            &mut turn_text,
            metrics_ctx,
        )
        .await?
        {
            StreamResult::Contents(c) => c,
            StreamResult::Cancelled => {
                return Ok(ToolLoopOutcome::Cancelled(turn_text));
            }
        };

        last_reasoning = extract_reasoning(&contents);

        let has_tool_calls =
            process_model_response(&contents, &mut chat_history).await;

        if !has_tool_calls {
            final_text = turn_text;
            break;
        }

        let turn_text_opt = if turn_text.is_empty() { None } else { Some(turn_text.as_str()) };

        let exec_result = execute_tool_calls(
            chat_service,
            tool_registry,
            ctx,
            &event_tx,
            metrics_ctx,
            &mut chat_history,
            &mut all_attachments,
            &contents,
            message_id,
            turn as u32,
            turn_text_opt,
            last_reasoning.as_ref(),
        )
        .await?;

        if let Some(outcome) = check_cancellation(&cancel_token, &event_tx, &turn_text).await {
            return Ok(outcome);
        }

        if ctx.shutdown_token.is_cancelled() {
            tracing::info!("Server shutting down, stopping tool loop after current tool");
            return Ok(ToolLoopOutcome::Completed {
                text: turn_text,
                attachments: all_attachments,
                lifecycle_event: None,
                reasoning: last_reasoning,
            });
        }

        if !exec_result.external_tools.is_empty() {
            let system_prompt_injection = exec_result.external_tools.last()
                .and_then(|(_, tcr)| tcr.system_prompt.clone());
            let tool_calls = exec_result.external_tools.into_iter()
                .map(|(te, _)| te)
                .collect();

            return Ok(ToolLoopOutcome::ExternalToolPending {
                turn_text,
                tool_calls,
                system_prompt: system_prompt_injection,
            });
        }

        // Check for task lifecycle events (complete_task, fail_task, defer_task)
        // and break immediately — no need for another inference turn.
        let lifecycle_event = exec_result
            .internal_tool_results
            .iter()
            .find_map(|r| r.task_event.clone());
        if lifecycle_event.is_some() {
            return Ok(ToolLoopOutcome::Completed {
                text: turn_text,
                attachments: all_attachments,
                lifecycle_event,
                reasoning: last_reasoning,
            });
        }

        for sp in exec_result.accumulated_system_prompts {
            current_system_prompt.push_str("\n\n");
            current_system_prompt.push_str(&sp);
        }

        if turn == max_tool_turns - 1 {
            event_tx.send(InferenceEvent {
                    kind: InferenceEventKind::Failed {
                        error: "Max tool turns reached".to_string(),
                    },
                });
        }
    }

    // Deduplicate attachments by path (e.g. produce_file + complete_task with same deliverable)
    let mut seen_paths = std::collections::HashSet::new();
    all_attachments.retain(|a| seen_paths.insert(a.path.clone()));

    Ok(ToolLoopOutcome::Completed {
        text: final_text,
        attachments: all_attachments,
        lifecycle_event: None,
        reasoning: last_reasoning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::completion::AssistantContent;

    #[test]
    fn extract_reasoning_from_contents() {
        let contents = vec![
            AssistantContent::Reasoning(
                rig_core::completion::message::Reasoning::new_with_signature(
                    "thinking hard",
                    Some("sig-123".to_string()),
                )
                .with_id("r-1".to_string()),
            ),
            AssistantContent::text("final answer"),
        ];
        let r = extract_reasoning(&contents);
        assert!(r.is_some());
        let r = r.unwrap();
        assert_eq!(r.content, "thinking hard");
        assert_eq!(r.id, Some("r-1".to_string()));
        assert_eq!(r.signature, Some("sig-123".to_string()));
    }

    #[test]
    fn extract_reasoning_returns_none_when_absent() {
        let contents = vec![
            AssistantContent::text("just text"),
        ];
        assert!(extract_reasoning(&contents).is_none());
    }

    #[test]
    fn extract_reasoning_joins_multi_chunk() {
        let contents = vec![
            AssistantContent::Reasoning(
                rig_core::completion::message::Reasoning::multi(
                    vec!["chunk1 ".to_string(), "chunk2".to_string()],
                ),
            ),
        ];
        let r = extract_reasoning(&contents).unwrap();
        assert_eq!(r.content, "chunk1 \nchunk2");
    }
}
