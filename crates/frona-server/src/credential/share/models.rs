use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::Entity;

/// Discriminated payload. Extensible — add new variants without schema changes.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", tag = "type", rename_all = "snake_case")]
pub enum ShareKind {
    /// Indirection to a workspace file.
    ///
    /// `public = false` (default): `/s/{id}` redirects to canonical
    /// `/api/files/{owner}/{path}` — auth enforced downstream by `FileAuth`.
    /// This is what channel adapters issue.
    ///
    /// `public = true`: `/s/{id}` mints a fresh presigned URL via
    /// `PresignService` and redirects to it — anyone with the short link
    /// can download without logging in. Reserved for a future
    /// "share publicly" UI; channel adapters never issue in this mode.
    File {
        owner: String,
        path: String,
        #[serde(default)]
        public: bool,
    },
    /// Issued lazily by SMS overflow so the body stays under one segment
    /// plus a `/s/{id}` link that resolves to `/chats/{chat_id}`.
    Chat {
        chat_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "share")]
pub struct Share {
    pub id: String,
    pub user_id: String,
    pub kind: ShareKind,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
