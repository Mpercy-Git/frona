use async_trait::async_trait;
use chrono::Utc;
use surrealdb::Surreal;
use surrealdb::engine::local::Db;
use tracing::info;

use crate::auth::models::{ADMINS_GROUP, UserGroup};
use crate::core::error::AppError;
use crate::core::repository::{Repository, new_id};
use crate::db::repo::generic::SurrealRepo;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
pub trait UserGroupRepository: Repository<UserGroup> + Send + Sync {
    async fn find_by_name(&self, name: &str) -> Result<Option<UserGroup>, AppError>;
    async fn list_all(&self) -> Result<Vec<UserGroup>, AppError>;
}

#[async_trait]
impl UserGroupRepository for SurrealRepo<UserGroup> {
    async fn find_by_name(&self, name: &str) -> Result<Option<UserGroup>, AppError> {
        let mut result = self
            .db()
            .query(format!(
                "{SELECT_CLAUSE} FROM user_group WHERE name = $name LIMIT 1"
            ))
            .bind(("name", name.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let group: Option<UserGroup> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(group)
    }

    async fn list_all(&self) -> Result<Vec<UserGroup>, AppError> {
        let mut result = self
            .db()
            .query(format!("{SELECT_CLAUSE} FROM user_group ORDER BY name ASC"))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let groups: Vec<UserGroup> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(groups)
    }
}

#[derive(Clone)]
pub struct UserGroupService {
    repo: SurrealRepo<UserGroup>,
    db: Surreal<Db>,
}

impl UserGroupService {
    pub fn new(db: Surreal<Db>) -> Self {
        let repo = SurrealRepo::<UserGroup>::new(db.clone());
        Self { repo, db }
    }

    /// Validates a group name. Matches `^[a-z][a-z0-9_-]{0,31}$`. Same grammar as username.
    pub fn validate_name(name: &str) -> Result<(), AppError> {
        if name.is_empty() || name.len() > 32 {
            return Err(AppError::Validation(
                "Group name must be 1-32 characters".into(),
            ));
        }
        if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(AppError::Validation(
                "Group name must start with a lowercase letter".into(),
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(AppError::Validation(
                "Group name may only contain lowercase letters, digits, hyphens, and underscores"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Idempotent. Ensures the built-in `admins` group exists with `built_in: true`.
    /// Repairs `built_in: false` if a row was created manually without it.
    pub async fn seed_built_in(&self) -> Result<(), AppError> {
        Self::validate_name(ADMINS_GROUP)?;
        let now = Utc::now();
        match self.repo.find_by_name(ADMINS_GROUP).await? {
            None => {
                let group = UserGroup {
                    id: new_id(),
                    name: ADMINS_GROUP.into(),
                    description: "Built-in administrators.".into(),
                    built_in: true,
                    created_at: now,
                    updated_at: now,
                };
                self.repo.create(&group).await?;
                info!("Seeded built-in user group: admins");
            }
            Some(existing) if !existing.built_in => {
                let mut repaired = existing.clone();
                repaired.built_in = true;
                repaired.updated_at = now;
                self.repo.update(&repaired).await?;
                info!("Repaired built_in flag on user group: admins");
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn find_by_name(&self, name: &str) -> Result<Option<UserGroup>, AppError> {
        self.repo.find_by_name(name).await
    }

    pub async fn list_all(&self) -> Result<Vec<UserGroup>, AppError> {
        self.repo.list_all().await
    }

    /// Validate that every name in the slice is a valid slug AND exists in the registry.
    pub async fn validate_assignment(&self, names: &[String]) -> Result<(), AppError> {
        for name in names {
            Self::validate_name(name)?;
            if self.repo.find_by_name(name).await?.is_none() {
                return Err(AppError::Validation(format!("unknown group: {name}")));
            }
        }
        Ok(())
    }

    /// Create a new (non-built-in) user group. Slug-validated, rejects duplicates.
    pub async fn create(&self, name: &str, description: &str) -> Result<UserGroup, AppError> {
        Self::validate_name(name)?;
        if self.repo.find_by_name(name).await?.is_some() {
            return Err(AppError::Validation(format!("group already exists: {name}")));
        }
        let now = Utc::now();
        let group = UserGroup {
            id: new_id(),
            name: name.into(),
            description: description.into(),
            built_in: false,
            created_at: now,
            updated_at: now,
        };
        self.repo.create(&group).await
    }

    /// Delete a group. Refuses if `built_in == true`. Strips the name from all users' `groups`.
    pub async fn delete(&self, name: &str) -> Result<(), AppError> {
        let existing = self
            .repo
            .find_by_name(name)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("group not found: {name}")))?;
        if existing.built_in {
            return Err(AppError::Conflict(format!(
                "cannot delete built-in group: {name}"
            )));
        }
        self.db
            .query("UPDATE user SET groups = array::filter(groups, |$g| $g != $name)")
            .bind(("name", name.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        self.repo.delete(&existing.id).await
    }

    /// Rename a group. Refuses if `built_in == true`. Sweeps `User.groups` to rename.
    pub async fn rename(&self, old: &str, new: &str) -> Result<UserGroup, AppError> {
        Self::validate_name(new)?;
        let existing = self
            .repo
            .find_by_name(old)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("group not found: {old}")))?;
        if existing.built_in {
            return Err(AppError::Conflict(format!(
                "cannot rename built-in group: {old}"
            )));
        }
        if self.repo.find_by_name(new).await?.is_some() {
            return Err(AppError::Validation(format!(
                "group already exists: {new}"
            )));
        }
        // Sweep membership in users first so any concurrent reads don't see a dangling name.
        self.db
            .query(
                "UPDATE user SET groups = array::map(groups, |$g| IF $g == $old THEN $new ELSE $g END)",
            )
            .bind(("old", old.to_string()))
            .bind(("new", new.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut updated = existing.clone();
        updated.name = new.into();
        updated.updated_at = Utc::now();
        self.repo.update(&updated).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_accepts_valid_slugs() {
        assert!(UserGroupService::validate_name("admins").is_ok());
        assert!(UserGroupService::validate_name("ops-team").is_ok());
        assert!(UserGroupService::validate_name("a").is_ok());
        assert!(UserGroupService::validate_name("dev_lead").is_ok());
    }

    #[test]
    fn validate_name_rejects_empty() {
        assert!(UserGroupService::validate_name("").is_err());
    }

    #[test]
    fn validate_name_rejects_uppercase() {
        assert!(UserGroupService::validate_name("Admins").is_err());
    }

    #[test]
    fn validate_name_rejects_leading_digit() {
        assert!(UserGroupService::validate_name("1admins").is_err());
    }

    #[test]
    fn validate_name_rejects_too_long() {
        let long = "a".repeat(33);
        assert!(UserGroupService::validate_name(&long).is_err());
    }

    #[test]
    fn validate_name_rejects_special_chars() {
        assert!(UserGroupService::validate_name("admins!").is_err());
        assert!(UserGroupService::validate_name("admins/ops").is_err());
    }
}
