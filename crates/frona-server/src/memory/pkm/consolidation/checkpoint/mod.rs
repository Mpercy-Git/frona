mod ingest;
mod record;
mod stage;

#[cfg(test)]
mod tests;

pub use ingest::IngestState;
pub(crate) use ingest::prepare_ingest_batch;
pub use record::{ConsolidationFailure, KnowledgeConsolidationRecord};
pub use stage::ConsolidationStageState;
