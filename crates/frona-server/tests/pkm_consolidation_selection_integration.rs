//! `PkmRepo::chats_needing_consolidation` - the consolidation-sweep chat picker.
//! Keyed on the message clock + unvalidated short memories, NOT `chat.updated_at`.

use chrono::{DateTime, Duration, Utc};
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

use frona::chat::message::models::MessageStatus;
use frona::db::repo::pkm::PkmRepo;

async fn fresh_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    frona::db::init::setup_schema(&db).await.unwrap();
    db
}

async fn add_chat(db: &Surreal<Db>, id: &str, archived: Option<DateTime<Utc>>, task: Option<&str>) {
    db.query(
        "UPSERT type::record('chat', $id) SET user_id = 'u', agent_id = 'a', \
         archived_at = $arch, task_id = $task, updated_at = $now",
    )
    .bind(("id", id.to_string()))
    .bind(("arch", archived))
    .bind(("task", task.map(|s| s.to_string())))
    .bind(("now", Utc::now()))
    .await
    .unwrap();
}

async fn add_message(
    db: &Surreal<Db>,
    chat_id: &str,
    created: DateTime<Utc>,
    status: Option<MessageStatus>,
) {
    db.query(
        "CREATE message SET chat_id = $c, role = 'agent', content = 'hi', \
         created_at = $t, status = $s",
    )
    .bind(("c", chat_id.to_string()))
    .bind(("t", created))
    .bind(("s", status))
    .await
    .unwrap();
}

async fn set_watermark(db: &Surreal<Db>, chat_id: &str, until: DateTime<Utc>) {
    db.query(
        "UPSERT type::record('knowledge_consolidation_watermark', $c) SET chat_id = $c, \
         consolidated_until = $u, updated_at = $now",
    )
    .bind(("c", chat_id.to_string()))
    .bind(("u", until))
    .bind(("now", Utc::now()))
    .await
    .unwrap();
}

async fn add_short(db: &Surreal<Db>, chat_id: &str, validated: bool) {
    db.query(
        "CREATE knowledge_short_memory SET user_id = 'u', source_chat_id = $c, \
         validated = $v, content = 'x', created_at = $t, last_accessed_at = $t",
    )
    .bind(("c", chat_id.to_string()))
    .bind(("v", validated))
    .bind(("t", Utc::now()))
    .await
    .unwrap();
}

#[tokio::test]
async fn selects_on_message_clock_and_short_memories_not_updated_at() {
    let db = fresh_db().await;
    let now = Utc::now();
    let watermark = now - Duration::minutes(120);
    let idle_cutoff = now - Duration::minutes(5);
    let idle_msg = now - Duration::minutes(60); // > watermark, < idle_cutoff (settled)
    let recent_msg = now - Duration::minutes(1); // >= idle_cutoff (active)

    add_chat(&db, "A", None, None).await;
    set_watermark(&db, "A", watermark).await;
    add_message(&db, "A", idle_msg, Some(MessageStatus::Completed)).await;

    add_chat(&db, "LEGACY", None, None).await;
    set_watermark(&db, "LEGACY", watermark).await;
    add_message(&db, "LEGACY", idle_msg, None).await;

    add_chat(&db, "B", None, None).await;
    set_watermark(&db, "B", watermark).await;
    add_message(&db, "B", watermark, Some(MessageStatus::Completed)).await; // == watermark, not past
    add_short(&db, "B", false).await;

    add_chat(&db, "C", None, None).await;
    set_watermark(&db, "C", watermark).await;
    add_message(&db, "C", watermark, Some(MessageStatus::Completed)).await;

    add_chat(&db, "ACTIVE", None, None).await;
    set_watermark(&db, "ACTIVE", watermark).await;
    add_message(&db, "ACTIVE", recent_msg, Some(MessageStatus::Completed)).await;

    add_chat(&db, "INFLIGHT", None, None).await;
    set_watermark(&db, "INFLIGHT", watermark).await;
    add_message(&db, "INFLIGHT", idle_msg, Some(MessageStatus::Executing)).await;

    add_chat(&db, "ARCHIVED", Some(now), None).await;
    set_watermark(&db, "ARCHIVED", watermark).await;
    add_message(&db, "ARCHIVED", idle_msg, Some(MessageStatus::Completed)).await;

    add_chat(&db, "TASK", None, Some("t1")).await;
    set_watermark(&db, "TASK", watermark).await;
    add_message(&db, "TASK", idle_msg, Some(MessageStatus::Completed)).await;

    let repo = PkmRepo::new(db.clone(), 8);
    let mut got = repo.chats_needing_consolidation(idle_cutoff).await.unwrap();
    got.sort();
    assert_eq!(
        got,
        vec!["A".to_string(), "B".to_string(), "LEGACY".to_string()],
        "only settled chats with unconsolidated terminal messages or short memories",
    );
}

#[tokio::test]
async fn validated_short_memory_does_not_retrigger() {
    let db = fresh_db().await;
    let now = Utc::now();
    let idle_cutoff = now - Duration::minutes(5);

    add_chat(&db, "B", None, None).await;
    set_watermark(&db, "B", now - Duration::minutes(120)).await;
    add_short(&db, "B", true).await; // already consolidated

    let repo = PkmRepo::new(db.clone(), 8);
    let got = repo.chats_needing_consolidation(idle_cutoff).await.unwrap();
    assert!(got.is_empty(), "a validated short memory must not re-trigger selection");
}
