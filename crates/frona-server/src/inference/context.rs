use rig_core::completion::Message as RigMessage;

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

/// Trims `history` to fit `history_truncation_pct` of the budget left over
/// after `max_output_tokens` and the system prompt. Newest messages are kept;
/// older ones are dropped first. Operates on a resolved window budget
/// (typically from `resolve_context_window`) - doesn't know about model identity.
pub fn truncate_history(
    history: Vec<RigMessage>,
    system_prompt: &str,
    context_window: usize,
    max_output_tokens: usize,
    history_truncation_pct: usize,
) -> Vec<RigMessage> {
    let window = context_window;
    let system_tokens = estimate_tokens(system_prompt);
    let budget = window
        .saturating_sub(max_output_tokens)
        .saturating_sub(system_tokens);
    let budget = budget * history_truncation_pct / 100;

    let total: usize = history.iter().map(estimate_message_tokens).sum();
    if total <= budget {
        return history;
    }

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

    for msg in history.into_iter().skip(tail_start).rev() {
        let cost = estimate_message_tokens(&msg);
        if used + cost > tail_budget {
            break;
        }
        used += cost;
        result.push(msg);
    }

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
        let result = truncate_history(msgs.clone(), "system", 200_000, 8192, 90);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_truncate_history_exceeds_budget() {
        let long = "x".repeat(500_000);
        let msgs = vec![RigMessage::user(&long), RigMessage::user("keep this")];
        let result = truncate_history(msgs, "system", 200_000, 8192, 90);
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
