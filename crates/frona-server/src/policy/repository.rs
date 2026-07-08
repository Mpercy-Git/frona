use async_trait::async_trait;

use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::Policy;

#[async_trait]
pub trait PolicyRepository: Repository<Policy> {
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<Policy>, AppError>;
    async fn find_system_policies(&self) -> Result<Vec<Policy>, AppError>;
    async fn find_by_name(&self, user_id: &str, name: &str) -> Result<Option<Policy>, AppError>;
    async fn find_system_by_name(&self, name: &str) -> Result<Option<Policy>, AppError>;
    async fn delete_by_ids(&self, ids: &[String]) -> Result<(), AppError>;
}
