//! Integration test for the `rename_vault_grant_to_principal` migration:
//! seed a row in the shape produced by the old binary (an `agent_id` string
//! and no `principal` field), run the migration, and assert the row is
//! rewritten to the new `principal` shape. Also asserts idempotency.

use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

async fn fresh_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();
    frona::db::init::setup_schema(&db).await.unwrap();
    db
}

async fn seed_pre_migration_grant(db: &Surreal<Db>, id: &str, agent_id: &str) {
    db.query(
        "CREATE type::record('vault_grant', $id) CONTENT {
            user_id: 'user1',
            connection_id: 'local',
            vault_item_id: 'item1',
            agent_id: $agent_id,
            query: 'github',
            expires_at: NONE,
            created_at: time::now(),
        }",
    )
    .bind(("id", id.to_string()))
    .bind(("agent_id", agent_id.to_string()))
    .await
    .unwrap()
    .check()
    .unwrap();
}

async fn principal_of(db: &Surreal<Db>, id: &str) -> serde_json::Value {
    let mut result = db
        .query("SELECT principal FROM type::record('vault_grant', $id)")
        .bind(("id", id.to_string()))
        .await
        .unwrap();
    let rows: Vec<serde_json::Value> = result.take(0).unwrap();
    rows.into_iter()
        .next()
        .and_then(|row| row.get("principal").cloned())
        .unwrap_or(serde_json::Value::Null)
}

#[tokio::test]
async fn migration_stamps_principal_on_old_rows() {
    use frona::core::Principal;
    use frona::credential::vault::models::VaultGrant;
    use frona::credential::vault::repository::VaultGrantRepository;
    use frona::db::repo::generic::SurrealRepo;
    use std::sync::Arc;

    let db = fresh_db().await;
    seed_pre_migration_grant(&db, "g1", "agent-foo").await;
    seed_pre_migration_grant(&db, "g2", "agent-bar").await;

    assert_eq!(principal_of(&db, "g1").await, serde_json::Value::Null);

    frona::db::migrations::run_migrations(&db).await.unwrap();

    let g1 = principal_of(&db, "g1").await;
    let g1_obj = g1.as_object().expect("principal should be an object");
    assert_eq!(
        g1_obj.get("id").and_then(|v| v.as_str()),
        Some("agent-foo")
    );
    assert!(
        g1_obj.get("kind").and_then(|v| v.as_object()).is_some_and(|k| k.contains_key("Agent")),
        "principal.kind should match SurrealValue's externally-tagged Agent shape"
    );

    let grant_repo: Arc<dyn VaultGrantRepository> =
        Arc::new(SurrealRepo::<VaultGrant>::new(db.clone()));
    let agent_grants = grant_repo
        .find_by_principal("user1", &Principal::agent("agent-foo"))
        .await
        .expect("the migrated row must deserialize through VaultGrantRepository");
    assert_eq!(
        agent_grants.len(),
        1,
        "find_by_principal should match the migrated row"
    );
    assert_eq!(agent_grants[0].vault_item_id, "item1");
}

#[tokio::test]
async fn migration_is_idempotent() {
    let db = fresh_db().await;
    seed_pre_migration_grant(&db, "g1", "agent-foo").await;

    frona::db::migrations::run_migrations(&db).await.unwrap();
    let after_first = principal_of(&db, "g1").await;

    frona::db::migrations::run_migrations(&db).await.unwrap();
    let after_second = principal_of(&db, "g1").await;

    assert_eq!(after_first, after_second);
}

#[tokio::test]
async fn migration_does_not_overwrite_rows_that_already_have_principal() {
    let db = fresh_db().await;

    db.query(
        "CREATE vault_grant:g1 CONTENT {
            user_id: 'user1',
            connection_id: 'local',
            vault_item_id: 'item1',
            agent_id: 'legacy-value',
            principal: { kind: 'mcp_server', id: 'srv1' },
            query: 'github',
            expires_at: NONE,
            created_at: time::now(),
        }",
    )
    .await
    .unwrap()
    .check()
    .unwrap();

    frona::db::migrations::run_migrations(&db).await.unwrap();

    let principal = principal_of(&db, "g1").await;
    let kind = principal
        .as_object()
        .and_then(|o| o.get("kind"))
        .and_then(|v| v.as_str())
        .expect("principal should still be the mcp_server one");
    assert_eq!(kind, "mcp_server");
    assert_eq!(
        principal.as_object().unwrap().get("id").and_then(|v| v.as_str()),
        Some("srv1"),
        "existing principal must not be overwritten by agent_id fallback"
    );
}
