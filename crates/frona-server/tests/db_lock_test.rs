use surrealdb::Surreal;
use surrealdb::engine::local::RocksDb;
use tempfile::tempdir;

#[tokio::test]
async fn test_rocksdb_lock_error_when_already_open() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("test_db");
    let path_str = path.to_str().unwrap();

    let _db1 = Surreal::new::<RocksDb>(path_str).await.unwrap();

    let db2 = Surreal::new::<RocksDb>(path_str).await;
    assert!(db2.is_err());

    let err = db2.unwrap_err().to_string();
    assert!(
        err.contains("LOCK"),
        "Expected lock error, got: {err}"
    );
}

#[tokio::test]
async fn test_init_retries_then_succeeds_when_lock_released() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("test_db");
    let path_str = path.to_str().unwrap();

    let db1 = Surreal::new::<RocksDb>(path_str).await.unwrap();

    let owned_path = path_str.to_string();
    let handle = tokio::spawn(async move {
        frona::db::init::init(&owned_path).await
    });

    // Release the lock after a short delay
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    drop(db1);

    let result = handle.await.unwrap();
    assert!(result.is_ok(), "init should succeed after lock is released, got: {:?}", result.err());
}
