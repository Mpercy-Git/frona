//! Extract memories and **provisional, untyped entity mentions** from the transcript.
//! The extractor is schema-**and**-instance-**blind**: it
//! emits what is said, assigning no type/kind and doing **no identity resolution**.
//! Classify types the mentions. Resolve merges duplicates with type filters before
//! Reconcile runs. An ingest failure must propagate so the transcript watermark does
//! not advance and the same input can be retried.

mod cleanup;
mod correction;
mod evidence;
mod run;
mod submission;
mod temporal;
mod validation;

pub use run::Ingest;

#[cfg(test)]
mod tests;
