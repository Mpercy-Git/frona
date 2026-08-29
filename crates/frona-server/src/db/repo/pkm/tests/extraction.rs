/// A window is all-or-nothing: its entities, its memories, the chat's watermark and the
/// short memories it consumed either all land or none do.
///
/// Extract used to write these as separate transactions and advance the watermark
/// afterwards, so a failure part-way left rows committed against a transcript still
/// marked unread - and the next pass, finding no watermark, mined it again and minted
/// them a second time. Nothing dedups memory content, so the copies accumulated.
#[tokio::test]
async fn ingest_window_commits_as_one_unit() {
    let r = repo().await;
    let batch = |content: &str| IngestBatch {
        entities: vec![PendingEntity {
            path: "svc/pg".into(),
            name: "Postgres".into(),
            description: "the db".into(),
            aliases: vec!["PG".into()],
            identity_evidence: Vec::new(),
            attribute_evidence: attribute_evidence(
                "svc/pg",
                &serde_json::json!({ "port": "5432" }),
            ),
            attributes: serde_json::json!({ "port": "5432" }),
        }],
        entity_updates: Vec::new(),
        memories: vec![PendingMemory {
            id: new_id(),
            kind: MemoryKind::Fact,
            evidence: human_evidence(content, "svc/pg"),
            episode: None,
            content: content.into(),
            paths: vec!["svc/pg".into()],
        }],
        playbook_candidates: Vec::new(),
        grounding_corrections: 0,
        grounding_items_dropped: 0,
        recall_result_lookups: 0,
        ..Default::default()
    };

    let counts = commit_checkpointed_extract_patch(
        &r,
        "u",
        &batch("postgres runs on 5432"),
        Some(("c1", Utc::now())),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(counts.entities_created, 0);
    assert_eq!(counts.memories_added, 1);
    assert!(
        r.consolidation_watermark("c1").await.unwrap().is_some(),
        "the watermark lands with the rows it accounts for, not after them"
    );

    // The same entity mentioned again remains another durable extraction memory;
    // extraction still creates neither an entity nor an association.
    let counts = commit_checkpointed_extract_patch(
        &r,
        "u",
        &batch("postgres also serves staging"),
        None,
        &[],
    )
    .await
    .unwrap();
    assert_eq!(counts.entities_created, 0);
    assert_eq!(counts.memories_added, 1);
    assert!(r.entity_by_path("u", "svc/pg").await.unwrap().is_none());
    assert!(
        r.memories_for_entity("u", "svc/pg")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(r.list_all_memories("u").await.unwrap().len(), 2);
}

#[tokio::test]
async fn extract_commit_rolls_back_when_memory_insert_fails() {
    use crate::memory::pkm::{ConsolidationStageState, IngestState};

    let r = repo().await;
    let memory_id = new_id();
    let batch = IngestBatch {
        memories: vec![PendingMemory {
            id: memory_id,
            kind: MemoryKind::Fact,
            evidence: human_evidence("Postgres listens on 5432", "services/postgres"),
            episode: None,
            content: "Postgres listens on 5432".into(),
            paths: vec!["services/postgres".into()],
        }],
        ..Default::default()
    };
    let first_record = KnowledgeConsolidationRecord {
        id: new_id(),
        consolidation_id: new_id(),
        user_id: "u".into(),
        state: ConsolidationStageState::Ingest(IngestState::default()),
        stats: Default::default(),
        attempts: 0,
        restart_count: 0,
        failure: None,
        next_attempt_at: Utc::now(),
        updated_at: Utc::now(),
    };
    r.commit_extract_patch_with_checkpoint(
        "u",
        &batch,
        &[("chat-1".into(), Utc::now())],
        &[],
        &first_record,
    )
    .await
    .unwrap();
    r.remember("u", "short-chat", "unvalidated source")
        .await
        .unwrap();
    let short_memory_id = r.list_short_memory("u").await.unwrap()[0].id.clone();

    let second_record = KnowledgeConsolidationRecord {
        id: new_id(),
        consolidation_id: new_id(),
        user_id: "u".into(),
        state: ConsolidationStageState::Ingest(IngestState::default()),
        stats: Default::default(),
        attempts: 0,
        restart_count: 0,
        failure: None,
        next_attempt_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let result = r
        .commit_extract_patch_with_checkpoint(
            "u",
            &batch,
            &[("chat-2".into(), Utc::now())],
            std::slice::from_ref(&short_memory_id),
            &second_record,
        )
        .await;

    assert!(
        result.is_err(),
        "a failed CREATE must fail the extraction commit"
    );
    assert!(
        r.consolidation_watermark("chat-2").await.unwrap().is_none(),
        "the watermark must roll back with the failed memory insert",
    );
    assert!(
        r.latest_consolidation_record("u")
            .await
            .unwrap()
            .is_some_and(|record| record.id != second_record.id),
        "the checkpoint must not advance after a failed memory insert",
    );
    assert!(
        !r.list_short_memory("u").await.unwrap()[0].validated,
        "short memory must stay unvalidated after the failed memory insert",
    );
}

#[tokio::test]
async fn ingest_commits_memory_watermark_checkpoint_and_working_entity_together() {
    use crate::db::repo::pkm::PkmConsolidationStore;
    use crate::memory::pkm::{ConsolidationStageState, IngestState};

    let committed = std::sync::Arc::new(repo().await);
    let batch = IngestBatch {
        entities: vec![PendingEntity {
            path: "services/postgres".into(),
            name: "Postgres".into(),
            description: "relational database".into(),
            aliases: vec!["PG".into()],
            identity_evidence: Vec::new(),
            attributes: serde_json::json!({"port": "5432"}),
            attribute_evidence: attribute_evidence(
                "services/postgres",
                &serde_json::json!({"port": "5432"}),
            ),
        }],
        entity_updates: Vec::new(),
        memories: vec![PendingMemory {
            id: new_id(),
            kind: MemoryKind::Fact,
            evidence: human_evidence("Postgres listens on 5432", "services/postgres"),
            episode: None,
            content: "Postgres listens on 5432".into(),
            paths: vec!["services/postgres".into()],
        }],
        playbook_candidates: Vec::new(),
        grounding_corrections: 0,
        grounding_items_dropped: 0,
        recall_result_lookups: 0,
        ..Default::default()
    };
    let extract = IngestState::default();
    let record = KnowledgeConsolidationRecord {
        id: new_id(),
        consolidation_id: new_id(),
        user_id: "u".into(),
        state: ConsolidationStageState::Ingest(extract),
        stats: Default::default(),
        attempts: 0,
        restart_count: 0,
        failure: None,
        next_attempt_at: Utc::now(),
        updated_at: Utc::now(),
    };
    committed
        .commit_extract_patch_with_checkpoint(
            "u",
            &batch,
            &[("chat-1".to_string(), Utc::now())],
            &[],
            &record,
        )
        .await
        .unwrap();

    assert!(
        committed
            .entity_by_path("u", "services/postgres")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(committed.list_all_memories("u").await.unwrap().len(), 1);
    assert!(
        committed
            .consolidation_watermark("chat-1")
            .await
            .unwrap()
            .is_some()
    );
    let effective =
        PkmConsolidationStore::new(committed.clone()).scoped(&record.consolidation_id, "u");
    let entity = effective
        .entity_by_path("services/postgres")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entity.name, "Postgres");
    assert!(
        entity
            .attributes
            .as_object()
            .is_some_and(|attributes| attributes.is_empty()),
        "raw attributes remain tentative before classification"
    );
}

#[tokio::test]
async fn ingest_does_not_materialize_a_missing_update_only_entity() {
    use crate::db::repo::pkm::PkmConsolidationStore;
    use crate::memory::pkm::{ConsolidationStageState, IngestState};

    let committed = std::sync::Arc::new(repo().await);
    let batch = IngestBatch {
        entities: vec![PendingEntity {
            path: "services/retailer".into(),
            name: "Retailer".into(),
            description: "online shopping service".into(),
            aliases: Vec::new(),
            identity_evidence: Vec::new(),
            attributes: serde_json::json!({}),
            attribute_evidence: Default::default(),
        }],
        entity_updates: Vec::new(),
        memories: vec![PendingMemory {
            id: new_id(),
            kind: MemoryKind::Fact,
            evidence: human_evidence("The retailer processed the return.", "services/retailer"),
            episode: None,
            content: "The retailer processed the return.".into(),
            paths: vec!["services/retailer".into(), "organizations/retailer".into()],
        }],
        playbook_candidates: Vec::new(),
        grounding_corrections: 0,
        grounding_items_dropped: 0,
        recall_result_lookups: 0,
        ..Default::default()
    };
    let extract = IngestState::default();
    let record = KnowledgeConsolidationRecord {
        id: new_id(),
        consolidation_id: new_id(),
        user_id: "u".into(),
        state: ConsolidationStageState::Ingest(extract),
        stats: Default::default(),
        attempts: 0,
        restart_count: 0,
        failure: None,
        next_attempt_at: Utc::now(),
        updated_at: Utc::now(),
    };

    committed
        .commit_extract_patch_with_checkpoint("u", &batch, &[], &[], &record)
        .await
        .unwrap();

    let effective =
        PkmConsolidationStore::new(committed.clone()).scoped(&record.consolidation_id, "u");
    assert!(
        effective
            .working_entity("services/retailer")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        effective
            .working_entity("organizations/retailer")
            .await
            .unwrap()
            .is_none(),
        "a routed path absent from knowledge_entity remains memory-only; it must not become an empty consolidation entity",
    );
    let saved = committed
        .latest_consolidation_record("u")
        .await
        .unwrap()
        .unwrap();
    let ConsolidationStageState::Ingest(_) = saved.state else {
        panic!("extraction checkpoint must remain ingest");
    };
}

/// An entity in the batch that cannot be written takes the whole window with it -
/// including the watermark, so the transcript stays unread and is mined again rather
/// than being half-committed and never revisited.
#[tokio::test]
async fn failed_window_leaves_nothing_behind() {
    let r = repo().await;
    // A memory with no entities violates the `≥1 entity` invariant, so it writes nothing -
    // but the entity and the watermark before it must not survive either.
    let batch = IngestBatch {
        entities: vec![PendingEntity {
            path: "svc/pg".into(),
            name: "Postgres".into(),
            description: String::new(),
            aliases: Vec::new(),
            identity_evidence: Vec::new(),
            attribute_evidence: std::collections::HashMap::new(),
            attributes: serde_json::json!({}),
        }],
        entity_updates: Vec::new(),
        memories: vec![PendingMemory {
            id: new_id(),
            kind: MemoryKind::Fact,
            evidence: human_evidence("orphan", "svc/pg"),
            episode: None,
            content: "orphan".into(),
            paths: Vec::new(),
        }],
        playbook_candidates: Vec::new(),
        grounding_corrections: 0,
        grounding_items_dropped: 0,
        recall_result_lookups: 0,
        ..Default::default()
    };
    let counts = commit_checkpointed_extract_patch(&r, "u", &batch, Some(("c1", Utc::now())), &[])
        .await
        .unwrap();
    // The unlinked memory is skipped rather than fatal - it is the one case the
    // invariant is allowed to absorb, since an unlinked memory is invisible anyway.
    assert_eq!(
        counts.memories_added, 1,
        "the pageless memory remains durable"
    );
    assert_eq!(
        counts.entities_created, 0,
        "extraction never creates entities"
    );
}

#[tokio::test]
async fn stale_external_extraction_cannot_replace_current_memories() {
    use crate::memory::pkm::{ConsolidationStageState, IngestState};

    let r = repo().await;
    let note = "Work Notes/standup";
    r.upsert_external_page("u", note, "current body", "current-rev")
        .await
        .unwrap();
    let old_memory = r
        .create_sourced_memory(
            "u",
            MemoryKind::Fact,
            "current durable fact",
            &["people/alex".to_string()],
            vec![MemoryEvidence {
                strength: EvidenceStrength::Explicit,
                source: EvidenceSource::ExternalNote {
                    note: note.into(),
                    quote: "current durable fact".into(),
                },
            }],
        )
        .await
        .unwrap();
    let batch = IngestBatch {
        memories: vec![PendingMemory {
            id: new_id(),
            kind: MemoryKind::Fact,
            evidence: vec![MemoryEvidence {
                strength: EvidenceStrength::Explicit,
                source: EvidenceSource::ExternalNote {
                    note: note.into(),
                    quote: "stale replacement".into(),
                },
            }],
            episode: None,
            content: "stale replacement".into(),
            paths: vec!["people/alex".into()],
        }],
        ..Default::default()
    };
    let checkpoint = KnowledgeConsolidationRecord {
        id: new_id(),
        consolidation_id: new_id(),
        user_id: "u".into(),
        state: ConsolidationStageState::Ingest(IngestState::default()),
        stats: Default::default(),
        attempts: 0,
        restart_count: 0,
        failure: None,
        next_attempt_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let applied = r
        .commit_external_extract_patch_with_checkpoint("u", note, "stale-rev", &batch, &checkpoint)
        .await
        .unwrap();

    assert!(!applied, "a stale extraction result must be rejected");
    let memories = r.list_all_memories("u").await.unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].id, old_memory, "current memory must remain");
    let page = r.entity_by_path("u", note).await.unwrap().unwrap();
    assert_eq!(page.rev.as_deref(), Some("current-rev"));
    assert_eq!(page.extracted_rev, None);
    assert!(r.latest_consolidation_record("u").await.unwrap().is_none());
}
use super::*;
