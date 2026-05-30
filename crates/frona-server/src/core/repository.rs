use async_trait::async_trait;
use uuid::{NoContext, Timestamp, Uuid};

use super::error::AppError;

pub trait Entity: Clone + Send + Sync {
    fn table() -> &'static str;
    fn id(&self) -> &str;
}

/// Generates a fresh entity ID. v7 (time-ordered) UUID rendered as a 36-char
/// hyphenated string, so RocksDB B-tree leaves stay monotonic across inserts.
pub fn new_id() -> String {
    Uuid::new_v7(Timestamp::now(NoContext)).to_string()
}

#[async_trait]
pub trait Repository<T: Send + Sync>: Send + Sync {
    async fn create(&self, entity: &T) -> Result<T, AppError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<T>, AppError>;
    async fn update(&self, entity: &T) -> Result<T, AppError>;
    async fn delete(&self, id: &str) -> Result<(), AppError>;
}
