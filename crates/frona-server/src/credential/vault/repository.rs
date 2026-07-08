use async_trait::async_trait;

use crate::core::Principal;
use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::{
    Credential, PrincipalCredentialBinding, VaultAccessLog, VaultConnection, VaultGrant,
};

#[async_trait]
pub trait CredentialRepository: Repository<Credential> {
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<Credential>, AppError>;
    async fn find_by_user_and_provider(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<Credential>, AppError>;
}

#[async_trait]
pub trait VaultConnectionRepository: Repository<VaultConnection> {
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<VaultConnection>, AppError>;
    async fn find_all_for_user(&self, user_id: &str) -> Result<Vec<VaultConnection>, AppError>;
    async fn find_system_managed(&self) -> Result<Vec<VaultConnection>, AppError>;
}

#[async_trait]
pub trait VaultGrantRepository: Repository<VaultGrant> {
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<VaultGrant>, AppError>;
    async fn find_matching_grant(
        &self,
        user_id: &str,
        principal: &Principal,
        query: &str,
    ) -> Result<Option<VaultGrant>, AppError>;
    async fn find_by_principal(
        &self,
        user_id: &str,
        principal: &Principal,
    ) -> Result<Vec<VaultGrant>, AppError>;
    async fn delete_by_principal(
        &self,
        user_id: &str,
        principal: &Principal,
    ) -> Result<(), AppError>;
    async fn delete_by_connection_id(&self, connection_id: &str) -> Result<(), AppError>;
}

#[async_trait]
pub trait VaultAccessLogRepository: Repository<VaultAccessLog> {
    async fn find_by_chat_id(&self, chat_id: &str) -> Result<Vec<VaultAccessLog>, AppError>;
    async fn find_by_chat_and_query(
        &self,
        chat_id: &str,
        query: &str,
        env_var_prefix: Option<&str>,
    ) -> Result<Option<VaultAccessLog>, AppError>;
}

#[async_trait]
pub trait PrincipalCredentialBindingRepository:
    Repository<PrincipalCredentialBinding>
{
    /// Look up the binding for an exact `(user, principal, query)` triple,
    /// honoring scope: returns the chat-scoped one if `chat_id` is supplied
    /// and a match exists, otherwise the durable one. Expired bindings are
    /// filtered out.
    async fn find_for_lookup(
        &self,
        user_id: &str,
        principal: &Principal,
        query: &str,
        chat_id: Option<&str>,
    ) -> Result<Option<PrincipalCredentialBinding>, AppError>;

    /// All non-expired bindings visible to a chat session: every durable
    /// binding for the principal plus any bindings scoped to this chat.
    async fn find_for_chat(
        &self,
        user_id: &str,
        principal: &Principal,
        chat_id: &str,
    ) -> Result<Vec<PrincipalCredentialBinding>, AppError>;

    /// Every non-expired binding for a principal regardless of scope.
    async fn find_for_principal(
        &self,
        user_id: &str,
        principal: &Principal,
    ) -> Result<Vec<PrincipalCredentialBinding>, AppError>;

    async fn delete_by_principal(
        &self,
        user_id: &str,
        principal: &Principal,
    ) -> Result<(), AppError>;

    async fn delete_for_item(
        &self,
        user_id: &str,
        principal: &Principal,
        connection_id: &str,
        vault_item_id: &str,
    ) -> Result<(), AppError>;

    async fn delete_by_chat(&self, chat_id: &str) -> Result<(), AppError>;
}
