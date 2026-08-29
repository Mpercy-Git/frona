use crate::db::repo::pkm::*;

impl PkmRepo {
    pub(crate) async fn commit_reconciliation(
        &self,
        write: &ReconcileCommit,
        checkpoint: &KnowledgeConsolidationRecord,
    ) -> Result<(), AppError> {
        if write.entity.as_ref().is_some_and(|entity| {
            entity.consolidation_id != checkpoint.consolidation_id
                || entity.user_id != checkpoint.user_id
        }) {
            return Err(AppError::Database(
                "pkm/reconcile_commit: entity and checkpoint scope mismatch".into(),
            ));
        }
        if let Some(entity) = &write.entity {
            entity.validate()?;
            if checkpoint.state.revision() != Some(entity.checkpoint_revision) {
                return Err(AppError::Database(
                    "pkm/reconcile_commit: entity and checkpoint revision mismatch".into(),
                ));
            }
        }

        let tx = self
            .db
            .clone()
            .begin()
            .await
            .map_err(|e| Self::err("reconcile_commit_begin", e))?;
        macro_rules! tx_try {
            ($expr:expr, $ctx:literal) => {
                match $expr {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = tx.cancel().await;
                        return Err(Self::err($ctx, error));
                    }
                }
            };
        }

        let now = Utc::now();
        let mut dirty_memories = std::collections::BTreeSet::new();
        for relation in &write.memory_relations {
            let mut response = tx_try!(
                tx.query(format!(
                    "{SELECT} FROM knowledge_memory
                     WHERE user_id = $uid AND id = type::record('knowledge_memory', $id)
                     LIMIT 1"
                ))
                .bind(("uid", checkpoint.user_id.clone()))
                .bind(("id", relation.subordinate_id.clone()))
                .await,
                "reconcile_commit_relation_read"
            );
            let mut memories: Vec<KnowledgeMemory> =
                tx_try!(response.take(0), "reconcile_commit_relation_take");
            let Some(mut memory) = memories.pop() else {
                continue;
            };
            let to = RecordId::new("knowledge_memory", relation.to_id.clone());
            if !memory
                .relations
                .iter()
                .any(|held| held.relation == relation.relation && held.to == to)
            {
                memory.relations.push(MemoryRelation {
                    relation: relation.relation,
                    to,
                    note: relation.note.clone(),
                });
                tx_try!(
                    tx.query(
                        "UPDATE type::record('knowledge_memory', $id) SET relations = $relations"
                    )
                    .bind(("id", relation.subordinate_id.clone()))
                    .bind(("relations", memory.relations))
                    .await
                    .and_then(|response| response.check()),
                    "reconcile_commit_relation_write"
                );
            }
            dirty_memories.insert(relation.subordinate_id.clone());
        }

        for outdated in &write.outdated_memories {
            tx_try!(
                tx.query(
                    "UPDATE type::record('knowledge_memory', $id)
                     SET disposition = $disposition, ended_at = $now, comment = $reason",
                )
                .bind(("id", outdated.memory_id.clone()))
                .bind(("disposition", Disposition::Outdated))
                .bind(("now", now))
                .bind((
                    "reason",
                    (!outdated.reason.is_empty()).then(|| outdated.reason.clone())
                ))
                .await
                .and_then(|response| response.check()),
                "reconcile_commit_outdated"
            );
            dirty_memories.insert(outdated.memory_id.clone());
        }

        for memory_id in dirty_memories {
            tx_try!(
                tx.query(
                    "UPDATE knowledge_entity SET updated_at = $now WHERE user_id = $uid AND path IN
                     (SELECT VALUE entity_path FROM knowledge_entity_source
                      WHERE user_id = $uid AND memory_id = $mid)",
                )
                .bind(("uid", checkpoint.user_id.clone()))
                .bind(("mid", memory_id))
                .bind(("now", now))
                .await
                .and_then(|response| response.check()),
                "reconcile_commit_dirty_pages"
            );
        }

        for source in &write.entity_link_sources {
            tx_try!(
                tx.query(
                    "UPDATE knowledge_entity_link SET source_memory_ids = $sources
                     WHERE user_id = $uid AND from_entity_path = $from AND to_entity_path = $to
                       AND relation = $relation AND origin = $origin",
                )
                .bind(("uid", checkpoint.user_id.clone()))
                .bind(("from", source.from_entity_path.clone()))
                .bind(("to", source.to_entity_path.clone()))
                .bind(("relation", source.relation.clone()))
                .bind(("origin", LinkOrigin::Asserted))
                .bind(("sources", source.source_memory_ids.clone()))
                .await
                .and_then(|response| response.check()),
                "reconcile_commit_link_sources"
            );
        }

        if let Some(entity) = &write.entity {
            tx_try!(
                tx.query("UPSERT type::record('knowledge_consolidation_entity', $id) CONTENT $row")
                    .bind(("id", entity.consolidation_entity_id.clone()))
                    .bind(("row", entity.clone()))
                    .await
                    .and_then(|response| response.check()),
                "reconcile_commit_page"
            );
        }

        let mut checkpoint = checkpoint.clone();
        checkpoint.updated_at = now;
        tx_try!(
            tx.query("UPSERT type::record('knowledge_consolidation_record', $id) CONTENT $record")
                .bind(("id", checkpoint.id.clone()))
                .bind(("record", checkpoint))
                .await
                .and_then(|response| response.check()),
            "reconcile_commit_checkpoint"
        );
        tx.commit()
            .await
            .map_err(|e| Self::err("reconcile_commit", e))?;
        Ok(())
    }
}
