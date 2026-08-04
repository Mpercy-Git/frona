use async_trait::async_trait;

use crate::chat::channel::models::{Channel, ChannelStatus};
use crate::chat::channel::repository::ChannelRepository;
use crate::core::error::AppError;

use super::generic::SurrealRepo;

pub type SurrealChannelRepo = SurrealRepo<Channel>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl ChannelRepository for SurrealRepo<Channel> {
    async fn find_by_user(&self, user_id: &str) -> Result<Vec<Channel>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM channel WHERE user_id = $user_id ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))
    }

    async fn find_by_space(&self, space_id: &str) -> Result<Option<Channel>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM channel WHERE space_id = $space_id LIMIT 1"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("space_id", space_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let mut rows: Vec<Channel> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(rows.pop())
    }

    async fn find_active(&self) -> Result<Vec<Channel>, AppError> {
        // Intent-keyed: everything the operator enabled that isn't terminally
        // failed. The supervisor rebuilds runtime status from here on boot/reconcile.
        let query = format!(
            "{SELECT_CLAUSE} FROM channel WHERE enabled = true AND status != $failed"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("failed", ChannelStatus::Failed))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))
    }

    async fn find_pairing_pending(&self) -> Result<Vec<Channel>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM channel WHERE user_address.pairing_code != NONE"
        );
        let mut result = self
            .db()
            .query(&query)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))
    }
}
