//! Per-user config compare-and-swap - drives `UserService::user_config` /
//! `update_user_config` against an in-memory SurrealDB. Pins the `updated_at`
//! optimistic-lock behavior end to end: default-when-absent, first-write create,
//! stale-token conflict, fresh-token success, and cache invalidation on write.

use surrealdb::Surreal;
use surrealdb::engine::local::Mem;

use frona::auth::user_service::UserService;
use frona::core::config::CacheConfig;
use frona::core::error::AppError;
use frona::core::user_config::{UserConfigPatch, UserMemoryConfig};
use frona::db::repo::generic::SurrealRepo;

const U: &str = "config-user";

async fn user_service() -> UserService {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    frona::db::init::setup_schema(&db).await.unwrap();
    UserService::new(SurrealRepo::new(db.clone()), &CacheConfig::default())
}

fn patch(dir: &str) -> UserConfigPatch {
    UserConfigPatch {
        memory: Some(UserMemoryConfig {
            shared_vault_directory: dir.to_string(),
        }),
    }
}

fn epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(0, 0).unwrap()
}

#[tokio::test]
async fn user_config_default_when_no_row() {
    let svc = user_service().await;
    let c = svc.user_config(U).await.unwrap();
    assert_eq!(c.memory.shared_vault_directory, "Memory");
    // The epoch sentinel is the "no row yet" CAS token.
    assert_eq!(c.updated_at, epoch());
}

#[tokio::test]
async fn first_write_creates_via_epoch_cas() {
    let svc = user_service().await;
    let token = svc.user_config(U).await.unwrap().updated_at; // epoch
    let saved = svc.update_user_config(U, token, patch("Brain")).await.unwrap();
    assert_eq!(saved.memory.shared_vault_directory, "Brain");
    assert_ne!(saved.updated_at, token, "updated_at advanced off the epoch sentinel");
    assert_eq!(
        svc.user_config(U).await.unwrap().memory.shared_vault_directory,
        "Brain",
        "the created value is read back"
    );
}

#[tokio::test]
async fn stale_cas_is_rejected() {
    let svc = user_service().await;
    let token = svc.user_config(U).await.unwrap().updated_at;
    svc.update_user_config(U, token, patch("Brain")).await.unwrap();
    let v1 = svc.user_config(U).await.unwrap().updated_at; // real T1

    // A fresh write bumps updated_at to T2 (this also proves fresh CAS succeeds).
    svc.update_user_config(U, v1, patch("Knowledge")).await.unwrap();

    // Re-using the now-stale T1 must be rejected and must not change the value.
    let err = svc.update_user_config(U, v1, patch("Vault")).await.unwrap_err();
    assert!(
        matches!(err, AppError::Conflict(_)),
        "stale CAS token → Conflict, got {err:?}"
    );
    assert_eq!(
        svc.user_config(U).await.unwrap().memory.shared_vault_directory,
        "Knowledge",
        "the rejected write left the stored value untouched"
    );
}

#[tokio::test]
async fn cache_invalidated_on_write() {
    let svc = user_service().await;
    // Prime the cache with the synthesized default.
    assert_eq!(
        svc.user_config(U).await.unwrap().memory.shared_vault_directory,
        "Memory"
    );
    let token = svc.user_config(U).await.unwrap().updated_at;
    svc.update_user_config(U, token, patch("Brain")).await.unwrap();
    // If the write hadn't invalidated the cache, this would still read the default.
    assert_eq!(
        svc.user_config(U).await.unwrap().memory.shared_vault_directory,
        "Brain"
    );
}
