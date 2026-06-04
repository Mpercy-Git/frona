pub use crate::inference::hitl::{
    Hitl, HitlDelivery, HitlOutcome, HitlRequest, HitlResponse, ResolveOutcome, VaultGrant,
};

/// Closed render taxonomy. Channel adapters branch on this, never on
/// `HitlRequest` — new request variants only need a [`kind_for`] mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitlKind {
    Approval,
    Choice { options: Vec<String> },
    /// No in-channel affordance — adapter posts the URL and the user resolves
    /// on the web (tools needing server-side pickers like vault selection).
    External,
}

pub fn kind_for(req: &HitlRequest) -> HitlKind {
    match req {
        HitlRequest::Question { options } => HitlKind::Choice {
            options: options.clone(),
        },
        HitlRequest::Takeover { .. } => HitlKind::Choice {
            options: vec!["Done".to_string()],
        },
        HitlRequest::App { .. } => HitlKind::Approval,
        HitlRequest::Credential { .. } => HitlKind::External,
    }
}

pub fn render_default_text(hitl: &Hitl) -> String {
    format!("{}\n\n{}", hitl.prompt, hitl.url)
}

/// Returns:
/// - `None` — no pending HITLs; adapter proceeds with normal inbound handling.
/// - `Some(Resolved)` — oldest Choice-kind HITL was resolved with `text`.
/// - `Some(AlreadyResolved)` — only Approval/External pending; adapter should
///   hint the user to use the URL rather than treat this as a user message.
pub async fn try_resolve_from_text(
    state: &crate::core::state::AppState,
    chat_id: &str,
    text: &str,
) -> Result<Option<ResolveOutcome>, crate::core::error::AppError> {
    let paused_msg = state
        .chat_service
        .find_paused_message_for_chat(chat_id)
        .await?;
    let Some(msg) = paused_msg else { return Ok(None) };

    let tool_calls = state.chat_service.get_tool_calls_by_message(&msg.id).await?;

    let pending: Vec<_> = tool_calls
        .iter()
        .filter(|tc| {
            tc.hitl
                .as_ref()
                .is_some_and(|h| h.status == crate::inference::tool_call::ToolStatus::Pending)
        })
        .collect();

    if pending.is_empty() {
        return Ok(None);
    }

    let chosen = pending.iter().find(|tc| {
        tc.hitl.as_ref().is_some_and(|h| {
            matches!(kind_for(&h.request), HitlKind::Choice { .. })
        })
    });

    if let Some(tc) = chosen {
        let outcome = state
            .channel_manager
            .resolve_hitl(&tc.id, HitlResponse::Choice(text.to_string()))
            .await?;
        return Ok(Some(outcome));
    }

    Ok(Some(ResolveOutcome::AlreadyResolved))
}

pub async fn skip_all_pending_for_chat(
    state: &crate::core::state::AppState,
    chat_id: &str,
) -> Result<usize, crate::core::error::AppError> {
    let paused_msg = state
        .chat_service
        .find_paused_message_for_chat(chat_id)
        .await?;
    let Some(msg) = paused_msg else { return Ok(0) };

    let tool_calls = state.chat_service.get_tool_calls_by_message(&msg.id).await?;
    let mut count = 0;
    for tc in tool_calls {
        let Some(h) = tc.hitl.as_ref() else { continue };
        if h.status != crate::inference::tool_call::ToolStatus::Pending {
            continue;
        }
        let denial = match kind_for(&h.request) {
            HitlKind::Approval => HitlResponse::Approval(false),
            HitlKind::Choice { .. } => HitlResponse::Choice("[skipped]".into()),
            HitlKind::External => continue,
        };
        if let Ok(crate::inference::hitl::ResolveOutcome::Resolved { .. }) = state
            .channel_manager
            .resolve_hitl(&tc.id, denial)
            .await
        {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_for_question_returns_choice_with_options() {
        let req = HitlRequest::Question {
            options: vec!["a".into(), "b".into()],
        };
        match kind_for(&req) {
            HitlKind::Choice { options } => {
                assert_eq!(options, vec!["a".to_string(), "b".to_string()]);
            }
            _ => panic!("expected Choice"),
        }
    }

    #[test]
    fn kind_for_takeover_returns_choice_with_done() {
        let req = HitlRequest::Takeover {
            reason: "manual debug".into(),
            debugger_url: "https://x".into(),
        };
        match kind_for(&req) {
            HitlKind::Choice { options } => {
                assert_eq!(options, vec!["Done".to_string()]);
            }
            _ => panic!("expected Choice"),
        }
    }

    #[test]
    fn kind_for_service_approval_returns_approval() {
        let req = HitlRequest::App {
            action: "deploy".into(),
            manifest: serde_json::json!({}),
            previous_manifest: None,
        };
        assert_eq!(kind_for(&req), HitlKind::Approval);
    }

    #[test]
    fn kind_for_vault_pick_returns_external() {
        let req = HitlRequest::Credential {
            query: "postgres".into(),
            reason: "ETL".into(),
        };
        assert_eq!(kind_for(&req), HitlKind::External);
    }

    #[test]
    fn kind_for_question_with_empty_options_still_returns_choice() {
        let req = HitlRequest::Question { options: vec![] };
        match kind_for(&req) {
            HitlKind::Choice { options } => assert!(options.is_empty()),
            _ => panic!("expected Choice"),
        }
    }

    #[test]
    fn render_default_text_contains_prompt_and_url() {
        let h = Hitl {
            prompt: "Deploy notes?".into(),
            url: "https://app.example/chats/abc".into(),
            request: HitlRequest::App {
                action: "deploy".into(),
                manifest: serde_json::json!({}),
                previous_manifest: None,
            },
            status: crate::inference::tool_call::ToolStatus::Pending,
            response: None,
            delivery: None,
        };
        let text = render_default_text(&h);
        assert!(text.contains("Deploy notes?"));
        assert!(text.contains("https://app.example/chats/abc"));
    }
}
