use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

use crate::Entity;
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "space")]
pub struct Space {
    pub id: String,
    pub user_id: String,
    pub name: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSpaceRequest {
    pub name: String,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSpaceRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// Partial metadata patch: keys with `null` values are removed; other keys are upserted.
    /// Keys not present in this map are left untouched.
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Serialize)]
pub struct SpaceResponse {
    pub id: String,
    pub name: String,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Space> for SpaceResponse {
    fn from(space: Space) -> Self {
        Self {
            id: space.id,
            name: space.name,
            metadata: space.metadata,
            created_at: space.created_at,
            updated_at: space.updated_at,
        }
    }
}
