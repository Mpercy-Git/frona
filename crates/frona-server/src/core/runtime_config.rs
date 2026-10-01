use serde::Serialize;
use serde::de::DeserializeOwned;
use surrealdb::Surreal;
use surrealdb::engine::local::Db;

use crate::core::error::AppError;

#[derive(Clone)]
pub struct RuntimeConfigStore {
    db: Surreal<Db>,
}

impl RuntimeConfigStore {
    pub fn new(db: Surreal<Db>) -> Self {
        Self { db }
    }

    pub async fn get<T>(&self, key: &str) -> Result<Option<T>, AppError>
    where
        T: DeserializeOwned,
    {
        let Some(raw) = self.get_raw(key).await? else {
            return Ok(None);
        };
        serde_json::from_str(&raw).map(Some).map_err(|error| {
            AppError::Internal(format!(
                "runtime config '{key}' is not valid {} JSON: {error}",
                std::any::type_name::<T>(),
            ))
        })
    }

    pub async fn set<T>(&self, key: &str, value: &T) -> Result<(), AppError>
    where
        T: Serialize,
    {
        let raw = encode(key, value)?;
        self.set_raw(key, &raw).await
    }

    pub async fn list_prefix<T>(&self, prefix: &str) -> Result<Vec<(String, T)>, AppError>
    where
        T: DeserializeOwned,
    {
        let mut result = self
            .db
            .query("SELECT `key`, `value` FROM runtime_config")
            .await
            .map_err(db_error)?;
        let rows: Vec<serde_json::Value> = result.take(0).map_err(db_error)?;
        rows.into_iter()
            .filter_map(|row| {
                let key = row.get("key")?.as_str()?.to_string();
                let value = row.get("value")?.as_str()?.to_string();
                key.starts_with(prefix).then_some((key, value))
            })
            .map(|(key, raw)| {
                serde_json::from_str(&raw)
                    .map(|value| (key.clone(), value))
                    .map_err(|error| {
                        AppError::Internal(format!(
                            "runtime config '{key}' is not valid {} JSON: {error}",
                            std::any::type_name::<T>(),
                        ))
                    })
            })
            .collect()
    }

    pub async fn compare_exchange<T>(
        &self,
        key: &str,
        expected: Option<&T>,
        replacement: Option<&T>,
    ) -> Result<bool, AppError>
    where
        T: Serialize,
    {
        let expected = expected.map(|value| encode(key, value)).transpose()?;
        let replacement = replacement.map(|value| encode(key, value)).transpose()?;
        let tx = self.db.clone().begin().await.map_err(db_error)?;

        let mut result = tx
            .query("SELECT VALUE `value` FROM runtime_config WHERE `key` = $key LIMIT 1")
            .bind(("key", key.to_string()))
            .await
            .map_err(db_error)?;
        let current: Option<String> = result.take(0).map_err(db_error)?;
        if current != expected {
            tx.cancel().await.map_err(db_error)?;
            return Ok(false);
        }

        let mutation = match replacement {
            Some(value) if current.is_some() => tx
                .query("UPDATE runtime_config SET `value` = $value, updated_at = $now WHERE `key` = $key")
                .bind(("key", key.to_string()))
                .bind(("value", value))
                .bind(("now", chrono::Utc::now()))
                .await,
            Some(value) => tx
                .query("CREATE runtime_config SET `key` = $key, `value` = $value, updated_at = $now")
                .bind(("key", key.to_string()))
                .bind(("value", value))
                .bind(("now", chrono::Utc::now()))
                .await,
            None => tx
                .query("DELETE runtime_config WHERE `key` = $key")
                .bind(("key", key.to_string()))
                .await,
        };
        if let Err(error) = mutation.and_then(|response| response.check()) {
            let _ = tx.cancel().await;
            if compare_failed(&error) {
                return Ok(false);
            }
            return Err(db_error(error));
        }
        if let Err(error) = tx.commit().await {
            if compare_failed(&error) {
                return Ok(false);
            }
            return Err(db_error(error));
        }
        Ok(true)
    }

    pub async fn get_raw(&self, key: &str) -> Result<Option<String>, AppError> {
        let mut result = self
            .db
            .query("SELECT `value` FROM runtime_config WHERE `key` = $key LIMIT 1")
            .bind(("key", key.to_string()))
            .await
            .map_err(db_error)?;
        let row: Option<serde_json::Value> = result.take(0).map_err(db_error)?;
        Ok(row.and_then(|value| {
            value
                .get("value")
                .and_then(|value| value.as_str())
                .map(String::from)
        }))
    }

    pub async fn set_raw(&self, key: &str, value: &str) -> Result<(), AppError> {
        self.db
            .query(
                "DELETE FROM runtime_config WHERE `key` = $key; \
                 CREATE runtime_config SET `key` = $key, `value` = $value, updated_at = $now",
            )
            .bind(("key", key.to_string()))
            .bind(("value", value.to_string()))
            .bind(("now", chrono::Utc::now()))
            .await
            .map_err(db_error)?
            .check()
            .map_err(db_error)?;
        Ok(())
    }
}

fn encode<T: Serialize>(key: &str, value: &T) -> Result<String, AppError> {
    serde_json::to_string(value).map_err(|error| {
        AppError::Internal(format!(
            "runtime config '{key}' could not be encoded: {error}"
        ))
    })
}

fn db_error(error: surrealdb::Error) -> AppError {
    AppError::Internal(format!("runtime config: {error}"))
}

fn compare_failed(error: &surrealdb::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("unique")
        || message.contains("transaction conflict")
        || message.contains("write conflict")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use surrealdb::engine::local::Mem;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Value {
        state: String,
        version: u32,
    }

    async fn store() -> RuntimeConfigStore {
        let db = Surreal::new::<Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        RuntimeConfigStore::new(db)
    }

    #[tokio::test]
    async fn typed_values_round_trip_and_list_by_prefix() {
        let store = store().await;
        let value = Value {
            state: "pending".into(),
            version: 1,
        };
        store.set("pkm.reset.u1", &value).await.unwrap();
        store
            .set(
                "other",
                &Value {
                    state: "other".into(),
                    version: 2,
                },
            )
            .await
            .unwrap();

        assert_eq!(
            store.get::<Value>("pkm.reset.u1").await.unwrap(),
            Some(value.clone())
        );
        assert_eq!(
            store.list_prefix::<Value>("pkm.reset.").await.unwrap(),
            vec![("pkm.reset.u1".into(), value)]
        );
    }

    #[tokio::test]
    async fn compare_exchange_creates_replaces_and_deletes_exact_values() {
        let store = store().await;
        let pending = Value {
            state: "pending".into(),
            version: 1,
        };
        let running = Value {
            state: "running".into(),
            version: 1,
        };

        assert!(
            store
                .compare_exchange("job", None, Some(&pending))
                .await
                .unwrap()
        );
        assert!(
            !store
                .compare_exchange("job", None, Some(&pending))
                .await
                .unwrap()
        );
        assert!(
            !store
                .compare_exchange("job", Some(&running), Some(&pending))
                .await
                .unwrap()
        );
        assert!(
            store
                .compare_exchange("job", Some(&pending), Some(&running))
                .await
                .unwrap()
        );
        assert!(
            store
                .compare_exchange("job", Some(&running), None)
                .await
                .unwrap()
        );
        assert_eq!(store.get::<Value>("job").await.unwrap(), None);
    }

    #[tokio::test]
    async fn compare_exchange_allows_only_one_concurrent_create() {
        let store = store().await;
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
        let mut tasks = Vec::new();
        for version in [1, 2] {
            let store = store.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                let value = Value {
                    state: "pending".into(),
                    version,
                };
                barrier.wait().await;
                store
                    .compare_exchange("one-job", None, Some(&value))
                    .await
                    .unwrap()
            }));
        }
        barrier.wait().await;
        let first = tasks.remove(0).await.unwrap();
        let second = tasks.remove(0).await.unwrap();
        assert_ne!(first, second);
        assert!(store.get::<Value>("one-job").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn raw_strings_remain_compatible() {
        let store = store().await;
        store
            .set_raw("encryption_secret", "not-json")
            .await
            .unwrap();
        assert_eq!(
            store.get_raw("encryption_secret").await.unwrap().as_deref(),
            Some("not-json")
        );
        assert!(store.get::<Value>("encryption_secret").await.is_err());
        assert_eq!(
            store.get_raw("encryption_secret").await.unwrap().as_deref(),
            Some("not-json")
        );
    }
}
