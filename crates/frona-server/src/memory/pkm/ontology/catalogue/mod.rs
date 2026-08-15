//! The ontology **catalogue** - every term the server can see - and the
//! **projection** a reasoning pass is actually cut from it.
//!
//! Two levels, and the split between them is the whole design:
//!
//!   - The [`OntologyCatalogue`] is *everything*: two roots (the downloaded
//!     frona-ontologies release, and whatever the user added) absorbed into one
//!     interned graph. It is searchable and it answers ancestry and disjointness,
//!     but it is **never materialised**. Nothing in it is loaded into a reasoner
//!     because it happens to exist.
//!   - An [`OntologyScope`] is the cut a pass reasons over: the terms the vault
//!     actually references, closed upward over ancestors, equivalence and axiom
//!     partners. ~0.4% of the catalogue.
//!
//! **The two answering paths are not interchangeable.** [`ancestors`]
//! and [`clash`] are graph walks over the interned index - no materialisation, no
//! allocation beyond the walk. Only ABox inference and SPARQL need triples, and only
//! of the cut, which is what [`project`] produces.
//!
//! [`ancestors`]: OntologyCatalogue::ancestors
//! [`clash`]: OntologyCatalogue::clash
//! [`project`]: OntologyCatalogue::project
//!
//! # Why the walk is allowed to stand in for a reasoner
//!
//! Reachability over this index returns *precisely* what OWL 2 RL derives, verified
//! against `reasonable` across the whole catalogue in both directions by
//! `tests/equivalent_to_reasoner.rs` in the `frona-ontologies` repo. That holds
//! because the artifacts contain **no anonymous class expressions**: dropping blank
//! nodes takes property chains, `someValuesFrom`, `intersectionOf` and cardinality
//! with them, leaving taxonomy, disjointness and equivalence - all of which are
//! reachability.
//!
//! A user-supplied ontology using class expressions breaks that equivalence, and the
//! failure mode is *thin answers*, not an error. So [`OntologyCatalogue::load`]
//! rejects them at load time - see [`scan_file`].

mod core;
mod loading;
mod roots;
mod scope;
mod search;

pub use core::OntologyCatalogue;
pub use roots::Roots;
pub use scope::{OntologyScope, VocabHit};
#[cfg(test)]
pub(super) use loading::format_of;

#[cfg(test)]
mod tests;
