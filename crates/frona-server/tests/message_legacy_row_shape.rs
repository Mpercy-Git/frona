//! Message rows written before a field existed must still load.
//!
//! `SurrealValue`'s derive hands a *missing* object key to `from_value` as
//! `NONE`, and a non-`Option` field rejects that unless it carries
//! `#[surreal(default)]` — serde's `#[serde(default)]` does not reach the
//! Surreal codec. `event.data.citations` (added with task citations) is the
//! field that stalled the PKM consolidation sweep on chats whose task
//! completions predate it: every read of that chat's history failed with
//! "Expected array<{title: none | string, url: string}>, got none".

use frona::agent::task::models::TaskStatus;
use frona::chat::message::models::{Message, MessageEvent, MessageRole};
use frona::chat::message::repository::MessageRepository;
use frona::core::repository::Repository;
use frona::db::init as db;
use frona::db::repo::messages::SurrealMessageRepo;
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

#[tokio::test]
async fn task_completion_row_written_before_citations_still_loads() {
    let db = test_db().await;
    let repo = SurrealMessageRepo::new(db.clone());

    let message = Message::builder(
        "chat-1",
        MessageRole::TaskCompletion,
        "shipped it".to_string(),
    )
    .agent_id("agent-1".to_string())
    .event(MessageEvent::TaskCompletion {
        task_id: "task-1".to_string(),
        chat_id: Some("chat-1".to_string()),
        status: TaskStatus::Completed,
        summary: Some("shipped it".to_string()),
        schema: None,
        citations: Vec::new(),
    })
    .build();
    repo.create(&message).await.unwrap();

    // Roll the row back to its pre-citations shape. Data-carrying enums are
    // stored externally tagged by variant identifier, so the field lives at
    // `event.TaskCompletion.citations`; assigning NONE drops the key entirely,
    // which is exactly how rows written before the field look.
    db.query("UPDATE message SET event.TaskCompletion.citations = NONE")
        .await
        .unwrap()
        .check()
        .unwrap();

    let mut keys = db
        .query("SELECT VALUE object::keys(event.TaskCompletion) FROM message")
        .await
        .unwrap();
    let keys: Vec<Vec<String>> = keys.take(0).unwrap();
    assert_eq!(keys.len(), 1);
    assert!(
        !keys[0].contains(&"citations".to_string()),
        "the row under test must actually be missing the key, got {:?}",
        keys[0]
    );

    let messages = repo.find_by_chat_id("chat-1").await.unwrap();
    assert_eq!(messages.len(), 1);
    let loaded = &messages[0];
    assert!(loaded.attachments.is_empty());
    assert!(loaded.metadata.is_empty());

    match loaded.event.as_ref().expect("event should deserialize") {
        MessageEvent::TaskCompletion {
            task_id,
            summary,
            citations,
            ..
        } => {
            assert_eq!(task_id, "task-1");
            assert_eq!(summary.as_deref(), Some("shipped it"));
            assert!(citations.is_empty(), "missing citations must default to []");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
