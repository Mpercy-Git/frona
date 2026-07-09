use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "keypair")]
pub struct KeyPair {
    pub id: String,
    pub owner: String,
    pub public_key_bytes: Vec<u8>,
    pub private_key_enc: Vec<u8>,
    pub nonce: Vec<u8>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
