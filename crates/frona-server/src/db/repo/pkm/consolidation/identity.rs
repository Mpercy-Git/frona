use crate::db::repo::pkm::*;
impl PkmRepo {
    pub(crate) async fn commit_entity_identity_merge(
        &self,
        canonical: &KnowledgeConsolidationEntity,
        losing: &KnowledgeConsolidationEntity,
        checkpoint: &KnowledgeConsolidationRecord,
    ) -> Result<(), AppError> {
        canonical.validate()?;
        losing.validate()?;
        if canonical.consolidation_id != checkpoint.consolidation_id
            || canonical.user_id != checkpoint.user_id
            || losing.consolidation_id != checkpoint.consolidation_id
            || losing.user_id != checkpoint.user_id
            || canonical.path == losing.path
            || losing.lifecycle != ConsolidationEntityLifecycle::Coalesced
            || losing.canonical_path.as_deref() != Some(canonical.path.as_str())
            || checkpoint.state.revision() != Some(canonical.checkpoint_revision)
            || losing.checkpoint_revision != canonical.checkpoint_revision
        {
            return Err(AppError::Database(
                "pkm/entity_identity_merge: transition scope or identity mismatch".into(),
            ));
        }
        let tx = self
            .db
            .clone()
            .begin()
            .await
            .map_err(|e| Self::err("entity_identity_merge_begin", e))?;
        if let Err(e) = tx
            .query("UPSERT type::record('knowledge_consolidation_entity', $id) CONTENT $row")
            .bind(("id", canonical.consolidation_entity_id.clone()))
            .bind(("row", canonical.clone()))
            .await
            .and_then(|response| response.check())
        {
            let _ = tx.cancel().await;
            return Err(Self::err("entity_identity_merge_canonical", e));
        }
        if let Err(e) = tx
            .query("UPSERT type::record('knowledge_consolidation_entity', $id) CONTENT $row")
            .bind(("id", losing.consolidation_entity_id.clone()))
            .bind(("row", losing.clone()))
            .await
            .and_then(|response| response.check())
        {
            let _ = tx.cancel().await;
            return Err(Self::err("entity_identity_merge_loser", e));
        }
        let mut checkpoint = checkpoint.clone();
        checkpoint.updated_at = Utc::now();
        if let Err(e) = tx
            .query("UPSERT type::record('knowledge_consolidation_record', $id) CONTENT $record")
            .bind(("id", checkpoint.id.clone()))
            .bind(("record", checkpoint))
            .await
            .and_then(|response| response.check())
        {
            let _ = tx.cancel().await;
            return Err(Self::err("entity_identity_merge_checkpoint", e));
        }
        tx.commit()
            .await
            .map_err(|e| Self::err("entity_identity_merge_commit", e))?;
        Ok(())
    }
}
