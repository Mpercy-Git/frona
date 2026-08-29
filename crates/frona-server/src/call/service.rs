use chrono::Utc;

use crate::core::error::AppError;
use crate::core::repository::Repository;
use crate::db::repo::generic::SurrealRepo;

use super::models::{Call, CallDirection, CallStatus};
use super::repository::CallRepository;

#[derive(Clone)]
pub struct CallService {
    repo: SurrealRepo<Call>,
}

impl CallService {
    pub fn new(repo: SurrealRepo<Call>) -> Self {
        Self { repo }
    }

    pub async fn create(
        &self,
        chat_id: &str,
        contact_id: &str,
        twilio_sid: &str,
        direction: CallDirection,
    ) -> Result<Call, AppError> {
        let now = Utc::now();
        let call = Call {
            id: crate::core::repository::new_id(),
            chat: surrealdb::types::RecordId::new("chat", chat_id),
            contact_id: contact_id.to_string(),
            status: CallStatus::Ringing,
            direction,
            twilio_sid: twilio_sid.to_string(),
            started_at: now,
            answered_at: None,
            ended_at: None,
            transfer_count: 0,
            created_at: now,
            updated_at: now,
        };
        self.repo.create(&call).await
    }

    pub async fn mark_active(&self, call_id: &str) -> Result<Call, AppError> {
        let mut call = self
            .repo
            .find_by_id(call_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Call not found".into()))?;
        call.status = CallStatus::Active;
        call.answered_at = Some(Utc::now());
        call.updated_at = Utc::now();
        self.repo.update(&call).await
    }

    pub async fn mark_completed(&self, call_id: &str) -> Result<Call, AppError> {
        let mut call = self
            .repo
            .find_by_id(call_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Call not found".into()))?;
        call.status = CallStatus::Completed;
        call.ended_at = Some(Utc::now());
        call.updated_at = Utc::now();
        self.repo.update(&call).await
    }

    pub async fn find_by_chat_id(&self, chat_id: &str) -> Result<Option<Call>, AppError> {
        self.repo.find_by_chat_id(chat_id).await
    }

    /// Bump the call's transfer count by one. Callers check the cap against
    /// the current count (via `find_by_chat_id`) before calling this — this
    /// method only records that a transfer happened, it doesn't enforce a
    /// limit itself.
    pub async fn increment_transfer_count(&self, call_id: &str) -> Result<Call, AppError> {
        let mut call = self
            .repo
            .find_by_id(call_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Call not found".into()))?;
        call.transfer_count += 1;
        call.updated_at = Utc::now();
        self.repo.update(&call).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_test_service() -> CallService {
        use surrealdb::Surreal;
        use surrealdb::engine::local::Mem;
        let db = Surreal::new::<Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        CallService::new(SurrealRepo::new(db))
    }

    #[tokio::test]
    async fn create_and_find_call() {
        let svc = make_test_service().await;
        let call = svc
            .create("chat-1", "contact-1", "SID123", CallDirection::Outbound)
            .await
            .unwrap();
        assert_eq!(call.status, CallStatus::Ringing);
        assert_eq!(call.twilio_sid, "SID123");

        let found = svc.find_by_chat_id("chat-1").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, call.id);
    }

    #[tokio::test]
    async fn mark_active_sets_answered_at() {
        let svc = make_test_service().await;
        let call = svc
            .create("chat-2", "contact-1", "SID456", CallDirection::Outbound)
            .await
            .unwrap();
        let updated = svc.mark_active(&call.id).await.unwrap();
        assert_eq!(updated.status, CallStatus::Active);
        assert!(updated.answered_at.is_some());
    }

    #[tokio::test]
    async fn mark_completed_sets_ended_at() {
        let svc = make_test_service().await;
        let call = svc
            .create("chat-3", "contact-1", "SID789", CallDirection::Outbound)
            .await
            .unwrap();
        let updated = svc.mark_completed(&call.id).await.unwrap();
        assert_eq!(updated.status, CallStatus::Completed);
        assert!(updated.ended_at.is_some());
    }

    #[tokio::test]
    async fn increment_transfer_count_starts_at_zero_and_accumulates() {
        let svc = make_test_service().await;
        let call = svc
            .create("chat-4", "contact-1", "SID999", CallDirection::Inbound)
            .await
            .unwrap();
        assert_eq!(call.transfer_count, 0);

        let after_one = svc.increment_transfer_count(&call.id).await.unwrap();
        assert_eq!(after_one.transfer_count, 1);

        let after_two = svc.increment_transfer_count(&call.id).await.unwrap();
        assert_eq!(after_two.transfer_count, 2);
    }
}
