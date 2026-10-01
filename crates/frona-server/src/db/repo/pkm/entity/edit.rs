use crate::db::repo::pkm::*;

impl PkmRepo {
    /// Apply one complete human page edit only if its planned base is still current.
    /// The revision check, optional skeleton creation, memory changes, body, and new
    /// revision share one database transaction.
    pub(crate) async fn commit_page_edit_cas(
        &self,
        user_id: &str,
        path: &str,
        write: &PageEditWrite,
    ) -> Result<PageEditCommit, AppError> {
        if matches!(write.base, PageEditBase::Missing) && write.new_page_name.is_none() {
            return Err(AppError::Validation(
                "new page edit requires a page name".into(),
            ));
        }
        for attempt in 0..CONFLICT_RETRIES {
            match self.try_commit_page_edit_cas(user_id, path, write).await {
                Err(error) if Self::is_write_conflict(&error) && attempt + 1 < CONFLICT_RETRIES => {
                    tokio::time::sleep(std::time::Duration::from_millis(5 << attempt)).await;
                }
                result => return result,
            }
        }
        unreachable!("the loop returns on its last attempt")
    }

    async fn try_commit_page_edit_cas(
        &self,
        user_id: &str,
        path: &str,
        write: &PageEditWrite,
    ) -> Result<PageEditCommit, AppError> {
        let now = Utc::now();
        let tx = self
            .db
            .clone()
            .begin()
            .await
            .map_err(|error| Self::err("page_edit_begin", error))?;

        let mut response = tx_try!(
            tx,
            query tx
                .query(format!(
                    "{SELECT} FROM knowledge_entity WHERE user_id = $uid AND path = $path LIMIT 1"
                ))
                .bind(("uid", user_id.to_string()))
                .bind(("path", path.to_string())),
            "page_edit_read"
        );
        let rows: Vec<KnowledgeEntity> = tx_try!(tx, response.take(0), "page_edit_read_take");
        let current = rows.into_iter().next();
        let matches_base = match (&write.base, &current) {
            (PageEditBase::Missing, None) => true,
            (PageEditBase::Revision(expected), Some(entity)) => {
                entity.rev.as_deref() == Some(expected.as_str()) && !expected.is_empty()
            }
            _ => false,
        };
        if !matches_base {
            let (head_rev, head_content) = current
                .map(|entity| (entity.rev, entity.sync_content))
                .unwrap_or((None, None));
            let _ = tx.cancel().await;
            return Ok(PageEditCommit::Conflict {
                head_rev,
                head_content,
            });
        }

        // A model result can wait for a long time. Check all referenced memories again
        // before the first mutation so a concurrent lifecycle change cannot produce a
        // partial or misdirected edit.
        for operation in &write.memory_ops {
            let memory_id = match operation {
                PageEditMemoryOp::Add { .. } => continue,
                PageEditMemoryOp::Supersede { older_id, .. } => older_id,
                PageEditMemoryOp::SetDisposition { memory_id, .. } => memory_id,
            };
            let mut response = tx_try!(
                tx,
                query tx
                    .query(
                        "SELECT VALUE memory_id FROM knowledge_entity_source
                         WHERE user_id = $uid AND entity_path = $path AND memory_id = $mid
                         LIMIT 1",
                    )
                    .bind(("uid", user_id.to_string()))
                    .bind(("path", path.to_string()))
                    .bind(("mid", memory_id.clone())),
                "page_edit_memory_source_read"
            );
            let sources: Vec<String> =
                tx_try!(tx, response.take(0), "page_edit_memory_source_take");
            if sources.is_empty() {
                let head_rev = current.as_ref().and_then(|entity| entity.rev.clone());
                let head_content = current
                    .as_ref()
                    .and_then(|entity| entity.sync_content.clone());
                let _ = tx.cancel().await;
                return Ok(PageEditCommit::Conflict {
                    head_rev,
                    head_content,
                });
            }
        }

        if current.is_none() {
            let name = write.new_page_name.as_deref().unwrap_or(path);
            let aliases = std::collections::HashSet::new();
            let search_text = derive_search_text(name, "", &aliases);
            let (search_names, search_name_tokens, search_assertions) = derive_resolution_search(
                name,
                &aliases,
                &serde_json::json!({}),
                std::iter::empty(),
            );
            let entity = KnowledgeEntity {
                id: new_id(),
                user_id: user_id.to_string(),
                path: path.to_string(),
                origin: EntityOrigin::Internal,
                category: EntityCategory::Concept,
                kinds: Vec::new(),
                name: name.to_string(),
                description: String::new(),
                identity_evidence: Vec::new(),
                attribute_sources: Vec::new(),
                source_memory_ids: Vec::new(),
                body: String::new(),
                sync_content: None,
                mirrored_rev: None,
                extracted_rev: None,
                related_playbooks: Vec::new(),
                search_text,
                search_names,
                search_name_tokens,
                search_assertions,
                attributes: serde_json::json!({}),
                use_count: 0,
                aliases,
                rev: None,
                updated_at: now,
                rendered_at: DateTime::<Utc>::MIN_UTC,
            };
            tx_try!(
                tx,
                query tx
                    .query("CREATE type::record('knowledge_entity', $id) CONTENT $entity")
                    .bind(("id", entity.id.clone()))
                    .bind(("entity", entity)),
                "page_edit_entity_insert"
            );
        }

        for operation in &write.memory_ops {
            match operation {
                PageEditMemoryOp::Add { kind, content } => {
                    tx_try!(
                        tx,
                        Self::insert_human_memory_in_tx(&tx, user_id, path, *kind, content, now,)
                            .await,
                        "page_edit_memory_add"
                    );
                }
                PageEditMemoryOp::Supersede {
                    older_id,
                    kind,
                    content,
                    note,
                } => {
                    let newer_id = tx_try!(
                        tx,
                        Self::insert_human_memory_in_tx(&tx, user_id, path, *kind, content, now,)
                            .await,
                        "page_edit_memory_supersede_insert"
                    );
                    let mut response = tx_try!(
                        tx,
                        query tx
                            .query(
                                "SELECT VALUE entity_path FROM knowledge_entity_source
                                 WHERE user_id = $uid AND memory_id = $mid",
                            )
                            .bind(("uid", user_id.to_string()))
                            .bind(("mid", older_id.clone())),
                        "page_edit_memory_supersede_sources"
                    );
                    let mut entity_paths: Vec<String> = tx_try!(
                        tx,
                        response.take(0),
                        "page_edit_memory_supersede_sources_take"
                    );
                    entity_paths.sort();
                    entity_paths.dedup();
                    for entity_path in entity_paths
                        .iter()
                        .filter(|entity_path| entity_path.as_str() != path)
                    {
                        tx_try!(
                            tx,
                            Self::insert_memory_source_in_tx(
                                &tx,
                                user_id,
                                &newer_id,
                                entity_path,
                                now,
                            )
                            .await,
                            "page_edit_memory_supersede_source_insert"
                        );
                    }
                    let relation = MemoryRelation {
                        relation: RelationType::Replace,
                        to: RecordId::new("knowledge_memory", newer_id),
                        note: note.clone(),
                    };
                    tx_try!(
                        tx,
                        query tx
                            .query(
                                "UPDATE type::record('knowledge_memory', $id)
                                 SET relations += [$relation] WHERE user_id = $uid",
                            )
                            .bind(("id", older_id.clone()))
                            .bind(("uid", user_id.to_string()))
                            .bind(("relation", relation)),
                        "page_edit_memory_supersede_relation"
                    );
                    tx_try!(
                        tx,
                        query tx
                            .query(
                                "UPDATE knowledge_entity SET updated_at = $now
                                 WHERE user_id = $uid AND path IN $paths",
                            )
                            .bind(("now", now))
                            .bind(("uid", user_id.to_string()))
                            .bind(("paths", entity_paths)),
                        "page_edit_memory_supersede_bump"
                    );
                }
                PageEditMemoryOp::SetDisposition {
                    memory_id,
                    disposition,
                } => {
                    let (ended_at, erroneous_at) = match disposition {
                        Disposition::Outdated => (Some(now), None),
                        Disposition::Erroneous => (None, Some(now)),
                        Disposition::None | Disposition::Suspect => (None, None),
                    };
                    tx_try!(
                        tx,
                        query tx
                            .query(
                                "UPDATE type::record('knowledge_memory', $id)
                                 SET disposition = $disposition, ended_at = $ended_at,
                                     erroneous_at = $erroneous_at
                                 WHERE user_id = $uid",
                            )
                            .bind(("id", memory_id.clone()))
                            .bind(("uid", user_id.to_string()))
                            .bind(("disposition", *disposition))
                            .bind(("ended_at", ended_at))
                            .bind(("erroneous_at", erroneous_at)),
                        "page_edit_memory_disposition"
                    );
                    tx_try!(
                        tx,
                        query tx
                            .query(
                                "UPDATE knowledge_entity SET updated_at = $now WHERE user_id = $uid
                                 AND path IN (SELECT VALUE entity_path FROM knowledge_entity_source
                                     WHERE user_id = $uid AND memory_id = $mid)",
                            )
                            .bind(("now", now))
                            .bind(("uid", user_id.to_string()))
                            .bind(("mid", memory_id.clone())),
                        "page_edit_memory_disposition_bump"
                    );
                }
            }
        }

        tx_try!(
            tx,
            query tx
                .query(
                    "UPDATE knowledge_entity SET body = $body, sync_content = $content, rev = $rev
                     WHERE user_id = $uid AND path = $path",
                )
                .bind(("body", write.body.clone()))
                .bind(("content", write.content.clone()))
                .bind(("rev", write.rev.clone()))
                .bind(("uid", user_id.to_string()))
                .bind(("path", path.to_string())),
            "page_edit_projection"
        );
        tx.commit()
            .await
            .map_err(|error| Self::err("page_edit_commit", error))?;
        Ok(PageEditCommit::Applied)
    }

    async fn insert_human_memory_in_tx(
        tx: &surrealdb::method::Transaction<Db>,
        user_id: &str,
        path: &str,
        kind: MemoryKind,
        content: &str,
        now: DateTime<Utc>,
    ) -> Result<String, surrealdb::Error> {
        let memory = KnowledgeMemory {
            id: new_id(),
            user_id: user_id.to_string(),
            created_at: now,
            kind,
            episode: None,
            content: content.to_string(),
            relations: Vec::new(),
            disposition: Disposition::None,
            ended_at: None,
            comment: None,
            erroneous_at: None,
            evidence: vec![MemoryEvidence {
                strength: crate::memory::pkm::model::EvidenceStrength::Explicit,
                source: crate::memory::pkm::model::EvidenceSource::HumanEdit {
                    page_path: path.to_string(),
                    quote: content.to_string(),
                },
            }],
        };
        let memory_id = memory.id.clone();
        tx.create::<Option<surrealdb::types::Value>>(("knowledge_memory", memory_id.clone()))
            .content(memory)
            .await?;
        Self::insert_memory_source_in_tx(tx, user_id, &memory_id, path, now).await?;
        Ok(memory_id)
    }

    async fn insert_memory_source_in_tx(
        tx: &surrealdb::method::Transaction<Db>,
        user_id: &str,
        memory_id: &str,
        path: &str,
        now: DateTime<Utc>,
    ) -> Result<(), surrealdb::Error> {
        let source = KnowledgeEntitySource {
            id: new_id(),
            user_id: user_id.to_string(),
            memory_id: memory_id.to_string(),
            entity_path: path.to_string(),
            created_at: now,
        };
        tx.create::<Option<surrealdb::types::Value>>((
            "knowledge_entity_source",
            source.id.clone(),
        ))
        .content(source)
        .await?;
        tx.query(
            "UPDATE knowledge_entity SET updated_at = $now
             WHERE user_id = $uid AND path = $path",
        )
        .bind(("now", now))
        .bind(("uid", user_id.to_string()))
        .bind(("path", path.to_string()))
        .await?
        .check()?;
        Ok(())
    }
}
