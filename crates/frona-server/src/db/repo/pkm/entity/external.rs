use super::*;

impl PkmRepo {
    /// Upsert a User Vault note as an `origin=External` entity - the DB side of the
    /// ingest cache: full vault `path` is its identity, `body` = the note,
    /// `search_text` = the note (so FTS reaches it), `rev` = note content hash.
    /// Returns the remaining work for the accepted revision. Matching content is
    /// complete only when both the mirror and extraction revisions match `rev`.
    pub async fn upsert_external_page(
        &self,
        user_id: &str,
        path: &str,
        body: &str,
        rev: &str,
    ) -> Result<ExternalPageProgress, AppError> {
        if let Some(existing) = self.entity_by_path(user_id, path).await? {
            if existing.origin != EntityOrigin::External {
                return Err(AppError::Conflict(format!(
                    "cannot replace Internal PKM entity {path} with an External note"
                )));
            }
            if existing.rev.as_deref() == Some(rev) {
                return Ok(Self::external_page_progress(&existing, rev));
            }
            self.db
                .query(
                    "UPDATE type::record('knowledge_entity', $id)
                     SET body = $body, search_text = $body, rev = $rev, updated_at = $now",
                )
                .bind(("id", existing.id))
                .bind(("body", body.to_string()))
                .bind(("rev", rev.to_string()))
                .bind(("now", Utc::now()))
                .await
                .and_then(|response| response.check())
                .map_err(|e| Self::err("external_update", e))?;
            return Ok(ExternalPageProgress {
                rev: rev.to_string(),
                mirror_pending: true,
                extraction_pending: true,
            });
        }
        let now = Utc::now();
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        let aliases = std::collections::HashSet::new();
        let (search_names, search_name_tokens, search_assertions) =
            derive_resolution_search(&name, &aliases, &serde_json::json!({}), std::iter::empty());
        let entity = KnowledgeEntity {
            id: new_id(),
            user_id: user_id.to_string(),
            path: path.to_string(),
            origin: EntityOrigin::External,
            category: EntityCategory::Concept,
            kinds: Vec::new(),
            name,
            description: String::new(),
            identity_evidence: Vec::new(),
            attribute_sources: Vec::new(),
            source_memory_ids: Vec::new(),
            body: body.to_string(),
            sync_content: None,
            mirrored_rev: None,
            extracted_rev: None,
            related_playbooks: Vec::new(),
            search_text: body.to_string(),
            search_names,
            search_name_tokens,
            search_assertions,
            attributes: serde_json::json!({}),
            use_count: 0,
            aliases,
            rev: Some(rev.to_string()),
            updated_at: now,
            rendered_at: now,
        };
        let _: Option<surrealdb::types::Value> = self
            .db
            .create(("knowledge_entity", entity.id.clone()))
            .content(entity)
            .await
            .map_err(|e| Self::err("external_insert", e))?;
        Ok(ExternalPageProgress {
            rev: rev.to_string(),
            mirror_pending: true,
            extraction_pending: true,
        })
    }

    pub(crate) async fn mark_external_page_mirrored(
        &self,
        user_id: &str,
        path: &str,
        expected_rev: &str,
    ) -> Result<bool, AppError> {
        self.mark_external_page_revision(
            user_id,
            path,
            expected_rev,
            "mirrored_rev",
            "external_mirror_cas",
        )
        .await
    }

    async fn mark_external_page_revision(
        &self,
        user_id: &str,
        path: &str,
        expected_rev: &str,
        field: &str,
        context: &str,
    ) -> Result<bool, AppError> {
        let mut response = self
            .db
            .query(format!(
                "UPDATE knowledge_entity SET {field} = $rev
                 WHERE user_id = $uid AND path = $path
                   AND origin = $origin AND rev = $rev
                 RETURN AFTER"
            ))
            .bind(("uid", user_id.to_string()))
            .bind(("path", path.to_string()))
            .bind(("origin", EntityOrigin::External))
            .bind(("rev", expected_rev.to_string()))
            .await
            .and_then(|response| response.check())
            .map_err(|error| Self::err(context, error))?;
        let updated: Vec<surrealdb::types::Value> = response
            .take(0)
            .map_err(|error| Self::err(context, error))?;
        Ok(!updated.is_empty())
    }

    fn external_page_progress(entity: &KnowledgeEntity, rev: &str) -> ExternalPageProgress {
        ExternalPageProgress {
            rev: rev.to_string(),
            mirror_pending: entity.mirrored_rev.as_deref() != Some(rev),
            extraction_pending: entity.extracted_rev.as_deref() != Some(rev),
        }
    }

    /// Drop the memories extracted from a note (`evidence[*].source.ExternalNote.note`) +
    /// their links. Used when a note is deleted. Changed-note replacement uses the
    /// atomic extraction commit. The variant key is PascalCase `ExternalNote` - a
    /// data-carrying
    /// `SurrealValue` enum stores externally-tagged by the *identifier*, and
    /// `rename_all` does not lowercase the struct-variant key (proven in
    /// `tests/pkm_persistence_integration.rs`).
    pub async fn drop_derived_memories(&self, user_id: &str, note: &str) -> Result<(), AppError> {
        let mut q = self
            .db
            .query(
                "SELECT VALUE meta::id(id) FROM knowledge_memory
                 WHERE user_id = $uid AND evidence[*].source.ExternalNote.note CONTAINS $note",
            )
            .bind(("uid", user_id.to_string()))
            .bind(("note", note.to_string()))
            .await
            .map_err(|e| Self::err("external_derived", e))?;
        let ids: Vec<String> = q
            .take(0)
            .map_err(|e| Self::err("external_derived_take", e))?;
        for id in ids {
            self.delete_memory(user_id, &id).await?;
        }
        Ok(())
    }

    /// Delete an External entity and drop the memories extracted from that note. The
    /// note's own mirror file is removed by the caller (storage).
    pub async fn delete_external_page(&self, user_id: &str, path: &str) -> Result<(), AppError> {
        self.drop_derived_memories(user_id, path).await?;
        self.db
            .query(
                "DELETE knowledge_entity
                 WHERE user_id = $uid AND path = $path AND origin = $origin",
            )
            .bind(("uid", user_id.to_string()))
            .bind(("path", path.to_string()))
            .bind(("origin", EntityOrigin::External))
            .await
            .map_err(|e| Self::err("external_delete", e))?;
        Ok(())
    }
}
