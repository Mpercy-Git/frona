use crate::db::repo::pkm::*;

impl PkmRepo {
    /// Recover a fatal downstream checkpoint from durable raw contributions, or mark it
    /// terminally Failed once the configured recovery budget is exhausted.
    pub async fn recover_or_fail_consolidation(
        &self,
        record: &KnowledgeConsolidationRecord,
        error: &str,
        cap: u32,
    ) -> Result<KnowledgeConsolidationRecord, AppError> {
        let mut response = self.db.query(
            "SELECT *, meta::id(id) AS consolidation_entity_id FROM knowledge_consolidation_entity
             WHERE consolidation_id = $cid AND user_id = $uid ORDER BY path",
        )
        .bind(("cid", record.consolidation_id.clone()))
        .bind(("uid", record.user_id.clone()))
        .await
        .map_err(|e| Self::err("recover_consolidation_rows", e))?;
        let rows: Vec<KnowledgeConsolidationEntity> = response.take(0)
            .map_err(|e| Self::err("recover_consolidation_rows_take", e))?;
        let affected_paths: Vec<String> = rows.iter().take(20).map(|row| row.path.clone()).collect();
        let mut next = record.clone();
        next.restart_count = next.restart_count.saturating_add(1);
        next.attempts = 0;
        next.next_attempt_at = Utc::now();
        next.updated_at = Utc::now();
        let diagnostic = crate::memory::pkm::ConsolidationFailure {
            stage: record.state.label().to_string(),
            error: error.chars().take(2_000).collect(),
            affected_paths,
            affected_count: rows.len(),
            failed_at: Utc::now(),
        };
        let has_raw_contributions = rows.iter().any(|row| !row.contributions.is_empty());
        let terminal = !has_raw_contributions || cap == 0 || next.restart_count >= cap;
        let mut pending_rows = Vec::new();
        if !terminal {
            for mut row in rows {
                if row.contributions.is_empty() {
                    continue;
                }
                let consolidation_entity_id = row.consolidation_entity_id.clone();
                let entity_id = row.entity_id.clone();
                let identity_evidence = row.identity_evidence.clone();
                let source_memory_ids = row.source_memory_ids.clone();
                let published = self.entity_by_path(&row.user_id, &row.path).await?;
                row = KnowledgeConsolidationEntity::pending(
                    &row.consolidation_id, &row.user_id, &row.path, row.category,
                    row.contributions,
                    source_memory_ids.iter().cloned().collect(),
                );
                row.consolidation_entity_id = consolidation_entity_id;
                row.entity_id = entity_id;
                if let Some(published) = published {
                    row.apply_committed(published);
                    row.lifecycle = ConsolidationEntityLifecycle::Pending;
                    row.searchable = true;
                }
                row.source_memory_ids.extend(source_memory_ids);
                row.source_memory_ids.sort();
                row.source_memory_ids.dedup();
                for evidence in identity_evidence {
                    if !row.identity_evidence.contains(&evidence) {
                        row.identity_evidence.push(evidence);
                    }
                }
                row.checkpoint_revision = 0;
                row.rederive_search();
                pending_rows.push(row);
            }
            next.state = crate::memory::pkm::ConsolidationStageState::Classify(
                crate::memory::pkm::ConsolidationWorkState::default(),
            );
            next.failure = None;
        } else {
            next.state = crate::memory::pkm::ConsolidationStageState::Failed;
            next.failure = Some(diagnostic);
        }

        let tx = self.db.clone().begin().await
            .map_err(|e| Self::err("recover_consolidation_begin", e))?;
        if terminal {
            if let Err(e) = tx.query(
                "DELETE knowledge_consolidation_entity WHERE consolidation_id = $cid",
            ).bind(("cid", record.consolidation_id.clone())).await
                .and_then(|response| response.check()) {
                let _ = tx.cancel().await;
                return Err(Self::err("recover_consolidation_clear", e));
            }
        } else {
            let retained: std::collections::HashSet<String> =
                pending_rows.iter().map(|row| row.consolidation_entity_id.clone()).collect();
            if let Err(e) = tx.query(
                "DELETE knowledge_consolidation_entity
                 WHERE consolidation_id = $cid AND meta::id(id) NOT IN $retained",
            )
            .bind(("cid", record.consolidation_id.clone()))
            .bind(("retained", retained.into_iter().collect::<Vec<_>>()))
            .await
            .and_then(|response| response.check()) {
                let _ = tx.cancel().await;
                return Err(Self::err("recover_consolidation_prune", e));
            }
            for row in pending_rows {
                if let Err(e) = tx.query(
                    "UPSERT type::record('knowledge_consolidation_entity', $id) CONTENT $row",
                ).bind(("id", row.consolidation_entity_id.clone())).bind(("row", row)).await
                    .and_then(|response| response.check()) {
                    let _ = tx.cancel().await;
                    return Err(Self::err("recover_consolidation_entity", e));
                }
            }
        }
        if let Err(e) = tx.query(
            "UPSERT type::record('knowledge_consolidation_record', $id) CONTENT $record",
        ).bind(("id", next.id.clone())).bind(("record", next.clone())).await
            .and_then(|response| response.check()) {
            let _ = tx.cancel().await;
            return Err(Self::err("recover_consolidation_record", e));
        }
        tx.commit().await.map_err(|e| Self::err("recover_consolidation_commit", e))?;
        Ok(next)
    }

}
