use std::collections::HashSet;

use crate::agent::signal::matcher::{Matcher, MatcherKind};
use crate::agent::signal::models::{CandidateEvent, Watch};

pub struct CategoryMatcher;

impl Matcher for CategoryMatcher {
    fn name(&self) -> &str {
        "category"
    }

    fn kind(&self) -> MatcherKind {
        MatcherKind::Scoring
    }

    fn is_active(&self, watch: &Watch) -> bool {
        !watch.expected_categories.is_empty()
    }

    fn evaluate(&self, candidate: &CandidateEvent, watch: &Watch) -> Option<u32> {
        let watch_set: HashSet<&str> =
            watch.expected_categories.iter().map(String::as_str).collect();
        let overlap = candidate.categories().filter(|c| watch_set.contains(c)).count() as u32;
        if overlap == 0 { None } else { Some(overlap) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::signal::models::Annotation;

    fn watch(cats: &[&str]) -> Watch {
        Watch {
            task_id: "t".into(),
            user_id: "u".into(),
            agent_id: "a".into(),
            source_chat_id: "c".into(),
            resume_parent: false,
            mode: crate::agent::task::models::SignalMode::Once,
            expected_categories: cats.iter().map(|s| s.to_string()).collect(),
            expected_channels: vec![],
            expected_contacts: vec![],
            expires_at: None,
            max_evaluations: 50,
            evaluation_count: 0,
        }
    }

    fn candidate(cats: &[&str]) -> CandidateEvent {
        CandidateEvent {
            annotations: cats
                .iter()
                .map(|c| Annotation::category("agent:test", *c))
                .collect(),
            ..crate::agent::signal::models::test_fixtures::candidate()
        }
    }

    #[test]
    fn empty_watch_categories_means_abstain() {
        let m = CategoryMatcher;
        assert!(!m.is_active(&watch(&[])));
    }

    #[test]
    fn no_overlap_returns_none() {
        let m = CategoryMatcher;
        assert_eq!(
            m.evaluate(&candidate(&["scheduling"]), &watch(&["verification_code"])),
            None,
        );
    }

    #[test]
    fn partial_overlap_returns_count() {
        let m = CategoryMatcher;
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
        let m = CategoryMatcher;
        assert_eq!(
            m.evaluate(&candidate(&["a", "b", "c"]), &watch(&["a", "b", "c"])),
            Some(3),
        );
    }
}
