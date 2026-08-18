mod ingest;
mod record;
mod stage;

#[cfg(test)]
mod tests;

pub use ingest::IngestState;
pub use record::{ConsolidationFailure, KnowledgeConsolidationRecord};
pub use stage::ConsolidationStageState;
pub(crate) use ingest::prepare_ingest_batch;
