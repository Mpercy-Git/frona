use chrono::{DateTime, Utc};
use tracing::info;

use crate::core::config::CacheConfig;
use crate::core::error::AppError;
use crate::core::repository::Repository;
use crate::db::repo::users::SurrealUserRepo;

use super::UserRepository;
use super::models::{ADMINS_GROUP, User};
use crate::core::user_config::{UserConfig, UserConfigPatch};

#[derive(Clone)]
pub struct UserService {
    repo: SurrealUserRepo,
    cache: moka::future::Cache<String, User>,
    /// Separate from the `User` cache: config lives in its own table and is read on
    /// every memory tool/sync call, so it gets its own cache-through.
    user_config_cache: moka::future::Cache<String, UserConfig>,
}

impl UserService {
    pub fn new(repo: SurrealUserRepo, cache_config: &CacheConfig) -> Self {
        let cache = moka::future::Cache::builder()
            .max_capacity(cache_config.entity_max_capacity)
            .time_to_live(std::time::Duration::from_secs(cache_config.entity_ttl_secs))
            .build();
        let user_config_cache = moka::future::Cache::builder()
            .max_capacity(cache_config.entity_max_capacity)
            .time_to_live(std::time::Duration::from_secs(cache_config.entity_ttl_secs))
            .build();
        Self {
            repo,
            cache,
            user_config_cache,
        }
    }

    /// The user's config, defaulted when they've never customized anything (no row
    /// → `UserConfig::default()`, whose `updated_at` is the epoch sentinel). The
    /// returned `updated_at` is the compare-and-swap token for `update_user_config`.
    pub async fn user_config(&self, user_id: &str) -> Result<UserConfig, AppError> {
        if let Some(config) = self.user_config_cache.get(user_id).await {
            return Ok(config);
        }
        let config = self.repo.user_config(user_id).await?.unwrap_or_default();
        self.user_config_cache
            .insert(user_id.to_string(), config.clone())
            .await;
        Ok(config)
    }

    /// Apply a config patch under an `updated_at` compare-and-swap. `expected` is the
    /// `updated_at` the caller read via [`user_config`]; a concurrent modification
    /// yields `AppError::Conflict`. Domain validation (e.g. of the memory directory)
    /// belongs in the calling domain service, not here.
    pub async fn update_user_config(
        &self,
        user_id: &str,
        expected: DateTime<Utc>,
        patch: UserConfigPatch,
    ) -> Result<UserConfig, AppError> {
        let saved = self
            .repo
            .patch_user_config(user_id, expected, patch, Utc::now())
            .await?;
        self.user_config_cache.invalidate(user_id).await;
        Ok(saved)
    }

    pub async fn find_by_id(&self, id: &str) -> Result<Option<User>, AppError> {
        if let Some(user) = self.cache.get(id).await {
            return Ok(Some(user));
        }
        let result = self.repo.find_by_id(id).await?;
        if let Some(ref user) = result {
            self.cache.insert(id.to_string(), user.clone()).await;
        }
        Ok(result)
    }

    pub async fn find_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        self.repo.find_by_email(email).await
    }

    pub async fn handle_of(&self, user_id: &str) -> Result<crate::core::Handle, AppError> {
        self.find_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("User {user_id} not found")))
            .map(|u| u.handle)
    }

    pub async fn find_by_handle(
        &self,
        handle: &crate::core::Handle,
    ) -> Result<Option<User>, AppError> {
        self.repo.find_by_handle(handle).await
    }

    pub async fn create(&self, user: &User) -> Result<User, AppError> {
        self.repo.create(user).await
    }

    pub async fn update(&self, user: &User) -> Result<User, AppError> {
        let result = self.repo.update(user).await?;
        self.cache.invalidate(&user.id).await;
        Ok(result)
    }

    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        self.cache.invalidate(id).await;
        self.user_config_cache.invalidate(id).await;
        self.repo.delete(id).await
    }

    pub async fn has_users(&self) -> Result<bool, AppError> {
        self.repo.has_users().await
    }

    pub async fn list_all(&self, include_deactivated: bool) -> Result<Vec<User>, AppError> {
        self.repo.list_all(include_deactivated).await
    }

    /// If no active admin exists, promote the oldest active user.
    pub async fn ensure_admin_invariant(&self) -> Result<(), AppError> {
        if self.repo.find_any_active_admin().await?.is_some() {
            return Ok(());
        }
        let Some(mut target) = self.repo.find_oldest_active().await? else {
            return Ok(());
        };
        if !target.groups.iter().any(|g| g == ADMINS_GROUP) {
            target.groups.push(ADMINS_GROUP.into());
        }
        target.updated_at = Utc::now();
        info!(
            user_id = %target.id,
            handle = %target.handle,
            "Promoted oldest active user to admins (invariant repair)"
        );
        self.cache.invalidate(&target.id).await;
        self.repo.update(&target).await?;
        Ok(())
    }

    pub async fn deactivate(&self, id: &str) -> Result<User, AppError> {
        let mut user = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user not found: {id}")))?;
        if user.deactivated_at.is_none() {
            let now = Utc::now();
            user.deactivated_at = Some(now);
            user.updated_at = now;
            self.cache.invalidate(id).await;
            self.repo.update(&user).await?;
        }
        Ok(user)
    }

    pub async fn reactivate(&self, id: &str) -> Result<User, AppError> {
        let mut user = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user not found: {id}")))?;
        if user.deactivated_at.is_some() {
            user.deactivated_at = None;
            user.updated_at = Utc::now();
            self.cache.invalidate(id).await;
            self.repo.update(&user).await?;
        }
        Ok(user)
    }
}
