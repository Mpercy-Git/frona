use async_trait::async_trait;
use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::Contact;

#[async_trait]
pub trait ContactRepository: Repository<Contact> {
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<Contact>, AppError>;
    async fn find_by_phone(&self, user_id: &str, phone: &str) -> Result<Option<Contact>, AppError>;
    /// `idx_contact_addresses` UNIQUE index guarantees one Contact per `(user, provider, address)` across spaces.
    async fn find_by_channel_address(
        &self,
        user_id: &str,
        provider: &str,
        address: &str,
    ) -> Result<Option<Contact>, AppError>;
}
