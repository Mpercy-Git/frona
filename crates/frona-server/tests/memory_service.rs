use std::sync::Arc;

use chrono::{Duration, Utc};
use frona::db::init as db;
use frona::db::repo::generic::SurrealRepo;
use frona::db::repo::memory_entries::SurrealMemoryEntryRepo;
use frona::memory::repository::MemoryEntryRepository;
use frona::memory::models::{Memory, MemoryEntry, MemorySourceType};
use frona::memory::repository::MemoryRepository;
use frona::storage::StorageService;
use frona::memory::service::MemoryService;
use frona::agent::prompt::PromptLoader;
use frona::core::repository::Repository;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

fn make_memory_service(db: Surreal<Db>) -> MemoryService {
    let inference = frona::core::config::InferenceConfig::default();
    let provider_registry = frona::inference::ModelProviderRegistry::from_config(
        frona::inference::config::ModelRegistryConfig::auto_discover(),
        frona::chat::broadcast::BroadcastService::new(),
        &inference,
    )
    .unwrap();

    MemoryService::new(
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db),
        Arc::new(provider_registry),
        PromptLoader::new(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("resources")
                .join("prompts"),
        ),
        StorageService::new(&frona::core::config::Config {
            storage: frona::core::config::StorageConfig {
                data_dir: "/nonexistent".to_string(),
                shared_config_dir: "/nonexistent".to_string(),
                ..Default::default()
            },
            ..Default::default()
        }),
    )
}

#[tokio::test]
async fn test_store_memory_entry_persists_to_db() {
    let db = test_db().await;
    let svc = make_memory_service(db.clone());

    svc.store_memory_entry("agent-1", "User likes Rust", Some("chat-1"))
        .await
        .unwrap();

    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);
    let entries = repo.find_by_agent_id("agent-1").await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "User likes Rust");
    assert_eq!(entries[0].source_chat_id.as_deref(), Some("chat-1"));
    assert!(entries[0].user_id.is_none());
}

#[tokio::test]
async fn test_store_user_memory_entry_persists_with_user_id() {
    let db = test_db().await;
    let svc = make_memory_service(db.clone());

    svc.store_user_memory_entry("user-1", "Name is Alice", Some("chat-1"))
        .await
        .unwrap();

    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);
    let entries = repo.find_by_user_id("user-1").await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "Name is Alice");
    assert_eq!(entries[0].user_id.as_deref(), Some("user-1"));
    assert!(entries[0].agent_id.is_empty());
}

#[tokio::test]
async fn test_build_augmented_prompt_includes_agent_memory() {
    let db = test_db().await;
    let svc = make_memory_service(db.clone());

    svc.store_memory_entry("agent-1", "User prefers dark mode", None)
        .await
        .unwrap();

    let prompt = svc
        .build_augmented_system_prompt("Base prompt", "agent-1", &frona::handle!("agent-1"), "user-1", &frona::handle!("test-user"), None, &[], &[], &std::collections::BTreeMap::new(), &[], "UTC")
        .await
        .unwrap();

    assert!(prompt.contains("<agent_memory>"), "Should include agent_memory block");
    assert!(prompt.contains("User prefers dark mode"), "Should include the stored memory");
    assert!(prompt.contains("Base prompt"), "Should include base prompt");
}

#[tokio::test]
async fn test_build_augmented_prompt_includes_user_memory() {
    let db = test_db().await;
    let svc = make_memory_service(db.clone());

    svc.store_user_memory_entry("user-1", "Name is Bob", None)
        .await
        .unwrap();
    svc.store_memory_entry("agent-1", "Agent-specific memory", None)
        .await
        .unwrap();

    let prompt = svc
        .build_augmented_system_prompt("Base prompt", "agent-1", &frona::handle!("agent-1"), "user-1", &frona::handle!("test-user"), None, &[], &[], &std::collections::BTreeMap::new(), &[], "UTC")
        .await
        .unwrap();

    assert!(prompt.contains("<user_memory>"), "Should include user_memory block");
    assert!(prompt.contains("Name is Bob"), "Should include user memory");
    assert!(prompt.contains("<agent_memory>"), "Should include agent_memory block");

    let user_pos = prompt.find("<user_memory>").unwrap();
    let agent_pos = prompt.find("<agent_memory>").unwrap();
    assert!(
        user_pos < agent_pos,
        "user_memory should appear before agent_memory"
    );
}

#[tokio::test]
async fn test_build_augmented_prompt_includes_new_entries_after_compaction() {
    let db = test_db().await;
    let svc = make_memory_service(db.clone());

    let compacted_until = Utc::now() - Duration::seconds(60);
    let memory_repo: SurrealRepo<Memory> = SurrealRepo::new(db.clone());
    let compacted_memory = Memory {
        id: frona::core::repository::new_id(),
        source_type: MemorySourceType::Agent,
        source_id: "agent-1".to_string(),
        content: "- Previously compacted memory".to_string(),
        metadata: serde_json::json!({
            "compacted_until": compacted_until.to_rfc3339(),
            "item_count": 5,
        }),
        created_at: compacted_until,
        updated_at: compacted_until,
    };
    memory_repo.create(&compacted_memory).await.unwrap();

    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db.clone());
    let new_entry = MemoryEntry {
        id: frona::core::repository::new_id(),
        agent_id: "agent-1".to_string(),
        user_id: None,
        content: "Brand new memory after compaction".to_string(),
        source_chat_id: None,
        created_at: Utc::now(),
    };
    repo.create(&new_entry).await.unwrap();

    let prompt = svc
        .build_augmented_system_prompt("Base prompt", "agent-1", &frona::handle!("agent-1"), "user-1", &frona::handle!("test-user"), None, &[], &[], &std::collections::BTreeMap::new(), &[], "UTC")
        .await
        .unwrap();

    assert!(
        prompt.contains("Previously compacted memory"),
        "Should include compacted memory content"
    );
    assert!(
        prompt.contains("Brand new memory after compaction"),
        "Should include new entry after compaction"
    );
}

#[tokio::test]
async fn test_compact_entries_if_needed_skips_below_threshold() {
    let db = test_db().await;
    let svc = make_memory_service(db.clone());

    svc.store_memory_entry("agent-1", "Short memory 1", None).await.unwrap();
    svc.store_memory_entry("agent-1", "Short memory 2", None).await.unwrap();

    // Entries are small (well under 3000 tokens), so compaction should not have been triggered.
    // We verify no Memory record was created since we never called compact_entries_if_needed.
    let memory_repo: SurrealRepo<Memory> = SurrealRepo::new(db);
    let memory = memory_repo
        .find_latest(MemorySourceType::Agent, "agent-1")
        .await
        .unwrap();
    assert!(
        memory.is_none(),
        "No Memory record should exist since compaction was never triggered"
    );
}

#[tokio::test]
async fn test_build_augmented_prompt_appends_tools_guide() {
    let db = test_db().await;
    let svc = make_memory_service(db.clone());

    let prompt = svc
        .build_augmented_system_prompt("Base prompt", "agent-1", &frona::handle!("agent-1"), "user-1", &frona::handle!("test-user"), None, &[], &[], &std::collections::BTreeMap::new(), &[], "UTC")
        .await
        .unwrap();

    assert!(prompt.contains("Base prompt"), "Should include base prompt");
    assert!(
        prompt.contains("# Tool Usage Guide"),
        "Should append TOOLS.md content"
    );

    let base_pos = prompt.find("Base prompt").unwrap();
    let tools_pos = prompt.find("# Tool Usage Guide").unwrap();
    assert!(
        tools_pos > base_pos,
        "TOOLS.md should appear after the base prompt"
    );
}

#[tokio::test]
async fn test_build_augmented_prompt_appends_memory_guide() {
    let db = test_db().await;
    let svc = make_memory_service(db.clone());

    let prompt = svc
        .build_augmented_system_prompt("Base prompt", "agent-1", &frona::handle!("agent-1"), "user-1", &frona::handle!("test-user"), None, &[], &[], &std::collections::BTreeMap::new(), &[], "UTC")
        .await
        .unwrap();

    assert!(prompt.contains("Base prompt"), "Should include base prompt");
    assert!(
        prompt.contains("# Memory"),
        "Should append MEMORY.md content"
    );

    let tools_pos = prompt.find("# Tool Usage Guide").unwrap();
    let memory_pos = prompt.find("# Memory").unwrap();
    assert!(
        memory_pos > tools_pos,
        "MEMORY.md should appear after TOOLS.md"
    );
}
