use crate::db::repo::pkm::*;

use super::ResolutionCandidateQuery;

impl PkmRepo {
    pub(crate) async fn list_consolidation_entities(
        &self,
        consolidation_id: &str,
        user_id: &str,
    ) -> Result<Vec<KnowledgeConsolidationEntity>, AppError> {
        let mut response = self
            .db
            .query(
                "SELECT *, meta::id(id) AS consolidation_entity_id
             FROM knowledge_consolidation_entity
             WHERE consolidation_id = $cid AND user_id = $uid ORDER BY path",
            )
            .bind(("cid", consolidation_id.to_string()))
            .bind(("uid", user_id.to_string()))
            .await
            .map_err(|e| Self::err("consolidation_entity_list", e))?;
        response
            .take(0)
            .map_err(|e| Self::err("consolidation_entity_list_take", e))
    }

    pub(crate) async fn list_effective_entities(
        &self,
        consolidation_id: &str,
        user_id: &str,
    ) -> Result<Vec<KnowledgeConsolidationEntity>, AppError> {
        let mut response = self
            .db
            .query(format!(
                "LET $shadowed = SELECT VALUE path FROM knowledge_consolidation_entity
                 WHERE consolidation_id = $cid AND user_id = $uid;
             {SELECT} FROM knowledge_entity
                 WHERE user_id = $uid AND path NOT IN $shadowed ORDER BY path;
             SELECT *, meta::id(id) AS consolidation_entity_id
                 FROM knowledge_consolidation_entity
                 WHERE consolidation_id = $cid AND user_id = $uid AND searchable = true
                 ORDER BY path;"
            ))
            .bind(("cid", consolidation_id.to_string()))
            .bind(("uid", user_id.to_string()))
            .await
            .map_err(|e| Self::err("effective_entity_list", e))?;
        let published: Vec<KnowledgeEntity> = response
            .take(1)
            .map_err(|e| Self::err("effective_entity_list_published_take", e))?;
        let working: Vec<KnowledgeConsolidationEntity> = response
            .take(2)
            .map_err(|e| Self::err("effective_entity_list_working_take", e))?;
        let mut entities: Vec<KnowledgeConsolidationEntity> = published
            .into_iter()
            .map(|entity| KnowledgeConsolidationEntity::from_committed(consolidation_id, entity))
            .chain(working)
            .collect();
        entities.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entities)
    }

    pub(crate) async fn upsert_consolidation_entity(
        &self,
        row: &KnowledgeConsolidationEntity,
    ) -> Result<(), AppError> {
        self.db
            .query("UPSERT type::record('knowledge_consolidation_entity', $id) CONTENT $row")
            .bind(("id", row.consolidation_entity_id.clone()))
            .bind(("row", row.clone()))
            .await
            .map_err(|e| Self::err("upsert_consolidation_entity", e))?
            .check()
            .map_err(|e| Self::err("upsert_consolidation_entity_check", e))?;
        Ok(())
    }

    pub(crate) async fn consolidation_entity_by_path(
        &self,
        consolidation_id: &str,
        user_id: &str,
        path: &str,
    ) -> Result<Option<KnowledgeConsolidationEntity>, AppError> {
        let mut response = self.db.query(
            "SELECT *, meta::id(id) AS consolidation_entity_id FROM knowledge_consolidation_entity
             WHERE consolidation_id = $cid AND user_id = $uid AND path = $path LIMIT 1",
        )
        .bind(("cid", consolidation_id.to_string()))
        .bind(("uid", user_id.to_string()))
        .bind(("path", path.to_string()))
        .await
        .map_err(|e| Self::err("consolidation_entity_by_path", e))?;
        let rows: Vec<KnowledgeConsolidationEntity> = response
            .take(0)
            .map_err(|e| Self::err("consolidation_entity_by_path_take", e))?;
        Ok(rows.into_iter().next())
    }

    pub(crate) async fn consolidation_entity_redirects(
        &self,
        consolidation_id: &str,
        user_id: &str,
    ) -> Result<std::collections::BTreeMap<String, String>, AppError> {
        #[derive(Deserialize, SurrealValue)]
        #[surreal(crate = "surrealdb::types")]
        struct Redirect {
            path: String,
            canonical_path: String,
        }

        let mut response = self
            .db
            .query(
                "SELECT path, canonical_path FROM knowledge_consolidation_entity
             WHERE consolidation_id = $cid AND user_id = $uid
               AND lifecycle = $lifecycle AND canonical_path != NONE",
            )
            .bind(("cid", consolidation_id.to_string()))
            .bind(("uid", user_id.to_string()))
            .bind(("lifecycle", ConsolidationEntityLifecycle::Coalesced))
            .await
            .map_err(|e| Self::err("consolidation_entity_redirects", e))?;
        let rows: Vec<Redirect> = response
            .take(0)
            .map_err(|e| Self::err("consolidation_entity_redirects_take", e))?;
        Ok(rows
            .into_iter()
            .map(|row| (row.path, row.canonical_path))
            .collect())
    }

    pub(crate) async fn delete_consolidation_entities(
        &self,
        consolidation_id: &str,
    ) -> Result<(), AppError> {
        self.db
            .query("DELETE knowledge_consolidation_entity WHERE consolidation_id = $cid")
            .bind(("cid", consolidation_id.to_string()))
            .await
            .map_err(|e| Self::err("delete_consolidation_entities", e))?
            .check()
            .map_err(|e| Self::err("delete_consolidation_entities_check", e))?;
        Ok(())
    }

    /// One database request over committed and run-local entities. A working row shadows
    /// the same committed path even when the working row is a non-searchable tombstone.
    pub(crate) async fn search_effective_entities(
        &self,
        consolidation_id: &str,
        user_id: &str,
        query_text: &str,
    ) -> Result<Vec<EntityHit>, AppError> {
        self.search_effective_entities_with_limit(
            consolidation_id,
            user_id,
            query_text,
            self.search_top_k,
        )
        .await
    }

    pub(crate) async fn search_effective_entities_with_limit(
        &self,
        consolidation_id: &str,
        user_id: &str,
        query_text: &str,
        limit: i64,
    ) -> Result<Vec<EntityHit>, AppError> {
        let mut response = self
            .db
            .query(
                "LET $shadowed = SELECT VALUE path FROM knowledge_consolidation_entity
                 WHERE consolidation_id = $cid AND user_id = $uid;
             LET $live = SELECT id, path, origin, category, kinds, name, description, aliases, body,
                    search_name_tokens, search_assertions,
                    use_count, search::score(0) AS score
                 FROM knowledge_entity
                 WHERE search_text @0,OR@ $q AND user_id = $uid
                   AND path NOT IN $shadowed
                 ORDER BY score DESC, use_count DESC LIMIT $k;
             LET $working = SELECT id, path, origin, category,
                    kinds, name, description, aliases, body, use_count,
                    search_name_tokens, search_assertions,
                    search::score(0) AS score
                 FROM knowledge_consolidation_entity
                 WHERE search_text @0,OR@ $q AND consolidation_id = $cid
                   AND user_id = $uid AND searchable = true
                 ORDER BY score DESC, use_count DESC LIMIT $k;
             RETURN search::rrf([$live, $working], $k, 60);",
            )
            .bind(("cid", consolidation_id.to_string()))
            .bind(("uid", user_id.to_string()))
            .bind(("q", query_text.to_string()))
            .bind(("k", limit.max(1)))
            .await
            .map_err(|e| Self::err("effective_entity_fts", e))?;
        #[derive(Deserialize, SurrealValue)]
        #[surreal(crate = "surrealdb::types")]
        struct Raw {
            path: String,
            origin: Option<EntityOrigin>,
            category: EntityCategory,
            kinds: Vec<String>,
            name: String,
            description: String,
            aliases: std::collections::HashSet<String>,
            search_name_tokens: Vec<String>,
            search_assertions: Vec<String>,
            body: String,
        }
        let rows: Vec<Raw> = response
            .take(3)
            .map_err(|e| Self::err("effective_entity_fts_take", e))?;
        Ok(rows
            .into_iter()
            .map(|row| EntityHit {
                path: row.path,
                origin: row.origin.unwrap_or_default(),
                category: row.category,
                kinds: row.kinds,
                name: row.name,
                description: row.description,
                aliases: row.aliases,
                body: row.body,
                search_name_tokens: row.search_name_tokens,
                search_assertions: row.search_assertions,
            })
            .collect())
    }

    pub(super) async fn search_effective_resolution_candidates(
        &self,
        consolidation_id: &str,
        user_id: &str,
        query: ResolutionCandidateQuery<'_>,
    ) -> Result<Vec<EntityHit>, AppError> {
        let projection = "path, origin, category, kinds, name, description, aliases, body, \
                          search_name_tokens, search_assertions, use_count";
        let sql = format!(
            "LET $shadowed = SELECT VALUE path FROM knowledge_consolidation_entity
                 WHERE consolidation_id = $cid AND user_id = $uid;
             LET $live_identity = SELECT {projection} FROM knowledge_entity
                 WHERE user_id = $uid AND path NOT IN $shadowed
                   AND (search_names CONTAINSANY $names
                     OR search_name_tokens CONTAINSANY $tokens
                     OR search_assertions CONTAINSANY $assertions)
                 LIMIT $k;
             LET $working_identity = SELECT {projection} FROM knowledge_consolidation_entity
                 WHERE consolidation_id = $cid AND user_id = $uid AND searchable = true
                   AND (search_names CONTAINSANY $names
                     OR search_name_tokens CONTAINSANY $tokens
                     OR search_assertions CONTAINSANY $assertions)
                 LIMIT $k;
             LET $live_type_names = SELECT {projection} FROM knowledge_entity
                 WHERE user_id = $uid AND path NOT IN $shadowed
                   AND kinds CONTAINSANY $kinds AND search_name_tokens CONTAINSANY $tokens
                 LIMIT $k;
             LET $working_type_names = SELECT {projection} FROM knowledge_consolidation_entity
                 WHERE consolidation_id = $cid AND user_id = $uid AND searchable = true
                   AND kinds CONTAINSANY $kinds AND search_name_tokens CONTAINSANY $tokens
                 LIMIT $k;
             LET $live_text = SELECT {projection}, search::score(0) AS score FROM knowledge_entity
                 WHERE user_id = $uid AND path NOT IN $shadowed AND search_text @0,OR@ $q
                 ORDER BY score DESC, use_count DESC LIMIT $k;
             LET $working_text = SELECT {projection}, search::score(0) AS score
                 FROM knowledge_consolidation_entity
                 WHERE consolidation_id = $cid AND user_id = $uid AND searchable = true
                   AND search_text @0,OR@ $q
                 ORDER BY score DESC, use_count DESC LIMIT $k;
             RETURN array::union(
                    array::union(array::union($live_identity, $working_identity),
                                 array::union($live_type_names, $working_type_names)),
                    search::rrf([$live_text, $working_text], $k, 60));"
        );
        let mut response = self
            .db
            .query(sql)
            .bind(("cid", consolidation_id.to_string()))
            .bind(("uid", user_id.to_string()))
            .bind(("names", query.names.to_vec()))
            .bind(("tokens", query.name_tokens.to_vec()))
            .bind(("assertions", query.assertions.to_vec()))
            .bind(("kinds", query.kinds.to_vec()))
            .bind(("q", query.text.to_string()))
            .bind(("k", query.limit.max(1)))
            .await
            .map_err(|e| Self::err("effective_resolution_candidates", e))?;
        #[derive(Deserialize, SurrealValue)]
        #[surreal(crate = "surrealdb::types")]
        struct Raw {
            path: String,
            origin: Option<EntityOrigin>,
            category: EntityCategory,
            kinds: Vec<String>,
            name: String,
            description: String,
            aliases: std::collections::HashSet<String>,
            body: String,
            search_name_tokens: Vec<String>,
            search_assertions: Vec<String>,
        }
        let rows: Vec<Raw> = response
            .take(7)
            .map_err(|e| Self::err("effective_resolution_candidates_take", e))?;
        let mut seen = std::collections::HashSet::new();
        Ok(rows
            .into_iter()
            .filter(|row| seen.insert(row.path.clone()))
            .map(|row| EntityHit {
                path: row.path,
                origin: row.origin.unwrap_or_default(),
                category: row.category,
                kinds: row.kinds,
                name: row.name,
                description: row.description,
                aliases: row.aliases,
                body: row.body,
                search_name_tokens: row.search_name_tokens,
                search_assertions: row.search_assertions,
            })
            .collect())
    }

    /// Whether a failed write was a transaction conflict the engine invites us to retry.
    ///
    /// Matched on the message because that is what the driver surfaces - SurrealDB says
    /// "Transaction conflict: Write conflict, retry the transaction". A false negative
    /// costs a propagated error the caller already handles; a false positive costs one
    /// wasted retry of an idempotent transaction. Neither loses data, which is why
    /// string-matching is tolerable here and would not be for a correctness decision.
    pub(crate) fn is_write_conflict(e: &AppError) -> bool {
        e.to_string().to_lowercase().contains("conflict")
    }
}
