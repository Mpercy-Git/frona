mod helpers;

use chrono::Utc;
use frona::chat::models::Chat;
use frona::chat::share::service::ChatShareService;
use frona::core::config::CacheConfig;
use frona::core::error::AppError;
use frona::core::repository::Repository;
use frona::db::init as db_init;
use frona::db::repo::generic::SurrealRepo;
use helpers::test_chat_service_with_db;
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db_init::setup_schema(&db).await.unwrap();
    db
}

fn test_user_service(db: &Surreal<Db>) -> frona::auth::UserService {
    frona::auth::UserService::new(SurrealRepo::new(db.clone()), &CacheConfig::default())
}

async fn seed_user(user_service: &frona::auth::UserService, id: &str) {
    let _ = user_service
        .create(&frona::auth::User {
            id: id.into(),
            handle: frona::core::Handle::try_new(id).expect("valid handle"),
            email: format!("{id}@example.com"),
            name: id.into(),
            password_hash: String::new(),
            timezone: None,
            groups: Vec::new(),
            deactivated_at: None,
            phone: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await;
}

async fn seed_chat(db: &Surreal<Db>, owner_id: &str) -> Chat {
    let now = Utc::now();
    let chat = Chat {
        id: frona::core::repository::new_id(),
        user_id: owner_id.to_string(),
        space_id: None,
        task_id: None,
        agent_id: "agent-1".to_string(),
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

fn share_service(db: &Surreal<Db>, user_service: frona::auth::UserService) -> ChatShareService {
    ChatShareService::new(SurrealRepo::new(db.clone()), user_service)
}

#[tokio::test]
async fn share_by_handle_then_find_and_unshare() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    seed_user(&users, "friend").await;
    let chat = seed_chat(&db, "owner").await;
    let svc = share_service(&db, users);

    svc.share("owner", &chat.id, "friend").await.unwrap();
    assert!(svc.is_shared_with(&chat.id, "friend").await.unwrap());

    // Idempotent re-share doesn't error or duplicate.
    svc.share("owner", &chat.id, "friend").await.unwrap();
    assert_eq!(svc.list_for_chat(&chat.id).await.unwrap().len(), 1);
    assert_eq!(svc.list_shared_with("friend").await.unwrap().len(), 1);

    svc.unshare(&chat.id, "friend").await.unwrap();
    assert!(!svc.is_shared_with(&chat.id, "friend").await.unwrap());
}

#[tokio::test]
async fn share_by_email_resolves_recipient() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    seed_user(&users, "friend").await;
    let chat = seed_chat(&db, "owner").await;
    let svc = share_service(&db, users);

    svc.share("owner", &chat.id, "friend@example.com").await.unwrap();
    assert!(svc.is_shared_with(&chat.id, "friend").await.unwrap());
}

#[tokio::test]
async fn cannot_share_with_self() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    let chat = seed_chat(&db, "owner").await;
    let svc = share_service(&db, users);

    let err = svc.share("owner", &chat.id, "owner").await.unwrap_err();
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

#[tokio::test]
async fn unknown_recipient_is_not_found() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    let chat = seed_chat(&db, "owner").await;
    let svc = share_service(&db, users);

    let err = svc.share("owner", &chat.id, "ghost").await.unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn get_accessible_honors_owner_share_and_forbids_strangers() {
    let (mut chat_service, db) = test_chat_service_with_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    seed_user(&users, "friend").await;
    seed_user(&users, "stranger").await;
    let chat = seed_chat(&db, "owner").await;

    let shares = share_service(&db, users);
    shares.share("owner", &chat.id, "friend").await.unwrap();
    chat_service.set_share_service(shares);

    let (_, is_owner) = chat_service.get_accessible("owner", &chat.id).await.unwrap();
    assert!(is_owner);

    let (_, is_owner) = chat_service.get_accessible("friend", &chat.id).await.unwrap();
    assert!(!is_owner);

    let err = chat_service.get_accessible("stranger", &chat.id).await.unwrap_err();
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
}

#[tokio::test]
async fn shared_chat_responses_marks_is_shared_and_skips_deleted_chats() {
    let (mut chat_service, db) = test_chat_service_with_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    seed_user(&users, "friend").await;
    let chat = seed_chat(&db, "owner").await;

    let shares = share_service(&db, users);
    shares.share("owner", &chat.id, "friend").await.unwrap();
    // A share pointing at a since-deleted chat must be skipped, not error.
    shares.share("owner", "nonexistent-chat", "friend").await.unwrap();
    chat_service.set_share_service(shares);

    let responses = chat_service.shared_chat_responses("friend").await.unwrap();
    assert_eq!(responses.len(), 1);
    assert!(responses[0].is_shared);
    assert_eq!(responses[0].shared_by.as_deref(), Some("owner"));
}
