use async_trait::async_trait;

use crate::core::Handle;
use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::McpServer;

#[async_trait]
pub trait McpServerRepository: Repository<McpServer> {
    async fn find_by_user(&self, user_id: &str) -> Result<Vec<McpServer>, AppError>;
    async fn find_running(&self) -> Result<Vec<McpServer>, AppError>;
    async fn find_by_handle(
        &self,
        user_id: &str,
        handle: &Handle,
    ) -> Result<Option<McpServer>, AppError>;
}
