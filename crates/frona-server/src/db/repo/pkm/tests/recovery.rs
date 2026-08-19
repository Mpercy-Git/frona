#[tokio::test]
async fn fatal_checkpoint_restarts_once_then_fails_and_clears_working_rows() {
    use crate::db::repo::pkm::{
        ConsolidationEntityLifecycle, KnowledgeConsolidationEntity, PkmConsolidationStore,
    };
    use crate::memory::pkm::{ConsolidationStageState, PendingEntityContribution};

    let committed = std::sync::Arc::new(repo().await);
    let effective = PkmConsolidationStore::new(committed.clone()).scoped("run-recovery", "u");
    effective
        .upsert_entity(KnowledgeConsolidationEntity::pending(
            "run-recovery",
            "u",
            "services/postgres",
            EntityCategory::Concept,
            vec![PendingEntityContribution {
                name: "Postgres".into(),
                description: "database".into(),
                aliases: Default::default(),
                attributes: serde_json::json!({"port": "5432"}),
                attribute_evidence: Default::default(),
                source_memory_ids: ["memory-1".to_string()].into_iter().collect(),
                existing_only: false,
                occurrence_count: 1,
            }],
            ["memory-1".to_string()].into_iter().collect(),
        ))
        .await
        .unwrap();
    let record = KnowledgeConsolidationRecord {
        id: new_id(),
        consolidation_id: "run-recovery".into(),
        user_id: "u".into(),
        state: ConsolidationStageState::PageAuthor,
        stats: Default::default(),
        attempts: 3,
        restart_count: 0,
        failure: None,
        next_attempt_at: Utc::now(),
        updated_at: Utc::now(),
    };
    committed.save_consolidation_record(&record).await.unwrap();

    let restarted = committed
        .recover_or_fail_consolidation(&record, "invalid graph", 2)
        .await
        .unwrap();
    assert_eq!(restarted.restart_count, 1);
    let ConsolidationStageState::Classify(_) = &restarted.state else {
        panic!("first fatal failure must restart at Classify");
    };
    let row = effective
        .working_entity("services/postgres")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.lifecycle, ConsolidationEntityLifecycle::Pending);
    assert!(row.entity_id.is_none());
    assert!(row.kinds.is_empty());

    let failed = committed
        .recover_or_fail_consolidation(&restarted, "invalid again", 2)
        .await
        .unwrap();
    assert!(matches!(failed.state, ConsolidationStageState::Failed));
    assert_eq!(failed.restart_count, 2);
    assert!(
        failed
            .failure
            .as_ref()
            .is_some_and(|failure| failure.error == "invalid again")
    );
    assert!(
        effective
            .working_entity("services/postgres")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !committed
            .users_with_open_consolidation()
            .await
            .unwrap()
            .contains(&"u".to_string())
    );
}

#[tokio::test]
async fn deleting_a_consolidation_record_cascades_its_working_entities() {
    use crate::db::repo::pkm::{KnowledgeConsolidationEntity, PkmConsolidationStore};
    use crate::memory::pkm::{ConsolidationStageState, PendingEntityContribution};

    let committed = std::sync::Arc::new(repo().await);
    let record = KnowledgeConsolidationRecord {
        id: new_id(),
        consolidation_id: "cascade-run".into(),
        user_id: "u".into(),
        state: ConsolidationStageState::PageAuthor,
        stats: Default::default(),
        attempts: 0,
        restart_count: 0,
        failure: None,
        next_attempt_at: Utc::now(),
        updated_at: Utc::now(),
    };
    committed.save_consolidation_record(&record).await.unwrap();
    let effective = PkmConsolidationStore::new(committed.clone()).scoped("cascade-run", "u");
    effective
        .upsert_entity(KnowledgeConsolidationEntity::pending(
            "cascade-run",
            "u",
            "services/postgres",
            EntityCategory::Concept,
            vec![PendingEntityContribution {
                name: "Postgres".into(),
                description: "database".into(),
                aliases: Default::default(),
                attributes: serde_json::json!({}),
                attribute_evidence: Default::default(),
                source_memory_ids: Default::default(),
                existing_only: false,
                occurrence_count: 1,
            }],
            Default::default(),
        ))
        .await
        .unwrap();
    committed
        .delete_consolidation_record(&record.id)
        .await
        .unwrap();
    assert!(
        effective
            .working_entity("services/postgres")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn cleanup_deletes_working_rows_and_marks_done_atomically() {
    use crate::db::repo::pkm::{KnowledgeConsolidationEntity, PkmConsolidationStore};
    use crate::memory::pkm::{ConsolidationStageState, PendingEntityContribution};

    let committed = std::sync::Arc::new(repo().await);
    let mut record = KnowledgeConsolidationRecord {
        id: new_id(),
        consolidation_id: "cleanup-run".into(),
        user_id: "u".into(),
        state: ConsolidationStageState::Cleanup,
        stats: Default::default(),
        attempts: 0,
        restart_count: 0,
        failure: None,
        next_attempt_at: Utc::now(),
        updated_at: Utc::now(),
    };
    committed.save_consolidation_record(&record).await.unwrap();
    let effective = PkmConsolidationStore::new(committed.clone()).scoped("cleanup-run", "u");
    effective
        .upsert_entity(KnowledgeConsolidationEntity::pending(
            "cleanup-run",
            "u",
            "services/postgres",
            EntityCategory::Concept,
            vec![PendingEntityContribution {
                name: "Postgres".into(),
                description: "database".into(),
                aliases: Default::default(),
                attributes: serde_json::json!({}),
                attribute_evidence: Default::default(),
                source_memory_ids: Default::default(),
                existing_only: false,
                occurrence_count: 1,
            }],
            Default::default(),
        ))
        .await
        .unwrap();
    record.state = ConsolidationStageState::Done;
    committed.complete_consolidation(&record).await.unwrap();
    assert!(
        effective
            .working_entity("services/postgres")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        committed
            .latest_consolidation_record("u")
            .await
            .unwrap()
            .unwrap()
            .state,
        ConsolidationStageState::Done,
    ));
}
use super::*;
