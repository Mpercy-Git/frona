//! Per-user configuration - the generic home for user-scoped settings, and the
//! user-level analog of the server [`Config`](crate::core::config::Config). Same
//! two-level shape: a top-level `UserConfig` container composes one nested
//! sub-struct per domain, each owning its own `Default`. A domain reads its section
//! off `user_service.user_config(uid).await?.<section>`; adding a section is a new
//! field here (plus one line in each writer), never a new method.
//!
//! Writes go through an `updated_at` compare-and-swap (see
//! `SurrealUserRepo::patch_user_config`): the caller passes the `updated_at` it read
//! and the write is rejected (`AppError::Conflict`) if the row moved on. The
//! generated `UserConfigPatch` (via `struct-patch`) carries only the sections a
//! writer touches; `id`/`user_id`/timestamps are `#[patch(skip)]`, so they can't be
//! set through a patch.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use struct_patch::Patch;
use surrealdb::types::SurrealValue;

use crate::Entity;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity, Patch)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "user_config")]
#[serde(default)]
#[patch(attribute(derive(Debug, Default, Serialize, Deserialize)))]
pub struct UserConfig {
    #[patch(skip)]
    pub id: String,
    #[patch(skip)]
    pub user_id: String,
    /// Memory / knowledge-sync settings. The only patchable section today.
    pub memory: UserMemoryConfig,
    #[patch(skip)]
    pub created_at: DateTime<Utc>,
    #[patch(skip)]
    pub updated_at: DateTime<Utc>,
}

impl Default for UserConfig {
    fn default() -> Self {
        // `updated_at` is the CAS token; the UNIX-epoch sentinel means "no row yet",
        // so a first write (caller read a synthesized default) hits the create path.
        let epoch = DateTime::from_timestamp(0, 0).expect("unix epoch is a valid timestamp");
        Self {
            id: String::new(),
            user_id: String::new(),
            memory: UserMemoryConfig::default(),
            created_at: epoch,
            updated_at: epoch,
        }
    }
}

/// Memory-domain per-user config. Its `Default` owns the `"Memory"` default; the
/// memory service owns *validation* of the directory name (single clean segment) -
/// this struct stays a dumb bag so the core layer learns no vault-path rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct UserMemoryConfig {
    /// Top-level directory in the user's Obsidian vault that the agent's memory
    /// syncs into - the bidirectional agent↔human surface. The only directory that
    /// surfaces in vault paths/wikilinks (`pkm` stays a server-internal detail).
    pub shared_vault_directory: String,
}

impl Default for UserMemoryConfig {
    fn default() -> Self {
        Self {
            shared_vault_directory: "Memory".to_string(),
        }
    }
}
