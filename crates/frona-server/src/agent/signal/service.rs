use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use tokio::sync::RwLock;

use crate::agent::prompt::PromptLoader;
use crate::agent::service::AgentService;
use crate::agent::task::executor::TaskExecutor;
use crate::agent::task::models::{Task, TaskKind, TaskStatus};
use crate::agent::task::service::TaskService;
use crate::contact::service::ContactService;
use crate::core::error::AppError;
use crate::policy::models::{PolicyAction, SignalContact};
use crate::policy::service::PolicyService;

use super::matcher::{Matcher, MatcherKind};
use super::matchers::{ChannelMatcher, ContactMatcher, TagMatcher};
use super::models::{CandidateEvent, Watch};

type WatchIndex = HashMap<String, HashMap<String, Watch>>;

pub struct SignalService {
    watches: RwLock<WatchIndex>,
    matchers: Vec<Arc<dyn Matcher>>,
    task_service: TaskService,
    task_executor: Arc<OnceLock<Arc<TaskExecutor>>>,
    agent_service: AgentService,
    contact_service: ContactService,
    policy_service: PolicyService,
    prompts: PromptLoader,
}

impl SignalService {
    pub fn new(
        task_service: TaskService,
        task_executor: Arc<OnceLock<Arc<TaskExecutor>>>,
        agent_service: AgentService,
        contact_service: ContactService,
        policy_service: PolicyService,
        prompts: PromptLoader,
    ) -> Self {
        let matchers: Vec<Arc<dyn Matcher>> = vec![
            Arc::new(TagMatcher),
            Arc::new(ChannelMatcher),
            Arc::new(ContactMatcher),
        ];
        Self::with_matchers(
            task_service,
            task_executor,
            agent_service,
            contact_service,
            policy_service,
            prompts,
            matchers,
        )
    }

    pub fn with_matchers(
        task_service: TaskService,
        task_executor: Arc<OnceLock<Arc<TaskExecutor>>>,
        agent_service: AgentService,
        contact_service: ContactService,
        policy_service: PolicyService,
        prompts: PromptLoader,
        matchers: Vec<Arc<dyn Matcher>>,
    ) -> Self {
        Self {
            watches: RwLock::new(HashMap::new()),
            matchers,
            task_service,
            task_executor,
            agent_service,
            contact_service,
            policy_service,
            prompts,
        }
    }

    pub async fn start(self: &Arc<Self>) -> Result<(), AppError> {
        self.rebuild_from_db().await?;
        Ok(())
    }

    async fn rebuild_from_db(&self) -> Result<(), AppError> {
        let tasks = self.task_service.list_pending_signal_tasks().await?;
        let mut index = self.watches.write().await;
        index.clear();
        for task in tasks {
            if let Some(watch) = Watch::from_task(&task) {
                index
                    .entry(watch.user_id.clone())
                    .or_default()
                    .insert(watch.task_id.clone(), watch);
            }
        }
        Ok(())
    }

    pub async fn register(&self, watch: Watch) {
        let mut index = self.watches.write().await;
        index
            .entry(watch.user_id.clone())
            .or_default()
            .insert(watch.task_id.clone(), watch);
    }

    pub async fn unregister(&self, user_id: &str, task_id: &str) {
        let mut index = self.watches.write().await;
        if let Some(user_watches) = index.get_mut(user_id) {
            user_watches.remove(task_id);
            if user_watches.is_empty() {
                index.remove(user_id);
            }
        }
    }

    /// Returns task_ids of fired watches.
    pub async fn evaluate(
        &self,
        user_id: &str,
        candidate: CandidateEvent,
    ) -> Result<Vec<String>, AppError> {
        let watches: Vec<Watch> = {
            let index = self.watches.read().await;
            index
                .get(user_id)
                .map(|m| m.values().cloned().collect())
                .unwrap_or_default()
        };

        let mut fired = Vec::new();
        for watch in watches {
            if !self.matches_watch(&candidate, &watch) {
                continue;
            }
            if !self.policy_allows(&candidate, &watch).await? {
                tracing::info!(
                    task_id = %watch.task_id,
                    agent_id = %watch.agent_id,
                    channel_id = ?candidate.channel_id,
                    sender = ?candidate.sender,
                    "Signal match denied by policy"
                );
                continue;
            }
            match self.fire_signal(&watch, &candidate).await {
                Ok(true) => fired.push(watch.task_id.clone()),
                Ok(false) => {}
                Err(e) => tracing::warn!(
                    task_id = %watch.task_id,
                    error = %e,
                    "Failed to fire signal",
                ),
            }
        }
        Ok(fired)
    }

    fn matches_watch(&self, candidate: &CandidateEvent, watch: &Watch) -> bool {
        evaluate_match(&self.matchers, candidate, watch)
    }

    async fn policy_allows(
        &self,
        candidate: &CandidateEvent,
        watch: &Watch,
    ) -> Result<bool, AppError> {
        let agent = self
            .agent_service
            .find_by_id(&watch.agent_id)
            .await?;
        let Some(agent) = agent else {
            return Ok(false);
        };

        let contact = if let Some(ref contact_id) = candidate.contact_id {
            match self
                .contact_service
                .get(&watch.user_id, contact_id)
                .await
            {
                Ok(c) => Some(SignalContact {
                    id: c.id,
                    user_id: watch.user_id.clone(),
                    name: c.name,
                    handles: [c.phone, c.email].into_iter().flatten().collect(),
                }),
                Err(e) => {
                    tracing::debug!(
                        contact_id = %contact_id,
                        error = %e,
                        "Could not resolve contact for receive_signal policy check"
                    );
                    None
                }
            }
        } else {
            None
        };

        let action = PolicyAction::ReceiveSignal {
            connector_id: candidate.connector_id.clone().unwrap_or_default(),
            channel_id: candidate.channel_id.clone().unwrap_or_default(),
            sender: candidate.sender.clone(),
            contact,
        };
        let decision = self
            .policy_service
            .authorize(&watch.user_id, &agent, action)
            .await?;
        Ok(decision.allowed)
    }

    /// Returns Ok(true) when the agent was actually invoked, Ok(false) when
    /// the fire was silently skipped (stale watch, budget exceeded).
    async fn fire_signal(
        &self,
        watch: &Watch,
        candidate: &CandidateEvent,
    ) -> Result<bool, AppError> {
        // Stale-watch check: if the underlying task is no longer pending,
        // unregister and skip. Avoids a broadcast subscription for cancellation
        // tracking (BroadcastService is SSE-only fan-out today).
        let Some(mut task) = self.task_service.find_by_id(&watch.task_id).await? else {
            self.unregister(&watch.user_id, &watch.task_id).await;
            return Ok(false);
        };
        if !matches!(task.status, TaskStatus::Pending | TaskStatus::InProgress) {
            self.unregister(&watch.user_id, &watch.task_id).await;
            return Ok(false);
        }

        let next_count = bump_evaluation_count(&mut task);
        if next_count > watch.max_evaluations.max(1) {
            self.task_service
                .mark_failed(&task.id, "exceeded evaluation budget".into())
                .await?;
            self.unregister(&watch.user_id, &watch.task_id).await;
            return Ok(false);
        }
        self.task_service.save(&task).await?;

        let Some(executor) = self.task_executor.get().cloned() else {
            return Err(AppError::Internal("TaskExecutor not initialized".into()));
        };
        let injected_message = self.build_candidate_block(candidate);
        executor
            .run_with_injected_message(&task, injected_message)
            .await?;
        Ok(true)
    }

    pub async fn watch_count(&self, user_id: &str) -> usize {
        let index = self.watches.read().await;
        index.get(user_id).map(|m| m.len()).unwrap_or(0)
    }
}

/// Aggregator: any active hard-filter `None` rejects; otherwise a watch
/// matches when at least one active matcher returned `Some(_)`, or when
/// only hard filters are active and all passed.
pub fn evaluate_match(
    matchers: &[Arc<dyn Matcher>],
    candidate: &CandidateEvent,
    watch: &Watch,
) -> bool {
    let mut had_scoring_match = false;
    let mut had_active_matcher = false;
    let mut had_active_scoring_matcher = false;

    for matcher in matchers {
        if !matcher.is_active(watch) {
            continue;
        }
        had_active_matcher = true;
        if matcher.kind() == MatcherKind::Scoring {
            had_active_scoring_matcher = true;
        }
        match matcher.evaluate(candidate, watch) {
            None => {
                if matcher.kind() == MatcherKind::HardFilter {
                    return false;
                }
            }
            Some(_score) => {
                had_scoring_match = true;
            }
        }
    }

    if !had_active_matcher {
        return false;
    }
    if !had_active_scoring_matcher {
        return true;
    }
    had_scoring_match
}

fn bump_evaluation_count(task: &mut Task) -> u32 {
    if let TaskKind::Signal {
        ref mut evaluation_count,
        ..
    } = task.kind
    {
        *evaluation_count = evaluation_count.saturating_add(1);
        return *evaluation_count;
    }
    0
}

impl SignalService {
    fn build_candidate_block(&self, c: &CandidateEvent) -> String {
        let channel = c.channel_id.as_deref().unwrap_or("(unknown channel)");
        let sender = c.sender.as_deref().unwrap_or("(unknown sender)");
        let summary = c.summary.as_deref().unwrap_or("(none)");
        self.prompts
            .read_with_vars(
                "signal_candidate.md",
                &[
                    ("channel", channel),
                    ("sender", sender),
                    ("content", &c.content),
                    ("summary", summary),
                ],
            )
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::signal::matchers::{ChannelMatcher, ContactMatcher, TagMatcher};
    use chrono::Utc;

    fn default_matchers() -> Vec<Arc<dyn Matcher>> {
        vec![
            Arc::new(TagMatcher),
            Arc::new(ChannelMatcher),
            Arc::new(ContactMatcher),
        ]
    }

    fn make_watch(
        tags: &[&str],
        channels: &[&str],
        contacts: &[&str],
    ) -> Watch {
        Watch {
            task_id: "t".into(),
            user_id: "u".into(),
            agent_id: "a".into(),
            source_chat_id: "c".into(),
            resume_parent: false,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            expected_channels: channels.iter().map(|s| s.to_string()).collect(),
            expected_contacts: contacts.iter().map(|s| s.to_string()).collect(),
            expires_at: None,
            max_evaluations: 50,
            evaluation_count: 0,
        }
    }

    fn make_candidate(
        tags: &[&str],
        channel: Option<&str>,
        contact: Option<&str>,
    ) -> CandidateEvent {
        CandidateEvent {
            user_id: "u".into(),
            space_id: None,
            chat_id: None,
            message_id: None,
            connector_id: None,
            channel_id: channel.map(|s| s.to_string()),
            contact_id: contact.map(|s| s.to_string()),
            sender: None,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            summary: None,
            content: String::new(),
        }
    }

    #[test]
    fn no_active_matchers_means_no_match() {
        let m = default_matchers();
        let watch = make_watch(&[], &[], &[]);
        let cand = make_candidate(&["verification_code"], Some("sms"), Some("c-1"));
        assert!(!evaluate_match(&m, &cand, &watch));
    }

    #[test]
    fn tag_overlap_alone_fires() {
        let m = default_matchers();
        let watch = make_watch(&["verification_code"], &[], &[]);
        let cand = make_candidate(&["verification_code"], None, None);
        assert!(evaluate_match(&m, &cand, &watch));
    }

    #[test]
    fn hard_filter_rejects_even_when_tag_overlap() {
        let m = default_matchers();
        let watch = make_watch(&["verification_code"], &["sms"], &[]);
        let cand = make_candidate(&["verification_code"], Some("email"), None);
        assert!(!evaluate_match(&m, &cand, &watch));
    }

    #[test]
    fn hard_filter_only_watch_fires_on_filter_pass() {
        let m = default_matchers();
        let watch = make_watch(&[], &["sms"], &[]);
        let cand = make_candidate(&[], Some("sms"), None);
        assert!(evaluate_match(&m, &cand, &watch));
    }

    #[test]
    fn hard_filter_only_watch_rejects_on_filter_fail() {
        let m = default_matchers();
        let watch = make_watch(&[], &["sms"], &[]);
        let cand = make_candidate(&[], Some("email"), None);
        assert!(!evaluate_match(&m, &cand, &watch));
    }

    #[test]
    fn tag_watch_with_no_overlap_does_not_fire() {
        let m = default_matchers();
        let watch = make_watch(&["verification_code"], &[], &[]);
        let cand = make_candidate(&["chitchat"], None, None);
        assert!(!evaluate_match(&m, &cand, &watch));
    }

    #[test]
    fn combined_tag_and_filters_all_must_pass() {
        let m = default_matchers();
        let watch = make_watch(&["verification_code"], &["sms"], &["c-bank"]);
        let cand_ok = make_candidate(&["verification_code"], Some("sms"), Some("c-bank"));
        assert!(evaluate_match(&m, &cand_ok, &watch));

        let cand_wrong_contact = make_candidate(&["verification_code"], Some("sms"), Some("c-other"));
        assert!(!evaluate_match(&m, &cand_wrong_contact, &watch));
    }

    #[test]
    fn bump_evaluation_count_increments_signal_kind() {
        let mut task = Task {
            id: "t".into(),
            user_id: "u".into(),
            agent_id: "a".into(),
            space_id: None,
            chat_id: None,
            title: "x".into(),
            description: "y".into(),
            status: TaskStatus::Pending,
            kind: TaskKind::Signal {
                source_chat_id: "c".into(),
                resume_parent: false,
                tags: vec!["t".into()],
                expected_channels: vec![],
                expected_contacts: vec![],
                expires_at: None,
                max_evaluations: 5,
                evaluation_count: 0,
            },
            run_at: None,
            result_summary: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(bump_evaluation_count(&mut task), 1);
        assert_eq!(bump_evaluation_count(&mut task), 2);
    }
}
