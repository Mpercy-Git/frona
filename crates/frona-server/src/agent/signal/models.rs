use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::task::models::{SignalMode, Task, TaskKind};

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct SignalOutput {
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub annotator: String,
    pub key: String,
    pub value: AnnotationValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationValue {
    Categorical(String),
    Number(f64),
    Bool(bool),
    Text(String),
}

impl Annotation {
    pub fn category(annotator: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            annotator: annotator.into(),
            key: "category".into(),
            value: AnnotationValue::Categorical(value.into()),
        }
    }

    pub fn summary(annotator: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            annotator: annotator.into(),
            key: "summary".into(),
            value: AnnotationValue::Text(value.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Watch {
    pub task_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub source_chat_id: String,
    pub resume_parent: bool,
    pub mode: SignalMode,
    pub expected_categories: Vec<String>,
    pub expected_channels: Vec<String>,
    pub expected_contacts: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub max_evaluations: u32,
    pub evaluation_count: u32,
}

impl Watch {
    pub fn from_task(task: &Task) -> Option<Self> {
        match &task.kind {
            TaskKind::Signal {
                source_chat_id,
                resume_parent,
                mode,
                expected_categories,
                expected_channels,
                expected_contacts,
                expires_at,
                max_evaluations,
                evaluation_count,
            } => Some(Self {
                task_id: task.id.clone(),
                user_id: task.user_id.clone(),
                agent_id: task.agent_id.clone(),
                source_chat_id: source_chat_id.clone(),
                resume_parent: *resume_parent,
                mode: *mode,
                expected_categories: expected_categories.clone(),
                expected_channels: expected_channels.clone(),
                expected_contacts: expected_contacts.clone(),
                expires_at: *expires_at,
                max_evaluations: *max_evaluations,
                evaluation_count: *evaluation_count,
            }),
            _ => None,
        }
    }

    pub fn has_criteria(&self) -> bool {
        !self.expected_categories.is_empty()
            || !self.expected_channels.is_empty()
            || !self.expected_contacts.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct CandidateEvent {
    pub channel: Option<crate::chat::channel::Channel>,
    pub chat: Option<crate::chat::models::Chat>,
    pub message: Option<crate::chat::message::models::Message>,
    pub contact: Option<crate::contact::models::ContactResponse>,
    pub sender: Option<String>,
    pub annotations: Vec<Annotation>,
    pub content: String,
}

impl CandidateEvent {
    pub fn categories(&self) -> impl Iterator<Item = &str> {
        self.annotations.iter().filter_map(|a| match (a.key.as_str(), &a.value) {
            ("category", AnnotationValue::Categorical(s)) => Some(s.as_str()),
            _ => None,
        })
    }

    pub fn summary(&self) -> Option<&str> {
        self.annotations.iter().find_map(|a| match (a.key.as_str(), &a.value) {
            ("summary", AnnotationValue::Text(s)) => Some(s.as_str()),
            _ => None,
        })
    }
}

#[cfg(test)]
pub mod test_fixtures {
    use super::CandidateEvent;
    use chrono::Utc;

    pub fn channel(provider: &str) -> crate::chat::channel::Channel {
        let now = Utc::now();
        crate::chat::channel::Channel {
            id: "ch".into(),
            user_id: "u".into(),
            handle: crate::core::Handle::try_new(provider).unwrap_or(crate::handle!("test-ch")),
            space_id: "s".into(),
            provider: provider.into(),
            agent_id: "a".into(),
            config: Default::default(),
            dispatch_mode: Default::default(),
            status: crate::chat::channel::models::ChannelStatus::Disconnected,
            error_message: None,
            last_started_at: None,
            user_address: None,
            setup: None,
            retry: None,
            created_at: now,
            updated_at: now,
            webhook_url: None,
        }
    }

    pub fn contact(id: &str) -> crate::contact::models::ContactResponse {
        let now = Utc::now();
        crate::contact::models::Contact {
            id: id.into(),
            user_id: "u".into(),
            name: id.into(),
            space_id: None,
            phone: None,
            email: None,
            company: None,
            job_title: None,
            notes: None,
            avatar: None,
            addresses: Vec::new(),
            metadata: Default::default(),
            created_at: now,
            updated_at: now,
        }
        .into()
    }

    pub fn candidate() -> CandidateEvent {
        CandidateEvent {
            channel: None,
            chat: None,
            message: None,
            contact: None,
            sender: None,
            annotations: Vec::new(),
            content: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::task::models::TaskStatus;

    fn signal_task() -> Task {
        Task {
            id: "task-1".into(),
            user_id: "user-1".into(),
            agent_id: "agent-1".into(),
            space_id: None,
            chat_id: None,
            title: "test".into(),
            description: "Wait for: code".into(),
            status: TaskStatus::Pending,
            kind: TaskKind::Signal {
                source_chat_id: "chat-A".into(),
                resume_parent: true,
                mode: SignalMode::Once,
                expected_categories: vec!["verification_code".into()],
                expected_channels: vec!["sms".into()],
                expected_contacts: vec![],
                expires_at: None,
                max_evaluations: 50,
                evaluation_count: 0,
            },
            run_at: None,
            result_summary: None,
            error_message: None,
            quarantined: false,
            result_schema: None,
            result_description: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn from_task_signal_builds_watch() {
        let task = signal_task();
        let watch = Watch::from_task(&task).expect("should build");
        assert_eq!(watch.task_id, "task-1");
        assert_eq!(watch.agent_id, "agent-1");
        assert_eq!(watch.source_chat_id, "chat-A");
        assert!(watch.resume_parent);
        assert_eq!(watch.expected_categories, vec!["verification_code".to_string()]);
        assert_eq!(watch.expected_channels, vec!["sms".to_string()]);
        assert_eq!(watch.max_evaluations, 50);
    }

    #[test]
    fn from_task_non_signal_returns_none() {
        let mut task = signal_task();
        task.kind = TaskKind::Direct {
            source_chat_id: Some("c".into()),
        };
        assert!(Watch::from_task(&task).is_none());
    }

    #[test]
    fn has_criteria_requires_at_least_one() {
        let task = signal_task();
        let mut watch = Watch::from_task(&task).unwrap();
        assert!(watch.has_criteria());

        watch.expected_categories.clear();
        watch.expected_channels.clear();
        watch.expected_contacts.clear();
        assert!(!watch.has_criteria());
    }

    #[test]
    fn signal_output_round_trips_through_json() {
        let out = SignalOutput {
            categories: vec!["verification_code".into(), "auth".into()],
            summary: Some("code arrived".into()),
        };
        let json = serde_json::to_value(&out).unwrap();
        let back: SignalOutput = serde_json::from_value(json).unwrap();
        assert_eq!(back.categories, out.categories);
        assert_eq!(back.summary, out.summary);
    }

    #[test]
    fn signal_output_summary_omitted_when_none() {
        let out = SignalOutput {
            categories: vec!["x".into()],
            summary: None,
        };
        let json = serde_json::to_value(&out).unwrap();
        assert!(json.get("summary").is_none(), "summary should be omitted, got {json}");
    }

    #[test]
    fn signal_output_schema_declares_categories_and_summary() {
        let schema = serde_json::to_value(schemars::schema_for!(SignalOutput)).unwrap();
        let props = schema
            .pointer("/properties")
            .expect("schema has properties");
        assert!(props.get("categories").is_some(), "schema missing categories: {schema}");
        assert!(props.get("summary").is_some(), "schema missing summary: {schema}");
    }

    #[test]
    fn candidate_event_helpers_extract_typed_values() {
        let cand = CandidateEvent {
            annotations: vec![
                Annotation::category("agent:a", "verification_code"),
                Annotation::category("agent:a", "auth"),
                Annotation::summary("agent:a", "code arrived"),
            ],
            ..super::test_fixtures::candidate()
        };
        let cats: Vec<&str> = cand.categories().collect();
        assert_eq!(cats, vec!["verification_code", "auth"]);
        assert_eq!(cand.summary(), Some("code arrived"));
    }
}
