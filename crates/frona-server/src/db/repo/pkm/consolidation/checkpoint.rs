use crate::db::repo::pkm::*;

impl PkmRepo {

    /// The user's newest consolidation pass, finished or not. Ids are UUIDv7, so newest
    /// is a plain `ORDER BY id DESC` - no separate clock, and no `updated_at` tiebreak.
    pub async fn latest_consolidation_record(
        &self,
        user_id: &str,
    ) -> Result<Option<KnowledgeConsolidationRecord>, AppError> {
        let mut q = self
            .db
            .query(format!(
                "{SELECT} FROM knowledge_consolidation_record
                 WHERE user_id = $uid ORDER BY id DESC LIMIT 1"
            ))
            .bind(("uid", user_id.to_string()))
            .await
            .map_err(|e| Self::err("latest_record", e))?;
        let rows: Vec<KnowledgeConsolidationRecord> =
            q.take(0).map_err(|e| Self::err("latest_record_take", e))?;
        Ok(rows.into_iter().next())
    }

    /// Every user with a pass still in flight - the sweep's other work source, alongside
    /// the chats needing consolidation. Checked even when no chat is eligible: a record
    /// that failed at author has nothing left to mine, and would otherwise never resume.
    pub async fn users_with_open_consolidation(&self) -> Result<Vec<String>, AppError> {
        let mut q = self
            .db
            .query(
                "SELECT VALUE user_id FROM knowledge_consolidation_record
                 WHERE state.Done = NONE AND state.Failed = NONE ORDER BY id DESC",
            )
            .await
            .map_err(|e| Self::err("open_records", e))?;
        let rows: Vec<String> = q.take(0).map_err(|e| Self::err("open_records_take", e))?;
        let mut seen = std::collections::HashSet::new();
        Ok(rows.into_iter().filter(|u| seen.insert(u.clone())).collect())
    }

    /// Write the record - the checkpoint. Upserts by id, so the driver's between-stage
    /// write and a stage's own mid-stage write are the same call.
    pub async fn save_consolidation_record(
        &self,
        record: &KnowledgeConsolidationRecord,
    ) -> Result<(), AppError> {
        let mut record = record.clone();
        record.updated_at = Utc::now();
        self.db
            .query("UPSERT type::record('knowledge_consolidation_record', $id) CONTENT $rec")
            .bind(("id", record.id.clone()))
            .bind(("rec", record))
            .await
            .map_err(|e| Self::err("save_record", e))?
            .check()
            .map_err(|e| Self::err("save_record_check", e))?;
        Ok(())
    }

    /// Cleanup's terminal database boundary: remove every working overlay row and mark
    /// the pass Done together. A crash can therefore only resume Cleanup with its rows
    /// intact, or observe a completed record with no temporary state.
    pub async fn complete_consolidation(
        &self,
        record: &KnowledgeConsolidationRecord,
    ) -> Result<(), AppError> {
        let tx = self.db.clone().begin().await
            .map_err(|e| Self::err("complete_consolidation_begin", e))?;
        if let Err(e) = tx.query(
            "DELETE knowledge_consolidation_entity WHERE consolidation_id = $cid",
        )
        .bind(("cid", record.consolidation_id.clone()))
        .await
        .and_then(|response| response.check()) {
            let _ = tx.cancel().await;
            return Err(Self::err("complete_consolidation_entities", e));
        }
        let mut record = record.clone();
        record.updated_at = Utc::now();
        if let Err(e) = tx.query(
            "UPSERT type::record('knowledge_consolidation_record', $id) CONTENT $record",
        )
        .bind(("id", record.id.clone()))
        .bind(("record", record))
        .await
        .and_then(|response| response.check()) {
            let _ = tx.cancel().await;
            return Err(Self::err("complete_consolidation_record", e));
        }
        tx.commit().await.map_err(|e| Self::err("complete_consolidation_commit", e))?;
        Ok(())
    }

    /// Drop a record outright - the give-up path, once a stage has burned its attempts.
    pub async fn delete_consolidation_record(&self, id: &str) -> Result<(), AppError> {
        self.db
            .query("DELETE type::record('knowledge_consolidation_record', $id)")
            .bind(("id", id.to_string()))
            .await
            .map_err(|e| Self::err("delete_record", e))?;
        Ok(())
    }

    /// Retention: keep the newest `keep` finished passes for a user, drop the rest.
    /// Unfinished records are never touched. Returns how many were removed.
    pub async fn prune_consolidation_records(
        &self,
        user_id: &str,
        keep: usize,
    ) -> Result<usize, AppError> {
        let mut q = self
            .db
            .query(
                "SELECT VALUE meta::id(id) FROM knowledge_consolidation_record
                 WHERE user_id = $uid AND state.Done != NONE ORDER BY id DESC",
            )
            .bind(("uid", user_id.to_string()))
            .await
            .map_err(|e| Self::err("prune_records", e))?;
        let ids: Vec<String> = q.take(0).map_err(|e| Self::err("prune_records_take", e))?;
        let stale: Vec<String> = ids.into_iter().skip(keep).collect();
        for id in &stale {
            self.delete_consolidation_record(id).await?;
        }
        Ok(stale.len())
    }


}
