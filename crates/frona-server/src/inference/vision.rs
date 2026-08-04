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

use super::config::{InferenceConfig, ModelGroup};
use super::registry::ModelProviderRegistry;
use super::usage::{UsageContext, UsageService};
use super::{InferenceKind, ModelRef};

const TRANSCRIBE_SYSTEM: &str =
    "You transcribe images for a downstream assistant that cannot see them. \
     Reply with only the transcription/description — no preamble, no commentary.";

const TRANSCRIBE_INSTRUCTION: &str =
    "Transcribe all text in the image verbatim, preserving structure (headings, \
     lists, tables, reference numbers). Briefly describe any diagrams, photos, or \
     figures. Do not summarize or add commentary.";

/// Effective vision capability for `model_ref`, letting explicit config
/// overrides win over the catalog result (`catalog_says`).
///
/// Precedence: `text_only_models` → `vision_models` → catalog → unknown. When
/// the catalog is silent and `transcribe_when_vision_unknown` is set, unknown
/// resolves to `Some(false)` so images get handled rather than risking a 404.
pub fn resolve_vision_capability(
    model_ref: &ModelRef,
    inference: &InferenceConfig,
    catalog_says: Option<bool>,
) -> Option<bool> {
    if model_matches_any(model_ref, &inference.text_only_models) {
        return Some(false);
    }
    if model_matches_any(model_ref, &inference.vision_models) {
        return Some(true);
    }
    match catalog_says {
        Some(v) => Some(v),
        None if inference.transcribe_when_vision_unknown => Some(false),
        None => None,
    }
}

/// Match a model ref against a configured id list. An entry matches the bare
/// model id, the "provider/model_id" pair, or the final path segment of the
/// model id (handling vendor-prefixed ids like "deepseek/deepseek-v4-flash").
fn model_matches_any(model_ref: &ModelRef, list: &[String]) -> bool {
    let model_id = model_ref.model_id.as_str();
    let composite = format!("{}/{}", model_ref.provider, model_id);
    let last_segment = model_id.rsplit('/').next().unwrap_or(model_id);
    list.iter()
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .any(|e| {
            model_id.eq_ignore_ascii_case(e)
                || composite.eq_ignore_ascii_case(e)
                || last_segment.eq_ignore_ascii_case(e)
        })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn mref(provider: &str, model_id: &str) -> ModelRef {
        ModelRef {
            provider: provider.into(),
            model_id: model_id.into(),
            additional_params: None,
        }
    }

    #[test]
    fn text_only_override_beats_catalog() {
        let mut c = InferenceConfig::default();
        c.text_only_models = vec!["deepseek-v4-flash".into()];
        // catalog wrongly claims vision; override forces text-only
        let m = mref("openrouter", "deepseek/deepseek-v4-flash");
        assert_eq!(resolve_vision_capability(&m, &c, Some(true)), Some(false));
    }

    #[test]
    fn vision_override_forces_true_over_unknown() {
        let mut c = InferenceConfig::default();
        c.vision_models = vec!["some-model".into()];
        assert_eq!(resolve_vision_capability(&mref("x", "some-model"), &c, None), Some(true));
    }

    #[test]
    fn text_only_wins_when_listed_in_both() {
        let mut c = InferenceConfig::default();
        c.vision_models = vec!["m".into()];
        c.text_only_models = vec!["m".into()];
        assert_eq!(resolve_vision_capability(&mref("x", "m"), &c, None), Some(false));
    }

    #[test]
    fn unknown_respects_toggle() {
        let c = InferenceConfig::default();
        assert_eq!(resolve_vision_capability(&mref("x", "m"), &c, None), None);
        let mut c2 = InferenceConfig::default();
        c2.transcribe_when_vision_unknown = true;
        assert_eq!(resolve_vision_capability(&mref("x", "m"), &c2, None), Some(false));
    }

    #[test]
    fn catalog_passes_through_without_overrides() {
        let c = InferenceConfig::default();
        assert_eq!(resolve_vision_capability(&mref("x", "m"), &c, Some(true)), Some(true));
        assert_eq!(resolve_vision_capability(&mref("x", "m"), &c, Some(false)), Some(false));
    }

    #[test]
    fn matching_handles_vendor_prefix_and_composite() {
        let list = vec!["deepseek-v4-flash".to_string()];
        assert!(model_matches_any(&mref("openrouter", "deepseek/deepseek-v4-flash"), &list));
        assert!(model_matches_any(&mref("deepseek", "deepseek-v4-flash"), &list));
        assert!(!model_matches_any(&mref("openai", "gpt-4o"), &list));

        let composite = vec!["openai/gpt-4o".to_string()];
        assert!(model_matches_any(&mref("openai", "gpt-4o"), &composite));
    }
}
