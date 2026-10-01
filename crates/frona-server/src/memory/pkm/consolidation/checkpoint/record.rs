use chrono::{DateTime, Utc};
use frona_derive::Entity;
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use super::ConsolidationStageState;
use crate::memory::pkm::consolidation::ConsolidationStats;

/// A consolidation pass in progress, or the log of one that finished.
///
/// `Knowledge*` per the module's naming rule (`pkm/mod.rs`): `pkm` names the subsystem,
/// `knowledge` names the persisted artifacts. The stage-state types below are fields of
/// this record rather than tables of their own, so they keep the plain names.
///
/// At most one pass is live per user - the sweep either resumes the open record or opens
/// a fresh one - so "the user's current pass" is the newest row. `id` is a UUIDv7, which
/// is time-ordered, so newest is `ORDER BY id DESC LIMIT 1` with no separate clock.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "knowledge_consolidation_record")]
pub struct KnowledgeConsolidationRecord {
    pub id: String,
    /// Immutable scope key for all durable working-entity rows in this pass.
    pub consolidation_id: String,
    pub user_id: String,
    /// Where the pass is. `Done` is terminal - the row survives as a pass log.
    pub state: ConsolidationStageState,
    /// Counts folded in as each stage completes; the whole pass's totals once `Done`.
    pub stats: ConsolidationStats,
    /// Consecutive failures **at the current stage**. Reset on advance, because making
    /// progress should restore the budget - a long pass that hiccups once at three
    /// different stages is healthy, not poisoned.
    pub attempts: u32,
    /// Fatal post-extraction recovery failures. Ordinary provider/model retries do not
    /// consume this budget.
    pub restart_count: u32,
    pub failure: Option<ConsolidationFailure>,
    /// Earliest the sweep may try this record again. Set to `now + base * 2^attempts` on
    /// failure so the sweep's selection stays a query rather than a fetch-then-filter.
    pub next_attempt_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct ConsolidationFailure {
    pub stage: String,
    pub error: String,
    pub affected_paths: Vec<String>,
    pub affected_count: usize,
    pub failed_at: DateTime<Utc>,
}
