use crate::db::repo::pkm::*;

impl PkmRepo {
    /// Publish one author result and its exact canonical Markdown bytes, and synchronize
    /// its working row in the same database transaction. The caller writes the file
    /// mirror only after this durable state commits; old files remain for Cleanup.
    pub async fn commit_authored_page(
        &self,
        consolidation_id: &str,
        user_id: &str,
        write: &AuthoredPageWrite,
    ) -> Result<(), AppError> {
        let tx = self.db.clone().begin().await
            .map_err(|e| Self::err("authored_page_begin", e))?;
        let existing: Result<Option<KnowledgeEntity>, _> = async {
            let mut response = tx.query(format!(
                "{SELECT} FROM knowledge_entity WHERE user_id = $uid AND path = $path LIMIT 1"
            )).bind(("uid", user_id.to_string())).bind(("path", write.path.clone())).await?;
            let rows: Vec<KnowledgeEntity> = response.take(0)?;
            Ok::<_, surrealdb::Error>(rows.into_iter().next())
        }.await;
        let mut entity = match existing {
            Ok(Some(entity)) => entity,
            Ok(None) => {
                let _ = tx.cancel().await;
                return Err(Self::err("authored_page_read", "entity disappeared"));
            }
            Err(e) => {
                let _ = tx.cancel().await;
                return Err(Self::err("authored_page_read", e));
            }
        };
        if !write.name.trim().is_empty() && entity.name != write.name {
            entity.aliases.insert(entity.name.clone());
            entity.name = write.name.clone();
        }
        if !write.description.trim().is_empty() {
            entity.description = write.description.clone();
        }
        entity.attributes = write.attributes.clone();
        entity.body = write.body.clone();
        entity.related_playbooks = write.related_playbooks.clone();
        entity.rev = Some(write.rev.clone());
        entity.sync_content = Some(write.content.clone());
        entity.rendered_at = Utc::now();
        entity.search_text = derive_search_text(&entity.name, &entity.description, &entity.aliases);
        if let Err(e) = tx.query(
            "UPSERT type::record('knowledge_entity', $id) CONTENT $entity",
        ).bind(("id", entity.id.clone())).bind(("entity", entity.clone())).await
            .and_then(|response| response.check()) {
            let _ = tx.cancel().await;
            return Err(Self::err("authored_page_live", e));
        }
        let working: Result<Option<KnowledgeConsolidationEntity>, _> = async {
            let mut response = tx.query(
                "SELECT *, meta::id(id) AS consolidation_entity_id FROM knowledge_consolidation_entity
                 WHERE consolidation_id = $cid AND user_id = $uid AND path = $path LIMIT 1"
            )
            .bind(("cid", consolidation_id.to_string()))
            .bind(("uid", user_id.to_string()))
            .bind(("path", write.path.clone()))
            .await?;
            let rows: Vec<KnowledgeConsolidationEntity> = response.take(0)?;
            Ok::<_, surrealdb::Error>(rows.into_iter().next())
        }.await;
        let mut working = match working {
            Ok(Some(row)) => row,
            Ok(None) => KnowledgeConsolidationEntity::pending(
                consolidation_id, user_id, &write.path, entity.category, Vec::new(),
                entity.source_memory_ids.iter().cloned().collect(),
            ),
            Err(e) => {
                let _ = tx.cancel().await;
                return Err(Self::err("authored_page_overlay_read", e));
            }
        };
        working.lifecycle = ConsolidationEntityLifecycle::Active;
        working.canonical_path = None;
        working.category = entity.category;
        working.source_memory_ids = entity.source_memory_ids.clone();
        working.entity_id = Some(entity.id.clone());
        working.apply_committed(entity);
        working.rederive_search();
        if let Err(e) = tx.query(
            "UPSERT type::record('knowledge_consolidation_entity', $id) CONTENT $row",
        ).bind(("id", working.consolidation_entity_id.clone())).bind(("row", working)).await
            .and_then(|response| response.check()) {
            let _ = tx.cancel().await;
            return Err(Self::err("authored_page_overlay", e));
        }
        tx.commit().await.map_err(|e| Self::err("authored_page_commit", e))?;
        Ok(())
    }

}
