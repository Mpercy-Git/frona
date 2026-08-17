use crate::core::error::AppError;
use crate::db::repo::pkm::PkmConsolidationRepo;
use crate::memory::pkm::model::{EntityHit, KnowledgeConsolidationEntity};

use super::draft::DraftEntityLookup;
use super::{EntityDraft, EntitySnapshot, EntityTransition};

#[derive(Clone)]
pub(crate) struct EntityViewManager {
    durable: PkmConsolidationRepo,
}

impl EntityViewManager {
    pub(crate) fn new(durable: PkmConsolidationRepo) -> Self {
        Self { durable }
    }

    pub(crate) fn consolidation_id(&self) -> &str {
        self.durable.consolidation_id()
    }

    pub(crate) async fn working_entity(
        &self,
        path: &str,
    ) -> Result<Option<KnowledgeConsolidationEntity>, AppError> {
        self.durable.working_entity(path).await
    }

    pub(crate) async fn redirects(
        &self,
    ) -> Result<std::collections::BTreeMap<String, String>, AppError> {
        self.durable.redirects().await
    }

    pub(crate) async fn entity_by_path(
        &self,
        path: &str,
    ) -> Result<Option<KnowledgeConsolidationEntity>, AppError> {
        self.durable.entity_by_path(path).await
    }

    pub(crate) async fn entity_by_path_with(
        &self,
        draft: &EntityDraft,
        path: &str,
    ) -> Result<Option<KnowledgeConsolidationEntity>, AppError> {
        match draft.resolve(path)? {
            DraftEntityLookup::Found(row) => Ok(Some(*row)),
            DraftEntityLookup::Missing => Ok(None),
            DraftEntityLookup::DurablePath(path) => self.entity_by_path(&path).await,
        }
    }

    pub(crate) async fn search_entities(&self, query: &str) -> Result<Vec<EntityHit>, AppError> {
        self.durable.search_entities(query).await
    }

    pub(crate) async fn resolution_candidates(
        &self,
        names: &[String],
        name_tokens: &[String],
        assertions: &[String],
        kinds: &[String],
        query_text: &str,
        limit: i64,
    ) -> Result<Vec<EntityHit>, AppError> {
        self.durable.resolution_candidates(
            names, name_tokens, assertions, kinds, query_text, limit,
        ).await
    }

    pub(crate) async fn snapshot_with(
        &self,
        draft: &EntityDraft,
    ) -> Result<EntitySnapshot, AppError> {
        Ok(EntitySnapshot::new(
            self.durable.list_entities().await?,
            draft,
        ))
    }

    pub(crate) async fn list_entities(
        &self,
    ) -> Result<Vec<KnowledgeConsolidationEntity>, AppError> {
        self.durable.list_entities().await
    }

    pub(crate) async fn rows(&self) -> Result<Vec<KnowledgeConsolidationEntity>, AppError> {
        self.durable.rows().await
    }

    pub(crate) async fn commit_transition(
        &self,
        transition: &EntityTransition,
    ) -> Result<(), AppError> {
        if transition.checkpoint.consolidation_id != self.durable.consolidation_id()
            || transition.checkpoint.user_id != self.durable.user_id()
        {
            return Err(AppError::Database(
                "entity transition: checkpoint scope mismatch".into(),
            ));
        }
        let revision = transition.checkpoint.state.revision();
        if !transition.rows.is_empty() && revision.is_none() {
            return Err(AppError::Database(
                "entity transition: entity rows require a revisioned stage".into(),
            ));
        }
        let mut paths = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();
        for row in &transition.rows {
            if row.consolidation_id != self.durable.consolidation_id()
                || row.user_id != self.durable.user_id()
            {
                return Err(AppError::Database(
                    "entity transition: entity scope mismatch".into(),
                ));
            }
            if revision != Some(row.checkpoint_revision) {
                return Err(AppError::Database(
                    "entity transition: row revision mismatch".into(),
                ));
            }
            if !paths.insert(row.path.as_str())
                || !ids.insert(row.consolidation_entity_id.as_str())
            {
                return Err(AppError::Database(
                    "entity transition: duplicate entity row".into(),
                ));
            }
        }
        self.durable
            .commit_transition(&transition.rows, &transition.checkpoint)
            .await
    }
}
