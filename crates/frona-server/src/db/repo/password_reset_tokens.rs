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
