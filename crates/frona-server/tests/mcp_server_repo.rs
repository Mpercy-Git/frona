use frona::core::repository::Repository;
use frona::db::init as db;
use frona::db::repo::generic::SurrealRepo;
use frona::tool::mcp::models::{
    CachedMcpTool, McpPackage, McpRuntime, McpServer, McpServerInfo, McpServerStatus,
};
use frona::tool::mcp::repository::McpServerRepository;
use std::collections::BTreeMap;
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

fn make_server(id: &str, user_id: &str, handle: &str, status: McpServerStatus) -> McpServer {
    let now = chrono::Utc::now();
    McpServer {
        id: id.to_string(),
        user_id: user_id.to_string(),
        handle: frona::core::Handle::try_new(handle).expect("test handle invalid"),
        display_name: format!("Server {id}"),
        description: Some("An MCP server".to_string()),
        repository_url: Some("https://github.com/example/server".to_string()),
        registry_id: Some(format!("io.github.example/{handle}")),
        server_info: None,
        package: McpPackage {
            runtime: McpRuntime::Npm,
            name: "@example/mcp-server".to_string(),
            version: "1.0.0".to_string(),
        },
        command: "npx".to_string(),
        args: vec!["-y".to_string(), "@example/mcp-server".to_string()],
        env: BTreeMap::new(),
        transports: vec![],
        active_transport: "stdio".into(),
        status,
        tool_cache: vec![],
        workspace_dir: format!("data/mcp/{id}"),        installed_at: now,
        last_started_at: None,
        updated_at: now,
    }
}

#[tokio::test]
async fn create_and_find_by_id() {
    let db = test_db().await;
    let repo = SurrealRepo::<McpServer>::new(db);

    let server = make_server("srv-1", "user-1", "example", McpServerStatus::Installed);
    let created = repo.create(&server).await.unwrap();
    assert_eq!(created.id, "srv-1");
    assert_eq!(created.handle.as_str(), "example");

    let found = repo.find_by_id("srv-1").await.unwrap().unwrap();
    assert_eq!(found.user_id, "user-1");
    assert_eq!(found.package.runtime, McpRuntime::Npm);
    assert_eq!(found.package.version, "1.0.0");
    assert!(matches!(found.status, McpServerStatus::Installed));
}

#[tokio::test]
async fn find_by_user_isolates_users() {
    let db = test_db().await;
    let repo = SurrealRepo::<McpServer>::new(db);

    repo.create(&make_server("s1", "user-1", "alpha", McpServerStatus::Installed))
        .await
        .unwrap();
    repo.create(&make_server("s2", "user-1", "beta", McpServerStatus::Running))
        .await
        .unwrap();
    repo.create(&make_server("s3", "user-2", "gamma", McpServerStatus::Running))
        .await
        .unwrap();

    let user1 = repo.find_by_user("user-1").await.unwrap();
    assert_eq!(user1.len(), 2);
    let user1_ids: Vec<&str> = user1.iter().map(|s| s.id.as_str()).collect();
    assert!(user1_ids.contains(&"s1"));
    assert!(user1_ids.contains(&"s2"));

    let user2 = repo.find_by_user("user-2").await.unwrap();
    assert_eq!(user2.len(), 1);
    assert_eq!(user2[0].id, "s3");
}

#[tokio::test]
async fn find_running_only_returns_running() {
    let db = test_db().await;
    let repo = SurrealRepo::<McpServer>::new(db);

    repo.create(&make_server("s1", "user-1", "aa", McpServerStatus::Installed))
        .await
        .unwrap();
    repo.create(&make_server("s2", "user-1", "bb", McpServerStatus::Running))
        .await
        .unwrap();
    repo.create(&make_server("s3", "user-1", "cc", McpServerStatus::Stopped))
        .await
        .unwrap();
    repo.create(&make_server("s4", "user-2", "dd", McpServerStatus::Running))
        .await
        .unwrap();
    repo.create(&make_server("s5", "user-1", "ee", McpServerStatus::Failed))
        .await
        .unwrap();

    let running = repo.find_running().await.unwrap();
    assert_eq!(running.len(), 2);
    let ids: Vec<&str> = running.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"s2"));
    assert!(ids.contains(&"s4"));
}

#[tokio::test]
async fn find_running_includes_starting_status() {
    let db = test_db().await;
    let repo = SurrealRepo::<McpServer>::new(db);

    repo.create(&make_server("s1", "user-1", "aa", McpServerStatus::Running))
        .await
        .unwrap();
    repo.create(&make_server("s2", "user-1", "bb", McpServerStatus::Starting))
        .await
        .unwrap();
    repo.create(&make_server("s3", "user-1", "cc", McpServerStatus::Installed))
        .await
        .unwrap();

    let running = repo.find_running().await.unwrap();
    assert_eq!(running.len(), 2);
    let ids: Vec<&str> = running.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"s1"));
    assert!(ids.contains(&"s2"));
    assert!(!ids.contains(&"s3"));
}

#[tokio::test]
async fn update_status_and_tool_cache() {
    let db = test_db().await;
    let repo = SurrealRepo::<McpServer>::new(db);

    let server = make_server("s1", "user-1", "example", McpServerStatus::Installed);
    repo.create(&server).await.unwrap();

    let mut server = repo.find_by_id("s1").await.unwrap().unwrap();
    server.status = McpServerStatus::Running;
    server.server_info = Some(McpServerInfo {
        name: "example-server".to_string(),
        version: "1.2.3".to_string(),
    });
    server.tool_cache = vec![CachedMcpTool {
        name: "echo".to_string(),
        description: "Echo the input".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    repo.update(&server).await.unwrap();

    let updated = repo.find_by_id("s1").await.unwrap().unwrap();
    assert!(matches!(updated.status, McpServerStatus::Running));
    assert_eq!(updated.server_info.as_ref().unwrap().version, "1.2.3");
    assert_eq!(updated.tool_cache.len(), 1);
    assert_eq!(updated.tool_cache[0].name, "echo");
}

#[tokio::test]
async fn delete_removes_row() {
    let db = test_db().await;
    let repo = SurrealRepo::<McpServer>::new(db);

    repo.create(&make_server("s1", "user-1", "aa", McpServerStatus::Installed))
        .await
        .unwrap();
    repo.delete("s1").await.unwrap();
    assert!(repo.find_by_id("s1").await.unwrap().is_none());
}

#[tokio::test]
async fn credential_refs_round_trip_verbatim() {
    let db = test_db().await;
    let repo = SurrealRepo::<McpServer>::new(db);

    let mut server = make_server("s1", "user-1", "example", McpServerStatus::Installed);
    server.env.insert(
        "GITHUB_TOKEN".to_string(),
        "$credential:github".to_string(),
    );
    server.env.insert("LOG_LEVEL".to_string(), "debug".to_string());
    repo.create(&server).await.unwrap();

    let found = repo.find_by_id("s1").await.unwrap().unwrap();
    assert_eq!(
        found.env.get("GITHUB_TOKEN").unwrap(),
        "$credential:github"
    );
    assert_eq!(found.env.get("LOG_LEVEL").unwrap(), "debug");
}
