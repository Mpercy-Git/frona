use crate::agent::signal::matcher::{Matcher, MatcherKind};
use crate::agent::signal::models::{CandidateEvent, Watch};

pub struct ContactMatcher;

impl Matcher for ContactMatcher {
    fn name(&self) -> &str {
        "contact"
    }

    fn kind(&self) -> MatcherKind {
        MatcherKind::HardFilter
    }

    fn is_active(&self, watch: &Watch) -> bool {
        !watch.expected_contacts.is_empty()
    }

    fn evaluate(&self, candidate: &CandidateEvent, watch: &Watch) -> Option<u32> {
        let candidate_contact = candidate.contact_id.as_deref()?;
        if watch
            .expected_contacts
            .iter()
            .any(|c| c == candidate_contact)
        {
            Some(0)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch(contacts: &[&str]) -> Watch {
        Watch {
            task_id: "t".into(),
            user_id: "u".into(),
            agent_id: "a".into(),
            source_chat_id: "c".into(),
            resume_parent: false,
            tags: vec![],
            expected_channels: vec![],
            expected_contacts: contacts.iter().map(|s| s.to_string()).collect(),
            expires_at: None,
            max_evaluations: 50,
            evaluation_count: 0,
        }
    }

    fn candidate(contact: Option<&str>) -> CandidateEvent {
        CandidateEvent {
            user_id: "u".into(),
            space_id: None,
            chat_id: None,
            message_id: None,
            connector_id: None,
            channel_id: None,
            contact_id: contact.map(|s| s.to_string()),
            sender: None,
            tags: vec![],
            summary: None,
            content: String::new(),
        }
    }

    #[test]
    fn empty_filter_means_abstain() {
        assert!(!ContactMatcher.is_active(&watch(&[])));
    }

    #[test]
    fn matching_contact_passes() {
        assert_eq!(
            ContactMatcher.evaluate(
                &candidate(Some("contact-sarah")),
                &watch(&["contact-sarah", "contact-bob"]),
            ),
            Some(0),
        );
    }

    #[test]
    fn non_matching_contact_rejects() {
        assert_eq!(
            ContactMatcher.evaluate(&candidate(Some("contact-x")), &watch(&["contact-sarah"])),
            None,
        );
    }

    #[test]
    fn missing_candidate_contact_rejects_when_filter_set() {
        assert_eq!(
            ContactMatcher.evaluate(&candidate(None), &watch(&["contact-sarah"])),
            None,
        );
    }
}
