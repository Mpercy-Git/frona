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
            .query(
                "DELETE FROM push_subscription WHERE user_id = $user_id AND endpoint = $endpoint",
            )
            .bind(("user_id", user_id.to_string()))
            .bind(("endpoint", endpoint.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::repository::Repository;

    async fn make_repo() -> SurrealPushSubscriptionRepo {
        use surrealdb::Surreal;
        use surrealdb::engine::local::Mem;
        let db = Surreal::new::<Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        SurrealRepo::new(db)
    }

    fn make_sub(user_id: &str, endpoint: &str) -> PushSubscription {
        PushSubscription {
            id: crate::core::repository::new_id(),
            user_id: user_id.to_string(),
            endpoint: endpoint.to_string(),
            expiration_time: None,
            p256dh_key: "p256dh".to_string(),
            auth_secret: "auth".to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    /// The table has to be declared in `setup_schema`: SurrealDB rejects a
    /// SELECT against an undefined table, so without the DEFINE every push
    /// delivery fails with "The table 'push_subscription' does not exist"
    /// before it ever reaches a push service.
    #[tokio::test]
    async fn find_by_user_id_works_on_a_freshly_initialised_schema() {
        let repo = make_repo().await;

        let subs = repo.find_by_user_id("user-1").await.unwrap();
        assert!(subs.is_empty());
    }

    #[tokio::test]
    async fn subscriptions_round_trip_per_user() {
        let repo = make_repo().await;
        repo.create(&make_sub("user-1", "https://push.example/a"))
            .await
            .unwrap();
        repo.create(&make_sub("user-1", "https://push.example/b"))
            .await
            .unwrap();
        repo.create(&make_sub("user-2", "https://push.example/c"))
            .await
            .unwrap();

        assert_eq!(repo.find_by_user_id("user-1").await.unwrap().len(), 2);
        assert_eq!(repo.find_by_user_id("user-2").await.unwrap().len(), 1);

        let found = repo
            .find_by_endpoint("user-1", "https://push.example/a")
            .await
            .unwrap();
        assert!(found.is_some());
        // The same endpoint under a different user is a different subscription.
        assert!(
            repo.find_by_endpoint("user-2", "https://push.example/a")
                .await
                .unwrap()
                .is_none()
        );

        repo.delete_by_endpoint("user-1", "https://push.example/a")
            .await
            .unwrap();
        assert_eq!(repo.find_by_user_id("user-1").await.unwrap().len(), 1);
    }

    /// Guards the `DEFINE INDEX ... UNIQUE` half of the schema: without it a
    /// device that re-subscribes would accumulate duplicate rows and get every
    /// notification twice.
    #[tokio::test]
    async fn a_duplicate_endpoint_for_one_user_is_rejected() {
        let repo = make_repo().await;
        repo.create(&make_sub("user-1", "https://push.example/a"))
            .await
            .unwrap();

        let duplicate = repo
            .create(&make_sub("user-1", "https://push.example/a"))
            .await;
        assert!(
            duplicate.is_err(),
            "the unique index should reject a second row for the same endpoint"
        );
    }

    /// Two accounts on one device share a push endpoint, so uniqueness is
    /// scoped to the user rather than the endpoint on its own.
    #[tokio::test]
    async fn the_same_endpoint_is_allowed_for_a_second_user() {
        let repo = make_repo().await;
        repo.create(&make_sub("user-1", "https://push.example/shared"))
            .await
            .unwrap();
        repo.create(&make_sub("user-2", "https://push.example/shared"))
            .await
            .unwrap();

        assert_eq!(repo.find_by_user_id("user-1").await.unwrap().len(), 1);
        assert_eq!(repo.find_by_user_id("user-2").await.unwrap().len(), 1);
    }
}
