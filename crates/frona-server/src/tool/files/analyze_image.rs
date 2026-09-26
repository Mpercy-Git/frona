use std::sync::Arc;

use base64::Engine;
use rig_core::completion::Message as RigMessage;
use rig_core::completion::message::{
    DocumentSourceKind, Image, ImageMediaType, MimeType, UserContent,
};
use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::core::error::AppError;
use crate::inference::config::ModelGroup;
use crate::inference::ModelProviderRegistry;
use crate::inference::usage::{InferenceKind, UsageContext, UsageService};
use crate::storage::service::StorageService;
use frona_derive::agent_tool;

use super::super::sandbox::SandboxManager;
use super::super::{InferenceContext, ToolOutput, active_chat};
use super::read::{is_supported_image, prepare_image};

const SYSTEM_PROMPT: &str = "You answer questions about an image for an assistant that cannot see it. \
     Answer only from what is visible in the image. Quote any text exactly as written. \
     If the image does not show enough to answer, say so plainly rather than guessing. \
     Reply with the answer only — no preamble.";

/// Ask a vision-capable model a question about an image file and return its
/// answer as text. Lets a text-only agent use images, and lets any agent get a
/// targeted answer without spending its own context on the raw image.
pub struct AnalyzeImageTool {
    storage: StorageService,
    sandbox_manager: Arc<SandboxManager>,
    registry: ModelProviderRegistry,
    usage_service: UsageService,
    prompts: PromptLoader,
}

impl AnalyzeImageTool {
    pub fn new(
        storage: StorageService,
        sandbox_manager: Arc<SandboxManager>,
        registry: ModelProviderRegistry,
        usage_service: UsageService,
        prompts: PromptLoader,
    ) -> Self {
        Self {
            storage,
            sandbox_manager,
            registry,
            usage_service,
            prompts,
        }
    }
}

#[agent_tool]
impl AnalyzeImageTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let path_arg = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing 'path' parameter".into()))?;
        let question = arguments
            .get("question")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| AppError::Validation("Missing 'question' parameter".into()))?;

        let resolved = super::resolve_path(path_arg, ctx, &self.storage)?;
        let sandbox = self.sandbox_manager.for_tool(ctx).await?;
        if !sandbox.is_readable(&resolved) {
            return Ok(ToolOutput::error(format!(
                "Read denied by sandbox policy: {} (resolved: {})",
                path_arg,
                resolved.display(),
            )));
        }
        if !tokio::fs::try_exists(&resolved).await.unwrap_or(false) {
            return Ok(ToolOutput::error(format!("file not found: {}", path_arg)));
        }
        let bytes = tokio::fs::read(&resolved)
            .await
            .map_err(|e| AppError::Internal(format!("read {}: {e}", resolved.display())))?;

        let mime = infer::get(&bytes).map(|t| t.mime_type().to_string());
        let Some(mime) = mime.filter(|m| is_supported_image(m)) else {
            return Ok(ToolOutput::error(format!(
                "{} is not a supported image (PNG, JPEG, GIF or WebP)",
                path_arg
            )));
        };
        let image = match prepare_image(&bytes, &mime, path_arg) {
            Ok(img) => img,
            Err(msg) => return Ok(ToolOutput::error(msg)),
        };

        let Some(vision_group) = crate::inference::vision::resolve_vision_model_group(
            &self.registry,
            &self.usage_service,
        ) else {
            return Ok(ToolOutput::error(
                "No vision-capable model is configured, so images can't be analysed. \
                 Ask the user to add a model group named `vision`, or one whose model accepts image input.",
            ));
        };
        let group = analysis_group(&vision_group);

        let request = RigMessage::User {
            content: vec![
                UserContent::text(question),
                UserContent::Image(Image {
                    data: DocumentSourceKind::Base64(
                        base64::engine::general_purpose::STANDARD.encode(&image.bytes),
                    ),
                    media_type: ImageMediaType::from_mime_type(&image.media_type),
                    detail: None,
                    additional_params: None,
                }),
            ],
        };
        let usage_ctx = UsageContext::new(
            InferenceKind::Transcription {
                agent_id: ctx.agent.id.clone(),
                chat_id: active_chat(ctx).map(|c| c.id.clone()).unwrap_or_default(),
                message_id: String::new(),
            },
            ctx.user.id.clone(),
            group.name.clone(),
        );

        match crate::inference::text_inference(
            &self.registry,
            &group,
            SYSTEM_PROMPT,
            vec![request],
            &self.usage_service,
            &usage_ctx,
        )
        .await
        {
            Ok(answer) => Ok(ToolOutput::text(answer.trim().to_string())),
            Err(e) => {
                tracing::warn!(
                    vision_model = %group.main.as_str(),
                    error = %e,
                    "analyze_image inference failed",
                );
                Ok(ToolOutput::error(format!(
                    "The vision model ({}) could not analyse {}: {e}",
                    group.main.as_str(),
                    path_arg
                )))
            }
        }
    }
}

/// The resolved vision group, with enough output budget for a detailed answer
/// (e.g. transcribing a dense screenshot).
fn analysis_group(base: &ModelGroup) -> ModelGroup {
    ModelGroup {
        name: "vision".to_string(),
        max_tokens: Some(base.max_tokens.unwrap_or(2048).max(2048)),
        ..base.clone()
    }
}
