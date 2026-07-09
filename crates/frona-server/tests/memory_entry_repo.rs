use chrono::{Duration, Utc};
use frona::db::init as db;
use frona::db::repo::generic::SurrealRepo;
use frona::db::repo::memory_entries::SurrealMemoryEntryRepo;
use frona::memory::models::MemoryEntry;
use frona::memory::repository::MemoryEntryRepository;
use frona::core::repository::Repository;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

fn make_entry(agent_id: &str, content: &str, created_at: chrono::DateTime<chrono::Utc>) -> MemoryEntry {
    MemoryEntry {
        id: frona::core::repository::new_id(),
        agent_id: agent_id.to_string(),
        user_id: None,
        content: content.to_string(),
        source_chat_id: None,
        created_at,
    }
}

fn make_user_entry(user_id: &str, content: &str, created_at: chrono::DateTime<chrono::Utc>) -> MemoryEntry {
    MemoryEntry {
        id: frona::core::repository::new_id(),
        agent_id: String::new(),
        user_id: Some(user_id.to_string()),
        content: content.to_string(),
        source_chat_id: None,
        created_at,
    }
}

#[tokio::test]
async fn test_find_by_agent_id_after_returns_newer_entries() {
    let db = test_db().await;
    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);

    let cutoff = Utc::now();
    let before = cutoff - Duration::seconds(60);
    let after = cutoff + Duration::seconds(60);

    let old = make_entry("agent-1", "old entry", before);
    let new = make_entry("agent-1", "new entry", after);

    repo.create(&old).await.unwrap();
    repo.create(&new).await.unwrap();

    let all = repo.find_by_agent_id("agent-1").await.unwrap();
    assert_eq!(all.len(), 2, "find_by_agent_id should return all entries");

    let after_cutoff = repo.find_by_agent_id_after("agent-1", cutoff).await.unwrap();
    assert_eq!(
        after_cutoff.len(),
        1,
        "find_by_agent_id_after should return only entries after cutoff, got {}",
        after_cutoff.len()
    );
    assert_eq!(after_cutoff[0].content, "new entry");
}

#[tokio::test]
async fn test_delete_by_agent_id_before_removes_older_entries() {
    let db = test_db().await;
    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);

    let cutoff = Utc::now();
    let before = cutoff - Duration::seconds(60);
    let after = cutoff + Duration::seconds(60);

    let old = make_entry("agent-1", "old entry", before);
    let new = make_entry("agent-1", "new entry", after);

    repo.create(&old).await.unwrap();
    repo.create(&new).await.unwrap();

    repo.delete_by_agent_id_before("agent-1", cutoff).await.unwrap();

    let remaining = repo.find_by_agent_id("agent-1").await.unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "delete_by_agent_id_before should remove old entries, {} remaining",
        remaining.len()
    );
    assert_eq!(remaining[0].content, "new entry");
}

#[tokio::test]
async fn test_datetime_roundtrip_preserves_value() {
    let db = test_db().await;
    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);

    let now = Utc::now();
    let entry = make_entry("agent-1", "test entry", now);

    repo.create(&entry).await.unwrap();

    let found = repo.find_by_id(&entry.id).await.unwrap().unwrap();
    assert_eq!(found.created_at, now, "DateTime should round-trip exactly");
}

#[tokio::test]
async fn test_find_by_agent_id_after_with_utc_now_boundary() {
    let db = test_db().await;
    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);

    let entry_time = Utc::now();
    let entry = make_entry("agent-1", "stored entry", entry_time);
    repo.create(&entry).await.unwrap();

    let cutoff_before = entry_time - Duration::seconds(1);
    let results = repo.find_by_agent_id_after("agent-1", cutoff_before).await.unwrap();
    assert_eq!(results.len(), 1, "Should find entry created after cutoff (1s before), got {}", results.len());

    let results = repo.find_by_agent_id_after("agent-1", entry_time).await.unwrap();
    assert_eq!(results.len(), 0, "Should NOT find entry with exact same timestamp (strict >), got {}", results.len());

    let cutoff_after = entry_time + Duration::seconds(1);
    let results = repo.find_by_agent_id_after("agent-1", cutoff_after).await.unwrap();
    assert_eq!(results.len(), 0, "Should NOT find entry created before cutoff (1s after), got {}", results.len());
}

#[tokio::test]
async fn test_stored_as_json_queried_as_datetime() {
    let db = test_db().await;
    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);

    let before_store = Utc::now() - Duration::seconds(1);

    let entry = make_entry("agent-1", "an entry", Utc::now());
    repo.create(&entry).await.unwrap();

    let all = repo.find_by_agent_id("agent-1").await.unwrap();
    assert_eq!(all.len(), 1);

    let after_results = repo.find_by_agent_id_after("agent-1", before_store).await.unwrap();
    assert_eq!(
        after_results.len(),
        1,
        "find_by_agent_id_after should find the entry stored after cutoff, got {}. \
         This fails when DateTime is stored as string but queried as native datetime.",
        after_results.len()
    );
}

// User-scoped repository tests

#[tokio::test]
async fn test_find_by_user_id_returns_user_entries() {
    let db = test_db().await;
    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);

    let now = Utc::now();
    let entry1 = make_user_entry("user-1", "user 1 memory A", now);
    let entry2 = make_user_entry("user-1", "user 1 memory B", now + Duration::seconds(1));
    let entry3 = make_user_entry("user-2", "user 2 memory", now);

    repo.create(&entry1).await.unwrap();
    repo.create(&entry2).await.unwrap();
    repo.create(&entry3).await.unwrap();

    let user1_entries = repo.find_by_user_id("user-1").await.unwrap();
    assert_eq!(user1_entries.len(), 2, "Should return 2 entries for user-1");

    let user2_entries = repo.find_by_user_id("user-2").await.unwrap();
    assert_eq!(user2_entries.len(), 1, "Should return 1 entry for user-2");
}

#[tokio::test]
async fn test_find_by_user_id_after_returns_newer_entries() {
    let db = test_db().await;
    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);

    let cutoff = Utc::now();
    let before = cutoff - Duration::seconds(60);
    let after = cutoff + Duration::seconds(60);

    let old = make_user_entry("user-1", "old user entry", before);
    let new = make_user_entry("user-1", "new user entry", after);

    repo.create(&old).await.unwrap();
    repo.create(&new).await.unwrap();

    let results = repo.find_by_user_id_after("user-1", cutoff).await.unwrap();
    assert_eq!(results.len(), 1, "Should return only newer entries");
    assert_eq!(results[0].content, "new user entry");
}

#[tokio::test]
async fn test_delete_by_user_id_before_removes_older_entries() {
    let db = test_db().await;
    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);

    let cutoff = Utc::now();
    let before = cutoff - Duration::seconds(60);
    let after = cutoff + Duration::seconds(60);

    let old = make_user_entry("user-1", "old user entry", before);
    let new = make_user_entry("user-1", "new user entry", after);

    repo.create(&old).await.unwrap();
    repo.create(&new).await.unwrap();

    repo.delete_by_user_id_before("user-1", cutoff).await.unwrap();

    let remaining = repo.find_by_user_id("user-1").await.unwrap();
    assert_eq!(remaining.len(), 1, "Should keep only newer entry");
    assert_eq!(remaining[0].content, "new user entry");
}

#[tokio::test]
async fn test_find_distinct_user_ids() {
    let db = test_db().await;
    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);

    let now = Utc::now();
    repo.create(&make_user_entry("user-a", "memory 1", now)).await.unwrap();
    repo.create(&make_user_entry("user-b", "memory 2", now)).await.unwrap();
    repo.create(&make_user_entry("user-a", "memory 3", now + Duration::seconds(1))).await.unwrap();

    let mut user_ids = repo.find_distinct_user_ids().await.unwrap();
    user_ids.sort();
    assert_eq!(user_ids, vec!["user-a", "user-b"]);
}

#[tokio::test]
async fn test_agent_and_user_entries_are_independent() {
    let db = test_db().await;
    let repo: SurrealMemoryEntryRepo = SurrealRepo::new(db);

    let now = Utc::now();
    let agent_entry = make_entry("agent-1", "agent-scoped entry", now);
    let user_entry = make_user_entry("user-1", "user-scoped entry", now);

    repo.create(&agent_entry).await.unwrap();
    repo.create(&user_entry).await.unwrap();

    let agent_results = repo.find_by_agent_id("agent-1").await.unwrap();
    assert_eq!(agent_results.len(), 1, "Agent query should return only agent entries");
    assert_eq!(agent_results[0].content, "agent-scoped entry");

    let user_results = repo.find_by_user_id("user-1").await.unwrap();
    assert_eq!(user_results.len(), 1, "User query should return only user entries");
    assert_eq!(user_results[0].content, "user-scoped entry");

    let agent_ids = repo.find_distinct_agent_ids().await.unwrap();
    assert_eq!(agent_ids.len(), 1, "Should only find agent-scoped agent IDs");

    let user_ids = repo.find_distinct_user_ids().await.unwrap();
    assert_eq!(user_ids.len(), 1, "Should only find user-scoped user IDs");
}
