//! **Classify** - classify untyped mentions and collect their proposals.
//!
//! Instance-blind: each mention is characterized from its own evidence, and the classes
//! it proposes are validated by a scoped reasoner pass before they are accepted. Nothing
//! here commits schema or stamps an entity - everything lands in [`ProposalSet`],
//! which `assemble` later turns into one CAS write.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn claim_automatic_identity_discovery(
    discovery_calls: &AtomicUsize,
    challenged: &AtomicBool,
) -> bool {
    discovery_calls.load(Ordering::Relaxed) == 0 && !challenged.swap(true, Ordering::Relaxed)
}

mod proposal;
mod run;

#[cfg(test)]
pub(super) use proposal::EntityProposal;
pub(super) use proposal::HasKeyMarker;
pub(crate) use proposal::{Classification, OntologyDeclaration, ProposalSet};

#[cfg(test)]
mod tests;
