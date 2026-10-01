use super::*;

/// One mention the extractor proposed, ready to commit.
pub struct PendingEntity {
    pub path: String,
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub identity_evidence: Vec<MemoryEvidence>,
    /// Free-text-keyed properties as stated (`{"employer": "Example Corp"}`). The extractor emits
    /// **no** relations: whether a property is a literal or an edge to another entity needs
    /// the catalogue and the entity set, so the Classify stage decides it a stage later.
    ///
    /// **Merged** into an existing entity, not confined to its creation - a key the entity does
    /// not carry and whose statement no fact-memory records is added, and gets a memory
    /// mirrored. Extract is the only stage that reads the transcript, so a property it drops
    /// here is recoverable by nobody: while these were create-only, an entity first seen in one
    /// conversation could never gain a property stated in a later one.
    ///
    /// A key the entity already carries keeps its value. Extract owns *whether a key exists*;
    /// reconcile owns *what it says*, and letting a stage that never sees the entity overwrite
    /// the stage that maintains it would have the two fight every pass. The dedupe is against
    /// the fact-memories rather than the map because promotion **deletes** the key it turned
    /// into an edge - so a missing key means either "never stated" or "already an edge", and
    /// only the memory distinguishes them.
    pub attributes: serde_json::Value,
    pub attribute_evidence: std::collections::HashMap<String, Vec<MemoryEvidence>>,
}

/// Attribute candidates for an entity the extractor was explicitly told already exists.
/// The commit path is update-only: if the entity disappeared or the model supplied an
/// unknown path, this entry is ignored rather than turning an update into entity creation.
pub struct PendingEntityUpdate {
    pub path: String,
    pub attributes: serde_json::Value,
    pub attribute_evidence: std::collections::HashMap<String, Vec<MemoryEvidence>>,
}

/// One entity's attribute decisions from classify, applied inside the schema commit.
///
/// Both halves have to land in the same transaction as the schema: a key re-keyed to a
/// CURIE the TBox never declared, or an edge whose property was never minted, is exactly
/// the split state `commit_schema_and_types` exists to prevent.
#[derive(Debug, Default)]
pub struct AttributeOps {
    pub path: String,
    /// `(old_key, new_curie)` - stays an attribute, re-keyed.
    pub rekeys: Vec<(String, String)>,
    /// `(key, relation_curie, target_path)` - the value named another entity, so the
    /// attribute becomes an edge: the key is dropped and a link created.
    pub promoted: Vec<(String, String, String)>,
    /// `(relation_curie, old_target_path)` asserted edges invalidated by replacement.
    pub retracted: Vec<(String, String)>,
}

/// One fact the extractor proposed, with the entities it belongs to.
pub struct PendingMemory {
    /// Allocated before the extraction transaction so the checkpoint can reference the
    /// exact durable row written beside it.
    pub id: String,
    pub kind: MemoryKind,
    pub evidence: Vec<MemoryEvidence>,
    pub episode: Option<crate::memory::pkm::model::Episode>,
    pub content: String,
    pub paths: Vec<String>,
}

/// Everything one transcript window produced, committed as a unit.
///
/// Extract used to write this straight out - a transaction per row - and only then
/// advance the watermark. A failure part-way left the earlier rows committed with the
/// watermark unmoved, so the next sweep re-read the same transcript and minted them
/// again; since nothing dedups memory content, the duplicates simply accumulated. Making
/// the window atomic removes the partial state rather than compensating for it.
#[derive(Default)]
pub struct IngestBatch {
    pub entities: Vec<PendingEntity>,
    pub entity_updates: Vec<PendingEntityUpdate>,
    pub memories: Vec<PendingMemory>,
    /// Provisional procedural-memory groups. The extraction transaction merges these
    /// values into Playbook consolidation entity rows.
    pub playbook_candidates: Vec<crate::memory::pkm::model::PendingPlaybookCandidate>,
    pub grounding_corrections: usize,
    pub grounding_items_dropped: usize,
    pub recall_result_lookups: usize,
    pub agent_evidence_no_tool_drops: usize,
    pub agent_evidence_strong_matches: usize,
    pub agent_evidence_fallback_reviews: usize,
    pub agent_evidence_fallback_retains: usize,
    pub agent_evidence_invalid_submissions: usize,
    pub agent_evidence_lookup_calls: usize,
    pub agent_evidence_terminal_drops: usize,
    pub research_coverage: crate::memory::pkm::ResearchCoverageStats,
}

impl IngestBatch {
    /// Move another mined window into this commit batch.
    pub fn merge_from(&mut self, other: &mut Self) {
        self.entities.append(&mut other.entities);
        self.entity_updates.append(&mut other.entity_updates);
        self.memories.append(&mut other.memories);
        self.playbook_candidates
            .append(&mut other.playbook_candidates);
        self.grounding_corrections += other.grounding_corrections;
        self.grounding_items_dropped += other.grounding_items_dropped;
        self.recall_result_lookups += other.recall_result_lookups;
        self.agent_evidence_no_tool_drops += other.agent_evidence_no_tool_drops;
        self.agent_evidence_strong_matches += other.agent_evidence_strong_matches;
        self.agent_evidence_fallback_reviews += other.agent_evidence_fallback_reviews;
        self.agent_evidence_fallback_retains += other.agent_evidence_fallback_retains;
        self.agent_evidence_invalid_submissions += other.agent_evidence_invalid_submissions;
        self.agent_evidence_lookup_calls += other.agent_evidence_lookup_calls;
        self.agent_evidence_terminal_drops += other.agent_evidence_terminal_drops;
        self.research_coverage.add(&other.research_coverage);
    }
}

pub struct IngestWindow {
    pub batch: IngestBatch,
    pub watermark: Option<(String, DateTime<Utc>)>,
    pub short_memory_ids: Vec<String>,
}

/// One canonical Playbook projection produced by Playbook Resolve. The repository owns
/// this write shape so every entity move, memory association, derived entity link, and the
/// checkpoint transition can share one database transaction.
#[derive(Debug, Clone)]
pub struct PlaybookResolutionWrite {
    pub candidate_ids: Vec<String>,
    pub candidate_paths: Vec<String>,
    pub existing_path: Option<String>,
    pub merge_from: Vec<String>,
    pub path: String,
    pub name: String,
    pub description: String,
    pub memory_ids: Vec<String>,
}

pub struct AuthoredPageWrite {
    pub path: String,
    pub name: String,
    pub description: String,
    pub attributes: serde_json::Value,
    pub body: String,
    pub related_playbooks: Vec<String>,
    /// Exact canonical Markdown bytes paired with `rev`.
    pub content: String,
    pub rev: String,
}

/// Durable progress for one accepted External note revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalPageProgress {
    pub rev: String,
    pub mirror_pending: bool,
    pub extraction_pending: bool,
}

impl ExternalPageProgress {
    pub fn is_complete(&self) -> bool {
        !self.mirror_pending && !self.extraction_pending
    }
}

/// The External note revision guarded by an extraction transaction.
#[derive(Debug, Clone)]
pub(crate) struct ExternalExtractionWrite {
    pub path: String,
    pub rev: String,
}

pub(crate) enum ExtractCommit {
    Applied(IngestCounts),
    Stale,
}

/// The page state that a human edit was planned against.
/// `Missing` is distinct from an existing entity whose projection has no revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageEditBase {
    Missing,
    Revision(String),
}

/// One validated memory change from a human page edit. The sync layer removes invalid
/// model output before it gives the plan to the repository.
#[derive(Debug, Clone)]
pub(crate) enum PageEditMemoryOp {
    Add {
        kind: MemoryKind,
        content: String,
    },
    Supersede {
        older_id: String,
        kind: MemoryKind,
        content: String,
        note: String,
    },
    SetDisposition {
        memory_id: String,
        disposition: Disposition,
    },
}

/// A complete human page edit. The repository applies this plan only when `base`
/// still identifies the current page state.
#[derive(Debug, Clone)]
pub(crate) struct PageEditWrite {
    pub base: PageEditBase,
    pub new_page_name: Option<String>,
    pub body: String,
    pub content: String,
    pub rev: String,
    pub memory_ops: Vec<PageEditMemoryOp>,
}

/// The result of the repository's final compare-and-set operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageEditCommit {
    Applied,
    Conflict {
        head_rev: Option<String>,
        head_content: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ReconcileMemoryRelationWrite {
    pub subordinate_id: String,
    pub relation: RelationType,
    pub to_id: String,
    pub note: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ReconcileOutdatedWrite {
    pub memory_id: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ReconcileEntityLinkSourceWrite {
    pub from_entity_path: String,
    pub to_entity_path: String,
    pub relation: String,
    pub source_memory_ids: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ReconcileCommit {
    pub memory_relations: Vec<ReconcileMemoryRelationWrite>,
    pub outdated_memories: Vec<ReconcileOutdatedWrite>,
    pub entity_link_sources: Vec<ReconcileEntityLinkSourceWrite>,
    pub entity: Option<KnowledgeConsolidationEntity>,
}

/// What committing a window actually created - the counts only the repo can know, since
/// whether an entity is new is decided inside the transaction.
#[derive(Debug, Default)]
pub struct IngestCounts {
    pub entities_created: usize,
    pub memories_added: usize,
}
