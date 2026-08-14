use crate::db::repo::pkm::*;

impl PkmRepo {
    pub(crate) async fn commit_consolidation_transition(
        &self,
        rows: &[KnowledgeConsolidationEntity],
        checkpoint: &KnowledgeConsolidationRecord,
    ) -> Result<(), AppError> {
        for row in rows {
            if row.consolidation_id != checkpoint.consolidation_id
                || row.user_id != checkpoint.user_id
            {
                return Err(AppError::Database(
                    "pkm/consolidation_transition: row and checkpoint scope mismatch".into(),
                ));
            }
            row.validate()?;
        }

        let tx = self.db.clone().begin().await
            .map_err(|e| Self::err("consolidation_transition_begin", e))?;
        for row in rows {
            if let Err(error) = tx
                .query("UPSERT type::record('knowledge_consolidation_entity', $id) CONTENT $row")
                .bind(("id", row.consolidation_entity_id.clone()))
                .bind(("row", row.clone()))
                .await
                .and_then(|response| response.check())
            {
                let _ = tx.cancel().await;
                return Err(Self::err("consolidation_transition_entity", error));
            }
        }
        let mut checkpoint = checkpoint.clone();
        checkpoint.updated_at = Utc::now();
        if let Err(error) = tx
            .query("UPSERT type::record('knowledge_consolidation_record', $id) CONTENT $record")
            .bind(("id", checkpoint.id.clone()))
            .bind(("record", checkpoint))
            .await
            .and_then(|response| response.check())
        {
            let _ = tx.cancel().await;
            return Err(Self::err("consolidation_transition_checkpoint", error));
        }
        tx.commit().await.map_err(|e| Self::err("consolidation_transition_commit", e))?;
        Ok(())
    }
}
