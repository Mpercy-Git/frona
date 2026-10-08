use async_trait::async_trait;

use crate::auth::oauth::models::OAuthIdentity;
use crate::auth::oauth::repository::OAuthRepository;
use crate::core::error::AppError;

use super::generic::SurrealRepo;

pub type SurrealOAuthRepo = SurrealRepo<OAuthIdentity>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl OAuthRepository for SurrealRepo<OAuthIdentity> {
    async fn find_identity_by_sub(
        &self,
        external_sub: &str,
    ) -> Result<Option<OAuthIdentity>, AppError> {
        let query =
            format!("{SELECT_CLAUSE} FROM oauth_identity WHERE external_sub = $sub LIMIT 1");
        let mut result = self
            .db()
            .query(&query)
            .bind(("sub", external_sub.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let identity: Option<OAuthIdentity> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(identity)
    }

    async fn find_identity(
        &self,
        issuer: &str,
        external_sub: &str,
    ) -> Result<Option<OAuthIdentity>, AppError> {
        let exact = format!(
            "{SELECT_CLAUSE} FROM oauth_identity WHERE external_sub = $sub AND issuer = $issuer LIMIT 1"
        );
        let legacy = format!(
            "{SELECT_CLAUSE} FROM oauth_identity WHERE external_sub = $sub AND (issuer = NONE OR issuer = NULL) LIMIT 1"
        );
        for query in [exact, legacy] {
            let mut result = self
                .db()
                .query(&query)
                .bind(("sub", external_sub.to_string()))
                .bind(("issuer", issuer.to_string()))
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            let identity: Option<OAuthIdentity> = result
                .take(0)
                .map_err(|e| AppError::Database(e.to_string()))?;
            if identity.is_some() {
                return Ok(identity);
            }
        }
        Ok(None)
    }

    async fn find_identities_by_user(&self, user_id: &str) -> Result<Vec<OAuthIdentity>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM oauth_identity WHERE user_id = $user_id ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let identities: Vec<OAuthIdentity> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(identities)
    }
}
