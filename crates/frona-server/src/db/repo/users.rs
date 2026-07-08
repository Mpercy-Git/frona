use async_trait::async_trait;
use crate::auth::UserRepository;
use crate::core::error::AppError;
use crate::auth::User;

use super::generic::SurrealRepo;

pub type SurrealUserRepo = SurrealRepo<User>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl UserRepository for SurrealRepo<User> {
    async fn find_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        let mut result = self
            .db()
            .query(format!("{SELECT_CLAUSE} FROM user WHERE email = $email LIMIT 1"))
            .bind(("email", email.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let user: Option<User> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(user)
    }

    async fn find_by_handle(&self, handle: &crate::core::Handle) -> Result<Option<User>, AppError> {
        let mut result = self
            .db()
            .query(format!("{SELECT_CLAUSE} FROM user WHERE handle = $handle LIMIT 1"))
            .bind(("handle", handle.as_ref().to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let user: Option<User> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(user)
    }

    async fn has_users(&self) -> Result<bool, AppError> {
        let mut result = self
            .db()
            .query("SELECT count() as total FROM user GROUP ALL LIMIT 1")
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let row: Option<serde_json::Value> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(row.is_some_and(|v| v.get("total").and_then(|t| t.as_u64()).unwrap_or(0) > 0))
    }

    async fn find_any_active_admin(&self) -> Result<Option<User>, AppError> {
        let mut result = self
            .db()
            .query(format!(
                "{SELECT_CLAUSE} FROM user \
                 WHERE deactivated_at IS NONE \
                   AND groups CONTAINS 'admins' \
                 LIMIT 1"
            ))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let user: Option<User> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(user)
    }

    async fn find_oldest_active(&self) -> Result<Option<User>, AppError> {
        let mut result = self
            .db()
            .query(format!(
                "{SELECT_CLAUSE} FROM user \
                 WHERE deactivated_at IS NONE \
                 ORDER BY created_at ASC \
                 LIMIT 1"
            ))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let user: Option<User> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(user)
    }

    async fn list_all(&self, include_deactivated: bool) -> Result<Vec<User>, AppError> {
        let query = if include_deactivated {
            format!("{SELECT_CLAUSE} FROM user ORDER BY created_at ASC")
        } else {
            format!(
                "{SELECT_CLAUSE} FROM user WHERE deactivated_at IS NONE ORDER BY created_at ASC"
            )
        };
        let mut result = self
            .db()
            .query(&query)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let users: Vec<User> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(users)
    }
}
