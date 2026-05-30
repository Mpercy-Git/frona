use async_trait::async_trait;
use chrono::{DateTime, Utc};
use crate::core::Handle;
use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::Agent;

#[async_trait]
pub trait AgentRepository: Repository<Agent> {
    async fn find_all(&self) -> Result<Vec<Agent>, AppError>;
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<Agent>, AppError>;
    async fn find_by_name(&self, user_id: &str, name: &str) -> Result<Option<Agent>, AppError>;
    async fn find_by_handle(&self, user_id: &str, handle: &Handle) -> Result<Option<Agent>, AppError>;
    async fn find_due_heartbeats(&self, now: DateTime<Utc>) -> Result<Vec<Agent>, AppError>;
}
