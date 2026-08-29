use super::*;

impl PkmRepo {
    /// The user's ontology delta row (OWL blob + version), or `None` if the user
    /// has minted nothing yet (their TBox is exactly the shared reference base).
    pub async fn ontology_get(&self, user_id: &str) -> Result<Option<KnowledgeOntology>, AppError> {
        let mut q = self
            .db
            .query(format!(
                "{SELECT} FROM knowledge_ontology WHERE user_id = $uid LIMIT 1"
            ))
            .bind(("uid", user_id.to_string()))
            .await
            .map_err(|e| Self::err("ontology_get", e))?;
        let rows: Vec<KnowledgeOntology> =
            q.take(0).map_err(|e| Self::err("ontology_get_take", e))?;
        Ok(rows.into_iter().next())
    }

    /// Compare-and-swap the user's ontology delta. `expected_version` is the
    /// version the caller read (0 for an absent row). On a match, the delta is
    /// written at `expected_version + 1` and the new version returned; on a
    /// mismatch nothing is written and `Ok(None)` is returned so the caller can
    /// reload and re-apply. Single-writer in the serial sweep - the CAS is a
    /// backstop against a racing sync/admin write.
    /// Commit a schema delta **and** every entity type stamped against it, atomically.
    ///
    /// These two are inseparable: an entity typed `frona:Service` where the TBox never
    /// declared `frona:Service` is an entity the reasoner cannot place, and the pipeline
    /// used to produce exactly that whenever adjudication failed part-way - it logged
    /// "stamping without schema" and carried on. One transaction removes the state.
    ///
    /// CAS on the ontology version is checked **inside** the transaction, so a
    /// concurrent writer cannot slip between the check and the write. Returns `false`
    /// on a CAS miss, which means the caller should re-plan against the newer delta.
    #[allow(clippy::too_many_arguments)]
    pub async fn commit_schema_and_types(
        &self,
        user_id: &str,
        owl: &str,
        format: &str,
        expected_version: i64,
        types: &[(String, Vec<String>)],
        rekeys: &[(String, String, String)],
        relation_type_renames: &[(String, String)],
        attributes: &[AttributeOps],
        materialize: &[KnowledgeEntity],
        coalesced_sources: &[(String, String, String)],
        coalesced_aliases: &[(String, Vec<String>)],
        working_outcomes: &[(String, Option<String>)],
        completed_checkpoint: Option<&KnowledgeConsolidationRecord>,
    ) -> Result<bool, AppError> {
        let now = Utc::now();
        let tx = self
            .db
            .clone()
            .begin()
            .await
            .map_err(|e| Self::err("schema_types_begin", e))?;

        // The CAS read lives inside the transaction, which is what makes it a compare-
        // and-swap rather than a check followed by a hopeful write.
        let mut response = tx_try!(
            tx,
            query tx
                .query(format!(
                    "{SELECT} FROM knowledge_ontology WHERE user_id = $uid LIMIT 1"
                ))
                .bind(("uid", user_id.to_string())),
            "schema_types_read"
        );
        let rows: Vec<KnowledgeOntology> = tx_try!(tx, response.take(0), "schema_types_read_take");
        let current = rows.into_iter().next();
        if current.as_ref().map(|o| o.version).unwrap_or(0) != expected_version {
            let _ = tx.cancel().await;
            return Ok(false);
        }

        let new_version = expected_version + 1;
        match current {
            Some(existing) => {
                tx_try!(
                    tx,
                    query tx.query(
                        "UPDATE type::record('knowledge_ontology', $id)
                         SET owl = $owl, format = $fmt, version = $ver, updated_at = $now",
                    )
                    .bind(("id", existing.id))
                    .bind(("owl", owl.to_string()))
                    .bind(("fmt", format.to_string()))
                    .bind(("ver", new_version))
                    .bind(("now", now)),
                    "schema_types_ontology"
                );
            }
            None => {
                tx_try!(
                    tx,
                    query tx
                        .query(
                            "CREATE type::record('knowledge_ontology', $id) SET
                                user_id = $uid, owl = $owl, format = $fmt, version = $ver,
                                effective_ontology = '', seeds = [], sources = [],
                                catalog_fingerprint = '', updated_at = $now",
                        )
                        .bind(("id", new_id()))
                        .bind(("uid", user_id.to_string()))
                        .bind(("owl", owl.to_string()))
                        .bind(("fmt", format.to_string()))
                        .bind(("ver", new_version))
                        .bind(("now", now)),
                    "schema_types_ontology"
                );
            }
        }

        // Extraction candidates become live entities only inside the same transaction as
        // the T-box declarations and mapped A-box assertions that make them valid.
        for offered in materialize {
            let mut response = tx_try!(
                tx,
                query tx.query(
                    "SELECT VALUE meta::id(id) FROM knowledge_entity
                     WHERE user_id = $uid AND path = $path LIMIT 1"
                )
                .bind(("uid", user_id.to_string()))
                .bind(("path", offered.path.clone())),
                "schema_types_page_read"
            );
            let existing: Vec<String> =
                tx_try!(tx, response.take(0), "schema_types_page_read_take");
            match existing {
                ids if ids.is_empty() => {
                    let mut entity = offered.clone();
                    entity.id = new_id();
                    entity.updated_at = now;
                    tx_try!(
                        tx,
                        query tx
                            .query("CREATE type::record('knowledge_entity', $id) CONTENT $entity")
                            .bind(("id", entity.id.clone()))
                            .bind(("entity", entity)),
                        "schema_types_page_create"
                    );
                }
                _ => {
                    tx_try!(
                        tx,
                        query tx.query(
                            "UPDATE knowledge_entity SET aliases = $aliases, attributes = $attrs,
                                 identity_evidence = $identity_evidence,
                                 search_text = $search, updated_at = $now
                             WHERE user_id = $uid AND path = $path"
                        )
                        .bind(("aliases", offered.aliases.clone()))
                        .bind(("attrs", offered.attributes.clone()))
                        .bind(("identity_evidence", offered.identity_evidence.clone()))
                        .bind(("search", offered.search_text.clone()))
                        .bind(("now", now))
                        .bind(("uid", user_id.to_string()))
                        .bind(("path", offered.path.clone())),
                        "schema_types_page_update"
                    );
                }
            }
            for memory_id in &offered.source_memory_ids {
                let mut response = tx_try!(
                    tx,
                    query tx.query(
                        "SELECT VALUE meta::id(id) FROM knowledge_entity_source
                         WHERE user_id = $uid AND memory_id = $memory
                           AND entity_path = $path LIMIT 1"
                    )
                    .bind(("uid", user_id.to_string()))
                    .bind(("memory", memory_id.clone()))
                    .bind(("path", offered.path.clone())),
                    "schema_types_page_source_read"
                );
                let existing_source: Vec<String> =
                    tx_try!(tx, response.take(0), "schema_types_page_source_read_take");
                match existing_source {
                    ids if !ids.is_empty() => {}
                    _ => {
                        let source = KnowledgeEntitySource {
                            id: new_id(),
                            user_id: user_id.to_string(),
                            memory_id: memory_id.clone(),
                            entity_path: offered.path.clone(),
                            created_at: now,
                        };
                        tx_try!(
                            tx,
                            query tx
                                .query("CREATE type::record('knowledge_entity_source', $id) CONTENT $source")
                                .bind(("id", source.id.clone()))
                                .bind(("source", source)),
                            "schema_types_page_source_create"
                        );
                    }
                }
            }
        }
        for (from_entity_path, entity_path, memory_id) in coalesced_sources {
            let mut response = tx_try!(
                tx,
                query tx.query(
                    "SELECT VALUE meta::id(id) FROM knowledge_entity_source
                     WHERE user_id = $uid AND memory_id = $memory
                       AND entity_path = $path LIMIT 1"
                )
                .bind(("uid", user_id.to_string()))
                .bind(("memory", memory_id.clone()))
                .bind(("path", entity_path.clone())),
                "schema_types_coalesced_source_read"
            );
            let duplicate: Vec<String> = tx_try!(
                tx,
                response.take(0),
                "schema_types_coalesced_source_read_take"
            );
            let create = duplicate.is_empty();
            if create {
                let source = KnowledgeEntitySource {
                    id: new_id(),
                    user_id: user_id.to_string(),
                    memory_id: memory_id.clone(),
                    entity_path: entity_path.clone(),
                    created_at: now,
                };
                tx_try!(
                    tx,
                    query tx
                        .query("CREATE type::record('knowledge_entity_source', $id) CONTENT $source")
                        .bind(("id", source.id.clone()))
                        .bind(("source", source)),
                    "schema_types_coalesced_source"
                );
            }
            tx_try!(
                tx,
                query tx.query(
                    "DELETE knowledge_entity_source
                     WHERE user_id = $uid AND memory_id = $memory AND entity_path = $from"
                )
                .bind(("uid", user_id.to_string()))
                .bind(("memory", memory_id.clone()))
                .bind(("from", from_entity_path.clone())),
                "schema_types_coalesced_source_delete"
            );
        }
        for (entity_path, aliases) in coalesced_aliases {
            let mut response = tx_try!(
                tx,
                query tx.query(format!(
                    "{SELECT} FROM knowledge_entity WHERE user_id = $uid AND path = $path LIMIT 1"
                ))
                .bind(("uid", user_id.to_string()))
                .bind(("path", entity_path.clone())),
                "schema_types_coalesced_alias_read"
            );
            let entities: Vec<KnowledgeEntity> =
                tx_try!(tx, response.take(0), "schema_types_coalesced_alias_take");
            let Some(mut canonical) = entities.into_iter().next() else {
                continue;
            };
            canonical.aliases.extend(
                aliases
                    .iter()
                    .filter(|alias| **alias != canonical.name)
                    .cloned(),
            );
            canonical.search_text =
                derive_search_text(&canonical.name, &canonical.description, &canonical.aliases);
            let (search_names, search_name_tokens, mut search_assertions) =
                derive_resolution_search(
                    &canonical.name,
                    &canonical.aliases,
                    &canonical.attributes,
                    std::iter::empty(),
                );
            search_assertions.extend(canonical.search_assertions);
            search_assertions.sort();
            search_assertions.dedup();
            tx_try!(
                tx,
                query tx.query(
                    "UPDATE type::record('knowledge_entity', $id)
                     SET aliases = $aliases, search_text = $search,
                         search_names = $search_names,
                         search_name_tokens = $search_name_tokens,
                         search_assertions = $search_assertions,
                         updated_at = $now"
                )
                .bind(("id", canonical.id))
                .bind(("aliases", canonical.aliases))
                .bind(("search", canonical.search_text))
                .bind(("search_names", search_names))
                .bind(("search_name_tokens", search_name_tokens))
                .bind(("search_assertions", search_assertions))
                .bind(("now", now)),
                "schema_types_coalesced_alias"
            );
        }

        // Resolve may coalesce an already-committed candidate, not only a provisional
        // entity from this extraction window. The canonical materialization above has
        // already copied every source memory, so retire the losing source rows, retarget
        // its graph edges, and remove the entity in this same schema/A-box transaction.
        for (from, canonical) in working_outcomes {
            let Some(into) = canonical else { continue };
            if from == into {
                continue;
            }
            tx_try!(
                tx,
                query tx.query(
                    "DELETE knowledge_entity_source
                         WHERE user_id = $uid AND entity_path = $from
                           AND memory_id IN (
                               SELECT VALUE memory_id FROM knowledge_entity_source
                               WHERE user_id = $uid AND entity_path = $into
                           );
                     UPDATE knowledge_entity_source SET entity_path = $into
                         WHERE user_id = $uid AND entity_path = $from;
                     UPDATE knowledge_entity_link SET from_entity_path = $into
                         WHERE user_id = $uid AND from_entity_path = $from;
                     UPDATE knowledge_entity_link SET to_entity_path = $into
                         WHERE user_id = $uid AND to_entity_path = $from;
                     DELETE knowledge_entity WHERE user_id = $uid AND path = $from"
                )
                .bind(("uid", user_id.to_string()))
                .bind(("from", from.clone()))
                .bind(("into", into.clone())),
                "schema_types_identity_merge"
            );
        }

        for (path, kinds) in types {
            tx_try!(
                tx,
                query tx.query(
                    "UPDATE knowledge_entity SET kinds = $kinds, updated_at = $now
                     WHERE user_id = $uid AND path = $path",
                )
                .bind(("kinds", kinds.clone()))
                .bind(("uid", user_id.to_string()))
                .bind(("path", path.clone()))
                .bind(("now", now)),
                "schema_types_kinds"
            );
        }

        for (path, old, new) in rekeys {
            if old == new {
                continue;
            }
            tx_try!(
                tx,
                query tx.query(
                    "UPDATE knowledge_entity_link SET relation = $new
                     WHERE user_id = $uid AND from_entity_path = $from AND relation = $old
                       AND origin != $inferred",
                )
                .bind(("uid", user_id.to_string()))
                .bind(("from", path.clone()))
                .bind(("old", old.clone()))
                .bind(("new", new.clone()))
                .bind(("inferred", LinkOrigin::Inferred)),
                "schema_types_rekey"
            );
        }
        for (from, to) in relation_type_renames {
            tx_try!(
                tx,
                query tx.query(
                    "UPDATE knowledge_entity_link SET relation = $to
                     WHERE user_id = $uid AND relation = $from AND origin != $inferred"
                )
                .bind(("uid", user_id.to_string()))
                .bind(("from", from.clone()))
                .bind(("to", to.clone()))
                .bind(("inferred", LinkOrigin::Inferred)),
                "schema_types_relation_rename"
            );
        }

        // Attribute decisions: re-key what stays a literal, and turn what named another
        // entity into an edge. Read-modify-write of the whole map rather than per-key
        // updates - SurrealDB has no key-rename, and the map is a handful of entries.
        for ops in attributes {
            if ops.rekeys.is_empty() && ops.promoted.is_empty() && ops.retracted.is_empty() {
                continue;
            }
            let mut response = tx_try!(
                tx,
                query tx
                    .query(format!(
                        "{SELECT} FROM knowledge_entity WHERE user_id = $uid AND path = $path LIMIT 1"
                    ))
                    .bind(("uid", user_id.to_string()))
                    .bind(("path", ops.path.clone())),
                "schema_types_attr_read"
            );
            let rows: Vec<KnowledgeEntity> =
                tx_try!(tx, response.take(0), "schema_types_attr_take");
            let entity = match rows.into_iter().next() {
                Some(page) => page,
                // Merged away by resolve between proposal and commit - its attributes went
                // with it, so there is nothing here to re-key.
                None => continue,
            };
            let mut map = match entity.attributes {
                serde_json::Value::Object(m) => m,
                _ => serde_json::Map::new(),
            };
            for (old, new) in &ops.rekeys {
                if old == new {
                    continue;
                }
                if let Some(v) = map.remove(old) {
                    map.insert(new.clone(), v);
                }
            }
            for (key, relation, target) in &ops.promoted {
                // The literal goes even if the edge already exists: the whole point is
                // that the fact stops being stored twice.
                map.remove(key);
                let mut response = tx_try!(
                    tx,
                    query tx
                        .query(
                            "SELECT VALUE meta::id(id) FROM knowledge_entity_link
                             WHERE user_id = $uid AND from_entity_path = $from AND to_entity_path = $to
                               AND relation = $rel LIMIT 1",
                        )
                        .bind(("uid", user_id.to_string()))
                        .bind(("from", ops.path.clone()))
                        .bind(("to", target.clone()))
                        .bind(("rel", relation.clone())),
                    "schema_types_promote_read"
                );
                let hits: Vec<String> = tx_try!(tx, response.take(0), "schema_types_promote_take");
                if !hits.is_empty() {
                    continue;
                }
                let link = KnowledgeEntityLink {
                    id: new_id(),
                    user_id: user_id.to_string(),
                    from_entity_path: ops.path.clone(),
                    to_entity_path: target.clone(),
                    relation: relation.clone(),
                    source_memory_ids: Vec::new(),
                    origin: LinkOrigin::Asserted,
                    created_at: now,
                };
                tx_try!(
                    tx,
                    query tx
                        .query("CREATE type::record('knowledge_entity_link', $id) CONTENT $link")
                        .bind(("id", link.id.clone()))
                        .bind(("link", link)),
                    "schema_types_promote_create"
                );
            }
            for (relation, target) in &ops.retracted {
                tx_try!(
                    tx,
                    query tx
                        .query(
                            "DELETE knowledge_entity_link
                             WHERE user_id = $uid AND from_entity_path = $from AND to_entity_path = $to
                               AND relation = $rel AND origin != $inferred",
                        )
                        .bind(("uid", user_id.to_string()))
                        .bind(("from", ops.path.clone()))
                        .bind(("to", target.clone()))
                        .bind(("rel", relation.clone()))
                        .bind(("inferred", LinkOrigin::Inferred)),
                    "schema_types_retract"
                );
            }
            tx_try!(
                tx,
                query tx
                    .query(
                        "UPDATE knowledge_entity SET attributes = $attrs, updated_at = $now
                         WHERE user_id = $uid AND path = $path",
                    )
                    .bind(("attrs", serde_json::Value::Object(map)))
                    .bind(("uid", user_id.to_string()))
                    .bind(("path", ops.path.clone()))
                    .bind(("now", now)),
                "schema_types_attr_write"
            );
        }

        if let Some(record) = completed_checkpoint {
            // Synchronize the durable overlay with the exact live projection produced by
            // this transaction. Subsequent phases shadow live rows with these working
            // rows, so leaving the extraction-only candidate here would make them read
            // stale, untyped data despite a successful ontology commit.
            let mut overlay_paths: std::collections::BTreeSet<String> = materialize
                .iter()
                .map(|entity| entity.path.clone())
                .collect();
            overlay_paths.extend(types.iter().map(|(path, _)| path.clone()));
            overlay_paths.extend(attributes.iter().map(|ops| ops.path.clone()));
            for offered_path in overlay_paths {
                let mut response = tx_try!(
                    tx,
                    query tx.query(format!(
                        "{SELECT} FROM knowledge_entity WHERE user_id = $uid AND path = $path LIMIT 1"
                    ))
                    .bind(("uid", user_id.to_string()))
                    .bind(("path", offered_path.clone())),
                    "schema_types_overlay_page_read"
                );
                let rows: Vec<KnowledgeEntity> =
                    tx_try!(tx, response.take(0), "schema_types_overlay_page_take");
                let Some(entity) = rows.into_iter().next() else {
                    continue;
                };
                let mut response = tx_try!(
                    tx,
                    query tx.query(
                        "SELECT *, meta::id(id) AS consolidation_entity_id FROM knowledge_consolidation_entity
                         WHERE consolidation_id = $cid AND user_id = $uid AND path = $path LIMIT 1"
                    )
                    .bind(("cid", record.consolidation_id.clone()))
                    .bind(("uid", user_id.to_string()))
                    .bind(("path", offered_path.clone())),
                    "schema_types_overlay_read"
                );
                let rows: Vec<KnowledgeConsolidationEntity> =
                    tx_try!(tx, response.take(0), "schema_types_overlay_take");
                let mut working = match rows.into_iter().next() {
                    Some(row) => row,
                    None => KnowledgeConsolidationEntity::pending(
                        &record.consolidation_id,
                        user_id,
                        &entity.path,
                        entity.category,
                        Vec::new(),
                        entity.source_memory_ids.iter().cloned().collect(),
                    ),
                };
                working.category = entity.category;
                working.lifecycle = crate::db::repo::pkm::ConsolidationEntityLifecycle::Active;
                working.canonical_path = None;
                working.source_memory_ids = entity.source_memory_ids.clone();
                working.entity_id = Some(entity.id.clone());
                working.apply_committed(entity);
                working.rederive_search();
                tx_try!(
                    tx,
                    query tx.query(
                        "UPSERT type::record('knowledge_consolidation_entity', $id) CONTENT $row"
                    )
                    .bind(("id", working.consolidation_entity_id.clone()))
                    .bind(("row", working)),
                    "schema_types_overlay_write"
                );
            }
            for (path, canonical) in working_outcomes {
                if let Some(canonical) = canonical {
                    let mut response = tx_try!(
                        tx,
                        query tx.query(
                            "SELECT *, meta::id(id) AS consolidation_entity_id
                             FROM knowledge_consolidation_entity
                             WHERE consolidation_id = $cid AND user_id = $uid AND path = $path LIMIT 1"
                        )
                        .bind(("cid", record.consolidation_id.clone()))
                        .bind(("uid", user_id.to_string()))
                        .bind(("path", path.clone())),
                        "schema_types_overlay_coalesced_read"
                    );
                    let rows: Vec<KnowledgeConsolidationEntity> =
                        tx_try!(tx, response.take(0), "schema_types_overlay_coalesced_take");
                    let losing = rows.into_iter().next();
                    if let Some(losing) = losing {
                        let mut response = tx_try!(
                            tx,
                            query tx.query(format!(
                                "{SELECT} FROM knowledge_entity
                                 WHERE user_id = $uid AND path = $path LIMIT 1"
                            ))
                            .bind(("uid", user_id.to_string()))
                            .bind(("path", canonical.clone())),
                            "schema_types_overlay_canonical_read"
                        );
                        let entities: Vec<KnowledgeEntity> =
                            tx_try!(tx, response.take(0), "schema_types_overlay_canonical_take");
                        if let Some(mut entity) = entities.into_iter().next() {
                            for evidence in losing.identity_evidence {
                                if !entity.identity_evidence.contains(&evidence) {
                                    entity.identity_evidence.push(evidence);
                                }
                            }
                            tx_try!(
                                tx,
                                query tx.query(
                                    "UPDATE knowledge_entity SET identity_evidence = $evidence, updated_at = $now
                                     WHERE user_id = $uid AND path = $path"
                                )
                                .bind(("evidence", entity.identity_evidence))
                                .bind(("now", now))
                                .bind(("uid", user_id.to_string()))
                                .bind(("path", canonical.clone())),
                                "schema_types_overlay_canonical_identity"
                            );
                        }
                    }
                    tx_try!(
                        tx,
                        query tx.query(
                            "UPDATE knowledge_consolidation_entity SET lifecycle = $lifecycle,
                                 searchable = false, canonical_path = $canonical, updated_at = $now
                             WHERE consolidation_id = $cid AND user_id = $uid AND path = $path"
                        )
                        .bind(("lifecycle", ConsolidationEntityLifecycle::Coalesced))
                        .bind(("canonical", canonical.clone()))
                        .bind(("now", now))
                        .bind(("cid", record.consolidation_id.clone()))
                        .bind(("uid", user_id.to_string()))
                        .bind(("path", path.clone())),
                        "schema_types_overlay_coalesced_write"
                    );
                    continue;
                }
                let lifecycle = crate::db::repo::pkm::ConsolidationEntityLifecycle::Discarded;
                let searchable = false;
                tx_try!(
                    tx,
                    query tx.query(
                        "UPDATE knowledge_consolidation_entity SET lifecycle = $lifecycle,
                             searchable = $searchable, canonical_path = $canonical, updated_at = $now
                         WHERE consolidation_id = $cid AND user_id = $uid AND path = $path"
                    )
                    .bind(("lifecycle", lifecycle))
                    .bind(("searchable", searchable))
                    .bind(("canonical", canonical.clone()))
                    .bind(("now", now))
                    .bind(("cid", record.consolidation_id.clone()))
                    .bind(("uid", user_id.to_string()))
                    .bind(("path", path.clone())),
                    "schema_types_overlay_outcome"
                );
            }
        }

        if let Some(record) = completed_checkpoint {
            tx_try!(
                tx,
                query tx
                    .query("UPSERT type::record('knowledge_consolidation_record', $id) CONTENT $record")
                    .bind(("id", record.id.clone()))
                    .bind(("record", record.clone())),
                "schema_types_checkpoint"
            );
        }

        tx.commit()
            .await
            .map_err(|e| Self::err("schema_types_commit", e))?;
        Ok(true)
    }

    pub async fn ontology_upsert_cas(
        &self,
        user_id: &str,
        owl: &str,
        format: &str,
        expected_version: i64,
    ) -> Result<Option<i64>, AppError> {
        let current = self.ontology_get(user_id).await?;
        let cur_version = current.as_ref().map(|o| o.version).unwrap_or(0);
        if cur_version != expected_version {
            return Ok(None); // CAS miss - stale base, no clobber
        }
        let new_version = expected_version + 1;
        let now = Utc::now();
        match current {
            Some(existing) => {
                for attempt in 0..CONFLICT_RETRIES {
                    let result = async {
                        let mut response = self
                            .db
                            .query(
                                "UPDATE type::record('knowledge_ontology', $id)
                                 SET owl = $owl, format = $fmt, version = $ver, updated_at = $now
                                 WHERE version = $expected
                                 RETURN AFTER",
                            )
                            .bind(("id", existing.id.clone()))
                            .bind(("owl", owl.to_string()))
                            .bind(("fmt", format.to_string()))
                            .bind(("ver", new_version))
                            .bind(("expected", expected_version))
                            .bind(("now", now))
                            .await
                            .and_then(|response| response.check())
                            .map_err(|e| Self::err("ontology_update", e))?;
                        let updated: Vec<surrealdb::types::Value> = response
                            .take(0)
                            .map_err(|e| Self::err("ontology_update_take", e))?;
                        Ok::<_, AppError>(!updated.is_empty())
                    }
                    .await;
                    match result {
                        Err(error)
                            if Self::is_write_conflict(&error)
                                && attempt + 1 < CONFLICT_RETRIES =>
                        {
                            tokio::time::sleep(std::time::Duration::from_millis(5 << attempt))
                                .await;
                        }
                        Ok(true) => return Ok(Some(new_version)),
                        Ok(false) => return Ok(None),
                        Err(error) => return Err(error),
                    }
                }
                unreachable!("the loop returns on its last attempt")
            }
            None => {
                let row = KnowledgeOntology {
                    // Initial writers must contend on the same record key. Relying only
                    // on the unique user_id index permits two concurrent transactions
                    // to both observe an absent index entry and report a successful
                    // create (notably with the in-memory engine). Existing rows retain
                    // their historical generated ids; new rows use the naturally
                    // unique owner id so CREATE provides the missing atomic primitive.
                    id: user_id.to_string(),
                    user_id: user_id.to_string(),
                    owl: owl.to_string(),
                    format: format.to_string(),
                    version: new_version,
                    // The projection is written by `ontology_set_scope`, not here: an
                    // edit changes the delta, and whether it also changes the *cut*
                    // depends on whether it referenced a term the vault did not
                    // already reach. The next load works that out and stores it.
                    effective_ontology: String::new(),
                    seeds: Vec::new(),
                    sources: Vec::new(),
                    catalog_fingerprint: String::new(),
                    updated_at: now,
                };
                let created: Result<Option<surrealdb::types::Value>, _> = self
                    .db
                    .create(("knowledge_ontology", row.id.clone()))
                    .content(row)
                    .await;
                if let Err(error) = created {
                    if self
                        .ontology_get(user_id)
                        .await?
                        .is_some_and(|ontology| ontology.version != expected_version)
                    {
                        return Ok(None);
                    }
                    return Err(Self::err("ontology_insert", error));
                }
            }
        }
        Ok(Some(new_version))
    }

    /// Store the effective ontology a pass reasons over.
    ///
    /// Guarded on `expected_version` but does **not** bump it: the version is the
    /// delta's CAS token, and the cut is derived from the delta plus the vault rather
    /// than authored. A racing edit means this cut was taken against a delta that has
    /// moved, so the write is dropped and the next load re-cuts - losing nothing,
    /// because recomputing is cheap and the stored row is only ever a merge target.
    ///
    /// Creates the row if absent: a user can reach a projection before ever committing
    /// a schema edit, since entity kinds alone seed one.
    pub async fn ontology_set_effective(
        &self,
        user_id: &str,
        expected_version: i64,
        effective_ontology: &str,
        seeds: &[String],
        sources: &[String],
        catalog_fingerprint: &str,
    ) -> Result<bool, AppError> {
        let current = self.ontology_get(user_id).await?;
        if current.as_ref().map(|o| o.version).unwrap_or(0) != expected_version {
            return Ok(false);
        }
        let now = Utc::now();
        match current {
            Some(existing) => {
                self.db
                    .query(
                        "UPDATE type::record('knowledge_ontology', $id)
                         SET effective_ontology = $eff, seeds = $seeds, sources = $sources,
                             catalog_fingerprint = $fp, updated_at = $now",
                    )
                    .bind(("id", existing.id))
                    .bind(("eff", effective_ontology.to_string()))
                    .bind(("seeds", seeds.to_vec()))
                    .bind(("sources", sources.to_vec()))
                    .bind(("fp", catalog_fingerprint.to_string()))
                    .bind(("now", now))
                    .await
                    .map_err(|e| Self::err("ontology_set_effective", e))?;
            }
            None => {
                let row = KnowledgeOntology {
                    id: new_id(),
                    user_id: user_id.to_string(),
                    owl: String::new(),
                    format: "ofn".to_string(),
                    version: 0,
                    effective_ontology: effective_ontology.to_string(),
                    seeds: seeds.to_vec(),
                    sources: sources.to_vec(),
                    catalog_fingerprint: catalog_fingerprint.to_string(),
                    updated_at: now,
                };
                let _: Option<surrealdb::types::Value> = self
                    .db
                    .create(("knowledge_ontology", row.id.clone()))
                    .content(row)
                    .await
                    .map_err(|e| Self::err("ontology_set_effective_insert", e))?;
            }
        }
        Ok(true)
    }
}
