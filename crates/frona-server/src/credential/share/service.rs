use std::sync::Arc;

use chrono::{Duration, Utc};

use super::models::{Share, ShareKind};
use super::repository::ShareRepository;
use crate::core::error::AppError;

#[derive(Clone)]
pub struct ShareService {
    repo: Arc<dyn ShareRepository>,
    default_ttl_secs: u64,
}

impl ShareService {
    pub fn new(repo: Arc<dyn ShareRepository>, default_ttl_secs: u64) -> Self {
        Self {
            repo,
            default_ttl_secs,
        }
    }

    pub fn default_ttl_secs(&self) -> u64 {
        self.default_ttl_secs
    }

    /// Returns the share id (not a URL); callers compose `/s/{id}` or
    /// `/p/{id}` themselves.
    pub async fn issue_file(
        &self,
        owner: &str,
        path: &str,
        user_id: &str,
        ttl_secs: u64,
    ) -> Result<String, AppError> {
        self.issue_file_with_visibility(owner, path, user_id, ttl_secs, false).await
    }

    /// Public (no-auth) share — resolved by minting a presigned URL on the
    /// fly. Not used by channel adapters; reserved for a future share-with-
    /// someone UI.
    pub async fn issue_file_public(
        &self,
        owner: &str,
        path: &str,
        user_id: &str,
        ttl_secs: u64,
    ) -> Result<String, AppError> {
        self.issue_file_with_visibility(owner, path, user_id, ttl_secs, true).await
    }

    async fn issue_file_with_visibility(
        &self,
        owner: &str,
        path: &str,
        user_id: &str,
        ttl_secs: u64,
        public: bool,
    ) -> Result<String, AppError> {
        let now = Utc::now();
        let id = nanoid::nanoid!(8);
        let row = Share {
            id: id.clone(),
            user_id: user_id.to_string(),
            kind: ShareKind::File {
                owner: owner.to_string(),
                path: path.to_string(),
                public,
            },
            expires_at: now + Duration::seconds(ttl_secs as i64),
            created_at: now,
        };
        self.repo.create(&row).await?;
        Ok(id)
    }

    /// One chat = one stable `/s/{id}` short link, reused across SMS
    /// overflow events instead of minting one row per long reply.
    /// Concurrent first-issues may produce two rows; both redirect to the
    /// same `/chats/{chat_id}`, so no uniqueness constraint is needed.
    pub async fn lookup_or_issue_chat(
        &self,
        chat_id: &str,
        user_id: &str,
        ttl_secs: u64,
    ) -> Result<String, AppError> {
        if let Some(existing) = self.repo.find_active_chat_share(user_id, chat_id).await? {
            return Ok(existing.id);
        }
        let now = Utc::now();
        let id = nanoid::nanoid!(8);
        let row = Share {
            id: id.clone(),
            user_id: user_id.to_string(),
            kind: ShareKind::Chat {
                chat_id: chat_id.to_string(),
            },
            expires_at: now + Duration::seconds(ttl_secs as i64),
            created_at: now,
        };
        self.repo.create(&row).await?;
        Ok(id)
    }

    /// `Ok(None)` for both "unknown id" and "expired" — the route handler
    /// returns byte-identical 404s so the route can't be used as an oracle.
    pub async fn resolve(&self, id: &str) -> Result<Option<Share>, AppError> {
        self.repo.find_active_by_id(id).await
    }

    pub async fn cleanup_expired(&self) -> Result<u64, AppError> {
        self.repo.delete_expired().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::generic::SurrealRepo;

    async fn make_test_service() -> ShareService {
        use surrealdb::Surreal;
        use surrealdb::engine::local::Mem;
        let db = Surreal::new::<Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        let repo: Arc<dyn ShareRepository> = Arc::new(SurrealRepo::<Share>::new(db));
        ShareService::new(repo, 30 * 24 * 60 * 60)
    }

    #[tokio::test]
    async fn issue_file_returns_8_char_id() {
        let svc = make_test_service().await;
        let id = svc
            .issue_file("agent:researcher", "report.md", "user-1", 60)
            .await
            .unwrap();
        assert_eq!(id.len(), 8);
        // nanoid alphabet is base64-url: A-Z a-z 0-9 _ -.
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[tokio::test]
    async fn resolve_round_trips_kind() {
        let svc = make_test_service().await;
        let id = svc
            .issue_file("agent:researcher", "report.md", "user-1", 60)
            .await
            .unwrap();
        let row = svc.resolve(&id).await.unwrap().unwrap();
        match row.kind {
            ShareKind::File { owner, path, public } => {
                assert_eq!(owner, "agent:researcher");
                assert_eq!(path, "report.md");
                assert!(!public);
            }
            other => panic!("unexpected kind: {other:?}"),
        }
    }

    #[tokio::test]
    async fn issue_file_public_sets_public_flag() {
        let svc = make_test_service().await;
        let id = svc
            .issue_file_public("agent:researcher", "report.md", "user-1", 60)
            .await
            .unwrap();
        let row = svc.resolve(&id).await.unwrap().unwrap();
        match row.kind {
            ShareKind::File { public, .. } => assert!(public),
            other => panic!("unexpected kind: {other:?}"),
        }
    }

    #[tokio::test]
    async fn lookup_or_issue_chat_mints_then_reuses() {
        let svc = make_test_service().await;
        let first = svc
            .lookup_or_issue_chat("chat-abc", "user-1", 3600)
            .await
            .unwrap();
        let second = svc
            .lookup_or_issue_chat("chat-abc", "user-1", 3600)
            .await
            .unwrap();
        assert_eq!(first, second, "second call must reuse the same row");

        let row = svc.resolve(&first).await.unwrap().unwrap();
        match row.kind {
            ShareKind::Chat { chat_id } => assert_eq!(chat_id, "chat-abc"),
            other => panic!("unexpected kind: {other:?}"),
        }
    }

    #[tokio::test]
    async fn lookup_or_issue_chat_scoped_per_user_and_chat() {
        let svc = make_test_service().await;
        let a = svc.lookup_or_issue_chat("chat-1", "user-1", 3600).await.unwrap();
        let b = svc.lookup_or_issue_chat("chat-2", "user-1", 3600).await.unwrap();
        let c = svc.lookup_or_issue_chat("chat-1", "user-2", 3600).await.unwrap();
        assert_ne!(a, b, "different chats must produce different rows");
        assert_ne!(a, c, "different users must produce different rows");
    }

    #[tokio::test]
    async fn lookup_or_issue_chat_remints_when_existing_expired() {
        let svc = make_test_service().await;
        let first = svc.lookup_or_issue_chat("chat-x", "user-1", 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let second = svc.lookup_or_issue_chat("chat-x", "user-1", 3600).await.unwrap();
        assert_ne!(first, second, "expired row must not be reused");
    }

    #[tokio::test]
    async fn resolve_unknown_id_returns_none() {
        let svc = make_test_service().await;
        let got = svc.resolve("does-not-exist").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn resolve_expired_returns_none() {
        let svc = make_test_service().await;
        // TTL = 0 → expires_at = now (≤ now), so the row is considered expired
        // on the very next read.
        let id = svc
            .issue_file("agent:r", "x.md", "user-1", 0)
            .await
            .unwrap();
        // Sleep to ensure now > expires_at.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let got = svc.resolve(&id).await.unwrap();
        assert!(got.is_none(), "expired row should resolve to None");
    }

    #[tokio::test]
    async fn cleanup_expired_only_deletes_past_rows() {
        let svc = make_test_service().await;
        // Three expired (zero TTL) + two future.
        for _ in 0..3 {
            svc.issue_file("agent:r", "x.md", "user-1", 0).await.unwrap();
        }
        for _ in 0..2 {
            svc.issue_file("agent:r", "x.md", "user-1", 3600).await.unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let deleted = svc.cleanup_expired().await.unwrap();
        assert_eq!(deleted, 3, "should delete exactly the expired rows");
    }
}
