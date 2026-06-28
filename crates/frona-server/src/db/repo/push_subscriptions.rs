use async_trait::async_trait;

use crate::core::error::AppError;
use crate::notification::push_model::PushSubscription;
use crate::notification::push_repository::PushSubscriptionRepository;

use super::generic::SurrealRepo;

pub type SurrealPushSubscriptionRepo = SurrealRepo<PushSubscription>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl PushSubscriptionRepository for SurrealRepo<PushSubscription> {
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<PushSubscription>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM push_subscription WHERE user_id = $user_id ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let subs: Vec<PushSubscription> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(subs)
    }

    async fn find_by_endpoint(
        &self,
        user_id: &str,
        endpoint: &str,
    ) -> Result<Option<PushSubscription>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM push_subscription WHERE user_id = $user_id AND endpoint = $endpoint LIMIT 1"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .bind(("endpoint", endpoint.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let subs: Vec<PushSubscription> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(subs.into_iter().next())
    }

    async fn delete_by_endpoint(&self, user_id: &str, endpoint: &str) -> Result<(), AppError> {
        self.db()
            .query("DELETE FROM push_subscription WHERE user_id = $user_id AND endpoint = $endpoint")
            .bind(("user_id", user_id.to_string()))
            .bind(("endpoint", endpoint.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}