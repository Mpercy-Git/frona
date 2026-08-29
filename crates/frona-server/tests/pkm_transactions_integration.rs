//! Verifies the SurrealDB transaction API the PKM atomic-writes work relies on:
//! `db.begin()` → `tx.create(...).content(struct)` + `tx.query(...)` mixed in one
//! transaction → `commit()` persists, `cancel()` rolls back (and reads outside the
//! tx don't see uncommitted writes).

use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

use frona::memory::pkm::model::KnowledgeEntitySource;

async fn count_links(db: &Surreal<Db>) -> usize {
    let mut r = db
        .query("SELECT VALUE meta::id(id) FROM knowledge_entity_source")
        .await
        .unwrap();
    let ids: Vec<String> = r.take(0).unwrap();
    ids.len()
}

async fn page_path_of(db: &Surreal<Db>, mid: &str) -> Option<String> {
    let mut r = db
        .query("SELECT VALUE entity_path FROM knowledge_entity_source WHERE memory_id = $m")
        .bind(("m", mid.to_string()))
        .await
        .unwrap();
    let v: Vec<String> = r.take(0).unwrap();
    v.into_iter().next()
}

fn link(id: &str, mid: &str, path: &str) -> KnowledgeEntitySource {
    KnowledgeEntitySource {
        id: id.to_string(),
        user_id: "u".to_string(),
        memory_id: mid.to_string(),
        entity_path: path.to_string(),
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn transaction_persists_when_create_and_query_commit() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    frona::db::init::setup_schema(&db).await.unwrap();

    let tx = db.clone().begin().await.unwrap();
    // struct insert via the builder (result type annotated, as the repo does) …
    let _: Option<surrealdb::types::Value> = tx
        .create(("knowledge_entity_source", "l1".to_string()))
        .content(link("l1", "m1", "people/bob"))
        .await
        .unwrap();
    // … mixed with a raw query, in the same transaction
    tx.query("UPDATE knowledge_entity_source SET entity_path = $p WHERE memory_id = $m")
        .bind(("p", "people/robert".to_string()))
        .bind(("m", "m1".to_string()))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(count_links(&db).await, 1, "commit persisted the create");
    assert_eq!(
        page_path_of(&db, "m1").await.as_deref(),
        Some("people/robert"),
        "the in-tx UPDATE persisted too"
    );
}

#[tokio::test]
async fn transaction_rolls_back_when_cancelled() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    frona::db::init::setup_schema(&db).await.unwrap();

    // Seed one committed row so we can tell "rolled back" from "empty DB".
    let tx = db.clone().begin().await.unwrap();
    let _: Option<surrealdb::types::Value> = tx
        .create(("knowledge_entity_source", "l1".to_string()))
        .content(link("l1", "m1", "people/bob"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(count_links(&db).await, 1);

    let tx = db.clone().begin().await.unwrap();
    let _: Option<surrealdb::types::Value> = tx
        .create(("knowledge_entity_source", "l2".to_string()))
        .content(link("l2", "m2", "people/alice"))
        .await
        .unwrap();
    tx.cancel().await.unwrap();

    assert_eq!(
        count_links(&db).await,
        1,
        "cancel rolled back the second create"
    );
    assert!(
        page_path_of(&db, "m2").await.is_none(),
        "m2's link never landed"
    );
}

#[tokio::test]
async fn transaction_closes_when_its_owner_task_is_aborted() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    frona::db::init::setup_schema(&db).await.unwrap();
    let (opened_tx, opened_rx) = tokio::sync::oneshot::channel();
    let task_db = db.clone();
    let owner = tokio::spawn(async move {
        let tx = task_db.begin().await.unwrap();
        let _: Option<surrealdb::types::Value> = tx
            .create(("knowledge_entity_source", "aborted-link".to_string()))
            .content(link("aborted-link", "aborted-memory", "people/alice"))
            .await
            .unwrap();
        opened_tx.send(()).unwrap();
        std::future::pending::<()>().await;
        tx.cancel().await.unwrap();
    });
    opened_rx.await.unwrap();

    owner.abort();
    owner.await.unwrap_err();

    let links = tokio::time::timeout(std::time::Duration::from_secs(1), count_links(&db))
        .await
        .expect("an aborted transaction owner must not leave the database writer locked");
    assert_eq!(
        links, 0,
        "the aborted transaction must roll back its uncommitted row"
    );
}
