use std::collections::BTreeMap;

use crate::chat::broadcast::{BroadcastService, EntityAction};
use crate::core::error::AppError;
use crate::core::metadata::apply_metadata_patch;
use crate::core::repository::Repository;
use crate::db::repo::spaces::SurrealSpaceRepo;

use super::models::Space;
use super::models::{CreateSpaceRequest, SpaceResponse, UpdateSpaceRequest};
use super::repository::SpaceRepository;

#[derive(Clone)]
pub struct SpaceService {
    repo: SurrealSpaceRepo,
    broadcast: BroadcastService,
}

impl SpaceService {
    pub fn new(repo: SurrealSpaceRepo, broadcast: BroadcastService) -> Self {
        Self { repo, broadcast }
    }

    fn broadcast_update(&self, space: &Space, action: EntityAction) {
        self.broadcast.broadcast_entity_updated(
            &space.user_id,
            "space",
            &space.id,
            action,
            Some(space.id.clone()),
            None,
        );
    }

    pub async fn create(
        &self,
        user_id: &str,
        req: CreateSpaceRequest,
    ) -> Result<SpaceResponse, AppError> {
        let now = chrono::Utc::now();
        let space = Space {
            id: crate::core::repository::new_id(),
            user_id: user_id.to_string(),
            name: req.name,
            metadata: req.metadata.unwrap_or_default(),
            created_at: now,
            updated_at: now,
        };

        let space = self.repo.create(&space).await?;
        self.broadcast_update(&space, EntityAction::Created);
        Ok(space.into())
    }

    pub async fn get(&self, user_id: &str, space_id: &str) -> Result<Space, AppError> {
        let space = self
            .repo
            .find_by_id(space_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Space not found".into()))?;
        if space.user_id != user_id {
            return Err(AppError::Forbidden("Not your space".into()));
        }
        Ok(space)
    }

    pub async fn find_by_id(&self, space_id: &str) -> Result<Option<Space>, AppError> {
        self.repo.find_by_id(space_id).await
    }

    pub async fn list(&self, user_id: &str) -> Result<Vec<SpaceResponse>, AppError> {
        let spaces = self.repo.find_by_user_id(user_id).await?;
        Ok(spaces.into_iter().map(Into::into).collect())
    }

    pub async fn update(
        &self,
        user_id: &str,
        space_id: &str,
        req: UpdateSpaceRequest,
    ) -> Result<SpaceResponse, AppError> {
        let mut space = self
            .repo
            .find_by_id(space_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Space not found".into()))?;

        if space.user_id != user_id {
            return Err(AppError::Forbidden("Not your space".into()));
        }

        if let Some(name) = req.name {
            space.name = name;
        }
        if let Some(patch) = req.metadata {
            apply_metadata_patch(&mut space.metadata, patch);
        }
        space.updated_at = chrono::Utc::now();

        let space = self.repo.update(&space).await?;
        self.broadcast_update(&space, EntityAction::Updated);
        Ok(space.into())
    }

    pub async fn patch_metadata(
        &self,
        space_id: &str,
        patch: BTreeMap<String, serde_json::Value>,
    ) -> Result<Space, AppError> {
        let mut space = self
            .repo
            .find_by_id(space_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Space not found".into()))?;
        apply_metadata_patch(&mut space.metadata, patch);
        space.updated_at = chrono::Utc::now();
        let saved = self.repo.update(&space).await?;
        self.broadcast_update(&saved, EntityAction::Updated);
        Ok(saved)
    }

    pub async fn delete(&self, user_id: &str, space_id: &str) -> Result<(), AppError> {
        let space = self
            .repo
            .find_by_id(space_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Space not found".into()))?;

        if space.user_id != user_id {
            return Err(AppError::Forbidden("Not your space".into()));
        }

        self.repo.delete(space_id).await?;
        self.broadcast_update(&space, EntityAction::Deleted);
        Ok(())
    }
}
