use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;
pub(crate) fn prepare_ingest_batch(batch: &mut crate::db::repo::pkm::IngestBatch) {
    let attributes: Vec<_> = batch.entities.iter()
        .map(|entity| (entity.path.clone(), entity.attributes.clone(), entity.attribute_evidence.clone()))
        .chain(batch.entity_updates.iter()
            .map(|entity| (entity.path.clone(), entity.attributes.clone(), entity.attribute_evidence.clone())))
        .collect();
    for (path, attributes, evidence_by_key) in attributes {
        for (key, value) in attributes.as_object().into_iter().flatten() {
            let Some(evidence) = evidence_by_key.get(key) else { continue };
            let rendered = match value {
                serde_json::Value::String(value) => value.clone(),
                value => value.to_string(),
            };
            batch.memories.push(crate::db::repo::pkm::PendingMemory {
                id: crate::core::repository::new_id(),
                kind: crate::memory::pkm::model::MemoryKind::Fact,
                evidence: evidence.clone(), episode: None,
                content: format!("{key}: {rendered}"), paths: vec![path.clone()],
            });
        }
    }
}

/// Ingest - the chats mined so far. A mined chat is finished because its memories and
/// watermark are already committed, so the position is all that is needed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct IngestState {
    pub mined: BTreeSet<String>,
}
