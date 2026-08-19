use crate::memory::pkm::{KnowledgeConsolidationRecord, model::KnowledgeConsolidationEntity};

pub(crate) struct EntityTransition {
    pub(crate) rows: Vec<KnowledgeConsolidationEntity>,
    pub(crate) checkpoint: KnowledgeConsolidationRecord,
}

impl EntityTransition {
    pub(crate) fn new(checkpoint: KnowledgeConsolidationRecord) -> Self {
        Self {
            rows: Vec::new(),
            checkpoint,
        }
    }

    pub(crate) fn with_row(mut self, row: KnowledgeConsolidationEntity) -> Self {
        self.rows.push(row);
        self
    }

    pub(crate) fn with_rows(
        mut self,
        rows: impl IntoIterator<Item = KnowledgeConsolidationEntity>,
    ) -> Self {
        self.rows.extend(rows);
        self
    }
}
