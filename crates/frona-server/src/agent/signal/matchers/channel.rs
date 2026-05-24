use crate::agent::signal::matcher::{Matcher, MatcherKind};
use crate::agent::signal::models::{CandidateEvent, Watch};

pub struct ChannelMatcher;

impl Matcher for ChannelMatcher {
    fn name(&self) -> &str {
        "channel"
    }

    fn kind(&self) -> MatcherKind {
        MatcherKind::HardFilter
    }

    fn is_active(&self, watch: &Watch) -> bool {
        !watch.expected_channels.is_empty()
    }

    fn evaluate(&self, candidate: &CandidateEvent, watch: &Watch) -> Option<u32> {
        let candidate_channel = candidate.channel.as_ref().map(|c| c.provider.as_str())?;
        if watch
            .expected_channels
            .iter()
            .any(|c| c == candidate_channel)
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

    fn watch(channels: &[&str]) -> Watch {
        Watch {
            task_id: "t".into(),
            user_id: "u".into(),
            agent_id: "a".into(),
            source_chat_id: "c".into(),
            resume_parent: false,
            mode: crate::agent::task::models::SignalMode::Once,
            expected_categories: vec![],
            expected_channels: channels.iter().map(|s| s.to_string()).collect(),
            expected_contacts: vec![],
            expires_at: None,
            max_evaluations: 50,
            evaluation_count: 0,
        }
    }

    fn candidate(channel: Option<&str>) -> CandidateEvent {
        CandidateEvent {
            channel: channel.map(crate::agent::signal::models::test_fixtures::channel),
            ..crate::agent::signal::models::test_fixtures::candidate()
        }
    }

    #[test]
    fn empty_filter_means_abstain() {
        assert!(!ChannelMatcher.is_active(&watch(&[])));
    }

    #[test]
    fn matching_channel_passes() {
        assert_eq!(
            ChannelMatcher.evaluate(&candidate(Some("sms")), &watch(&["sms", "telegram"])),
            Some(0),
        );
    }

    #[test]
    fn non_matching_channel_rejects() {
        assert_eq!(
            ChannelMatcher.evaluate(&candidate(Some("email")), &watch(&["sms"])),
            None,
        );
    }

    #[test]
    fn missing_candidate_channel_rejects_when_filter_set() {
        assert_eq!(
            ChannelMatcher.evaluate(&candidate(None), &watch(&["sms"])),
            None,
        );
    }
}
