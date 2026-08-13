use crate::db::repo::pkm::*;

impl PkmRepo {
    /// Insert an agent-observed memory (the extract stage). See
    /// [`create_sourced_memory`](Self::create_sourced_memory).
    pub async fn create_memory_with_entities(
        &self,
        user_id: &str,
        agent_id: &str,
        chat_id: &str,
        kind: MemoryKind,
        content: &str,
        entity_paths: &[String],
    ) -> Result<String, AppError> {
        self.create_sourced_memory(
            user_id,
            kind,
            content,
            entity_paths,
            vec![MemoryEvidence {
                strength: crate::memory::pkm::model::EvidenceStrength::Explicit,
                source: crate::memory::pkm::model::EvidenceSource::AgentMessage {
                    message_id: String::new(),
                    agent_id: agent_id.to_string(),
                    chat_id: chat_id.to_string(),
                    quote: content.to_string(),
                },
            }],
        )
        .await
    }

    /// Insert a memory with explicit evidence and attach it to the given
    /// entity paths. Rejects memories with no entity links (the knowledge invariant).
    /// Returns the new memory id.
    pub async fn create_sourced_memory(
        &self,
        user_id: &str,
        kind: MemoryKind,
        content: &str,
        entity_paths: &[String],
        evidence: Vec<MemoryEvidence>,
    ) -> Result<String, AppError> {
        if entity_paths.is_empty() {
            return Err(AppError::Validation("memory must belong to ≥1 entity".into()));
        }
        // Two chats for the same user are mined concurrently, and both bump the entities
        // they touch - so two transactions naming one entity genuinely collide. The
        // engine reports that as a retryable write conflict, and the transaction is a
        // fresh insert with a fresh id, so re-running it is safe. Without this the
        // conflict surfaces as a failed ingest and holds the chat's watermark.
        for attempt in 0..CONFLICT_RETRIES {
            match self
                .try_create_sourced_memory(user_id, kind, content, entity_paths, evidence.clone())
                .await
            {
                Err(e) if Self::is_write_conflict(&e) && attempt + 1 < CONFLICT_RETRIES => {
                    tokio::time::sleep(std::time::Duration::from_millis(5 << attempt)).await;
                }
                other => return other,
            }
        }
        unreachable!("the loop returns on its last attempt")
    }

    async fn try_create_sourced_memory(
        &self,
        user_id: &str,
        kind: MemoryKind,
        content: &str,
        entity_paths: &[String],
        evidence: Vec<MemoryEvidence>,
    ) -> Result<String, AppError> {
        let now = Utc::now();
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
            evidence,
        };
        let memory_id = memory.id.clone();

        // Memory + its entity links + entity bumps commit together (the "≥1 link"
        // invariant): a partial insert would leave an orphaned, invisible memory. A
        // fresh memory can't have duplicate links, so no dedup is needed.
        let tx = self
            .db
            .clone()
            .begin()
            .await
            .map_err(|e| Self::err("create_memory_begin", e))?;
        let mem_res: Result<Option<surrealdb::types::Value>, _> = tx
            .create(("knowledge_memory", memory_id.clone()))
            .content(memory)
            .await;
        if let Err(e) = mem_res {
            let _ = tx.cancel().await;
            return Err(Self::err("memory_insert", e));
        }
        for path in entity_paths {
            let link = KnowledgeEntitySource {
                id: new_id(),
                user_id: user_id.to_string(),
                memory_id: memory_id.clone(),
                entity_path: path.clone(),
                created_at: now,
            };
            let link_res: Result<Option<surrealdb::types::Value>, _> = tx
                .create(("knowledge_entity_source", link.id.clone()))
                .content(link)
                .await;
            if let Err(e) = link_res {
                let _ = tx.cancel().await;
                return Err(Self::err("link_insert", e));
            }
            tx_try!(
                tx,
                query tx
                    .query("UPDATE knowledge_entity SET updated_at = $now WHERE user_id = $uid AND path = $path")
                    .bind(("now", now))
                    .bind(("uid", user_id.to_string()))
                    .bind(("path", path.clone())),
                "bump_page_ts"
            );
        }
        tx.commit().await.map_err(|e| Self::err("create_memory_commit", e))?;
        Ok(memory_id)
    }

    pub async fn commit_extract_patch_with_checkpoint(
        &self,
        user_id: &str,
        batch: &IngestBatch,
        watermarks: &[(String, DateTime<Utc>)],
        short_memory_ids: &[String],
        checkpoint: &KnowledgeConsolidationRecord,
    ) -> Result<IngestCounts, AppError> {
        for attempt in 0..CONFLICT_RETRIES {
            match self.try_commit_extract_window(
                user_id, batch, watermarks, short_memory_ids, Some(checkpoint), None,
            ).await {
                Ok(ExtractCommit::Applied(counts)) => return Ok(counts),
                Ok(ExtractCommit::Stale) => {
                    return Err(AppError::Database(
                        "pkm/extract_window: internal extraction became stale".into(),
                    ));
                }
                Err(e) if Self::is_write_conflict(&e) && attempt + 1 < CONFLICT_RETRIES => {
                    tokio::time::sleep(std::time::Duration::from_millis(5 << attempt)).await;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("the loop returns on its last attempt")
    }

    /// Replace memories derived from one External note and record the extracted
    /// revision in the same transaction. Returns `false` when a newer note revision
    /// became current before this extraction could commit.
    pub(crate) async fn commit_external_extract_patch_with_checkpoint(
        &self,
        user_id: &str,
        path: &str,
        expected_rev: &str,
        batch: &IngestBatch,
        checkpoint: &KnowledgeConsolidationRecord,
    ) -> Result<bool, AppError> {
        let external = ExternalExtractionWrite {
            path: path.to_string(),
            rev: expected_rev.to_string(),
        };
        for attempt in 0..CONFLICT_RETRIES {
            match self.try_commit_extract_window(
                user_id, batch, &[], &[], Some(checkpoint), Some(&external),
            ).await {
                Ok(ExtractCommit::Applied(_)) => return Ok(true),
                Ok(ExtractCommit::Stale) => return Ok(false),
                Err(e) if Self::is_write_conflict(&e) && attempt + 1 < CONFLICT_RETRIES => {
                    tokio::time::sleep(std::time::Duration::from_millis(5 << attempt)).await;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("the loop returns on its last attempt")
    }

    async fn try_commit_extract_window(
        &self,
        user_id: &str,
        batch: &IngestBatch,
        watermarks: &[(String, DateTime<Utc>)],
        short_memory_ids: &[String],
        checkpoint: Option<&KnowledgeConsolidationRecord>,
        external: Option<&ExternalExtractionWrite>,
    ) -> Result<ExtractCommit, AppError> {
        let now = Utc::now();
        let mut counts = IngestCounts::default();
        let tx = self
            .db
            .clone()
            .begin()
            .await
            .map_err(|e| Self::err("extract_window_begin", e))?;

        if let Some(external) = external {
            let mut head_response = tx_try!(
                tx,
                query tx.query(format!(
                    "{SELECT} FROM knowledge_entity
                     WHERE user_id = $uid AND path = $path
                       AND origin = $origin AND rev = $rev LIMIT 1"
                ))
                .bind(("uid", user_id.to_string()))
                .bind(("path", external.path.clone()))
                .bind(("origin", EntityOrigin::External))
                .bind(("rev", external.rev.clone())),
                "external_extract_head_read"
            );
            let head: Vec<KnowledgeEntity> = tx_try!(
                tx,
                head_response.take(0),
                "external_extract_head_take"
            );
            if head.is_empty() {
                let _ = tx.cancel().await;
                return Ok(ExtractCommit::Stale);
            }

            let mut memory_response = tx_try!(
                tx,
                query tx.query(
                    "SELECT VALUE meta::id(id) FROM knowledge_memory
                     WHERE user_id = $uid
                       AND evidence[*].source.ExternalNote.note CONTAINS $path"
                )
                .bind(("uid", user_id.to_string()))
                .bind(("path", external.path.clone())),
                "external_extract_old_memories_read"
            );
            let old_memory_ids: Vec<String> = tx_try!(
                tx,
                memory_response.take(0),
                "external_extract_old_memories_take"
            );
            if !old_memory_ids.is_empty() {
                let mut path_response = tx_try!(
                    tx,
                    query tx.query(
                        "SELECT VALUE entity_path FROM knowledge_entity_source
                         WHERE user_id = $uid AND memory_id IN $ids"
                    )
                    .bind(("uid", user_id.to_string()))
                    .bind(("ids", old_memory_ids.clone())),
                    "external_extract_old_paths_read"
                );
                let affected_paths: Vec<String> = tx_try!(
                    tx,
                    path_response.take(0),
                    "external_extract_old_paths_take"
                );
                if !affected_paths.is_empty() {
                    tx_try!(
                        tx,
                        query tx.query(
                            "UPDATE knowledge_entity SET updated_at = $now
                             WHERE user_id = $uid AND path IN $paths"
                        )
                        .bind(("now", now))
                        .bind(("uid", user_id.to_string()))
                        .bind(("paths", affected_paths)),
                        "external_extract_old_pages_bump"
                    );
                }
                let old_memory_records: Vec<RecordId> = old_memory_ids.iter()
                    .map(|id| RecordId::new("knowledge_memory", id.clone()))
                    .collect();
                tx_try!(
                    tx,
                    query tx.query(
                        "DELETE knowledge_entity_source
                         WHERE user_id = $uid AND memory_id IN $ids"
                    )
                    .bind(("uid", user_id.to_string()))
                    .bind(("ids", old_memory_ids)),
                    "external_extract_old_sources_delete"
                );
                tx_try!(
                    tx,
                    query tx.query(
                        "DELETE knowledge_memory WHERE user_id = $uid AND id IN $ids"
                    )
                    .bind(("uid", user_id.to_string()))
                    .bind(("ids", old_memory_records)),
                    "external_extract_old_memories_delete"
                );
            }
        }

        for mem in &batch.memories {
            let n = tx_try!(
                tx,
                Self::insert_memory_in_tx(
                    &tx,
                    &mem.id,
                    user_id,
                    mem.kind,
                    mem.episode.as_ref(),
                    &mem.content,
                    &mem.paths,
                    &mem.evidence,
                    now,
                )
                .await,
                "extract_window_memory"
            );
            counts.memories_added += n;
        }

        // The watermark is what stops this transcript being read again, so it belongs in
        // the same transaction as the rows it accounts for. Splitting them is exactly how
        // a crash between the two replayed a whole window.
        for (chat_id, until) in watermarks {
            tx_try!(
                tx,
                query tx.query(
                    "UPSERT type::record('knowledge_consolidation_watermark', $chat) SET
                         user_id = $uid, chat_id = $chat, consolidated_until = $until,
                         updated_at = $now"
                )
                .bind(("chat", chat_id.to_string()))
                .bind(("uid", user_id.to_string()))
                .bind(("until", *until))
                .bind(("now", now)),
                "extract_window_watermark"
            );
        }

        if !short_memory_ids.is_empty() {
            let recs: Vec<RecordId> = short_memory_ids
                .iter()
                .map(|id| RecordId::new("knowledge_short_memory", id.clone()))
                .collect();
            tx_try!(
                tx,
                query tx.query("UPDATE knowledge_short_memory SET validated = true WHERE id IN $ids")
                    .bind(("ids", recs)),
                "extract_window_short_memory"
            );
        }

        if let Some(record) = checkpoint {
            if let crate::memory::pkm::ConsolidationStageState::Ingest(state) = &record.state {
                let _ = state;
                let mut rows_response = tx_try!(
                    tx,
                    query tx.query(
                        "SELECT *, meta::id(id) AS consolidation_entity_id
                         FROM knowledge_consolidation_entity
                         WHERE consolidation_id = $cid AND user_id = $uid",
                    )
                    .bind(("cid", record.consolidation_id.clone()))
                    .bind(("uid", user_id.to_string())),
                    "extract_window_working_rows_read"
                );
                let existing_rows: Vec<KnowledgeConsolidationEntity> = tx_try!(
                    tx,
                    rows_response.take(0),
                    "extract_window_working_rows_take"
                );
                let mut working_rows: std::collections::BTreeMap<String, KnowledgeConsolidationEntity> =
                    existing_rows.into_iter().map(|row| (row.path.clone(), row)).collect();
                let mut changed_paths = std::collections::BTreeSet::new();
                let memory_ids_for = |path: &str| -> std::collections::BTreeSet<String> {
                    batch.memories.iter()
                        .filter(|memory| memory.paths.iter().any(|held| held == path))
                        .map(|memory| memory.id.clone())
                        .collect()
                };
                for entity in &batch.entities {
                    let source_memory_ids = memory_ids_for(&entity.path);
                    let contribution = PendingEntityContribution {
                        name: entity.name.clone(),
                        description: entity.description.clone(),
                        aliases: entity.aliases.iter().cloned().collect(),
                        attributes: entity.attributes.clone(),
                        attribute_evidence: entity.attribute_evidence.iter()
                            .map(|(key, value)| (key.clone(), value.clone())).collect(),
                        source_memory_ids: source_memory_ids.clone(),
                        existing_only: false,
                        occurrence_count: 1,
                    };
                    let row = working_rows.entry(entity.path.clone()).or_insert_with(|| {
                        KnowledgeConsolidationEntity::pending(
                            &record.consolidation_id, user_id, &entity.path,
                            EntityCategory::Concept, Vec::new(), Default::default(),
                        )
                    });
                    row.merge_contribution(contribution);
                    for evidence in &entity.identity_evidence {
                        if !row.identity_evidence.contains(evidence) {
                            row.identity_evidence.push(evidence.clone());
                        }
                    }
                    changed_paths.insert(entity.path.clone());
                }
                for entity in &batch.entity_updates {
                    let source_memory_ids = memory_ids_for(&entity.path);
                    let contribution = PendingEntityContribution {
                        name: String::new(),
                        description: String::new(),
                        aliases: Default::default(),
                        attributes: entity.attributes.clone(),
                        attribute_evidence: entity.attribute_evidence.iter()
                            .map(|(key, value)| (key.clone(), value.clone())).collect(),
                        source_memory_ids,
                        existing_only: true,
                        occurrence_count: 1,
                    };
                    let row = working_rows.entry(entity.path.clone()).or_insert_with(|| {
                        KnowledgeConsolidationEntity::pending(
                            &record.consolidation_id, user_id, &entity.path,
                            EntityCategory::Concept, Vec::new(), Default::default(),
                        )
                    });
                    row.merge_contribution(contribution);
                    changed_paths.insert(entity.path.clone());
                }
                for memory in &batch.memories {
                    for path in &memory.paths {
                        let row = working_rows.entry(path.clone()).or_insert_with(|| {
                            KnowledgeConsolidationEntity::pending(
                                &record.consolidation_id, user_id, path,
                                EntityCategory::Concept,
                                vec![PendingEntityContribution {
                                    name: String::new(), description: String::new(),
                                    aliases: Default::default(), attributes: serde_json::json!({}),
                                    attribute_evidence: Default::default(),
                                    source_memory_ids: [memory.id.clone()].into_iter().collect(),
                                    existing_only: true, occurrence_count: 1,
                                }],
                                [memory.id.clone()].into_iter().collect(),
                            )
                        });
                        if !row.source_memory_ids.contains(&memory.id) {
                            row.source_memory_ids.push(memory.id.clone());
                        }
                        changed_paths.insert(path.clone());
                    }
                }
                for candidate in &batch.playbook_candidates {
                    let contribution = PendingEntityContribution {
                        name: candidate.name.clone(), description: candidate.description.clone(),
                        aliases: Default::default(), attributes: serde_json::json!({}),
                        attribute_evidence: Default::default(),
                        source_memory_ids: candidate.source_memory_ids.clone(),
                        existing_only: false, occurrence_count: 1,
                    };
                    let row = working_rows.entry(candidate.path.clone()).or_insert_with(|| {
                        let mut row = KnowledgeConsolidationEntity::pending(
                            &record.consolidation_id, user_id, &candidate.path,
                            EntityCategory::Playbook, Vec::new(), Default::default(),
                        );
                        row.consolidation_entity_id = candidate.id.clone();
                        row
                    });
                    row.category = EntityCategory::Playbook;
                    row.merge_contribution(contribution);
                    changed_paths.insert(candidate.path.clone());
                }
                for path in changed_paths {
                    let Some(mut row) = working_rows.remove(&path) else { continue; };
                    let update_only = !row.contributions.is_empty()
                        && row.contributions.iter().all(|contribution| contribution.existing_only);
                    if row.contributions.iter().any(|contribution| contribution.existing_only) {
                        let mut baseline = tx_try!(
                            tx,
                            query tx.query(format!(
                                "{SELECT} FROM knowledge_entity
                                 WHERE user_id = $uid AND path = $path LIMIT 1"
                            ))
                            .bind(("uid", user_id.to_string()))
                            .bind(("path", row.path.clone())),
                            "extract_window_working_baseline_read"
                        );
                        let entities: Vec<KnowledgeEntity> = tx_try!(
                            tx,
                            baseline.take(0),
                            "extract_window_working_baseline_take"
                        );
                        if let Some(entity) = entities.into_iter().next() {
                            for evidence in entity.identity_evidence {
                                if !row.identity_evidence.contains(&evidence) { row.identity_evidence.push(evidence); }
                            }
                            row.entity_id = Some(entity.id.clone());
                            row.search_text = entity.search_text;
                            let mut names: std::collections::BTreeSet<String> = entity.aliases
                                .iter().map(|name| name.trim().to_lowercase())
                                .filter(|name| !name.is_empty()).collect();
                            if !entity.name.trim().is_empty() {
                                names.insert(entity.name.trim().to_lowercase());
                            }
                            row.search_names = names.into_iter().collect();
                        } else if update_only {
                            tx_try!(
                                tx,
                                query tx.query(
                                    "DELETE knowledge_consolidation_entity
                                     WHERE consolidation_id = $cid AND user_id = $uid AND path = $path",
                                )
                                .bind(("cid", record.consolidation_id.clone()))
                                .bind(("uid", user_id.to_string()))
                                .bind(("path", row.path.clone())),
                                "extract_window_missing_update_only_page_delete"
                            );
                            continue;
                        }
                    }
                    tx_try!(
                        tx,
                        query tx.query(
                            "UPSERT type::record('knowledge_consolidation_entity', $id) CONTENT $row",
                        )
                        .bind(("id", row.consolidation_entity_id.clone()))
                        .bind(("row", row)),
                        "extract_window_working_page_write"
                    );
                }
            }
            tx_try!(
                tx,
                query tx.query("UPSERT type::record('knowledge_consolidation_record', $id) CONTENT $rec")
                    .bind(("id", record.id.clone()))
                    .bind(("rec", record.clone())),
                "extract_window_checkpoint_write"
            );
        }

        if let Some(external) = external {
            let mut mark_response = tx_try!(
                tx,
                query tx.query(
                    "UPDATE knowledge_entity SET extracted_rev = $rev
                     WHERE user_id = $uid AND path = $path
                       AND origin = $origin AND rev = $rev
                     RETURN AFTER"
                )
                .bind(("uid", user_id.to_string()))
                .bind(("path", external.path.clone()))
                .bind(("origin", EntityOrigin::External))
                .bind(("rev", external.rev.clone())),
                "external_extract_mark"
            );
            let updated: Vec<surrealdb::types::Value> = tx_try!(
                tx,
                mark_response.take(0),
                "external_extract_mark_take"
            );
            if updated.is_empty() {
                let _ = tx.cancel().await;
                return Ok(ExtractCommit::Stale);
            }
        }

        tx.commit()
            .await
            .map_err(|e| Self::err("extract_window_commit", e))?;
        Ok(ExtractCommit::Applied(counts))
    }

    /// Insert one memory inside an open extraction transaction. Entity associations are
    /// deliberately deferred until the classified graph is committed; an extracted
    /// memory remains durable even when every proposed entity is later discarded.
    #[allow(clippy::too_many_arguments)]
    async fn insert_memory_in_tx(
        tx: &surrealdb::method::Transaction<Db>,
        memory_id: &str,
        user_id: &str,
        kind: MemoryKind,
        episode: Option<&crate::memory::pkm::model::Episode>,
        content: &str,
        entity_paths: &[String],
        evidence: &[MemoryEvidence],
        now: DateTime<Utc>,
    ) -> Result<usize, surrealdb::Error> {
        let memory = KnowledgeMemory {
            id: memory_id.to_string(),
            user_id: user_id.to_string(),
            created_at: now,
            kind,
            episode: episode.cloned(),
            content: content.to_string(),
            relations: Vec::new(),
            disposition: Disposition::None,
            ended_at: None,
            comment: None,
            erroneous_at: None,
            evidence: evidence.to_vec(),
        };
        tx.query("CREATE type::record('knowledge_memory', $id) CONTENT $mem")
            .bind(("id", memory_id.to_string()))
            .bind(("mem", memory))
            .await?
            .check()?;
        let _ = entity_paths;
        Ok(1)
    }

}
