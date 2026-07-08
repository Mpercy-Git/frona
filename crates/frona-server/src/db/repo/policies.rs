use async_trait::async_trait;

use crate::core::error::AppError;
use crate::policy::models::Policy;
use crate::policy::repository::PolicyRepository;

use super::generic::SurrealRepo;

pub type SurrealPolicyRepo = SurrealRepo<Policy>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl PolicyRepository for SurrealRepo<Policy> {
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<Policy>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM policy WHERE user_id = $user_id ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let policies: Vec<Policy> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(policies)
    }

    async fn find_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Policy>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM policy WHERE user_id = $user_id AND name = $name LIMIT 1"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .bind(("name", name.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let policy: Option<Policy> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(policy)
    }

    async fn find_system_policies(&self) -> Result<Vec<Policy>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM policy WHERE user_id = NONE ORDER BY created_at ASC"
        );
        let mut result = self
            .db()
            .query(&query)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let policies: Vec<Policy> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(policies)
    }

    async fn find_system_by_name(&self, name: &str) -> Result<Option<Policy>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM policy WHERE user_id = NONE AND name = $name LIMIT 1"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("name", name.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let policy: Option<Policy> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(policy)
    }

    async fn delete_by_ids(&self, ids: &[String]) -> Result<(), AppError> {
        for id in ids {
            self.db()
                .query("DELETE type::record('policy', $id)")
                .bind(("id", id.to_string()))
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        Ok(())
    }
}
