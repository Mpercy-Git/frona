use std::collections::HashMap;

use crate::core::error::AppError;
use crate::memory::pkm::model::{EvidenceSource, MemoryEvidence};

/// Bidirectional identifiers scoped to one model conversation.
///
/// Durable ids never cross the prompt boundary. Accepted aliases are expanded before
/// validation, checkpointing or persistence; a resumed conversation simply builds a
/// fresh deterministic map from its durable inputs.
#[derive(Debug, Clone)]
pub(crate) struct PromptIds {
    prefix: &'static str,
    real_to_local: HashMap<String, String>,
    local_to_real: HashMap<String, String>,
}

impl PromptIds {
    pub(crate) fn new(
        prefix: &'static str,
        reals: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut out = Self {
            prefix,
            real_to_local: HashMap::new(),
            local_to_real: HashMap::new(),
        };
        for real in reals {
            if out.real_to_local.contains_key(&real) {
                continue;
            }
            let local = format!("{prefix}{}", out.real_to_local.len() + 1);
            out.real_to_local.insert(real.clone(), local.clone());
            out.local_to_real.insert(local, real);
        }
        out
    }

    pub(crate) fn local<'a>(&'a self, real: &'a str) -> &'a str {
        self.real_to_local.get(real).map(String::as_str).unwrap_or(real)
    }

    pub(crate) fn expand(&self, local: &str) -> Result<String, AppError> {
        self.local_to_real.get(local.trim()).cloned().ok_or_else(|| {
            AppError::Internal(format!("unknown model-local {} id `{}`", self.prefix, local.trim()))
        })
    }

    pub(crate) fn expand_all(&self, values: &mut [String]) -> Result<(), AppError> {
        for value in values.iter_mut() {
            *value = self.expand(value)?;
        }
        Ok(())
    }
}

/// Evidence projection for prompts: retain epistemic value, never infrastructure ids.
pub(crate) fn prompt_evidence(evidence: &[MemoryEvidence]) -> String {
    evidence.iter().map(|item| {
        let (source, quote) = match &item.source {
            EvidenceSource::UserMessage { quote, .. } => ("UserMessage", quote.as_str()),
            EvidenceSource::UserConfirmation { quote, .. } => ("UserConfirmation", quote.as_str()),
            EvidenceSource::AgentMessage { quote, .. } => ("AgentMessage", quote.as_str()),
            EvidenceSource::WebSearch { quote, .. } => ("WebSearch", quote.as_str()),
            EvidenceSource::WebPage { quote, .. } => ("WebPage", quote.as_str()),
            EvidenceSource::ToolResult { quote, .. } => ("ToolResult", quote.as_str()),
            EvidenceSource::TaskLifecycle { .. } => ("TaskLifecycle", ""),
            EvidenceSource::HumanEdit { quote, .. } => ("HumanEdit", quote.as_str()),
            EvidenceSource::ExternalNote { quote, .. } => ("ExternalNote", quote.as_str()),
        };
        if quote.trim().is_empty() {
            format!("{source}/{:?}", item.strength)
        } else {
            format!("{source}/{:?} quote={:?}", item.strength, quote)
        }
    }).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_short_stable_and_expand_only_known_values() {
        let ids = PromptIds::new("m", ["uuid-a".into(), "uuid-b".into(), "uuid-a".into()]);
        assert_eq!(ids.local("uuid-a"), "m1");
        assert_eq!(ids.local("uuid-b"), "m2");
        assert_eq!(ids.expand("m2").unwrap(), "uuid-b");
        assert!(ids.expand("m3").is_err());
    }

    #[test]
    fn prompt_evidence_keeps_quote_but_hides_infrastructure_ids() {
        let evidence = vec![MemoryEvidence {
            strength: crate::memory::pkm::model::EvidenceStrength::Explicit,
            source: EvidenceSource::AgentMessage {
                message_id: "019-message-uuid".into(),
                agent_id: "019-agent-uuid".into(),
                chat_id: "019-chat-uuid".into(),
                quote: "Casey Owner works at Example Corp".into(),
            },
        }];
        let rendered = prompt_evidence(&evidence);
        assert!(rendered.contains("Casey Owner works at Example Corp"));
        assert!(!rendered.contains("019-"));
    }
}
