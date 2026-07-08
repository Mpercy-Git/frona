use async_trait::async_trait;

use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::{Channel, ChannelStatus};

#[async_trait]
pub trait ChannelRepository: Repository<Channel> {
    async fn find_by_user(&self, user_id: &str) -> Result<Vec<Channel>, AppError>;
    async fn find_by_space(&self, space_id: &str) -> Result<Option<Channel>, AppError>;
    async fn find_active(&self) -> Result<Vec<Channel>, AppError>;
    async fn find_in_status(&self, status: ChannelStatus) -> Result<Vec<Channel>, AppError>;
}
