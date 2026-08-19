use crate::auth::User;
use crate::auth::UserRepository;
use crate::core::error::AppError;
use crate::core::repository::new_id;
use crate::core::user_config::{UserConfig, UserConfigPatch};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use struct_patch::Patch;

use super::generic::SurrealRepo;

pub type SurrealUserRepo = SurrealRepo<User>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

/// Per-user config lives in its own table (`user_config`, 1:1 with `user`) but is
/// owned by `UserService`, so its persistence hangs off the user repo. Raw
/// SurrealQL stays here (repos own SQL); the service adds the cache + `Utc::now()`.
impl SurrealRepo<User> {
    /// The user's config row, or `None` if they've never customized anything.
    /// `UserService::user_config` turns `None` into `UserConfig::default()`.
    pub async fn user_config(&self, user_id: &str) -> Result<Option<UserConfig>, AppError> {
        let mut result = self
            .db()
            .query(format!(
                "{SELECT_CLAUSE} FROM user_config WHERE user_id = $uid LIMIT 1"
            ))
            .bind(("uid", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let config: Option<UserConfig> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(config)
    }

    /// Apply `patch` under an `updated_at` compare-and-swap. When a row exists, the
    /// present sections are written iff its `updated_at` still equals `expected`
    /// (else `AppError::Conflict` - the caller re-reads and retries). When none
    /// exists, a defaults+patch row is created; a lost create race (two callers at
    /// once) trips the `UNIQUE(user_id)` index and surfaces as `AppError::Database`,
    /// which the caller can likewise retry. Returns the persisted value.
    pub async fn patch_user_config(
        &self,
        user_id: &str,
        expected: DateTime<Utc>,
        patch: UserConfigPatch,
        now: DateTime<Utc>,
    ) -> Result<UserConfig, AppError> {
        match self.user_config(user_id).await? {
            Some(existing) => {
                // Apply the patch in Rust (only the present sections; id/user_id/
                // timestamps are `#[patch(skip)]`), then write the whole record under
                // the CAS: `CONTENT` replaces it iff `updated_at` is still `expected`.
                // Binding the struct serializes via `SurrealValue` (proper datetimes +
                // record id), same as `.content()`.
                let mut row = existing;
                row.apply(patch);
                row.updated_at = now;
                let mut result = self
                    .db()
                    .query(
                        "UPDATE type::record('user_config', $id) \
                         CONTENT $row WHERE updated_at = $expected RETURN AFTER",
                    )
                    .bind(("id", row.id.clone()))
                    .bind(("row", row.clone()))
                    .bind(("expected", expected))
                    .await
                    .map_err(|e| AppError::Database(e.to_string()))?;
                // The row is read back only to detect the CAS hit - an empty result
                // means `updated_at` moved on (or the row vanished). We return the
                // value we wrote rather than decode it (avoids the record-id shape).
                let updated: Option<surrealdb::types::Value> = result
                    .take(0)
                    .map_err(|e| AppError::Database(e.to_string()))?;
                if updated.is_none() {
                    return Err(AppError::Conflict(
                        "user config was modified concurrently; re-read and retry".into(),
                    ));
                }
                Ok(row)
            }
            None => {
                let mut row = UserConfig {
                    id: new_id(),
                    user_id: user_id.to_string(),
                    created_at: now,
                    updated_at: now,
                    ..UserConfig::default()
                };
                row.apply(patch);
                // `apply` can't touch timestamps (they're skipped), so they stay `now`.
                let _: Option<surrealdb::types::Value> = self
                    .db()
                    .create(("user_config", row.id.clone()))
                    .content(row.clone())
                    .await
                    .map_err(|e| AppError::Database(e.to_string()))?;
                Ok(row)
            }
        }
    }
}

#[async_trait]
impl UserRepository for SurrealRepo<User> {
    async fn find_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        let mut result = self
            .db()
            .query(format!(
                "{SELECT_CLAUSE} FROM user WHERE string::lowercase(email) = $email LIMIT 1"
            ))
            .bind(("email", email.trim().to_lowercase()))
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
            .query(format!(
                "{SELECT_CLAUSE} FROM user WHERE handle = $handle LIMIT 1"
            ))
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
