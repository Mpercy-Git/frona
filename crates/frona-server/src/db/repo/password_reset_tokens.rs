use async_trait::async_trait;
use chrono::Utc;

use crate::auth::password_reset::models::PasswordResetToken;
use crate::auth::password_reset::repository::PasswordResetRepository;
use crate::core::error::AppError;

use super::generic::SurrealRepo;

pub type SurrealPasswordResetRepo = SurrealRepo<PasswordResetToken>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl PasswordResetRepository for SurrealRepo<PasswordResetToken> {
    async fn find_by_hash(&self, token_hash: &str) -> Result<Option<PasswordResetToken>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM password_reset_token WHERE token_hash = $token_hash LIMIT 1"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("token_hash", token_hash.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let token: Option<PasswordResetToken> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(token)
    }

    async fn take_by_hash(
        &self,
        token_hash: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<String>, AppError> {
        // RETURN BEFORE hands back what the DELETE removed, so a request that
        // loses a race to redeem the same secret gets nothing back.
        let query = format!(
            "DELETE password_reset_token WHERE token_hash = $token_hash AND expires_at > $now \
             RETURN BEFORE"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("token_hash", token_hash.to_string()))
            .bind(("now", now))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        // Read as JSON: the raw record id is not the `String` the entity uses.
        let deleted: Vec<serde_json::Value> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(deleted
            .first()
            .and_then(|row| row.get("user_id"))
            .and_then(|v| v.as_str())
            .map(str::to_string))
    }

    async fn latest_created_at(
        &self,
        user_id: &str,
    ) -> Result<Option<chrono::DateTime<Utc>>, AppError> {
        let mut result = self
            .db()
            .query(
                "SELECT created_at FROM password_reset_token WHERE user_id = $user_id \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows: Vec<serde_json::Value> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(rows
            .first()
            .and_then(|row| row.get("created_at"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok()))
    }

    async fn delete_by_user_id(&self, user_id: &str) -> Result<(), AppError> {
        self.db()
            .query("DELETE FROM password_reset_token WHERE user_id = $user_id")
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    async fn delete_expired(&self) -> Result<(), AppError> {
        self.db()
            .query("DELETE FROM password_reset_token WHERE expires_at <= $now")
            .bind(("now", Utc::now()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}
