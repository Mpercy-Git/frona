//! **Resolve** - decide whether a mention is the same entity as an existing entity.
//!
//! Identity-signal-filtered by design: FTS retrieves broadly, proven type disjointness
//! removes impossible identities, and exact, ordered, token-order, or event-participant
//! name evidence prunes noise. Missing subsumption remains uncertainty for the model to
//! inspect.
//! Types in force include what this pass merely *proposed*, since nothing is stamped
//! until `assemble` runs.
//!
//! One class of candidate skips that filter: an entity the reasoner has concluded is
//! `owl:sameAs` this one, which a `functional` property derives when a subject that can
//! have only one value turns out to have two. That is a conclusion about identity
//! drawn from the graph itself, so it neither needs the name search that finds
//! candidates nor the type check that prunes them - it is strictly stronger evidence
//! than either. It is still only a *candidate*: the merge deletes an entity, and the
//! conclusion rests on a characteristic proposed during Classify, so it earns a verdict
//! like any other.

mod evidence;
mod run;

pub(super) use evidence::{
    IdentityMatch, IdentityResolution, ResolutionDecisionContext, identity_matches,
    pair_change_requires_judgment, resolution_identity_fingerprint, resolution_pair_fingerprint,
    resolution_pair_key,
};
