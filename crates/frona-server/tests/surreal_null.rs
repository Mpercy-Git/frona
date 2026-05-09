use chrono::Utc;
use frona::agent::models::Agent;
use frona::agent::repository::AgentRepository;
use frona::db::init as db;
use frona::db::repo::agents::SurrealAgentRepo;
use frona::db::repo::chats::SurrealChatRepo;
use frona::chat::models::Chat;
use frona::chat::repository::ChatRepository;
use frona::core::repository::Repository;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

fn test_agent(user_id: Option<&str>) -> Agent {
    let now = Utc::now();
    Agent {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.map(|s| s.to_string()),
        name: "Test Agent".to_string(),
        description: "A test agent".to_string(),
        model_group: "primary".to_string(),
        enabled: true,
        skills: None,
        sandbox_limits: None,
        max_concurrent_tasks: None,
        avatar: None,
        identity: std::collections::BTreeMap::new(),
        prompt: None,
        heartbeat_interval: None,
        next_heartbeat_at: None,
        heartbeat_chat_id: None,
        created_at: now,
        updated_at: now,
    }
}

fn test_chat(user_id: &str, space_id: Option<&str>, title: Option<&str>) -> Chat {
    let now = Utc::now();
    Chat {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        space_id: space_id.map(|s| s.to_string()),
        task_id: None,
        agent_id: "some-agent".to_string(),
        title: title.map(|s| s.to_string()),
        archived_at: None,
        channel_id: None,
        channel_external_id: None,
        metadata: Default::default(),
        created_at: now,
        updated_at: now,
    }
}

// ---------------------------------------------------------------------------
// 4a. Seeded agent with JSON null user_id round-trips after fix
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_seeded_agent_with_absent_user_id_round_trips() {
    let db = test_db().await;
    let repo = SurrealAgentRepo::new(db.clone());

    let agent_id = "test-config-agent";
    db.query(
        "CREATE type::record('agent', $id) SET
            name = $id,
            description = '',
            model_group = 'primary',
            enabled = true,
            tools = [],
            skills = [],
            identity = {},
            created_at = time::now(),
            updated_at = time::now()"
    )
    .bind(("id", agent_id))
    .await
    .unwrap();

    let agents = repo.find_by_user_id("any-user").await.unwrap();
    let found = agents.iter().find(|a| a.id == agent_id);
    assert!(found.is_some(), "seeded agent should appear in find_by_user_id results");

    let agent = found.unwrap();
    assert_eq!(agent.user_id, None);
    assert!(agent.sandbox_limits.is_none());
}

// ---------------------------------------------------------------------------
// 4b. Agent: user_id=None round-trips via SurrealValue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_agent_none_user_id_round_trips_via_repo() {
    let db = test_db().await;
    let repo = SurrealAgentRepo::new(db);

    let agent = test_agent(None);
    repo.create(&agent).await.unwrap();

    let found = repo.find_by_id(&agent.id).await.unwrap().unwrap();
    assert_eq!(found.user_id, None);
    assert!(found.sandbox_limits.is_none());

    let agents = repo.find_by_user_id("any-user").await.unwrap();
    assert!(
        agents.iter().any(|a| a.id == agent.id),
        "agent with user_id=None should appear in find_by_user_id (matched by IS NONE)"
    );
}

// ---------------------------------------------------------------------------
// 4c. Chat: space_id=None and title=None round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_none_optional_fields_round_trip() {
    let db = test_db().await;
    let repo = SurrealChatRepo::new(db);

    let chat = test_chat("user-1", None, None);
    repo.create(&chat).await.unwrap();

    let found = repo.find_by_id(&chat.id).await.unwrap().unwrap();
    assert_eq!(found.space_id, None);
    assert_eq!(found.title, None);

    let standalone = repo.find_standalone_by_user_id("user-1").await.unwrap();
    assert!(
        standalone.iter().any(|c| c.id == chat.id),
        "chat with space_id=None should appear in find_standalone_by_user_id"
    );
}

#[tokio::test]
async fn test_chat_with_space_id_excluded_from_standalone() {
    let db = test_db().await;
    let repo = SurrealChatRepo::new(db);

    let standalone_chat = test_chat("user-1", None, Some("Standalone"));
    let space_chat = test_chat("user-1", Some("space-1"), Some("In Space"));
    repo.create(&standalone_chat).await.unwrap();
    repo.create(&space_chat).await.unwrap();

    let standalone = repo.find_standalone_by_user_id("user-1").await.unwrap();
    assert_eq!(standalone.len(), 1);
    assert_eq!(standalone[0].id, standalone_chat.id);
}

// ---------------------------------------------------------------------------
// 4e. JSON null cannot deserialize — confirms the bug this fix addresses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_json_null_user_id_fails_deserialization() {
    let db = test_db().await;

    let now = Utc::now();
    let agent_json = serde_json::json!({
        "user_id": null,
        "name": "broken-agent",
        "description": "",
        "model_group": "primary",
        "enabled": true,
        "tools": [],
        "created_at": now,
        "updated_at": now,
    });

    let _: Option<surrealdb::types::Value> = db
        .create(("agent", "broken-agent"))
        .content(agent_json)
        .await
        .unwrap();

    let result: Result<Option<Agent>, _> = db
        .query("SELECT *, meta::id(id) as id FROM agent WHERE id = $id LIMIT 1")
        .bind(("id", surrealdb::types::RecordId::new("agent", "broken-agent")))
        .await
        .unwrap()
        .take(0);

    assert!(
        result.is_err(),
        "JSON null should fail SurrealValue deserialization — this is the bug we fixed in seed_config_agents"
    );
}
