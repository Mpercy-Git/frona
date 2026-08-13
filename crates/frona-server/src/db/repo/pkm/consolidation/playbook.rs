use crate::db::repo::pkm::*;

impl PkmRepo {
    /// Project all resolved Playbooks and advance the consolidation record atomically.
    /// A crash therefore observes either the banked Resolve checkpoint with no live
    /// Playbook effects, or every effect together with `PlaybookAuthor` as the next stage.
    pub async fn commit_playbook_resolutions(
        &self,
        user_id: &str,
        writes: &[PlaybookResolutionWrite],
        completed_checkpoint: &KnowledgeConsolidationRecord,
    ) -> Result<(), AppError> {
        let now = Utc::now();
        let tx = self.db.clone().begin().await
            .map_err(|e| Self::err("playbook_resolve_begin", e))?;

        for write in writes {
            let mut sources = write.merge_from.clone();
            if let Some(source) = write.existing_path.as_ref().filter(|p| *p != &write.path) {
                sources.push(source.clone());
            }
            sources.sort();
            sources.dedup();

            let mut merged_aliases = std::collections::HashSet::new();

            for source in sources {
                let source_page: Result<Vec<KnowledgeEntity>, _> = async {
                    let mut response = tx.query(
                        format!("{SELECT} FROM knowledge_entity WHERE user_id = $uid AND path = $path LIMIT 1")
                    ).bind(("uid", user_id.to_string())).bind(("path", source.clone())).await?;
                    response.take(0)
                }.await;
                match source_page {
                    Ok(rows) => {
                        for entity in rows {
                            merged_aliases.insert(entity.name);
                            merged_aliases.extend(entity.aliases);
                        }
                    }
                    Err(error) => {
                        let _ = tx.cancel().await;
                        return Err(Self::err("playbook_resolve_source_read", error));
                    }
                }
                let result = tx.query(
                    "UPDATE knowledge_entity_source SET entity_path = $to
                         WHERE user_id = $uid AND entity_path = $from;
                     UPDATE knowledge_entity_link SET from_entity_path = $to
                         WHERE user_id = $uid AND from_entity_path = $from;
                     UPDATE knowledge_entity_link SET to_entity_path = $to
                         WHERE user_id = $uid AND to_entity_path = $from;
                     DELETE knowledge_entity WHERE user_id = $uid AND path = $from",
                )
                .bind(("uid", user_id.to_string()))
                .bind(("from", source))
                .bind(("to", write.path.clone()))
                .await
                .and_then(|response| response.check());
                if let Err(error) = result {
                    let _ = tx.cancel().await;
                    return Err(Self::err("playbook_resolve_move", error));
                }
            }

            let existing: Result<Vec<KnowledgeEntity>, _> = async {
                let mut response = tx.query(
                    format!("{SELECT} FROM knowledge_entity WHERE user_id = $uid AND path = $path LIMIT 1")
                )
                .bind(("uid", user_id.to_string()))
                .bind(("path", write.path.clone()))
                .await?;
                response.take(0)
            }.await;
            let existing = match existing {
                Ok(mut rows) => rows.pop(),
                Err(error) => {
                    let _ = tx.cancel().await;
                    return Err(Self::err("playbook_resolve_read", error));
                }
            };
            if let Some(entity) = existing {
                let mut aliases = entity.aliases;
                let mut kinds = entity.kinds;
                if !kinds.iter().any(|kind| kind == PLAYBOOK_KIND_IRI) {
                    kinds.push(PLAYBOOK_KIND_IRI.to_string());
                }
                if entity.name != write.name {
                    aliases.insert(entity.name);
                }
                aliases.extend(merged_aliases);
                let search_text = derive_search_text(&write.name, &write.description, &aliases);
                let (search_names, search_name_tokens, mut search_assertions) =
                    derive_resolution_search(
                        &write.name, &aliases, &entity.attributes, std::iter::empty(),
                    );
                search_assertions.extend(entity.search_assertions);
                search_assertions.sort();
                search_assertions.dedup();
                if let Err(error) = tx.query(
                    "UPDATE type::record('knowledge_entity', $id) SET
                         category = $category, name = $name, description = $description,
                         kinds = $kinds, aliases = $aliases, search_text = $search_text,
                         search_names = $search_names,
                         search_name_tokens = $search_name_tokens,
                         search_assertions = $search_assertions,
                         updated_at = $now",
                )
                .bind(("id", entity.id))
                .bind(("category", EntityCategory::Playbook))
                .bind(("name", write.name.clone()))
                .bind(("description", write.description.clone()))
                .bind(("kinds", kinds))
                .bind(("aliases", aliases))
                .bind(("search_text", search_text))
                .bind(("search_names", search_names))
                .bind(("search_name_tokens", search_name_tokens))
                .bind(("search_assertions", search_assertions))
                .bind(("now", now))
                .await
                .and_then(|response| response.check()) {
                    let _ = tx.cancel().await;
                    return Err(Self::err("playbook_resolve_update", error));
                }
            } else {
                let aliases = merged_aliases;
                let (search_names, search_name_tokens, search_assertions) =
                    derive_resolution_search(
                        &write.name, &aliases, &serde_json::json!({}), std::iter::empty(),
                    );
                let entity = KnowledgeEntity {
                    id: new_id(), user_id: user_id.to_string(), path: write.path.clone(),
                    origin: EntityOrigin::Internal, category: EntityCategory::Playbook,
                    kinds: vec![PLAYBOOK_KIND_IRI.to_string()],
                    name: write.name.clone(), description: write.description.clone(),
                    identity_evidence: Vec::new(),
                    attribute_sources: Vec::new(), source_memory_ids: Vec::new(), body: String::new(),
                    sync_content: None,
                    mirrored_rev: None,
                    extracted_rev: None,
                    related_playbooks: Vec::new(),
                    search_text: derive_search_text(&write.name, &write.description, &aliases),
                    search_names, search_name_tokens, search_assertions,
                    attributes: serde_json::json!({}), use_count: 0, aliases, rev: None,
                    updated_at: now, rendered_at: DateTime::<Utc>::MIN_UTC,
                };
                if let Err(error) = tx.query("CREATE type::record('knowledge_entity', $id) CONTENT $entity")
                    .bind(("id", entity.id.clone())).bind(("entity", entity)).await
                    .and_then(|response| response.check()) {
                    let _ = tx.cancel().await;
                    return Err(Self::err("playbook_resolve_create", error));
                }
            }

            for memory_id in &write.memory_ids {
                let links: Result<Vec<KnowledgeEntitySource>, _> = async {
                    let mut response = tx.query(
                        format!("{SELECT} FROM knowledge_entity_source WHERE user_id = $uid AND memory_id = $mid")
                    ).bind(("uid", user_id.to_string())).bind(("mid", memory_id.clone())).await?;
                    response.take(0)
                }.await;
                let links = match links {
                    Ok(links) => links,
                    Err(error) => {
                        let _ = tx.cancel().await;
                        return Err(Self::err("playbook_resolve_sources", error));
                    }
                };
                let provisional_paths = write.candidate_paths.iter()
                    .filter(|path| *path != &write.path).cloned().collect::<Vec<_>>();
                if !provisional_paths.is_empty()
                    && let Err(error) = tx.query(
                        "DELETE knowledge_entity_source
                         WHERE user_id = $uid AND memory_id = $mid AND entity_path IN $paths",
                    )
                    .bind(("uid", user_id.to_string()))
                    .bind(("mid", memory_id.clone()))
                    .bind(("paths", provisional_paths.clone()))
                    .await
                    .and_then(|response| response.check())
                {
                    let _ = tx.cancel().await;
                    return Err(Self::err("playbook_resolve_detach_candidate", error));
                }
                if !links.iter().any(|link| link.entity_path == write.path) {
                    let link = KnowledgeEntitySource {
                        id: new_id(), user_id: user_id.to_string(), memory_id: memory_id.clone(),
                        entity_path: write.path.clone(), created_at: now,
                    };
                    if let Err(error) = tx.query("CREATE type::record('knowledge_entity_source', $id) CONTENT $link")
                        .bind(("id", link.id.clone())).bind(("link", link)).await
                        .and_then(|response| response.check()) {
                        let _ = tx.cancel().await;
                        return Err(Self::err("playbook_resolve_attach", error));
                    }
                }
                for source in links.into_iter().map(|link| link.entity_path)
                    .filter(|source| source != &write.path && !provisional_paths.contains(source))
                {
                    let edge_exists: Result<Vec<String>, _> = async {
                        let mut response = tx.query(
                            "SELECT VALUE meta::id(id) FROM knowledge_entity_link
                             WHERE user_id = $uid AND from_entity_path = $from
                               AND to_entity_path = $to AND relation = 'playbook' LIMIT 1"
                        ).bind(("uid", user_id.to_string()))
                            .bind(("from", source.clone()))
                            .bind(("to", write.path.clone())).await?;
                        response.take(0)
                    }.await;
                    match edge_exists {
                        Ok(ids) if !ids.is_empty() => continue,
                        Ok(_) => {}
                        Err(error) => {
                            let _ = tx.cancel().await;
                            return Err(Self::err("playbook_resolve_link_read", error));
                        }
                    }
                    let edge = KnowledgeEntityLink {
                        id: new_id(), user_id: user_id.to_string(), from_entity_path: source,
                        to_entity_path: write.path.clone(), relation: "playbook".into(),
                        source_memory_ids: vec![memory_id.clone()], origin: LinkOrigin::Asserted,
                        created_at: now,
                    };
                    if let Err(error) = tx.query(
                        "CREATE type::record('knowledge_entity_link', $id) CONTENT $edge"
                    ).bind(("id", edge.id.clone())).bind(("edge", edge)).await
                        .and_then(|response| response.check()) {
                        let _ = tx.cancel().await;
                        return Err(Self::err("playbook_resolve_link", error));
                    }
                }
            }

            let final_page: Result<Option<KnowledgeEntity>, _> = async {
                let mut response = tx.query(format!(
                    "{SELECT} FROM knowledge_entity WHERE user_id = $uid AND path = $path LIMIT 1"
                ))
                .bind(("uid", user_id.to_string()))
                .bind(("path", write.path.clone()))
                .await?;
                let rows: Vec<KnowledgeEntity> = response.take(0)?;
                Ok::<_, surrealdb::Error>(rows.into_iter().next())
            }.await;
            let final_page = match final_page {
                Ok(Some(entity)) => entity,
                Ok(None) => {
                    let _ = tx.cancel().await;
                    return Err(Self::err("playbook_resolve_overlay", "projected entity missing"));
                }
                Err(error) => {
                    let _ = tx.cancel().await;
                    return Err(Self::err("playbook_resolve_overlay_read", error));
                }
            };
            let target_row: Result<Option<KnowledgeConsolidationEntity>, _> = async {
                let mut response = tx.query(
                    "SELECT *, meta::id(id) AS consolidation_entity_id FROM knowledge_consolidation_entity
                     WHERE consolidation_id = $cid AND user_id = $uid AND path = $path LIMIT 1"
                )
                .bind(("cid", completed_checkpoint.consolidation_id.clone()))
                .bind(("uid", user_id.to_string()))
                .bind(("path", write.path.clone()))
                .await?;
                let rows: Vec<KnowledgeConsolidationEntity> = response.take(0)?;
                Ok::<_, surrealdb::Error>(rows.into_iter().next())
            }.await;
            let mut target_row = match target_row {
                Ok(Some(row)) => row,
                Ok(None) => KnowledgeConsolidationEntity::pending(
                    &completed_checkpoint.consolidation_id, user_id, &write.path,
                    EntityCategory::Playbook, Vec::new(),
                    write.memory_ids.iter().cloned().collect(),
                ),
                Err(error) => {
                    let _ = tx.cancel().await;
                    return Err(Self::err("playbook_resolve_overlay_target", error));
                }
            };
            target_row.lifecycle = crate::db::repo::pkm::ConsolidationEntityLifecycle::Active;
            target_row.canonical_path = None;
            target_row.entity_id = Some(final_page.id.clone());
            target_row.apply_committed(final_page);
            target_row.source_memory_ids = write.memory_ids.clone();
            target_row.rederive_search();
            if let Err(error) = tx.query(
                "UPSERT type::record('knowledge_consolidation_entity', $id) CONTENT $row"
            )
            .bind(("id", target_row.consolidation_entity_id.clone()))
            .bind(("row", target_row))
            .await
            .and_then(|response| response.check()) {
                let _ = tx.cancel().await;
                return Err(Self::err("playbook_resolve_overlay_write", error));
            }
            for (candidate_id, source) in write.candidate_ids.iter()
                .zip(&write.candidate_paths).filter(|(_, path)| *path != &write.path)
            {
                let preserve_existing = write.existing_path.as_deref() != Some(source.as_str())
                    && !write.merge_from.contains(source);
                let live_exists: Result<bool, _> = async {
                    let mut response = tx.query(
                        "SELECT VALUE count() FROM knowledge_entity
                         WHERE user_id = $uid AND path = $source GROUP ALL",
                    )
                    .bind(("uid", user_id.to_string()))
                    .bind(("source", source.clone()))
                    .await?;
                    let counts: Vec<i64> = response.take(0)?;
                    Ok::<_, surrealdb::Error>(counts.first().copied().unwrap_or_default() > 0)
                }.await;
                let query = match live_exists {
                    Ok(true) if preserve_existing => tx.query(
                        "DELETE type::record('knowledge_consolidation_entity', $candidate_id)",
                    ),
                    Ok(_) => tx.query(
                        "UPDATE type::record('knowledge_consolidation_entity', $candidate_id) SET
                             lifecycle = $lifecycle, searchable = false,
                             canonical_path = $canonical, updated_at = $now",
                    )
                    .bind(("lifecycle", crate::db::repo::pkm::ConsolidationEntityLifecycle::Coalesced))
                    .bind(("canonical", write.path.clone()))
                    .bind(("now", now)),
                    Err(error) => {
                        let _ = tx.cancel().await;
                        return Err(Self::err("playbook_resolve_candidate_page_read", error));
                    }
                };
                if let Err(error) = query
                    .bind(("candidate_id", candidate_id.clone()))
                    .bind(("cid", completed_checkpoint.consolidation_id.clone()))
                    .bind(("uid", user_id.to_string()))
                    .bind(("source", source.clone()))
                    .await
                    .and_then(|response| response.check()) {
                    let _ = tx.cancel().await;
                    return Err(Self::err("playbook_resolve_overlay_coalesce", error));
                }
            }
        }

        let mut checkpoint = completed_checkpoint.clone();
        checkpoint.updated_at = now;
        if let Err(error) = tx.query(
            "UPSERT type::record('knowledge_consolidation_record', $id) CONTENT $record"
        ).bind(("id", checkpoint.id.clone())).bind(("record", checkpoint)).await
            .and_then(|response| response.check()) {
            let _ = tx.cancel().await;
            return Err(Self::err("playbook_resolve_checkpoint", error));
        }
        tx.commit().await.map_err(|e| Self::err("playbook_resolve_commit", e))?;
        Ok(())
    }

}
