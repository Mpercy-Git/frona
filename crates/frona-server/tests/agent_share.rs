use std::sync::Arc;

use chrono::Utc;
use frona::agent::models::Agent;
use frona::agent::service::{AgentAccess, AgentService};
use frona::agent::share::models::ShareLevel;
use frona::agent::share::service::AgentShareService;
use frona::core::config::CacheConfig;
use frona::core::error::AppError;
use frona::core::repository::Repository;
use frona::db::init as db;
use frona::db::repo::generic::SurrealRepo;
use frona::policy::service::PolicyService;
use frona::tool::manager::ToolManager;
use frona::tool::sandbox::driver::resource_monitor::SystemResourceManager;
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

fn test_user_service(db: &Surreal<Db>) -> frona::auth::UserService {
    frona::auth::UserService::new(
        SurrealRepo::new(db.clone()),
        &CacheConfig::default(),
    )
}

fn test_policy_service(db: &Surreal<Db>) -> PolicyService {
    let schema = frona::policy::schema::build_schema();
    let repo: Arc<dyn frona::policy::repository::PolicyRepository> =
        Arc::new(SurrealRepo::<frona::policy::models::Policy>::new(db.clone()));
    let storage = frona::storage::StorageService::new(&frona::core::config::Config::default());
    PolicyService::new(
        repo,
        schema,
        Arc::new(ToolManager::new(false)),
        storage,
        test_user_service(db),
    )
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

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

async fn seed_agent(db: &Surreal<Db>, owner_id: &str) -> Agent {
    let now = Utc::now();
    let agent = Agent {
        id: frona::core::repository::new_id(),
        user_id: owner_id.to_string(),
        handle: frona::core::Handle::try_new("shared-agent").unwrap(),
        name: "Shared Agent".to_string(),
        description: String::new(),
        model_group: "primary".to_string(),
        enabled: true,
        skills: None,
        sandbox_limits: None,
        max_concurrent_tasks: None,
        avatar: None,
        voice_id: None,
        identity: std::collections::BTreeMap::new(),
        prompt: None,
        heartbeat_interval: None,
        next_heartbeat_at: None,
        heartbeat_chat_id: None,
        created_at: now,
        updated_at: now,
    };
    let repo: SurrealRepo<Agent> = SurrealRepo::new(db.clone());
    repo.create(&agent).await.unwrap()
}

fn share_service(db: &Surreal<Db>) -> AgentShareService {
    AgentShareService::new(SurrealRepo::new(db.clone()), test_user_service(db))
}

#[tokio::test]
async fn share_by_handle_then_find_and_unshare() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    seed_user(&users, "wife").await;
    let agent = seed_agent(&db, "owner").await;
    let svc = share_service(&db);

    // Share by handle.
    svc.share("owner", &agent.id, "wife").await.unwrap();
    assert_eq!(
        svc.find_level(&agent.id, "wife").await.unwrap(),
        Some(ShareLevel::Use)
    );

    // Idempotent re-share doesn't error or duplicate.
    svc.share("owner", &agent.id, "wife").await.unwrap();
    assert_eq!(svc.list_for_agent(&agent.id).await.unwrap().len(), 1);
    assert_eq!(svc.list_shared_with("wife").await.unwrap().len(), 1);

    // Revoke.
    svc.unshare(&agent.id, "wife").await.unwrap();
    assert_eq!(svc.find_level(&agent.id, "wife").await.unwrap(), None);
}

#[tokio::test]
async fn share_by_email_resolves_recipient() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    seed_user(&users, "wife").await;
    let agent = seed_agent(&db, "owner").await;
    let svc = share_service(&db);

    svc.share("owner", &agent.id, "wife@example.com").await.unwrap();
    assert_eq!(
        svc.find_level(&agent.id, "wife").await.unwrap(),
        Some(ShareLevel::Use)
    );
}

#[tokio::test]
async fn cannot_share_with_self() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    let agent = seed_agent(&db, "owner").await;
    let svc = share_service(&db);

    let err = svc.share("owner", &agent.id, "owner").await.unwrap_err();
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

#[tokio::test]
async fn unknown_recipient_is_not_found() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    let agent = seed_agent(&db, "owner").await;
    let svc = share_service(&db);

    let err = svc.share("owner", &agent.id, "ghost").await.unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn delegation_flag_defaults_off_and_toggles() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    seed_user(&users, "wife").await;
    let agent = seed_agent(&db, "owner").await;
    let svc = share_service(&db);

    svc.share("owner", &agent.id, "wife").await.unwrap();
    let flag = |db: &Surreal<Db>, agent_id: String| {
        let svc = share_service(db);
        async move {
            svc.find_share(&agent_id, "wife")
                .await
                .unwrap()
                .unwrap()
                .delegate_credentials
        }
    };

    assert!(!flag(&db, agent.id.clone()).await, "defaults off");

    svc.set_delegation(&agent.id, "wife", true).await.unwrap();
    assert!(flag(&db, agent.id.clone()).await);

    svc.set_delegation(&agent.id, "wife", false).await.unwrap();
    assert!(!flag(&db, agent.id.clone()).await);
}

#[tokio::test]
async fn credential_delegation_owner_reflects_flag() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    seed_user(&users, "wife").await;
    let agent = seed_agent(&db, "owner").await;

    let shares = share_service(&db);
    shares.share("owner", &agent.id, "wife").await.unwrap();

    let mut agents = AgentService::new(
        SurrealRepo::new(db.clone()),
        &CacheConfig::default(),
        Arc::new(SystemResourceManager::new(80.0, 80.0, 90.0, 90.0)),
        test_policy_service(&db),
        test_user_service(&db),
    );
    agents.set_share_service(shares.clone());

    // Shared but delegation off → no owner.
    assert_eq!(
        agents.credential_delegation_owner(&agent, "wife").await.unwrap(),
        None
    );
    // The owner never delegates to themselves.
    assert_eq!(
        agents.credential_delegation_owner(&agent, "owner").await.unwrap(),
        None
    );

    shares.set_delegation(&agent.id, "wife", true).await.unwrap();
    assert_eq!(
        agents.credential_delegation_owner(&agent, "wife").await.unwrap(),
        Some("owner".to_string())
    );
}

#[tokio::test]
async fn get_accessible_honors_owner_share_and_forbids_strangers() {
    let db = test_db().await;
    let users = test_user_service(&db);
    seed_user(&users, "owner").await;
    seed_user(&users, "wife").await;
    seed_user(&users, "stranger").await;
    let agent = seed_agent(&db, "owner").await;

    let shares = share_service(&db);
    shares.share("owner", &agent.id, "wife").await.unwrap();

    let mut agents = AgentService::new(
        SurrealRepo::new(db.clone()),
        &CacheConfig::default(),
        Arc::new(SystemResourceManager::new(80.0, 80.0, 90.0, 90.0)),
        test_policy_service(&db),
        test_user_service(&db),
    );
    agents.set_share_service(shares);

    // Owner: full access.
    let (_, access) = agents.get_accessible("owner", &agent.id).await.unwrap();
    assert_eq!(access, AgentAccess::Owner);

    // Recipient: use-only.
    let (_, access) = agents.get_accessible("wife", &agent.id).await.unwrap();
    assert_eq!(access, AgentAccess::SharedUse);

    // Everyone else: forbidden.
    let err = agents.get_accessible("stranger", &agent.id).await.unwrap_err();
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
}
