use async_trait::async_trait;
use crate::core::Handle;
use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::App;

#[async_trait]
pub trait AppRepository: Repository<App> {
    async fn find_by_agent_id(&self, agent_id: &str) -> Result<Vec<App>, AppError>;
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<App>, AppError>;
    async fn find_by_user_handle(
        &self,
        user_id: &str,
        handle: &Handle,
    ) -> Result<Option<App>, AppError>;
    async fn find_running(&self) -> Result<Vec<App>, AppError>;
}
