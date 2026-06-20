use chrono::{DateTime, Utc};
use crate::Entity;
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "lowercase")]
#[surreal(crate = "surrealdb::types", lowercase)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", lowercase)]
pub enum SignalMode {
    #[default]
    Once,
    Continuous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", lowercase)]
pub enum CronMode {
    #[default]
    Singleton,
    PerInstance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", lowercase)]
pub enum CronConcurrency {
    Allow,
    Forbid,
    #[default]
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(tag = "type")]
#[surreal(crate = "surrealdb::types", tag = "type")]
pub enum TaskKind {
    Direct {
        #[serde(default)]
        source_chat_id: Option<String>,
    },
    Delegation {
        source_agent_id: String,
        source_chat_id: String,
        #[serde(default)]
        resume_parent: bool,
    },
    Cron {
        cron_expression: String,
        #[serde(default)]
        timezone: Option<String>,
        next_run_at: Option<DateTime<Utc>>,
        source_agent_id: Option<String>,
        source_chat_id: Option<String>,
        #[serde(default)]
        #[surreal(default)]
        mode: CronMode,
        #[serde(default)]
        #[surreal(default)]
        concurrency: CronConcurrency,
        #[serde(default)]
        #[surreal(default)]
        process_result: bool,
    },
    CronRun {
        source_cron_id: String,
        #[serde(default)]
        source_chat_id: Option<String>,
        #[serde(default)]
        source_agent_id: Option<String>,
        fire_at: DateTime<Utc>,
        sequence_num: u64,
    },
    Signal {
        source_chat_id: String,
        #[serde(default)]
        resume_parent: bool,
        #[serde(default)]
        mode: SignalMode,
        #[serde(default, alias = "tags")]
        expected_categories: Vec<String>,
        #[serde(default)]
        expected_channels: Vec<String>,
        #[serde(default)]
        expected_contacts: Vec<String>,
        #[serde(default)]
        expires_at: Option<DateTime<Utc>>,
        #[serde(default)]
        max_evaluations: u32,
        #[serde(default)]
        evaluation_count: u32,
    },
}

impl Default for TaskKind {
    fn default() -> Self {
        TaskKind::Direct { source_chat_id: None }
    }
}

impl TaskKind {
    pub fn source_chat_id(&self) -> Option<&str> {
        match self {
            TaskKind::Direct { source_chat_id } => source_chat_id.as_deref(),
            TaskKind::Delegation { source_chat_id, .. } => Some(source_chat_id),
            TaskKind::Cron { source_chat_id, .. } => source_chat_id.as_deref(),
            TaskKind::CronRun { source_chat_id, .. } => source_chat_id.as_deref(),
            TaskKind::Signal { source_chat_id, .. } => Some(source_chat_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "task")]
pub struct Task {
    pub id: String,
    pub user_id: String,
    pub agent_id: String,
    pub space_id: Option<String>,
    pub chat_id: Option<String>,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(default)]
    pub run_at: Option<DateTime<Utc>>,
    pub result_summary: Option<String>,
    pub error_message: Option<String>,
    #[serde(default)]
    #[surreal(default)]
    pub quarantined: bool,
    #[serde(default)]
    pub result_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub result_description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    /// Without a typed `complete_task.result` channel the executor falls
    /// back to `send_message`, so a bare `result_description` must still
    /// synthesize a string schema at read time.
    pub fn effective_result_schema(&self) -> Option<serde_json::Value> {
        if let Some(schema) = &self.result_schema {
            return Some(schema.clone());
        }
        let description = self.result_description.as_ref()?;
        Some(serde_json::json!({
            "type": "string",
            "description": description,
        }))
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub agent_id: String,
    pub space_id: Option<String>,
    pub chat_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub source_agent_id: Option<String>,
    pub source_chat_id: Option<String>,
    pub resume_parent: Option<bool>,
    pub run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub quarantined: bool,
    #[serde(default)]
    pub result_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub result_description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTaskRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
}

#[derive(Debug, Serialize)]
pub struct TaskResponse {
    pub id: String,
    pub agent_id: String,
    pub space_id: Option<String>,
    pub chat_id: Option<String>,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub kind: TaskKind,
    pub run_at: Option<DateTime<Utc>>,
    pub result_summary: Option<String>,
    pub error_message: Option<String>,
    pub quarantined: bool,
    pub result_schema: Option<serde_json::Value>,
    pub result_description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Task> for TaskResponse {
    fn from(task: Task) -> Self {
        Self {
            id: task.id,
            agent_id: task.agent_id,
            space_id: task.space_id,
            chat_id: task.chat_id,
            title: task.title,
            description: task.description,
            status: task.status,
            kind: task.kind,
            run_at: task.run_at,
            result_summary: task.result_summary,
            error_message: task.error_message,
            quarantined: task.quarantined,
            result_schema: task.result_schema,
            result_description: task.result_description,
            created_at: task.created_at,
            updated_at: task.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare_task() -> Task {
        Task {
            id: "t".into(),
            user_id: "u".into(),
            agent_id: "a".into(),
            space_id: None,
            chat_id: None,
            title: "t".into(),
            description: "d".into(),
            status: TaskStatus::Pending,
            kind: TaskKind::Direct { source_chat_id: None },
            run_at: None,
            result_summary: None,
            error_message: None,
            quarantined: false,
            result_schema: None,
            result_description: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn effective_result_schema_none_when_both_unset() {
        assert!(bare_task().effective_result_schema().is_none());
    }

    #[test]
    fn effective_result_schema_passes_through_explicit_schema() {
        let mut t = bare_task();
        let schema = serde_json::json!({"type": "number"});
        t.result_schema = Some(schema.clone());
        assert_eq!(t.effective_result_schema(), Some(schema));
    }

    #[test]
    fn effective_result_schema_synthesizes_string_from_description() {
        let mut t = bare_task();
        t.result_description = Some("a friendly reminder".into());
        assert_eq!(
            t.effective_result_schema(),
            Some(serde_json::json!({
                "type": "string",
                "description": "a friendly reminder",
            }))
        );
    }

    #[test]
    fn effective_result_schema_prefers_schema_when_both_set() {
        let mut t = bare_task();
        let schema = serde_json::json!({"type": "boolean"});
        t.result_schema = Some(schema.clone());
        t.result_description = Some("should be ignored".into());
        assert_eq!(t.effective_result_schema(), Some(schema));
    }

    #[test]
    fn source_chat_id_direct() {
        assert_eq!(TaskKind::Direct { source_chat_id: None }.source_chat_id(), None);
    }

    #[test]
    fn source_chat_id_direct_with_source() {
        let kind = TaskKind::Direct { source_chat_id: Some("c0".to_string()) };
        assert_eq!(kind.source_chat_id(), Some("c0"));
    }

    #[test]
    fn source_chat_id_delegation() {
        let kind = TaskKind::Delegation {
            source_agent_id: "a1".to_string(),
            source_chat_id: "c1".to_string(),
            resume_parent: false,
        };
        assert_eq!(kind.source_chat_id(), Some("c1"));
    }

    #[test]
    fn source_chat_id_cron_with_value() {
        let kind = TaskKind::Cron {
            cron_expression: "0 9 * * *".to_string(),
            timezone: None,
            next_run_at: None,
            source_agent_id: None,
            source_chat_id: Some("c2".to_string()),
            mode: CronMode::Singleton,
            concurrency: CronConcurrency::Replace,
            process_result: false,
        };
        assert_eq!(kind.source_chat_id(), Some("c2"));
    }

    #[test]
    fn source_chat_id_cron_without_value() {
        let kind = TaskKind::Cron {
            cron_expression: "0 9 * * *".to_string(),
            timezone: None,
            next_run_at: None,
            source_agent_id: None,
            source_chat_id: None,
            mode: CronMode::Singleton,
            concurrency: CronConcurrency::Replace,
            process_result: false,
        };
        assert_eq!(kind.source_chat_id(), None);
    }

    #[test]
    fn source_chat_id_signal() {
        let kind = TaskKind::Signal {
            source_chat_id: "c3".to_string(),
            resume_parent: true,
            mode: SignalMode::Once,
            expected_categories: vec!["verification_code".to_string()],
            expected_channels: vec![],
            expected_contacts: vec![],
            expires_at: None,
            max_evaluations: 50,
            evaluation_count: 0,
        };
        assert_eq!(kind.source_chat_id(), Some("c3"));
    }

    #[test]
    fn signal_mode_default_is_once() {
        assert_eq!(SignalMode::default(), SignalMode::Once);
    }

    #[test]
    fn signal_mode_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(SignalMode::Continuous).unwrap(),
            serde_json::json!("continuous")
        );
        assert_eq!(
            serde_json::to_value(SignalMode::Once).unwrap(),
            serde_json::json!("once")
        );
    }

    #[test]
    fn signal_kind_deserializes_without_mode_as_once() {
        let json = serde_json::json!({
            "type": "Signal",
            "source_chat_id": "c1",
            "resume_parent": false,
            "tags": ["x"],
            "expected_channels": [],
            "expected_contacts": [],
            "expires_at": null,
            "max_evaluations": 50,
            "evaluation_count": 0,
        });
        let kind: TaskKind = serde_json::from_value(json).unwrap();
        match kind {
            TaskKind::Signal { mode, .. } => assert_eq!(mode, SignalMode::Once),
            _ => panic!("expected Signal"),
        }
    }
}
