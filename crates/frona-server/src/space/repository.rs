use async_trait::async_trait;
use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::Space;

#[async_trait]
pub trait SpaceRepository: Repository<Space> {
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<Space>, AppError>;
    async fn find_all(&self) -> Result<Vec<Space>, AppError>;
}
