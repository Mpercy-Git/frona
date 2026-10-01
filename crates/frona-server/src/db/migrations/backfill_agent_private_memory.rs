//! Backfills `private_memory` on agent rows written before the field existed.
//!
//! `Agent::private_memory` is a plain `bool` carrying `#[serde(default)]`, which
//! covers serde — but agent rows are read back through `SurrealValue`, whose
//! derive has no `default` attribute. A row stored without the key reads as
//! `NONE`, and every read of that agent fails with
//! `Failed to deserialize field 'private_memory' on type 'Agent': Expected
//! bool, got none` — so an install that upgraded into the per-agent private
//! memory release can't load the agents it already had.
//!
//! Writing the absent field is the whole fix: `false` is what the field means
//! when it isn't there, and every agent written since carries it explicitly.
//! Re-entrant — `IS NONE` means a second run matches nothing, and an agent
//! since marked private is never reset.

use surrealdb::Surreal;
use surrealdb::engine::local::Db;

use frona_derive::migration;

#[migration("2026-09-12T00:00:00Z")]
async fn backfill_agent_private_memory(db: &Surreal<Db>) -> Result<(), surrealdb::Error> {
    db.query("UPDATE agent SET private_memory = false WHERE private_memory IS NONE")
        .await?
        .check()?;

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

    /// A row as the pre-`private_memory` releases wrote it: no such key at all.
    async fn seed(db: &Surreal<Db>, id: &str, private_memory: Option<bool>) {
        let set = match private_memory {
            Some(v) => format!(", private_memory = {v}"),
            None => String::new(),
        };
        db.query(format!(
            "CREATE type::record('agent', $id) SET name = 'old agent'{set}"
        ))
        .bind(("id", id.to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
    }

    async fn private_memory_of(db: &Surreal<Db>, id: &str) -> Option<bool> {
        db.query("SELECT VALUE private_memory FROM type::record('agent', $id)")
            .bind(("id", id.to_string()))
            .await
            .unwrap()
            .take(0)
            .unwrap()
    }

    #[tokio::test]
    async fn fills_in_the_missing_field_and_leaves_stated_ones_alone() {
        let db = mem_db().await;
        seed(&db, "legacy", None).await;
        seed(&db, "shared", Some(false)).await;
        seed(&db, "private", Some(true)).await;

        backfill_agent_private_memory(&db).await.unwrap();

        assert_eq!(
            private_memory_of(&db, "legacy").await,
            Some(false),
            "a row from before the field now reads as a plain bool"
        );
        assert_eq!(private_memory_of(&db, "shared").await, Some(false));
        assert_eq!(
            private_memory_of(&db, "private").await,
            Some(true),
            "an agent the user made private stays private"
        );
    }

    #[tokio::test]
    async fn is_reentrant() {
        let db = mem_db().await;
        seed(&db, "legacy", None).await;
        seed(&db, "private", Some(true)).await;

        backfill_agent_private_memory(&db).await.unwrap();
        backfill_agent_private_memory(&db).await.unwrap();

        assert_eq!(private_memory_of(&db, "legacy").await, Some(false));
        assert_eq!(private_memory_of(&db, "private").await, Some(true));
    }
}
