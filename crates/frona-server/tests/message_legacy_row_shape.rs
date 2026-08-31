//! Message rows written before a field existed must still load.
//!
//! `SurrealValue`'s derive hands a *missing* object key to `from_value` as
//! `NONE`, and a non-`Option` field rejects that unless it carries
//! `#[surreal(default)]` — serde's `#[serde(default)]` does not reach the
//! Surreal codec. `event.data.citations` (added with task citations) is the
//! field that stalled the PKM consolidation sweep on chats whose task
//! completions predate it: every read of that chat's history failed with
//! "Expected array<{title: none | string, url: string}>, got none".

use frona::chat::message::models::{MessageEvent, MessageRole};
use frona::chat::message::repository::MessageRepository;
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

    // The pre-citations on-disk shape: no `citations` under `event.data`, and
    // none of the later top-level defaults either.
    db.query(
        "CREATE message:legacy CONTENT { \
            chat_id: 'chat-1', \
            role: 'agent', \
            content: 'done', \
            agent_id: 'agent-1', \
            event: { \
                type: 'TaskCompletion', \
                data: { \
                    task_id: 'task-1', \
                    chat_id: 'chat-1', \
                    status: 'completed', \
                    summary: 'shipped it' \
                } \
            }, \
            created_at: time::now() \
        }",
    )
    .await
    .unwrap()
    .check()
    .unwrap();

    let repo = SurrealMessageRepo::new(db.clone());
    let messages = repo.find_by_chat_id("chat-1").await.unwrap();

    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.role, MessageRole::Agent);
    assert!(message.attachments.is_empty());
    assert!(message.metadata.is_empty());

    match message.event.as_ref().expect("event should deserialize") {
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
