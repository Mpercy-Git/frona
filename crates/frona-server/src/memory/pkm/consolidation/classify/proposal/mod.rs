mod entity;
mod schema;

pub(crate) use entity::ProposalSet;
pub(crate) use schema::{Classification, OntologyDeclaration};
pub(crate) use entity::EntityProposal;
pub(crate) use schema::HasKeyMarker;
pub(super) use schema::{
    AttributeDecisions, AttributeMapping, NewEntity, RelationMapping,
    accept_mints, attribute_edits,
    classification_edits, render_value, search_terms, ATTRIBUTE_CANDIDATES,
    EVIDENCE_VOCAB_HITS,
};
#[cfg(test)]
pub(super) use schema::{
    AcceptedMint, EntityShape, ClassChoice,
};
