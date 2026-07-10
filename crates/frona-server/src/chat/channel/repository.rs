use async_trait::async_trait;

use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::Channel;

#[async_trait]
pub trait ChannelRepository: Repository<Channel> {
    async fn find_by_user(&self, user_id: &str) -> Result<Vec<Channel>, AppError>;
    async fn find_by_space(&self, space_id: &str) -> Result<Option<Channel>, AppError>;
    /// Boot + reconcile candidate set: enabled and not terminally failed.
    async fn find_active(&self) -> Result<Vec<Channel>, AppError>;
    /// Channels with a pending pairing overlay (`user_address.pairing_code`).
    async fn find_pairing_pending(&self) -> Result<Vec<Channel>, AppError>;
}
