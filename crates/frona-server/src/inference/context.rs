use rig_core::completion::Message as RigMessage;

/// Provider-calibrated input size for one exact, in-memory request. This is an
/// estimate, not a token limit: old usage cannot identify individual row costs,
/// and system prompts/tool definitions may have changed since that call.
#[derive(Clone, Copy, Debug)]
pub struct InputTokenEstimate {
    tokens: usize,
    fingerprint: u64,
}

impl InputTokenEstimate {
    pub(crate) fn new(
        tokens: usize,
        model: &crate::inference::ModelConfig,
        system_prompt: &str,
        history: &[RigMessage],
        tools: &[rig_core::completion::request::ToolDefinition],
    ) -> Self {
        Self {
            tokens,
            fingerprint: Self::fingerprint(model, system_prompt, history, tools),
        }
    }

    pub(crate) fn for_request(
        self,
        model: &crate::inference::ModelConfig,
        system_prompt: &str,
        history: &[RigMessage],
        tools: &[rig_core::completion::request::ToolDefinition],
    ) -> Option<usize> {
        (self.fingerprint == Self::fingerprint(model, system_prompt, history, tools))
            .then_some(self.tokens)
    }

    fn fingerprint(
        model: &crate::inference::ModelConfig,
        system_prompt: &str,
        history: &[RigMessage],
        tools: &[rig_core::completion::request::ToolDefinition],
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        model.as_str().hash(&mut hash);
        serde_json::to_vec(&(&model.provider, &model.request_settings.extra_params))
            .expect("provider configuration is serializable")
            .hash(&mut hash);
        system_prompt.hash(&mut hash);
        serde_json::to_vec(&(history, tools))
            .expect("model request is serializable")
            .hash(&mut hash);
        hash.finish()
    }
}

/// Last-resort context window when the model isn't in the catalog AND no
/// config override is set. 128K is the floor most modern chat models meet.
/// Used by `ProviderRegistry::resolve_model_group` when baking the window
/// into `ModelGroup.context_window`.
pub const DEFAULT_CONTEXT_WINDOW: usize = 128_000;

pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4 + 4
}

pub fn estimate_message_tokens(msg: &RigMessage) -> usize {
    let content_len: usize = match msg {
        RigMessage::User { content } => content
            .iter()
            .map(|c| -> usize {
                match c {
                    rig_core::completion::message::UserContent::Text(t) => t.text.len(),
                    rig_core::completion::message::UserContent::ToolResult(tr) => tr
                        .content
                        .iter()
                        .map(|c| -> usize {
                            match c {
                                rig_core::completion::message::ToolResultContent::Text(t) => {
                                    t.text.len()
                                }
                                _ => 100,
                            }
                        })
                        .sum::<usize>(),
                    _ => 100,
                }
            })
            .sum::<usize>(),
        RigMessage::Assistant { content, .. } => content
            .iter()
            .map(|c| -> usize {
                match c {
                    rig_core::completion::AssistantContent::Text(t) => t.text.len(),
                    rig_core::completion::AssistantContent::ToolCall(tc) => {
                        tc.function.name.len() + tc.function.arguments.to_string().len()
                    }
                    _ => 100,
                }
            })
            .sum::<usize>(),
        RigMessage::System { content } => content.len(),
    };

    content_len / 4 + 4
}

pub fn estimate_messages_tokens(messages: &[RigMessage], system_prompt: &str) -> usize {
    let system_tokens = estimate_tokens(system_prompt);
    let message_tokens: usize = messages.iter().map(estimate_message_tokens).sum();
    system_tokens + message_tokens
}

/// Operates on a resolved window budget (typically from
/// `resolve_context_window`). Doesn't know about model identity.
pub fn needs_compaction(
    messages: &[RigMessage],
    system_prompt: &str,
    context_window: usize,
    max_output_tokens: usize,
    compaction_trigger_pct: usize,
) -> bool {
    let used = estimate_messages_tokens(messages, system_prompt);
    let available = context_window.saturating_sub(max_output_tokens);
    used > available * compaction_trigger_pct / 100
}

/// Retain the rolling summary and the newest suffix that begins with user input.
/// Tool-result messages (including ones with text) cannot open that suffix.
/// A nonempty conversation must retain a complete user-led turn; dropping every
/// live message and sending only the summary would answer a different request.
pub fn truncate_history(
    mut history: Vec<RigMessage>,
    system_prompt: &str,
    context_window: usize,
    max_output_tokens: usize,
    history_truncation_pct: usize,
) -> Result<Vec<RigMessage>, String> {
    use rig_core::completion::message::UserContent;

    let budget = context_window
        .saturating_sub(max_output_tokens)
        .saturating_sub(estimate_tokens(system_prompt))
        * history_truncation_pct
        / 100;

    // Empty input intentionally means a system-only request. The provider adds
    // an empty user message; account for it without turning a discarded history
    // into this different kind of request.
    if history.is_empty() {
        return if estimate_message_tokens(&RigMessage::user("")) <= budget {
            Ok(history)
        } else {
            Err("Empty user prompt exceeds the history budget".into())
        };
    }

<<<<<<< ours
    let original_len = history.len();

    // The rolling summary is the only record of everything compacted away
    // (typically the conversation's opening request), so it is pinned: dropping
    // it first, as plain oldest-first trimming would, is exactly how the agent
    // "forgets" what it was originally asked.
    let pinned = history.first().filter(|m| is_summary_message(m)).cloned();
    let pinned_cost = pinned.as_ref().map(estimate_message_tokens).unwrap_or(0);
    let tail_budget = budget.saturating_sub(pinned_cost);
    let tail_start = usize::from(pinned.is_some());

    let mut result: Vec<RigMessage> = Vec::new();
    let mut used = 0usize;
||||||| base
    let original_len = history.len();
    let mut result: Vec<RigMessage> = Vec::new();
    let mut used = 0usize;
=======
    let summary_end = usize::from(matches!(history.first(), Some(RigMessage::User { content })
        if matches!(content.as_slice(), [UserContent::Text(text)]
            if text.text.starts_with("<conversation_summary>\n")
                && text.text.ends_with("\n</conversation_summary>"))));
    let summary_tokens = history[..summary_end]
        .iter()
        .map(estimate_message_tokens)
        .sum::<usize>();
    if summary_tokens > budget {
        return Err("Conversation summary exceeds the history budget".into());
    }
    if summary_end == history.len() {
        return Ok(history);
    }
>>>>>>> upstream

<<<<<<< ours
    for msg in history.into_iter().skip(tail_start).rev() {
        let cost = estimate_message_tokens(&msg);
        if used + cost > tail_budget {
||||||| base
    for msg in history.into_iter().rev() {
        let cost = estimate_message_tokens(&msg);
        if used + cost > budget {
=======
    let mut used = summary_tokens;
    let mut start = None;
    for index in (summary_end..history.len()).rev() {
        used += estimate_message_tokens(&history[index]);
        if used > budget {
>>>>>>> upstream
            break;
        }
        if matches!(&history[index], RigMessage::User { content }
            if !content.is_empty() && content.iter().all(|item| !matches!(item, UserContent::ToolResult(_))))
        {
            start = Some(index);
        }
    }
    let start = start.ok_or("No complete user-led history fits the context budget")?;
    validate_tool_groups(&history[start..])?;
    if start > summary_end {
        tracing::warn!(
            budget,
            dropped_messages = start - summary_end,
            kept_messages = history.len() - start + summary_end,
            "history truncated at a user boundary, preserving the rolling summary"
        );
        history.drain(summary_end..start);
    }
    Ok(history)
}

<<<<<<< ours
    result.reverse();
    // Cutting mid-conversation can leave the tail opening on a model turn or on
    // a tool result whose call was dropped. Gemini (and others) reject a
    // conversation that doesn't start on a user turn, or a function response
    // with no call before it, so trim to the first plain user message.
    let leading_invalid = result
        .iter()
        .take_while(|m| !is_plain_user_message(m))
        .count();
    if leading_invalid < result.len() {
        result.drain(..leading_invalid);
    }
    if let Some(summary) = pinned {
        result.insert(0, summary);
    }
    // We only get here when the history is over budget and messages are being
    // dropped oldest-first - a silent context loss the caller can't see. Surface
    // it: for the compaction summarizer (`text_inference`) this means the backlog
    // exceeded the summarizer's own window and the oldest messages won't reach the
    // summary either. No-op truncations returned above, so this never fires spuriously.
    tracing::warn!(
        estimated_tokens = total,
        budget,
        dropped_messages = original_len - result.len(),
        kept_messages = result.len(),
        "history over budget: dropped oldest messages to fit (silent context loss)"
    );
    result
||||||| base
    result.reverse();
    // We only get here when the history is over budget and messages are being
    // dropped oldest-first - a silent context loss the caller can't see. Surface
    // it: for the compaction summarizer (`text_inference`) this means the backlog
    // exceeded the summarizer's own window and the oldest messages won't reach the
    // summary either. No-op truncations returned above, so this never fires spuriously.
    tracing::warn!(
        estimated_tokens = total,
        budget,
        dropped_messages = original_len - result.len(),
        kept_messages = result.len(),
        "history over budget: dropped oldest messages to fit (silent context loss)"
    );
    result
=======
/// Keep every retained call paired with exactly one result before the next
/// assistant response. Results may span several User messages, or share one
/// message with text, so User roles alone are not sufficient to check pairing.
fn validate_tool_groups(history: &[RigMessage]) -> Result<(), String> {
    use rig_core::completion::{AssistantContent, message::UserContent};
    let mut pending = std::collections::HashSet::new();
    for message in history {
        match message {
            RigMessage::Assistant { content, .. } => {
                if !pending.is_empty() {
                    return Err(
                        "Assistant message interrupts an incomplete tool-result group".into(),
                    );
                }
                for item in content {
                    if let AssistantContent::ToolCall(call) = item
                        && !pending.insert(&call.id)
                    {
                        return Err("Tool-call group contains a duplicate call ID".into());
                    }
                }
            }
            RigMessage::User { content } => {
                for item in content {
                    if let UserContent::ToolResult(result) = item
                        && !pending.remove(&result.call)
                    {
                        return Err("Tool result has no matching pending call".into());
                    }
                }
                if !pending.is_empty()
                    && content
                        .iter()
                        .any(|item| !matches!(item, UserContent::ToolResult(_)))
                {
                    return Err("User input interrupts an incomplete tool-result group".into());
                }
            }
            RigMessage::System { .. } if !pending.is_empty() => {
                return Err("System message interrupts an incomplete tool-result group".into());
            }
            RigMessage::System { .. } => {}
        }
    }
    if pending.is_empty() {
        Ok(())
    } else {
        Err("History ends with an incomplete tool-result group".into())
    }
>>>>>>> upstream
}

const SUMMARY_OPEN_TAG: &str = "<conversation_summary>";

fn is_summary_message(msg: &RigMessage) -> bool {
    match msg {
        RigMessage::User { content } => content.iter().any(|c| {
            matches!(c, rig_core::completion::message::UserContent::Text(t)
                if t.text.starts_with(SUMMARY_OPEN_TAG))
        }),
        _ => false,
    }
}

fn is_plain_user_message(msg: &RigMessage) -> bool {
    match msg {
        RigMessage::User { content } => !content
            .iter()
            .any(|c| matches!(c, rig_core::completion::message::UserContent::ToolResult(_))),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrated_input_is_bound_to_model_provider_system_history_and_tools() {
        let mut model = crate::inference::ModelConfig {
            catalog_provider: String::new(),
            provider_handle: crate::handle!("fixture"),
            model_id: "model".into(),
            provider: crate::core::config::ProviderModel::Generic,
            request_settings: Default::default(),
        };
        let history = vec![RigMessage::user("exact history")];
        let tools = vec![rig_core::completion::request::ToolDefinition {
            name: "read".into(),
            description: "exact tool".into(),
            parameters: serde_json::json!({"type":"object"}),
        }];
        let estimate = InputTokenEstimate::new(123, &model, "exact system", &history, &tools);
        assert_eq!(
            estimate.for_request(&model, "exact system", &history, &tools),
            Some(123)
        );
        assert_eq!(
            estimate.for_request(&model, "changed system", &history, &tools),
            None
        );
        assert_eq!(
            estimate.for_request(
                &model,
                "exact system",
                &[RigMessage::user("changed")],
                &tools
            ),
            None
        );
        assert_eq!(
            estimate.for_request(&model, "exact system", &history, &[]),
            None
        );
        model
            .request_settings
            .extra_params
            .insert("changed".into(), serde_json::json!(true));
        assert_eq!(
            estimate.for_request(&model, "exact system", &history, &tools),
            None
        );
        model.request_settings.extra_params.clear();
        model.model_id = "other-model".into();
        assert_eq!(
            estimate.for_request(&model, "exact system", &history, &tools),
            None
        );
        model.model_id = "model".into();
        model.provider = crate::core::config::ProviderModel::Anthropic {
            params: Default::default(),
        };
        assert_eq!(
            estimate.for_request(&model, "exact system", &history, &tools),
            None
        );
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 4);
        assert_eq!(estimate_tokens("hello world"), 6); // 11/4 + 4 = 6
    }

    #[test]
    fn test_needs_compaction() {
        let short_msg = vec![RigMessage::user("hello")];
        assert!(!needs_compaction(&short_msg, "system", 200_000, 8192, 80));
    }

    #[test]
    fn test_truncate_history_within_budget() {
        let msgs = vec![RigMessage::user("hello"), RigMessage::user("world")];
        let result = truncate_history(msgs.clone(), "system", 200_000, 8192, 90).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_truncate_history_exceeds_budget() {
        let long = "x".repeat(500_000);
        let msgs = vec![RigMessage::user(&long), RigMessage::user("keep this")];
        let result = truncate_history(msgs, "system", 200_000, 8192, 90).unwrap();
        assert!(result.len() <= 2);
    }

    #[test]
    fn test_truncate_history_pins_summary() {
        let summary =
            RigMessage::user("<conversation_summary>\nopening request\n</conversation_summary>");
        let big = "x".repeat(4_000);
        let msgs = vec![
            summary.clone(),
            RigMessage::user(&big),
            RigMessage::user(&big),
            RigMessage::user("latest"),
        ];
        // Budget fits the summary plus roughly one big message.
        let result = truncate_history(msgs, "", 1_300, 0, 100);
        assert_eq!(result.first(), Some(&summary));
        assert_eq!(result.last(), Some(&RigMessage::user("latest")));
        assert!(result.len() < 4);
    }

    #[test]
    fn test_truncate_history_starts_on_user_turn() {
        let big = "x".repeat(4_000);
        let msgs = vec![
            RigMessage::user(&big),
            RigMessage::assistant(&big),
            RigMessage::user("next question"),
            RigMessage::assistant("answer"),
        ];
        let result = truncate_history(msgs, "", 1_100, 0, 100);
        assert!(matches!(result.first(), Some(RigMessage::User { .. })));
    }
}
