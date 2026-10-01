mod helpers;

use chrono::Utc;
use frona::chat::models::Chat;
use frona::core::error::AppError;
use frona::core::repository::Repository;
use frona::db::repo::generic::SurrealRepo;
use helpers::test_chat_service_with_db;
use surrealdb::Surreal;
use surrealdb::engine::local::Db;

async fn seed_chat(db: &Surreal<Db>, owner_id: &str, agent_id: &str) -> Chat {
    let now = Utc::now();
    let chat = Chat {
        id: frona::core::repository::new_id(),
        user_id: owner_id.to_string(),
        space_id: None,
        task_id: None,
        agent_id: agent_id.to_string(),
        title: Some("Test chat".to_string()),
        archived_at: None,
        channel_id: None,
        channel_external_id: None,
        metadata: Default::default(),
        created_at: now,
        updated_at: now,
    };
    let repo: SurrealRepo<Chat> = SurrealRepo::new(db.clone());
    repo.create(&chat).await.unwrap()
}

#[tokio::test]
async fn reassign_agent_updates_chat_agent_id() {
    let (chat_service, db) = test_chat_service_with_db().await;
    let chat = seed_chat(&db, "owner", "receptionist").await;

    let response = chat_service
        .reassign_agent("owner", &chat.id, "billing")
        .await
        .unwrap();
    assert_eq!(response.agent_id, "billing");

    // Persisted, not just returned in the response.
    let refetched = chat_service.get_chat("owner", &chat.id).await.unwrap();
    assert_eq!(refetched.agent_id, "billing");
}

#[tokio::test]
async fn reassign_agent_forbids_non_owner() {
    let (chat_service, db) = test_chat_service_with_db().await;
    let chat = seed_chat(&db, "owner", "receptionist").await;

    let err = chat_service
        .reassign_agent("stranger", &chat.id, "billing")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
}
