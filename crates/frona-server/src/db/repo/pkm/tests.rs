use super::*;
    use super::test_support::{
        commit_checkpointed_extract_patch, mark_entity_rendered,
        seed_asserted_entity_link, seed_reconciled_entity,
    };
    use crate::memory::pkm::model::{EvidenceSource, EvidenceStrength, classify_memories};
    use surrealdb::engine::local::Mem;

    async fn repo() -> PkmRepo {
        let db = Surreal::new::<Mem>(()).await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        PkmRepo::new(db, 10)
    }

    fn human_evidence(content: &str, entity_path: &str) -> Vec<MemoryEvidence> {
        vec![MemoryEvidence {
            strength: EvidenceStrength::Explicit,
            source: EvidenceSource::HumanEdit {
                page_path: entity_path.to_string(),
                quote: content.to_string(),
            },
        }]
    }

    fn attribute_evidence(
        path: &str,
        attrs: &serde_json::Value,
    ) -> std::collections::HashMap<String, Vec<MemoryEvidence>> {
        attrs.as_object().into_iter().flatten().map(|(key, value)| {
            let value = match value {
                serde_json::Value::String(value) => value.clone(),
                value => value.to_string(),
            };
            (key.clone(), human_evidence(&format!("{key}: {value}"), path))
        }).collect()
    }

mod checkpoint;
mod entity;
mod extraction;
mod memory;
mod overlay;
mod playbook;
mod recovery;
