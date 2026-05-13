use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::core::config::CacheConfig;
use crate::db::repo::agents::SurrealAgentRepo;
use crate::core::error::AppError;
use crate::core::repository::Repository;
use crate::policy::sandbox::SandboxPolicy;
use crate::policy::service::PolicyService;
use crate::tool::sandbox::driver::resource_monitor::SystemResourceManager;

use super::models::{CreateAgentRequest, UpdateAgentRequest};
use super::models::Agent;
use super::repository::AgentRepository;

#[derive(Clone)]
pub struct AgentService {
    repo: SurrealAgentRepo,
    cache: moka::future::Cache<String, Agent>,
    shared_agents_dir: PathBuf,
    resource_manager: Arc<SystemResourceManager>,
    policy_service: PolicyService,
}

impl AgentService {
    pub fn new(
        repo: SurrealAgentRepo,
        cache_config: &CacheConfig,
        shared_agents_dir: PathBuf,
        resource_manager: Arc<SystemResourceManager>,
        policy_service: PolicyService,
    ) -> Self {
        let cache = moka::future::Cache::builder()
            .max_capacity(cache_config.entity_max_capacity)
            .time_to_live(std::time::Duration::from_secs(cache_config.entity_ttl_secs))
            .build();
        Self {
            repo,
            cache,
            shared_agents_dir,
            resource_manager,
            policy_service,
        }
    }

    pub async fn sync_agent_limits(&self) -> Result<(), AppError> {
        let agents = self.repo.find_all().await?;
        for agent in agents {
            if let Some(ref limits) = agent.sandbox_limits {
                self.resource_manager.set_agent_limits(&agent.id, Some(limits.max_cpu_pct), Some(limits.max_memory_pct));
            }
        }
        Ok(())
    }

    fn push_agent_limits(&self, agent_id: &str, agent: &Agent) {
        if let Some(ref limits) = agent.sandbox_limits {
            self.resource_manager.set_agent_limits(agent_id, Some(limits.max_cpu_pct), Some(limits.max_memory_pct));
        }
    }

    pub fn builtin_agent_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.shared_agents_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    && let Some(name) = entry.file_name().to_str()
                {
                    ids.push(name.to_string());
                }
            }
        }
        ids.sort();
        ids
    }

    pub async fn create(
        &self,
        user_id: &str,
        req: CreateAgentRequest,
    ) -> Result<Agent, AppError> {
        let id = if let Some(custom_id) = req.id {
            let custom_id = custom_id.to_lowercase();
            if !custom_id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || custom_id.is_empty() {
                return Err(AppError::Validation("Agent ID must contain only lowercase letters, digits, and hyphens".into()));
            }
            if self.repo.find_by_id(&custom_id).await?.is_some() {
                return Err(AppError::Validation(format!("Agent with ID '{custom_id}' already exists")));
            }
            custom_id
        } else {
            crate::core::repository::new_id()
        };

        let now = chrono::Utc::now();

        let agent = Agent {
            id,
            user_id: Some(user_id.to_string()),
            name: req.name,
            description: req.description,
            model_group: req.model_group.unwrap_or_else(|| "primary".to_string()),
            enabled: true,
            skills: req.skills,
            sandbox_limits: req.sandbox_limits,
            max_concurrent_tasks: None,
            avatar: None,
            identity: std::collections::BTreeMap::new(),
            prompt: None,
            heartbeat_interval: None,
            next_heartbeat_at: None,
            heartbeat_chat_id: None,
            created_at: now,
            updated_at: now,
        };

        let agent = self.repo.create(&agent).await?;
        self.push_agent_limits(&agent.id, &agent);
        // Reconcile with default-empty even when no policy was sent: if this
        // agent id was used before, the planner clears any leftover owned rows.
        let policy = req.sandbox_policy.as_ref().cloned().unwrap_or_default();
        self.policy_service
            .reconcile_sandbox_policy(
                user_id,
                crate::policy::reconcile::EntityRef::Agent(agent.id.clone()),
                &policy,
            )
            .await?;
        Ok(agent)
    }

    pub async fn find_by_id(&self, agent_id: &str) -> Result<Option<Agent>, AppError> {
        if let Some(agent) = self.cache.get(agent_id).await {
            return Ok(Some(agent));
        }
        let result = self.repo.find_by_id(agent_id).await?;
        if let Some(ref agent) = result {
            self.cache.insert(agent_id.to_string(), agent.clone()).await;
        }
        Ok(result)
    }

    pub async fn get(
        &self,
        user_id: &str,
        agent_id: &str,
    ) -> Result<Agent, AppError> {
        let agent = self
            .repo
            .find_by_id(agent_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;

        if agent.user_id.as_deref().is_some_and(|id| id != user_id) {
            return Err(AppError::Forbidden("Not your agent".into()));
        }

        Ok(agent)
    }

    pub async fn list(
        &self,
        user_id: &str,
    ) -> Result<Vec<Agent>, AppError> {
        self.repo.find_by_user_id(user_id).await
    }

    pub async fn heartbeat_chat_ids(&self, user_id: &str) -> Vec<String> {
        self.repo
            .find_by_user_id(user_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| a.heartbeat_chat_id)
            .collect()
    }

    pub async fn update(
        &self,
        user_id: &str,
        agent_id: &str,
        req: UpdateAgentRequest,
    ) -> Result<Agent, AppError> {
        let mut agent = self
            .repo
            .find_by_id(agent_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;

        if agent.user_id.as_deref().is_some_and(|id| id != user_id) {
            return Err(AppError::Forbidden("Not your agent".into()));
        }

        let explicit_name = req.name.is_some();
        if let Some(ref name) = req.name {
            agent.name = name.clone();
            agent.identity.insert("name".to_string(), name.clone());
        }
        if let Some(description) = req.description {
            agent.description = description;
        }
        if let Some(model_group) = req.model_group {
            agent.model_group = model_group;
        }
        if let Some(enabled) = req.enabled {
            agent.enabled = enabled;
        }
        if let Some(skills) = req.skills {
            if skills.len() == 1 && skills[0] == "*" {
                agent.skills = None;
            } else {
                agent.skills = Some(skills);
            }
        }
        if let Some(sandbox_limits) = req.sandbox_limits {
            agent.sandbox_limits = Some(sandbox_limits);
        }
        if let Some(prompt) = req.prompt {
            agent.prompt = if prompt.is_empty() { None } else { Some(prompt) };
        }
        if let Some(ref identity) = req.identity {
            if let Some(avatar) = identity.get("avatar")
                && avatar.starts_with("data:")
            {
                return Err(AppError::Validation(
                    "Inline data: URLs are not allowed for avatars".into(),
                ));
            }
            if !explicit_name
                && let Some(new_name) = identity.get("name").filter(|n| !n.is_empty())
                && agent.identity.get("name") != Some(new_name)
            {
                agent.name = new_name.clone();
            }
            agent.identity = req.identity.unwrap();
        }
        agent.updated_at = chrono::Utc::now();

        let agent = self.repo.update(&agent).await?;
        self.push_agent_limits(agent_id, &agent);
        self.cache.invalidate(agent_id).await;
        if let Some(policy) = req.sandbox_policy.as_ref() {
            self.policy_service
                .reconcile_sandbox_policy(
                    user_id,
                    crate::policy::reconcile::EntityRef::Agent(agent_id.to_string()),
                    policy,
                )
                .await?;
        }
        Ok(agent)
    }

    pub async fn find_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<super::models::Agent>, AppError> {
        self.repo.find_by_name(user_id, name).await
    }

    pub async fn delete(
        &self,
        user_id: &str,
        agent_id: &str,
    ) -> Result<(), AppError> {
        let agent = self
            .repo
            .find_by_id(agent_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;

        if agent.user_id.as_deref().is_some_and(|id| id != user_id) {
            return Err(AppError::Forbidden("Not your agent".into()));
        }

        self.cache.invalidate(agent_id).await;
        self.policy_service
            .reconcile_sandbox_policy(
                user_id,
                crate::policy::reconcile::EntityRef::Agent(agent_id.to_string()),
                &SandboxPolicy::permissive(),
            )
            .await?;
        self.repo.delete(agent_id).await
    }

    pub async fn find_due_heartbeats(&self, now: DateTime<Utc>) -> Result<Vec<Agent>, AppError> {
        self.repo.find_due_heartbeats(now).await
    }

    pub async fn update_next_heartbeat(
        &self,
        agent_id: &str,
        next: Option<DateTime<Utc>>,
    ) -> Result<Agent, AppError> {
        let mut agent = self
            .repo
            .find_by_id(agent_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;

        agent.next_heartbeat_at = next;
        agent.updated_at = chrono::Utc::now();
        let agent = self.repo.update(&agent).await?;
        self.cache.invalidate(agent_id).await;
        Ok(agent)
    }

    pub async fn update_heartbeat_chat(
        &self,
        agent_id: &str,
        chat_id: &str,
    ) -> Result<Agent, AppError> {
        let mut agent = self
            .repo
            .find_by_id(agent_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;

        agent.heartbeat_chat_id = Some(chat_id.to_string());
        agent.updated_at = chrono::Utc::now();
        let agent = self.repo.update(&agent).await?;
        self.cache.invalidate(agent_id).await;
        Ok(agent)
    }

    pub async fn set_heartbeat(
        &self,
        agent_id: &str,
        interval_minutes: Option<u64>,
    ) -> Result<Agent, AppError> {
        let mut agent = self
            .repo
            .find_by_id(agent_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;

        agent.heartbeat_interval = interval_minutes;
        match interval_minutes {
            Some(mins) if mins > 0 => {
                agent.next_heartbeat_at = Some(Utc::now() + chrono::Duration::minutes(mins as i64));
            }
            _ => {
                agent.next_heartbeat_at = None;
            }
        }
        agent.updated_at = Utc::now();
        let agent = self.repo.update(&agent).await?;
        self.cache.invalidate(agent_id).await;
        Ok(agent)
    }
}
