use std::collections::HashSet;

use crate::agent::signal::matcher::{Matcher, MatcherKind};
use crate::agent::signal::models::{CandidateEvent, Watch};

pub struct TagMatcher;

impl Matcher for TagMatcher {
    fn name(&self) -> &str {
        "tag"
    }

    fn kind(&self) -> MatcherKind {
        MatcherKind::Scoring
    }

    fn is_active(&self, watch: &Watch) -> bool {
        !watch.tags.is_empty()
    }

    fn evaluate(&self, candidate: &CandidateEvent, watch: &Watch) -> Option<u32> {
        let watch_set: HashSet<&str> = watch.tags.iter().map(String::as_str).collect();
        let overlap = candidate
            .tags
            .iter()
            .filter(|t| watch_set.contains(t.as_str()))
            .count() as u32;
        if overlap == 0 {
            None
        } else {
            Some(overlap)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch(tags: &[&str]) -> Watch {
        Watch {
            task_id: "t".into(),
            user_id: "u".into(),
            agent_id: "a".into(),
            source_chat_id: "c".into(),
            resume_parent: false,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            expected_channels: vec![],
            expected_contacts: vec![],
            expires_at: None,
            max_evaluations: 50,
            evaluation_count: 0,
        }
    }

    fn candidate(tags: &[&str]) -> CandidateEvent {
        CandidateEvent {
            user_id: "u".into(),
            space_id: None,
            chat_id: None,
            message_id: None,
            connector_id: None,
            channel_id: None,
            contact_id: None,
            sender: None,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            summary: None,
            content: String::new(),
        }
    }

    #[test]
    fn empty_watch_tags_means_abstain() {
        let m = TagMatcher;
        assert!(!m.is_active(&watch(&[])));
    }

    #[test]
    fn no_overlap_returns_none() {
        let m = TagMatcher;
        assert_eq!(m.evaluate(&candidate(&["scheduling"]), &watch(&["verification_code"])), None);
    }

    #[test]
    fn partial_overlap_returns_count() {
        let m = TagMatcher;
        assert_eq!(
            m.evaluate(
                &candidate(&["verification_code", "auth"]),
                &watch(&["verification_code", "scheduling"]),
            ),
            Some(1),
        );
    }

    #[test]
    fn full_overlap_returns_count() {
        let m = TagMatcher;
        assert_eq!(
            m.evaluate(
                &candidate(&["a", "b", "c"]),
                &watch(&["a", "b", "c"]),
            ),
            Some(3),
        );
    }
}
