use async_trait::async_trait;

use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::push_model::PushSubscription;

#[async_trait]
pub trait PushSubscriptionRepository: Repository<PushSubscription> {
    /// Find all push subscriptions for a user.
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<PushSubscription>, AppError>;
    /// Find a subscription by its endpoint URL (for dedup / delete).
    async fn find_by_endpoint(
        &self,
        user_id: &str,
        endpoint: &str,
    ) -> Result<Option<PushSubscription>, AppError>;
    /// Delete a subscription by endpoint URL.
    async fn delete_by_endpoint(&self, user_id: &str, endpoint: &str) -> Result<(), AppError>;
}