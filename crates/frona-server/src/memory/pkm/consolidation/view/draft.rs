use std::collections::{BTreeMap, HashSet};

use crate::core::error::AppError;
use crate::memory::pkm::model::{ConsolidationEntityLifecycle, KnowledgeConsolidationEntity};

pub(super) enum DraftEntityLookup {
    Found(Box<KnowledgeConsolidationEntity>),
    Missing,
    DurablePath(String),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct EntityDraft {
    rows: BTreeMap<String, KnowledgeConsolidationEntity>,
}

impl EntityDraft {
    pub(crate) fn from_rows(rows: impl IntoIterator<Item = KnowledgeConsolidationEntity>) -> Self {
        Self {
            rows: rows
                .into_iter()
                .map(|row| (row.path.clone(), row))
                .collect(),
        }
    }

    pub(crate) fn rows(&self) -> impl Iterator<Item = &KnowledgeConsolidationEntity> {
        self.rows.values()
    }

    pub(super) fn resolve(&self, path: &str) -> Result<DraftEntityLookup, AppError> {
        let mut current_path = path;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current_path.to_string()) {
                return Err(AppError::Internal(format!(
                    "entity view draft: redirect cycle at `{current_path}`"
                )));
            }
            let Some(row) = self.rows.get(current_path) else {
                return Ok(DraftEntityLookup::DurablePath(current_path.to_string()));
            };
            match row.lifecycle {
                ConsolidationEntityLifecycle::Coalesced => {
                    let Some(canonical_path) = row.canonical_path.as_deref() else {
                        return Err(AppError::Internal(format!(
                            "entity view draft: coalesced row `{current_path}` has no canonical path"
                        )));
                    };
                    current_path = canonical_path;
                }
                ConsolidationEntityLifecycle::Discarded => {
                    return Ok(DraftEntityLookup::Missing);
                }
                ConsolidationEntityLifecycle::Pending | ConsolidationEntityLifecycle::Active => {
                    return Ok(DraftEntityLookup::Found(Box::new(row.clone())));
                }
            }
        }
    }
}
