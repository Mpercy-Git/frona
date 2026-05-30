pub mod hooks;
pub mod config;
pub mod context;
pub mod conversation;
pub mod error;
pub mod provider;
pub mod registry;
pub mod request;
pub mod retry;
pub mod tool_call;
pub mod tool_loop;

pub use error::InferenceError;
pub use provider::ModelRef;
pub use registry::ModelProviderRegistry;
pub use request::{InferenceRequest, InferenceResponse, InferenceContext};
pub use crate::chat::broadcast::EventSender;
pub use rig_core::completion::request::Usage;
pub use tool_loop::{InferenceEvent, InferenceEventKind};


use rig_core::completion::Message as RigMessage;

use crate::core::error::AppError;
use crate::core::metrics::InferenceMetricsContext;

use self::config::ModelGroup;

pub async fn inference(request: InferenceRequest) -> Result<InferenceResponse, AppError> {
    let metrics_ctx = InferenceMetricsContext {
        user_id: request.ctx.user.id.clone(),
        agent_id: request.ctx.agent.id.clone(),
        model_group: request.model_group.name.clone(),
    };

    if request.tool_registry.is_empty() {
        use tool_loop::extract_reasoning;
        let max_output = request
            .model_group
            .max_tokens
            .unwrap_or(request.model_group.inference.default_max_tokens)
            as usize;
        let history = context::truncate_history(
            request.history,
            &request.system_prompt,
            &request.model_group.main.model_id,
            request.model_group.context_window,
            max_output,
            request.model_group.inference.history_truncation_pct,
        );

        let mut response_text = String::new();
        let event_tx = &request.ctx.event_tx;
        match retry::stream_with_retry_and_fallback(
            &request.registry,
            &request.model_group,
            &request.system_prompt,
            &history,
            &[],
            event_tx,
            &request.cancel_token,
            &mut response_text,
            &metrics_ctx,
        )
        .await?
        {
            retry::StreamResult::Contents(contents) => {
                let reasoning = extract_reasoning(&contents);
                event_tx.send(InferenceEvent {
                    kind: InferenceEventKind::Done(String::new()),
                });
                Ok(InferenceResponse::Completed {
                    text: response_text,
                    attachments: vec![],
                    lifecycle_event: None,
                    reasoning,
                })
            }
            retry::StreamResult::Cancelled => {
                event_tx.send(InferenceEvent {
                    kind: InferenceEventKind::Cancelled(String::new()),
                });
                Ok(InferenceResponse::Cancelled(response_text))
            }
        }
    } else {
        let event_tx = request.ctx.event_tx.clone();
        let outcome = tool_loop::run_tool_loop(
            &request.registry,
            &request.model_group,
            &request.system_prompt,
            request.history,
            &request.tool_registry,
            event_tx,
            request.cancel_token,
            &request.ctx,
            &metrics_ctx,
            &request.chat_service,
            &request.message_id,
        )
        .await?;

        Ok(match outcome {
            tool_loop::ToolLoopOutcome::Completed { text, attachments, lifecycle_event, reasoning } => {
                InferenceResponse::Completed { text, attachments, lifecycle_event, reasoning }
            }
            tool_loop::ToolLoopOutcome::Cancelled(text) => InferenceResponse::Cancelled(text),
            tool_loop::ToolLoopOutcome::ExternalToolPending {
                turn_text,
                tool_calls,
                system_prompt,
            } => InferenceResponse::ExternalToolPending {
                turn_text,
                tool_calls,
                system_prompt,
            },
        })
    }
}

pub async fn text_inference(
    registry: &ModelProviderRegistry,
    model_group: &ModelGroup,
    system_prompt: &str,
    history: Vec<RigMessage>,
    metrics_ctx: &InferenceMetricsContext,
) -> Result<String, InferenceError> {
    let (contents, _usage) = retry::inference_with_retry_and_fallback(
        registry,
        model_group,
        system_prompt,
        history,
        vec![],
        metrics_ctx,
    )
    .await?;
    provider::extract_text_from_choice(&contents)
}

pub async fn structured_inference<T>(
    registry: &ModelProviderRegistry,
    model_group: &ModelGroup,
    system_prompt: &str,
    history: Vec<RigMessage>,
    metrics_ctx: &InferenceMetricsContext,
) -> Result<T, InferenceError>
where
    T: schemars::JsonSchema + serde::de::DeserializeOwned + Send + 'static,
{
    let schema = serde_json::to_value(schemars::schema_for!(T))
        .map_err(|e| InferenceError::InferenceFailed(format!("schema_for failed: {e}")))?;
    let value = retry::structured_inference_with_retry_and_fallback(
        registry,
        model_group,
        system_prompt,
        history,
        schema,
        metrics_ctx,
    )
    .await?;
    serde_json::from_value::<T>(value).map_err(|e| {
        InferenceError::InferenceFailed(format!("submit args deserialization failed: {e}"))
    })
}
