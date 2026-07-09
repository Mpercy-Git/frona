use async_trait::async_trait;

use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::Call;

#[async_trait]
pub trait CallRepository: Repository<Call> {
    async fn find_by_chat_id(&self, chat_id: &str) -> Result<Option<Call>, AppError>;
}
