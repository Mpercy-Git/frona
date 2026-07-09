use async_trait::async_trait;

use super::models::KeyPair;
use crate::core::error::AppError;
use crate::core::repository::Repository;

#[async_trait]
pub trait KeyPairRepository: Repository<KeyPair> {
    async fn find_active_by_owner(&self, owner: &str) -> Result<Option<KeyPair>, AppError>;
    async fn find_by_kid(&self, kid: &str) -> Result<Option<KeyPair>, AppError>;
    async fn find_all_active(&self) -> Result<Vec<KeyPair>, AppError>;
}
