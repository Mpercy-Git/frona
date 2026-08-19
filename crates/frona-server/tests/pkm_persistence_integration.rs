//! PKM persistence round-trip: the memory-sync fields (`disposition`,
//! `ended_at`/`erroneous_at`, `source` - including the `External { note }` path -
//! on `KnowledgeMemory`; `origin`, `rev` on `KnowledgeEntity`) must survive a
//! create→read cycle through `SurrealValue` + the `SELECT *, meta::id(id) as id`
//! projection, including the new enums with `surreal(lowercase)`.

use chrono::Utc;
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

use frona::memory::pkm::model::{
    Disposition, EvidenceSource, EvidenceStrength, KnowledgeMemory, KnowledgeEntity,
    KnowledgeEntitySource, MemoryEvidence, MemoryKind, EntityCategory, EntityOrigin,
};
use frona::db::repo::pkm::PkmRepo;

fn evidence(source: EvidenceSource) -> Vec<MemoryEvidence> {
    vec![MemoryEvidence { strength: EvidenceStrength::Explicit, source }]
}

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    frona::db::init::setup_schema(&db).await.unwrap();
    db
}

#[tokio::test]
async fn author_eligibility_uses_entity_memory_membership_and_keeps_the_self_entity() {
    let db = test_db().await;
    let repo = PkmRepo::new(db.clone(), 10);

    for (path, category) in [
        ("people/me", EntityCategory::Concept),
        ("entities/unreferenced-shell", EntityCategory::Concept),
        ("procedures/restart-service", EntityCategory::Playbook),
    ] {
        repo.upsert_entity_skeleton(
            "u1", path, category, &[], path, "pending authoring", &[],
        ).await.unwrap();
    }

    let source = KnowledgeEntitySource {
        id: "source-1".into(),
        user_id: "u1".into(),
        memory_id: "memory-1".into(),
        entity_path: "procedures/restart-service".into(),
        created_at: Utc::now(),
    };
    let _: Option<surrealdb::types::Value> = db
        .create(("knowledge_entity_source", source.id.clone()))
        .content(source)
        .await
        .unwrap();

    let concepts = repo.entities_needing_reconciliation_by_category(
        "u1", EntityCategory::Concept,
    ).await.unwrap();
    assert!(concepts.contains(&"people/me".to_string()), "the self page is always authored");
    assert!(!concepts.contains(&"entities/unreferenced-shell".to_string()),
        "an ordinary page without a memory remains ineligible");

    let playbooks = repo.entities_needing_reconciliation_by_category(
        "u1", EntityCategory::Playbook,
    ).await.unwrap();
    assert_eq!(playbooks, vec!["procedures/restart-service".to_string()],
        "playbook membership lives in knowledge_entity_source, not page assertion provenance");
}

const SELECT: &str = "SELECT *, meta::id(id) as id";

#[tokio::test]
async fn web_evidence_round_trips_without_copying_the_execution_payload() {
    let db = test_db().await;
    let mem = KnowledgeMemory {
        id:"web-memory".into(),user_id:"u1".into(),created_at:Utc::now(),kind:MemoryKind::Fact,
        episode:None,content:"Acme released 4.2".into(),relations:Vec::new(),
        disposition:Disposition::None,ended_at:None,erroneous_at:None,comment:None,
        evidence:evidence(EvidenceSource::WebSearch {
            message_id:"m1".into(),chat_id:"c1".into(),tool_call_id:"tool-1".into(),
            quote:"Acme 4.2 is available".into(),query:Some("Acme 4.2".into()),
            url:Some("https://acme.example/releases/4.2".into()),
        }),
    };
    let _:Option<surrealdb::types::Value> = db.create(("knowledge_memory","web-memory"))
        .content(mem).await.unwrap();
    let mut result = db.query(format!("{SELECT} FROM knowledge_memory WHERE user_id = 'u1'"))
        .await.unwrap();
    let rows:Vec<KnowledgeMemory> = result.take(0).unwrap();
    assert!(matches!(&rows[0].evidence[0].source,
        EvidenceSource::WebSearch { tool_call_id, url, .. }
            if tool_call_id == "tool-1" && url.as_deref() == Some("https://acme.example/releases/4.2")));
}

#[tokio::test]
async fn knowledge_memory_new_fields_round_trip() {
    let db = test_db().await;
    let ended = Utc::now();
    let mem = KnowledgeMemory {
        id: "m1".into(),
        user_id: "u1".into(),
        created_at: Utc::now(),
        kind: MemoryKind::Fact,
        episode: None,
        content: "Alex worked at Globex".into(),
        relations: Vec::new(),
        disposition: Disposition::Outdated,
        ended_at: Some(ended),
        erroneous_at: None,
        comment: None,
        evidence: evidence(EvidenceSource::ExternalNote {
            note: "Journal/2026-07-10".into(), quote: "Alex worked at Globex".into(),
        }),
    };
    let _: Option<surrealdb::types::Value> = db
        .create(("knowledge_memory", "m1"))
        .content(mem)
        .await
        .unwrap();

    let mut res = db
        .query(format!("{SELECT} FROM knowledge_memory WHERE user_id = 'u1'"))
        .await
        .unwrap();
    let rows: Vec<KnowledgeMemory> = res.take(0).unwrap();
    assert_eq!(rows.len(), 1);
    let got = &rows[0];
    assert_eq!(got.id, "m1");
    assert_eq!(got.disposition, Disposition::Outdated);
    assert!(matches!(&got.evidence[0].source, EvidenceSource::ExternalNote { note, .. } if note == "Journal/2026-07-10"));
    assert!(got.ended_at.is_some(), "ended_at survives round-trip");
    assert!(got.erroneous_at.is_none());
}

/// Empirical proof of the on-disk shape of a data-carrying `SurrealValue` enum,
/// which pins the exact predicate `drop_derived_memories` relies on. A struct
/// variant with no `tag`/`content` stores externally-tagged by its **identifier**
/// (`{ External: { note } }`) - `rename_all = "snake_case"` does *not* lowercase
/// the variant key. So the note path is queryable at `source.External.note`
/// (PascalCase), and `source.external.note` (lowercase) silently matches nothing.
#[tokio::test]
async fn external_source_note_is_queryable_at_pascalcase_nested_path() {
    let db = test_db().await;
    let mem = KnowledgeMemory {
        id: "m1".into(),
        user_id: "u1".into(),
        created_at: Utc::now(),
        kind: MemoryKind::Fact,
        episode: None,
        content: "from a note".into(),
        relations: Vec::new(),
        disposition: Disposition::None,
        ended_at: None,
        erroneous_at: None,
        comment: None,
        evidence: evidence(EvidenceSource::ExternalNote {
            note: "Work Notes/standup".into(), quote: "from a note".into(),
        }),
    };
    let _: Option<surrealdb::types::Value> =
        db.create(("knowledge_memory", "m1")).content(mem).await.unwrap();

    // PascalCase variant key - matches (this is what the repo query uses).
    let mut res = db
        .query("SELECT VALUE meta::id(id) FROM knowledge_memory WHERE evidence[*].source.ExternalNote.note CONTAINS 'Work Notes/standup'")
        .await
        .unwrap();
    let hit: Vec<String> = res.take(0).unwrap();
    assert_eq!(hit, vec!["m1".to_string()], "evidence source ExternalNote matches");

    // Lowercase variant key - matches nothing (the wrong casing would silently
    // never fire, so pin it down as a regression guard).
    let mut res = db
        .query("SELECT VALUE meta::id(id) FROM knowledge_memory WHERE evidence[*].source.external_note.note CONTAINS 'Work Notes/standup'")
        .await
        .unwrap();
    let miss: Vec<String> = res.take(0).unwrap();
    assert!(miss.is_empty(), "source.external.note (lowercase) must NOT match");
}

#[tokio::test]
async fn knowledge_memory_optional_fields_default_to_none() {
    // A memory written by our code with only the required new fields set leaves the
    // Option fields None and disposition/source at their defaults - the common case.
    let db = test_db().await;
    let mem = KnowledgeMemory {
        id: "m2".into(),
        user_id: "u1".into(),
        created_at: Utc::now(),
        kind: MemoryKind::Fact,
        episode: None,
        content: "Alex works at Acme".into(),
        relations: Vec::new(),
        disposition: Disposition::None,
        ended_at: None,
        erroneous_at: None,
        comment: None,
        evidence: evidence(EvidenceSource::AgentMessage {
            message_id: "m2".into(), agent_id: "a1".into(), chat_id: "c1".into(),
            quote: "Alex works at Acme".into(),
        }),
    };
    let _: Option<surrealdb::types::Value> =
        db.create(("knowledge_memory", "m2")).content(mem).await.unwrap();

    let mut res = db
        .query(format!("{SELECT} FROM knowledge_memory WHERE user_id = 'u1'"))
        .await
        .unwrap();
    let rows: Vec<KnowledgeMemory> = res.take(0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].disposition, Disposition::None);
    assert_eq!(
        rows[0].evidence[0].source,
        EvidenceSource::AgentMessage {
            message_id: "m2".into(), agent_id: "a1".into(), chat_id: "c1".into(),
            quote: "Alex works at Acme".into(),
        },
        "Agent source carries agent/chat through the round-trip"
    );
    assert!(rows[0].ended_at.is_none() && rows[0].erroneous_at.is_none());
}

#[tokio::test]
async fn knowledge_entity_origin_and_rev_round_trip() {
    let db = test_db().await;
    let page = KnowledgeEntity {
        id: "p1".into(),
        user_id: "u1".into(),
        path: "Work Notes/standup".into(),
        origin: EntityOrigin::External,
        category: EntityCategory::Concept,
        kinds: Vec::new(),
        name: "Standup".into(),
        description: String::new(),
        identity_evidence: vec![MemoryEvidence {
            strength: EvidenceStrength::Explicit,
            source: EvidenceSource::HumanEdit {
                page_path: "Work Notes/standup".into(),
                quote: "Standup".into(),
            },
        }],
        attribute_sources: Vec::new(),
        source_memory_ids: Vec::new(),
        body: "raw user note".into(),
        sync_content: None,
        mirrored_rev: None,
        extracted_rev: None,
        related_playbooks: Vec::new(),
        search_text: "Standup".into(),
        search_names: Vec::new(), search_name_tokens: Vec::new(), search_assertions: Vec::new(),
        attributes: serde_json::json!({}),
        use_count: 0,
        aliases: Default::default(),
        rev: Some("abc123".into()),
        updated_at: Utc::now(),
        rendered_at: Utc::now(),
    };
    let _: Option<surrealdb::types::Value> = db
        .create(("knowledge_entity", "p1"))
        .content(page)
        .await
        .unwrap();

    let mut res = db
        .query(format!("{SELECT} FROM knowledge_entity WHERE user_id = 'u1'"))
        .await
        .unwrap();
    let rows: Vec<KnowledgeEntity> = res.take(0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].origin, EntityOrigin::External);
    assert_eq!(rows[0].rev.as_deref(), Some("abc123"));
    assert_eq!(rows[0].path, "Work Notes/standup");
    assert_eq!(rows[0].identity_evidence.len(), 1);
}

#[tokio::test]
async fn knowledge_entity_internal_origin_and_no_rev_round_trip() {
    // An Internal page with no rev yet (the state right after skeleton creation).
    let db = test_db().await;
    let page = KnowledgeEntity {
        id: "p2".into(),
        user_id: "u1".into(),
        path: "people/bob".into(),
        origin: EntityOrigin::Internal,
        category: EntityCategory::Concept,
        kinds: vec!["https://schema.org/Person".into()],
        name: "Bob".into(),
        description: String::new(),
        identity_evidence: Vec::new(),
        attribute_sources: Vec::new(),
        source_memory_ids: Vec::new(),
        body: String::new(),
        sync_content: None,
        mirrored_rev: None,
        extracted_rev: None,
        related_playbooks: Vec::new(),
        search_text: "Bob".into(),
        search_names: Vec::new(), search_name_tokens: Vec::new(), search_assertions: Vec::new(),
        attributes: serde_json::json!({}),
        use_count: 0,
        aliases: Default::default(),
        rev: None,
        updated_at: Utc::now(),
        rendered_at: Utc::now(),
    };
    let _: Option<surrealdb::types::Value> =
        db.create(("knowledge_entity", "p2")).content(page).await.unwrap();

    let mut res = db
        .query(format!("{SELECT} FROM knowledge_entity WHERE user_id = 'u1'"))
        .await
        .unwrap();
    let rows: Vec<KnowledgeEntity> = res.take(0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].origin, EntityOrigin::Internal);
    assert!(rows[0].rev.is_none());
    assert_eq!(rows[0].path, "people/bob");
}
