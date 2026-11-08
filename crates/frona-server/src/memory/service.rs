//! The memory-system seam.
//!
//! A [`MemoryService`] is the abstraction over *how memory works* - the
//! foreground-facing surface only. Exactly one service is selected at boot from
//! config (`basic` or `pkm`); switching requires a restart, so the
//! trait is deliberately not designed for hot-swap.
//!
//! Foreground collection returns immutable, structured contributions. The chat
//! layer owns their persistence and replay; sources never mutate mapped history.
//! Background maintenance remains owned by each memory implementation.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core::error::AppError;
use crate::tool::AgentTool;

/// The active memory system. One implementation is chosen at boot.
#[async_trait]
pub trait MemoryService: Send + Sync {
    /// Tools this service contributes to the agent. Folded into the builtin
    /// tool set and Cedar-gated like any other tool.
    fn tools(&self) -> Vec<Arc<dyn AgentTool>>;

    /// Backend usage/navigation guidance for the standing system prompt.
    /// This is configuration-scoped text, outside snapshot hashing and diffs.
    async fn instructions(&self, user: &crate::auth::User) -> Result<String, AppError>;

    /// Collect selected memory with stable entry identities and exact rendered text.
    /// Failure aborts context preparation; an empty result is a successful read.
    async fn collect(
        &self,
        scope: &crate::agent::context::AgentContextScope<'_>,
    ) -> Result<Vec<crate::agent::context::AgentContextSection>, AppError>;

    /// Register background-maintenance jobs with the scheduler. A registration
    /// lifecycle hook (not a business method): each service registers whatever
    /// periodic upkeep it needs via `scheduler.register_periodic(...)`. Default
    /// no-op (e.g. an event-driven service that maintains itself elsewhere).
    /// Called once at `Scheduler::start()`.
    fn register_maintenance(&self, _scheduler: &crate::scheduler::Scheduler) {}
}

/// What one memory lookup costs against the run it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupVerdict {
    /// Nothing like it this run. Serve it quietly.
    Fresh,
    /// This exact query already ran in this run - `nth` counts this one. The
    /// knowledge base has not changed since, so the answer is the previous
    /// answer, and the caller says so instead of letting the agent believe it
    /// has made progress.
    Repeat { nth: usize },
    /// The run has spent its lookup budget. Refuse, and tell the agent to
    /// answer from what it has.
    Exhausted { spent: usize },
}

/// The memory lookups one inference run has already made.
///
/// A looping agent is not a bug the tool can see from inside a single call: the
/// same `memory_search` returns the same rows forever, and nothing in the
/// transcript tells the model its last three calls were identical. The ledger is
/// that memory. It rides on [`InferenceContext`](crate::inference::InferenceContext)
/// rather than in the tool because tools are built once at boot and shared by
/// every run of every user, while "have I asked this already?" is a question
/// only about *this* run.
///
/// The background consolidation stages have had a hard research-tool budget from
/// the start (`Research-tool budget exhausted. Submit the best complete result
/// now.`); this is the same idea for the foreground surface, which had none.
#[derive(Clone, Default)]
pub struct MemoryLookupLedger {
    inner: Arc<std::sync::Mutex<Ledger>>,
}

/// One run's lookups: the queries it has asked, the pages each of them returned, and
/// the pages whose text has already been put in front of the agent.
#[derive(Default)]
struct Ledger {
    queries: Vec<String>,
    served: Vec<Served>,
    text_sent: std::collections::BTreeSet<String>,
}

/// The pages one query returned, so a later query that returns nothing new can be told
/// so even though its wording is new.
struct Served {
    query: String,
    paths: std::collections::BTreeSet<String>,
}

impl MemoryLookupLedger {
    /// Record one lookup and say what it is worth. `budget` is the maximum number
    /// of lookups allowed in a run; `0` means unlimited.
    pub fn record(&self, query: &str, budget: usize) -> LookupVerdict {
        let normalized = Self::normalize(query);
        let mut ledger = self.lock();
        if budget > 0 && ledger.queries.len() >= budget {
            return LookupVerdict::Exhausted {
                spent: ledger.queries.len(),
            };
        }
        let seen = ledger.queries.iter().filter(|q| **q == normalized).count();
        ledger.queries.push(normalized);
        match seen {
            0 => LookupVerdict::Fresh,
            n => LookupVerdict::Repeat { nth: n + 1 },
        }
    }

    /// Record which pages a query returned, and name an earlier query if this one
    /// surfaced nothing the run has not already been handed.
    ///
    /// [`record`](Self::record) only catches a query retyped verbatim, and an agent
    /// circling a subject almost never retypes one: "upstairs motion sensors", "first
    /// floor motion sensors", "Home Assistant motion sensors" are three fresh queries
    /// over the same handful of pages. Ranked retrieval is why - every rewording of one
    /// subject ranks the same pages first - so the pages, not the wording, are what say
    /// the search has stopped making progress.
    pub fn record_hits(&self, query: &str, paths: &[String]) -> Option<String> {
        let normalized = Self::normalize(query);
        let mut ledger = self.lock();
        let seen_before: std::collections::BTreeSet<&str> = ledger
            .served
            .iter()
            .flat_map(|s| s.paths.iter().map(String::as_str))
            .collect();
        // Union, not any single earlier query: two searches that each returned half of
        // these pages have between them left this one with nothing to add.
        let nothing_new =
            !paths.is_empty() && paths.iter().all(|p| seen_before.contains(p.as_str()));
        let first_to_serve = nothing_new
            .then(|| {
                ledger
                    .served
                    .iter()
                    .find(|s| paths.iter().any(|p| s.paths.contains(p)))
                    .map(|s| s.query.clone())
            })
            .flatten();
        ledger.served.push(Served {
            query: normalized,
            paths: paths.iter().cloned().collect(),
        });
        first_to_serve
    }

    /// Whether this page's text still has to be sent, marking it sent if so.
    ///
    /// A run that searches four ways around one subject is handed the same top pages
    /// every time. Their text is already in the transcript by then, so sending it again
    /// spends the context the inlining exists to save - and the agent can simply use
    /// what it was given.
    pub fn text_needs_sending(&self, page: &str) -> bool {
        self.lock().text_sent.insert(page.to_string())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Ledger> {
        match self.inner.lock() {
            Ok(guard) => guard,
            // A poisoned lock means some other lookup panicked mid-record. Losing
            // loop detection is not worth failing a turn over.
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Case- and whitespace-insensitive: an agent that loops rarely retypes a
    /// query byte-for-byte, and `Postgres  port` is not a different question
    /// from `postgres port`.
    fn normalize(query: &str) -> String {
        query
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::{LookupVerdict, MemoryLookupLedger};

    #[test]
    fn ledger_reports_repeats_ignoring_case_and_spacing() {
        let ledger = MemoryLookupLedger::default();
        assert_eq!(ledger.record("postgres port", 0), LookupVerdict::Fresh);
        assert_eq!(
            ledger.record("  Postgres   PORT ", 0),
            LookupVerdict::Repeat { nth: 2 },
            "a retyped query is the same query"
        );
        assert_eq!(
            ledger.record("postgres port", 0),
            LookupVerdict::Repeat { nth: 3 }
        );
        assert_eq!(
            ledger.record("redis port", 0),
            LookupVerdict::Fresh,
            "a different question is not a repeat"
        );
    }

    /// The loop that actually happens: not one query retyped, but one subject reworded
    /// until the turn runs out of budget, every rewording ranking the same pages first.
    #[test]
    fn ledger_reports_a_reworded_query_that_returns_pages_already_served() {
        let ledger = MemoryLookupLedger::default();
        let upstairs = ["devices/upstairs-motion".to_string()];
        let both = [
            "devices/upstairs-motion".to_string(),
            "devices/landing-motion".to_string(),
        ];
        assert_eq!(
            ledger.record_hits("upstairs motion sensors", &both),
            None,
            "the first search of a run has nothing to repeat"
        );
        assert_eq!(
            ledger
                .record_hits("home assistant motion sensors upstairs", &upstairs)
                .as_deref(),
            Some("upstairs motion sensors"),
            "a rewording that surfaces nothing new names the search that served it"
        );
        assert_eq!(
            ledger.record_hits(
                "landing lights",
                &["devices/landing-light".to_string(), both[1].clone()]
            ),
            None,
            "one page the run has not seen makes the search worth its turn"
        );
    }

    /// Half from one search, half from another: between them the run has it all, and a
    /// third search that returns only those pages is still a lap of the same loop.
    #[test]
    fn ledger_pools_pages_across_earlier_searches() {
        let ledger = MemoryLookupLedger::default();
        ledger.record_hits("upstairs sensors", &["devices/a".to_string()]);
        ledger.record_hits("first floor sensors", &["devices/b".to_string()]);
        assert_eq!(
            ledger
                .record_hits(
                    "motion sensors",
                    &["devices/a".to_string(), "devices/b".to_string()]
                )
                .as_deref(),
            Some("upstairs sensors"),
        );
    }

    #[test]
    fn a_search_that_found_nothing_is_not_a_repeat_of_everything() {
        let ledger = MemoryLookupLedger::default();
        ledger.record_hits("upstairs sensors", &["devices/a".to_string()]);
        assert_eq!(
            ledger.record_hits("quantum mechanics", &[]),
            None,
            "an empty result has its own wording for the miss"
        );
    }

    #[test]
    fn ledger_stops_a_run_at_its_budget() {
        let ledger = MemoryLookupLedger::default();
        assert_eq!(ledger.record("a", 2), LookupVerdict::Fresh);
        assert_eq!(ledger.record("b", 2), LookupVerdict::Fresh);
        assert_eq!(
            ledger.record("c", 2),
            LookupVerdict::Exhausted { spent: 2 },
            "the third lookup in a 2-lookup run is refused"
        );
        assert_eq!(
            ledger.record("d", 2),
            LookupVerdict::Exhausted { spent: 2 },
            "a refused lookup doesn't itself count, so the message stays stable"
        );
    }

    #[test]
    fn ledger_budget_of_zero_is_unlimited() {
        let ledger = MemoryLookupLedger::default();
        for _ in 0..50 {
            assert!(!matches!(
                ledger.record("anything", 0),
                LookupVerdict::Exhausted { .. }
            ));
        }
    }

    /// The ledger is shared by clone (it rides on a cloned `InferenceContext`),
    /// so two holders must see one run's history, not two.
    #[test]
    fn ledger_clones_share_one_history() {
        let ledger = MemoryLookupLedger::default();
        let other = ledger.clone();
        assert_eq!(ledger.record("same", 0), LookupVerdict::Fresh);
        assert_eq!(other.record("same", 0), LookupVerdict::Repeat { nth: 2 });
    }
}
