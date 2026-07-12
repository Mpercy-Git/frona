//! Vision fallback for text-only agent models.
//!
//! When an agent's model can't accept image input, sending images makes
//! providers (e.g. OpenRouter) reject the whole request with a 404. Instead of
//! failing — or silently dropping the image — we transcribe/describe the image
//! with a vision-capable model and inline the result as text so the agent can
//! still use it. The vision model is resolved with the same "named override,
//! else sensible default" convention the other utilities use, except the
//! default here is capability-aware (auto-select a model that supports images).

use rig_core::completion::Message as RigMessage;
use rig_core::completion::message::UserContent;

use super::config::ModelGroup;
use super::registry::ModelProviderRegistry;
use super::usage::{UsageContext, UsageService};
use super::InferenceKind;

const TRANSCRIBE_SYSTEM: &str =
    "You transcribe images for a downstream assistant that cannot see them. \
     Reply with only the transcription/description — no preamble, no commentary.";

const TRANSCRIBE_INSTRUCTION: &str =
    "Transcribe all text in the image verbatim, preserving structure (headings, \
     lists, tables, reference numbers). Briefly describe any diagrams, photos, or \
     figures. Do not summarize or add commentary.";

/// Resolve a vision-capable model group for the transcription pre-pass.
///
/// 1. Override: a model group explicitly named `vision` (same convention as
///    `title`/`compaction`).
/// 2. Auto-select: the first configured group (deterministic by name) whose
///    main model the catalog reports as supporting image input.
/// 3. `None` when nothing vision-capable is available — callers should fall
///    back to stripping images.
pub fn resolve_vision_model_group(
    registry: &ModelProviderRegistry,
    usage_service: &UsageService,
) -> Option<ModelGroup> {
    if let Ok(group) = registry.get_model_group("vision") {
        return Some(group.clone());
    }
    let mut candidates: Vec<&ModelGroup> = registry
        .iter_model_groups()
        .filter(|g| usage_service.model_supports_vision(&g.main) == Some(true))
        .collect();
    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    candidates.into_iter().next().cloned()
}

/// Derive a transcription-tuned group from the resolved vision base: keep the
/// model + fallbacks, but ensure enough output budget for a dense page.
fn transcription_group(base: &ModelGroup) -> ModelGroup {
    ModelGroup {
        name: "transcription".to_string(),
        main: base.main.clone(),
        fallbacks: base.fallbacks.clone(),
        max_tokens: Some(base.max_tokens.unwrap_or(2048).max(2048)),
        temperature: base.temperature,
        context_window: base.context_window,
        retry: base.retry.clone(),
        inference: base.inference.clone(),
    }
}

/// Replace embedded image blocks in each user message with a text transcription
/// produced by `vision_group`. On a per-message transcription failure, that
/// message's images are stripped with a marker instead (never left to 404 the
/// agent turn). Returns the number of images transcribed.
#[allow(clippy::too_many_arguments)]
pub async fn transcribe_images_in_history(
    history: &mut [RigMessage],
    vision_group: &ModelGroup,
    registry: &ModelProviderRegistry,
    usage_service: &UsageService,
    user_id: &str,
    agent_id: &str,
    chat_id: &str,
    message_id: &str,
) -> usize {
    let group = transcription_group(vision_group);
    let mut transcribed = 0;

    for msg in history.iter_mut() {
        let RigMessage::User { content } = msg else {
            continue;
        };
        let images: Vec<UserContent> = content
            .iter()
            .filter(|c| matches!(c, UserContent::Image(_)))
            .cloned()
            .collect();
        if images.is_empty() {
            continue;
        }

        // Build the transcription request: instruction + the image(s).
        let mut req_content: Vec<UserContent> = Vec::with_capacity(images.len() + 1);
        req_content.push(UserContent::text(TRANSCRIBE_INSTRUCTION));
        req_content.extend(images.iter().cloned());
        let Ok(req_content) = rig_core::OneOrMany::many(req_content) else {
            continue;
        };
        let req_msg = RigMessage::User { content: req_content };

        let usage_ctx = UsageContext::new(
            InferenceKind::Transcription {
                agent_id: agent_id.to_string(),
                chat_id: chat_id.to_string(),
                message_id: message_id.to_string(),
            },
            user_id.to_string(),
            group.name.clone(),
        );

        let replacement = match crate::inference::text_inference(
            registry,
            &group,
            TRANSCRIBE_SYSTEM,
            vec![req_msg],
            usage_service,
            &usage_ctx,
        )
        .await
        {
            Ok(text) => {
                transcribed += images.len();
                format!("<image_transcription>\n{}\n</image_transcription>", text.trim())
            }
            Err(e) => {
                tracing::warn!(error = %e, "image transcription failed; stripping instead");
                "[Attachment omitted: image could not be processed for this model.]".to_string()
            }
        };

        // Rebuild the message with images removed and the replacement appended.
        let kept: Vec<UserContent> = content
            .iter()
            .filter(|c| !matches!(c, UserContent::Image(_)))
            .cloned()
            .chain(std::iter::once(UserContent::text(&replacement)))
            .collect();
        if let Ok(new_content) = rig_core::OneOrMany::many(kept) {
            *content = new_content;
        }
    }

    transcribed
}
