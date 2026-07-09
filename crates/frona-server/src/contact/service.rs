use chrono::Utc;

use crate::chat::broadcast::{BroadcastService, EntityAction};
use crate::db::repo::generic::SurrealRepo;
use crate::core::error::AppError;
use crate::core::repository::Repository;

use super::models::{Contact, ContactAddress, ContactResponse, CreateContactRequest, UpdateContactRequest};
use super::repository::ContactRepository;

#[derive(Clone)]
pub struct ContactService {
    repo: SurrealRepo<Contact>,
    broadcast: BroadcastService,
}

impl ContactService {
    pub fn new(repo: SurrealRepo<Contact>, broadcast: BroadcastService) -> Self {
        Self { repo, broadcast }
    }

    fn broadcast_update(&self, contact: &Contact, action: EntityAction) {
        self.broadcast.broadcast_entity_updated(
            &contact.user_id,
            "contact",
            &contact.id,
            action,
            contact.space_id.clone(),
            None,
        );
    }

    pub async fn find_or_create_by_phone(
        &self,
        user_id: &str,
        phone: &str,
        name: &str,
    ) -> Result<ContactResponse, AppError> {
        if let Some(c) = self.repo.find_by_phone(user_id, phone).await? {
            return Ok(c.into());
        }
        let now = Utc::now();
        let contact = Contact {
            id: crate::core::repository::new_id(),
            user_id: user_id.to_string(),
            name: name.to_string(),
            space_id: None,
            phone: Some(phone.to_string()),
            email: None,
            company: None,
            job_title: None,
            notes: None,
            avatar: None,
            addresses: Vec::new(),
            metadata: Default::default(),
            created_at: now,
            updated_at: now,
        };
        let saved = self.repo.create(&contact).await?;
        self.broadcast_update(&saved, EntityAction::Created);
        Ok(saved.into())
    }

    pub async fn list(&self, user_id: &str) -> Result<Vec<ContactResponse>, AppError> {
        let contacts = self.repo.find_by_user_id(user_id).await?;
        Ok(contacts.into_iter().map(Into::into).collect())
    }

    pub async fn get(&self, user_id: &str, contact_id: &str) -> Result<ContactResponse, AppError> {
        let contact = self
            .repo
            .find_by_id(contact_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Contact not found".into()))?;
        if contact.user_id != user_id {
            return Err(AppError::Forbidden("Not your contact".into()));
        }
        Ok(contact.into())
    }

    pub async fn create(
        &self,
        user_id: &str,
        req: CreateContactRequest,
    ) -> Result<ContactResponse, AppError> {
        let now = Utc::now();
        let contact = Contact {
            id: crate::core::repository::new_id(),
            user_id: user_id.to_string(),
            name: req.name,
            space_id: req.space_id,
            phone: req.phone,
            email: req.email,
            company: req.company,
            job_title: req.job_title,
            notes: req.notes,
            avatar: req.avatar,
            addresses: Vec::new(),
            metadata: req.metadata.unwrap_or_default(),
            created_at: now,
            updated_at: now,
        };
        let saved = self.repo.create(&contact).await?;
        self.broadcast_update(&saved, EntityAction::Created);
        Ok(saved.into())
    }

    pub async fn update(
        &self,
        user_id: &str,
        contact_id: &str,
        req: UpdateContactRequest,
    ) -> Result<ContactResponse, AppError> {
        let mut contact = self
            .repo
            .find_by_id(contact_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Contact not found".into()))?;
        if contact.user_id != user_id {
            return Err(AppError::Forbidden("Not your contact".into()));
        }
        if let Some(name) = req.name {
            contact.name = name;
        }
        if let Some(space_id) = req.space_id {
            contact.space_id = Some(space_id);
        }
        if let Some(phone) = req.phone {
            contact.phone = Some(phone);
        }
        if let Some(email) = req.email {
            contact.email = Some(email);
        }
        if let Some(company) = req.company {
            contact.company = Some(company);
        }
        if let Some(job_title) = req.job_title {
            contact.job_title = Some(job_title);
        }
        if let Some(notes) = req.notes {
            contact.notes = Some(notes);
        }
        if let Some(avatar) = req.avatar {
            contact.avatar = Some(avatar);
        }
        if let Some(patch) = req.metadata {
            crate::core::metadata::apply_metadata_patch(&mut contact.metadata, patch);
        }
        contact.updated_at = Utc::now();
        let saved = self.repo.update(&contact).await?;
        self.broadcast_update(&saved, EntityAction::Updated);
        Ok(saved.into())
    }

    pub async fn upsert_by_channel_address(
        &self,
        user_id: &str,
        space_id: &str,
        provider: &str,
        address: &str,
        channel_id: Option<&str>,
        default_name: &str,
    ) -> Result<Contact, AppError> {
        if let Some(c) = self
            .repo
            .find_by_channel_address(user_id, provider, address)
            .await?
        {
            return Ok(c);
        }
        let now = Utc::now();
        let contact = Contact {
            id: crate::core::repository::new_id(),
            user_id: user_id.to_string(),
            name: default_name.to_string(),
            space_id: Some(space_id.to_string()),
            phone: None,
            email: None,
            company: None,
            job_title: None,
            notes: None,
            avatar: None,
            addresses: vec![ContactAddress {
                provider: provider.to_string(),
                address: address.to_string(),
                channel_id: channel_id.map(|s| s.to_string()),
                label: None,
            }],
            metadata: std::collections::BTreeMap::new(),
            created_at: now,
            updated_at: now,
        };
        let saved = self.repo.create(&contact).await?;
        self.broadcast_update(&saved, EntityAction::Created);
        Ok(saved)
    }

    pub async fn patch_metadata(
        &self,
        contact_id: &str,
        patch: std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Contact, AppError> {
        let mut contact = self
            .repo
            .find_by_id(contact_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Contact not found".into()))?;
        crate::core::metadata::apply_metadata_patch(&mut contact.metadata, patch);
        contact.updated_at = Utc::now();
        let saved = self.repo.update(&contact).await?;
        self.broadcast_update(&saved, EntityAction::Updated);
        Ok(saved)
    }

    pub async fn delete(&self, user_id: &str, contact_id: &str) -> Result<(), AppError> {
        let contact = self
            .repo
            .find_by_id(contact_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Contact not found".into()))?;
        if contact.user_id != user_id {
            return Err(AppError::Forbidden("Not your contact".into()));
        }
        self.repo.delete(contact_id).await?;
        self.broadcast_update(&contact, EntityAction::Deleted);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_test_service() -> ContactService {
        use surrealdb::Surreal;
        use surrealdb::engine::local::Mem;
        let db = Surreal::new::<Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        ContactService::new(SurrealRepo::new(db), BroadcastService::new())
    }

    #[tokio::test]
    async fn find_or_create_by_phone_creates_new_contact() {
        let svc = make_test_service().await;

        let result = svc.find_or_create_by_phone("user-1", "+15555551234", "Alice").await.unwrap();
        assert_eq!(result.name, "Alice");
        assert_eq!(result.phone.as_deref(), Some("+15555551234"));
        assert_eq!(result.user_id, "user-1");
    }

    #[tokio::test]
    async fn find_or_create_by_phone_returns_existing() {
        let svc = make_test_service().await;

        let first = svc.find_or_create_by_phone("user-1", "+15555551234", "Alice").await.unwrap();
        let second = svc.find_or_create_by_phone("user-1", "+15555551234", "Alice2").await.unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.name, "Alice");
    }

    #[tokio::test]
    async fn upsert_by_channel_address_creates_then_returns_existing() {
        let svc = make_test_service().await;
        let first = svc
            .upsert_by_channel_address("user-1", "space-1", "telegram", "42", Some("ch-1"), "Bob")
            .await
            .unwrap();
        assert_eq!(first.space_id.as_deref(), Some("space-1"));
        assert_eq!(first.addresses.len(), 1);
        assert_eq!(first.addresses[0].provider, "telegram");
        assert_eq!(first.addresses[0].address, "42");
        assert_eq!(first.addresses[0].channel_id.as_deref(), Some("ch-1"));
        assert_eq!(first.name, "Bob");

        let second = svc
            .upsert_by_channel_address("user-1", "space-1", "telegram", "42", Some("ch-1"), "Bob2")
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.name, "Bob");
        assert_eq!(second.addresses.len(), 1, "duplicate (provider,address) must not append");

        let third = svc
            .upsert_by_channel_address("user-1", "space-1", "telegram", "99", Some("ch-1"), "Carol")
            .await
            .unwrap();
        assert_ne!(third.id, first.id);
        assert_eq!(third.name, "Carol");
    }

    #[tokio::test]
    async fn upsert_by_channel_address_is_global_per_user() {
        let svc = make_test_service().await;
        let in_a = svc
            .upsert_by_channel_address("user-1", "space-A", "telegram", "42", None, "BobA")
            .await
            .unwrap();
        let in_b = svc
            .upsert_by_channel_address("user-1", "space-B", "telegram", "42", None, "BobB")
            .await
            .unwrap();
        assert_eq!(in_a.id, in_b.id);
        assert_eq!(in_b.name, "BobA");
        assert_eq!(in_b.space_id.as_deref(), Some("space-A"));
    }

    #[tokio::test]
    async fn upsert_by_channel_address_scopes_by_user() {
        let svc = make_test_service().await;
        let one = svc
            .upsert_by_channel_address("user-1", "space-A", "telegram", "42", None, "Bob1")
            .await
            .unwrap();
        let two = svc
            .upsert_by_channel_address("user-2", "space-A", "telegram", "42", None, "Bob2")
            .await
            .unwrap();
        assert_ne!(one.id, two.id);
        assert_eq!(one.user_id, "user-1");
        assert_eq!(two.user_id, "user-2");
    }
}
