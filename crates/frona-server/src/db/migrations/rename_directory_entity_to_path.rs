//! Rename `Policy::Directory::"…"` → `Policy::Path::"…"` in stored Cedar
//! `policy_text` rows. The Cedar entity type was renamed so that virtual
//! paths (`user://`, `agent://`) and absolute filesystem paths share a
//! single resource type — the schema, evaluator, and reconciler all use
//! `Policy::Path` now. Idempotent: rows that already use `Policy::Path`
//! are left untouched.

use surrealdb::Surreal;
use surrealdb::engine::local::Db;

use frona_derive::migration;

const OLD: &str = "Policy::Directory";
const NEW: &str = "Policy::Path";

#[migration("2026-05-02T00:00:00Z")]
async fn rename_directory_entity_to_path(db: &Surreal<Db>) -> Result<(), surrealdb::Error> {
    let mut result = db
        .query("SELECT meta::id(id) as id, policy_text FROM policy")
        .await?;
    let rows: Vec<serde_json::Value> = result.take(0)?;

    for row in rows {
        let Some(id) = row.get("id").and_then(|v| v.as_str()).map(str::to_string) else {
            continue;
        };
        let Some(text) = row.get("policy_text").and_then(|v| v.as_str()) else {
            continue;
        };
        if !text.contains(OLD) {
            continue;
        }
        let migrated = text.replace(OLD, NEW);
        db.query("UPDATE type::record('policy', $id) SET policy_text = $text, updated_at = $now")
            .bind(("id", id))
            .bind(("text", migrated))
            .bind(("now", chrono::Utc::now()))
            .await?
            .check()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::engine::local::Mem;

    async fn mem_db() -> Surreal<Db> {
        let db = Surreal::new::<Mem>(()).await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        db
    }

    #[tokio::test]
    async fn rewrites_directory_to_path() {
        let db = mem_db().await;
        let now = chrono::Utc::now();
        db.query(
            "CREATE policy CONTENT {
                user_id: 'u1',
                name: 'p1',
                description: '',
                policy_text: $text,
                enabled: true,
                created_at: $now,
                updated_at: $now,
            }",
        )
        .bind(("text", r#"permit(principal, action == Policy::Action::"read", resource == Policy::Directory::"/data");"#))
        .bind(("now", now))
        .await
        .unwrap();

        rename_directory_entity_to_path(&db).await.unwrap();

        let mut res = db.query("SELECT VALUE policy_text FROM policy").await.unwrap();
        let texts: Vec<String> = res.take(0).unwrap();
        assert_eq!(texts.len(), 1);
        assert!(texts[0].contains("Policy::Path::\"/data\""));
        assert!(!texts[0].contains("Policy::Directory"));
    }

    #[tokio::test]
    async fn is_idempotent() {
        let db = mem_db().await;
        let now = chrono::Utc::now();
        db.query(
            "CREATE policy CONTENT {
                user_id: 'u1',
                name: 'p1',
                description: '',
                policy_text: $text,
                enabled: true,
                created_at: $now,
                updated_at: $now,
            }",
        )
        .bind(("text", r#"permit(principal, action == Policy::Action::"read", resource == Policy::Path::"/data");"#))
        .bind(("now", now))
        .await
        .unwrap();

        rename_directory_entity_to_path(&db).await.unwrap();
        rename_directory_entity_to_path(&db).await.unwrap();

        let mut res = db.query("SELECT VALUE policy_text FROM policy").await.unwrap();
        let texts: Vec<String> = res.take(0).unwrap();
        assert_eq!(texts.len(), 1);
        assert!(texts[0].contains("Policy::Path::\"/data\""));
    }

    #[tokio::test]
    async fn leaves_unrelated_rows_alone() {
        let db = mem_db().await;
        let now = chrono::Utc::now();
        db.query(
            "CREATE policy CONTENT {
                user_id: 'u1',
                name: 'p1',
                description: '',
                policy_text: $text,
                enabled: true,
                created_at: $now,
                updated_at: $now,
            }",
        )
        .bind(("text", r#"permit(principal, action == Policy::Action::"connect", resource == Policy::NetworkDestination::"gmail.com");"#))
        .bind(("now", now))
        .await
        .unwrap();

        rename_directory_entity_to_path(&db).await.unwrap();

        let mut res = db.query("SELECT VALUE policy_text FROM policy").await.unwrap();
        let texts: Vec<String> = res.take(0).unwrap();
        assert!(texts[0].contains("NetworkDestination"));
    }
}
