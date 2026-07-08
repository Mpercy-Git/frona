use super::models::{CandidateEvent, Watch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatcherKind {
    HardFilter,
    Scoring,
}

pub trait Matcher: Send + Sync {
    fn name(&self) -> &str;
    fn kind(&self) -> MatcherKind;
    /// `false` skips this matcher entirely for `watch` (abstain — neither
    /// matches nor rejects).
    fn is_active(&self, watch: &Watch) -> bool;
    /// Called only when `is_active(watch)` is true.
    /// HardFilter: `Some(_)` passes, `None` vetoes.
    /// Scoring:    `Some(n)` matches with score `n`, `None` contributes nothing.
    fn evaluate(&self, candidate: &CandidateEvent, watch: &Watch) -> Option<u32>;
}
