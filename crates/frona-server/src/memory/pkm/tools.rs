//! Foreground tool surface for the knowledge service: `memory_search`,
//! `memory_remember`, `memory_cite`. Reading a page is the general `read`
//! tool (pages are self-describing `.md` files). Tool definitions live in
//! `resources/prompts/tools/pkm/<name>.md` (loaded by the `#[agent_tool]` macro).

use std::sync::Arc;

use serde_json::Value;

use frona_derive::agent_tool;

use crate::agent::prompt::PromptLoader;
use crate::auth::user_service::UserService;
use crate::core::error::AppError;
use crate::memory::service::LookupVerdict;
use crate::tool::{InferenceContext, ToolOutput, active_chat, str_list_arg};

use super::model::{EntityCategory, EntityOrigin};
use super::storage::PkmStorage;
use super::vault::VaultScope;
use crate::db::repo::pkm::PkmRepo;

pub fn all(
    repo: Arc<PkmRepo>,
    storage: PkmStorage,
    prompts: PromptLoader,
    user_service: UserService,
    max_lookups_per_run: usize,
) -> Vec<Arc<dyn crate::tool::AgentTool>> {
    let vault = VaultResolver {
        storage,
        user_service,
    };
    vec![
        Arc::new(RememberTool {
            repo: repo.clone(),
            prompts: prompts.clone(),
        }),
        Arc::new(SearchTool {
            repo: repo.clone(),
            prompts: prompts.clone(),
            vault: vault.clone(),
            max_lookups_per_run,
        }),
        Arc::new(CitePageTool {
            repo,
            prompts,
            vault,
        }),
    ]
}

/// The two dependencies [`VaultScope::resolve`] needs, as one collaborator.
///
/// `memory_search` and `memory_cite` both work in page paths, so both have to know where
/// this user's files live - and neither wants `PkmStorage` or `UserService` for anything
/// else. Carried loose, they were two fields apiece whose reason for existing was a call
/// spelled out identically in both, which is one edit away from two answers to "which
/// directory is the vault?".
#[derive(Clone)]
struct VaultResolver {
    storage: PkmStorage,
    user_service: UserService,
}

impl VaultResolver {
    async fn for_caller(&self, ctx: &InferenceContext) -> Result<VaultScope, AppError> {
        VaultScope::resolve(
            &self.user_service,
            &self.storage,
            &ctx.user.id,
            &ctx.user.handle,
        )
        .await
    }
}

pub struct RememberTool {
    repo: Arc<PkmRepo>,
    prompts: PromptLoader,
}

#[agent_tool(name = "memory_remember", dir = "pkm")]
impl RememberTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        // One note per call was a per-fact tool turn: a conversation that surfaced a
        // host, a port and a password cost three. The unit of *storage* is still one
        // sentence - the consolidation grounds and files each separately - but the
        // unit of *calling* no longer has to be.
        let contents = str_list_arg(&arguments, "content", "contents");
        if contents.is_empty() {
            return Err(AppError::Validation("missing 'content'".into()));
        }
        let chat = active_chat(ctx)?;
        // A private-memory agent's note is scoped to the agent, so it never reaches
        // the user's other agents or the consolidation that builds the vault.
        let private_agent = ctx.agent.private_memory.then_some(ctx.agent.id.as_str());
        for content in &contents {
            self.repo
                .remember(&ctx.user.id, &chat.id, content, private_agent)
                .await?;
        }
        Ok(ToolOutput::text(format!(
            "Remembered:\n{}",
            contents
                .iter()
                .map(|c| format!("- {c}"))
                .collect::<Vec<_>>()
                .join("\n")
        )))
    }
}

pub struct SearchTool {
    repo: Arc<PkmRepo>,
    prompts: PromptLoader,
    vault: VaultResolver,
    /// `memory.pkm_max_lookups_per_turn` - how many searches one run may make
    /// before the tool stops answering. `0` disables the cap.
    max_lookups_per_run: usize,
}

#[agent_tool(name = "memory_search", dir = "pkm")]
impl SearchTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        // Looking a connection string up field by field is the right instinct - each
        // field IS a separate lookup - but it used to cost one tool turn each. Several
        // queries now ride in one call; each is still charged to the run's lookup
        // budget, so the loop guard is unchanged.
        let queries = str_list_arg(&arguments, "query", "queries");
        if queries.is_empty() {
            return Err(AppError::Validation("missing 'query'".into()));
        }
        let vault = self.vault.for_caller(ctx).await?;

        // One text budget for the whole call, not one per query: five queries that each
        // inlined their own pages would answer a batch with five times the page text,
        // and batching is supposed to save turns, not spend context.
        let mut budget = InlineBudget::new(ctx.memory_lookups.clone());

        if let [only] = queries.as_slice() {
            return Ok(ToolOutput::text(
                self.run_query(only, ctx, &vault, &mut budget).await?,
            ));
        }
        let mut out = String::new();
        for query in &queries {
            out.push_str(&format!("### {query}\n\n"));
            out.push_str(&self.run_query(query, ctx, &vault, &mut budget).await?);
            out.push_str("\n\n");
        }
        Ok(ToolOutput::text(out.trim_end().to_string()))
    }
}

impl SearchTool {
    /// One query's worth of the answer. Charges the run's lookup budget, so a batch of
    /// five queries spends five lookups - batching saves tool turns, not searches.
    async fn run_query(
        &self,
        query: &str,
        ctx: &InferenceContext,
        vault: &VaultScope,
        budget: &mut InlineBudget,
    ) -> Result<String, AppError> {
        // Charge the lookup to this run before doing any work, so a loop is cut off
        // at the point the agent stops learning anything - not after it has burned
        // the whole tool-turn budget and failed the turn with "Max tool turns
        // reached".
        let verdict = ctx.memory_lookups.record(query, self.max_lookups_per_run);
        if let LookupVerdict::Exhausted { spent } = verdict {
            tracing::warn!(
                user = %ctx.user.id,
                agent = %ctx.agent.id,
                query = %query,
                spent,
                "memory_search: run lookup budget spent"
            );
            return Ok(format!(
                "Memory-lookup budget for this turn is spent ({spent} searches). \
                 Another search will not return anything new - the knowledge base has \
                 not changed since the first one.\n{}",
                elsewhere(&ctx.mcp_servers)
            ));
        }
        let hits = self.repo.search_entities(&ctx.user.id, query).await?;
        if hits.is_empty() {
            return Ok(match verdict {
                // Repeating a query that found nothing is the cheapest loop to fall
                // into, so the second identical miss closes the door rather than
                // inviting another reformulation.
                LookupVerdict::Repeat { .. } => format!(
                    "No pages matched - the same answer as the last time you ran this \
                     exact query in this turn. The KB does not model this, and no \
                     wording changes that.\n{}",
                    elsewhere(&ctx.mcp_servers)
                ),
                _ => format!(
                    "No pages matched. The KB doesn't model this yet - reformulate once \
                     with the specific name if another query could plausibly find it; \
                     otherwise the answer is somewhere else.\n{}",
                    elsewhere(&ctx.mcp_servers)
                ),
            });
        }
        let mut out = String::new();
        if let LookupVerdict::Repeat { nth } = verdict {
            out.push_str(&format!(
                "Repeat lookup - this is search #{nth} for this exact query in this turn, \
                 and the knowledge base has not changed since the first. What follows is \
                 the same result. Don't run it again: read one of the paths, or tell the \
                 user the value isn't in the KB.\n\n"
            ));
        }
        // A search whose wording is new but whose pages are not: the run has already
        // been handed everything this query found, so the honest answer is "you have
        // this", not another ranked list that reads like progress.
        let paths: Vec<String> = hits.iter().map(|h| h.path.clone()).collect();
        if let Some(earlier) = ctx.memory_lookups.record_hits(query, &paths) {
            out.push_str(&format!(
                "These are the same pages your earlier search ({earlier:?}) already \
                 returned - a rewording finds them again because they rank first for \
                 the whole subject. Nothing here is new, and a third wording returns \
                 them a third time. If these pages don't answer it, the answer isn't \
                 in the KB.\n{}\n",
                elsewhere(&ctx.mcp_servers)
            ));
        }
        // Emit the ABSOLUTE `.md` file path so the agent can `read(<path>)`
        // verbatim - no root-prepending or extension-guessing (both of which it
        // gets wrong, e.g. reading `.../me` before retrying `.../me.md`).
        // Internal (Memory) pages are directory-prefixed under the root; External
        // (User Vault) notes live at their own full vault path and are tagged
        // `[external]` (read-only - the agent may read/cite but never edit them).
        out.push_str(
            "Top matches, page text included — call read(paths=[…]) only for a page \
             whose text is cut off or not shown below, or when you need a page's \
             frontmatter (attributes, links):\n\n",
        );
        for h in hits {
            let (tag, abspath) = match h.origin {
                EntityOrigin::External => ("external".to_string(), vault.abs_vault_file(&h.path)),
                EntityOrigin::Internal => {
                    let tag = match h.category {
                        EntityCategory::Playbook => "playbook".to_string(),
                        // An untyped concept renders as an empty tag - the extractor is
                        // schema-blind, so a page is untyped until the Classify stage types it.
                        EntityCategory::Concept => {
                            crate::memory::pkm::ontology::PrefixMap::standard()
                                .display_joined(&h.kinds)
                        }
                    };
                    (tag, vault.abs_page_file(&h.path))
                }
            };
            // A row is searchable from the moment the entity is created, but its file
            // only exists once the Author stage projects it (or, for a User Vault note,
            // once the mirror write lands). Handing out a path to a file that isn't
            // there earns a `file not found` from `read`, which the agent answers with
            // another search for the same page - so say what is actually true instead.
            let readable = tokio::fs::try_exists(&abspath).await.unwrap_or(false);
            if !readable {
                tracing::warn!(
                    user = %ctx.user.id,
                    page = %h.path,
                    file = %abspath,
                    "memory_search: page has no file on disk yet"
                );
            }
            let locator = if readable {
                abspath
            } else {
                "(not written to disk yet — nothing to read; don't search for it again)".to_string()
            };
            out.push_str(&format!(
                "- {}  [{tag}]\n  {}\n  {locator}\n",
                h.name, h.description
            ));
            // The page's own prose, which the agent used to spend a `read` call on for
            // every hit it cared about - the single biggest multiplier on a knowledge
            // question's tool count. It is already in hand (the search row carries the
            // authored body), so handing back a path and nothing else was asking for a
            // round-trip to fetch text we had.
            if readable {
                out.push_str(&budget.render(&h.path, &locator, &h.body));
            }
            out.push('\n');
        }
        Ok(out)
    }
}

/// How much page text one `memory_search` call may carry inline, across every query in
/// it. A knowledge question is usually answered by the top page or two; the budget is
/// what stops a batch of five queries from answering with a small library.
const INLINE_TOTAL_CHARS: usize = 6000;

/// At most this many server handles are named before the list is summarised - enough
/// to point somewhere without turning a refusal into an inventory.
const ELSEWHERE_SERVERS_NAMED: usize = 8;

/// Where the answer is, when it isn't in the knowledge base.
///
/// "Stop searching" on its own is an instruction an agent can only obey by giving up,
/// so it doesn't: it rewords the query, or moves the same question to `grep`, and the
/// turn ends having looked in the one place that was never going to have the answer.
/// Every refusal here names somewhere to go instead, starting with the systems this run
/// can actually reach - which is where a question about live state was always going to
/// be answered.
fn elsewhere(servers: &[String]) -> String {
    let mut out = String::from("\nWhere the answer lives instead:\n");
    if !servers.is_empty() {
        let named = servers.len().min(ELSEWHERE_SERVERS_NAMED);
        let mut list = servers[..named].join(", ");
        if servers.len() > named {
            list.push_str(&format!(
                " (+{} more in <mcpservers>)",
                servers.len() - named
            ));
        }
        out.push_str(&format!(
            "- What a connected system knows right now - state, readings, messages, \
             entries, what is actually configured there: call that server. You have \
             {list}. Memory holds notes ABOUT these systems; it never holds their \
             current state, so no search here can answer that question.\n"
        ));
    }
    out.push_str(
        "- Something on disk: `read`, `grep`, or the shell.\n\
         - Something on the open web: `web_search`.\n\
         - Something only the user knows: ask them. That is a better answer than \
         another search, and a much better one than a plausible guess.\n",
    );
    out
}

/// How much of that budget any single page may take, so one long page can't crowd out
/// the text of every hit under it.
const INLINE_PAGE_CHARS: usize = 2000;

/// The page text one search call has spent, and on which pages.
///
/// Inlining is best-effort by design: what doesn't fit still comes back as a path, and
/// `read` is still there. The point is that the common case - a couple of short pages -
/// needs no second call at all.
struct InlineBudget {
    remaining: usize,
    /// Which pages the *run* has already been shown - not just this call. Two calls in
    /// one turn that rank the same page first send its text once between them.
    sent: crate::memory::service::MemoryLookupLedger,
}

impl InlineBudget {
    fn new(sent: crate::memory::service::MemoryLookupLedger) -> Self {
        Self {
            remaining: INLINE_TOTAL_CHARS,
            sent,
        }
    }

    /// One hit's text block, and what it costs. `key` is the entity path - stable
    /// across the queries in a batch, so a page hit by two of them is written out once -
    /// and `path` the absolute file the agent would pass to `read`.
    fn render(&mut self, key: &str, path: &str, body: &str) -> String {
        let body = body.trim();
        if body.is_empty() {
            // A row with no prose: the file is frontmatter only, and saying so beats an
            // empty block the agent reads as a failed inline.
            return "  (no page text — frontmatter only; read it if you need the attributes)\n"
                .to_string();
        }
        if !self.sent.text_needs_sending(key) {
            return "  (page text already sent this turn — use it rather than reading the page again)\n"
                .to_string();
        }
        if self.remaining == 0 {
            return "  (page text not shown — this call's text budget is spent; read(paths=[…]) if you need it)\n".to_string();
        }
        let allowance = self.remaining.min(INLINE_PAGE_CHARS);
        let (text, cut) = match body.char_indices().nth(allowance) {
            Some((i, _)) => (&body[..i], true),
            None => (body, false),
        };
        self.remaining = self.remaining.saturating_sub(text.chars().count());
        let tail = if cut {
            format!("\n… [cut off — read(\"{path}\") for the rest]")
        } else {
            String::new()
        };
        format!("<page path=\"{path}\">\n{text}{tail}\n</page>\n")
    }
}

pub struct CitePageTool {
    repo: Arc<PkmRepo>,
    prompts: PromptLoader,
    vault: VaultResolver,
}

#[agent_tool(name = "memory_cite", dir = "pkm")]
impl CitePageTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        // Answering from three pages used to cost three cite calls on top of the three
        // reads. Citing only biases ranking, so it is never worth a tool turn of its
        // own: pass every page you used at once.
        let raw_paths = str_list_arg(&arguments, "path", "paths");
        if raw_paths.is_empty() {
            return Err(AppError::Validation("missing 'path'".into()));
        }
        let vault = self.vault.for_caller(ctx).await?;
        let mut lines = Vec::new();
        for raw in &raw_paths {
            lines.push(self.cite_one(raw, &vault, ctx).await?);
        }
        Ok(ToolOutput::text(lines.join("\n")))
    }
}

impl CitePageTool {
    async fn cite_one(
        &self,
        raw: &str,
        vault: &VaultScope,
        ctx: &InferenceContext,
    ) -> Result<String, AppError> {
        // A Memory page and a User Vault note are addressed differently, and the agent
        // is handed whichever `memory_search` emitted - so try every spelling the path
        // could mean and let the database pick.
        let candidates = vault.entity_path_candidates(raw);
        let mut last_rejection = None;
        for candidate in &candidates {
            match self.repo.bump_entity_use(&ctx.user.id, candidate).await {
                Ok(n) => {
                    return Ok(format!("Cited '{candidate}' (total uses: {n})."));
                }
                Err(AppError::Validation(msg)) => last_rejection = Some(msg),
                Err(e) => return Err(e),
            }
        }
        // Terminal on purpose. The old wording ("use the absolute path returned by
        // memory_search") read as an instruction to search again, and since the next
        // search returned the same path the agent had just been refused, it looped.
        // Citing only biases future ranking, so there is nothing here worth another
        // lookup.
        tracing::debug!(
            path = %raw,
            tried = ?candidates,
            rejection = ?last_rejection,
            "memory_cite: no entity matched the cited path"
        );
        Ok(format!(
            "Couldn't cite '{raw}' - no page in the knowledge base has that path. \
             Citing only biases future ranking, so nothing is lost: answer from what \
             you already read. Do NOT search again to find a citable path."
        ))
    }
}

#[cfg(test)]
mod elsewhere_tests {
    use super::*;

    #[test]
    fn a_refusal_names_the_servers_this_run_can_reach() {
        let out = elsewhere(&["homeassistant".into(), "gmail".into()]);
        assert!(out.contains("homeassistant, gmail"), "{out}");
        assert!(
            out.contains("never holds their current state"),
            "and says why memory was the wrong place to ask:\n{out}"
        );
    }

    /// With nothing connected there is still somewhere to go - the refusal must not
    /// collapse back to "stop", which is the advice that produced the rewording loop.
    #[test]
    fn with_no_servers_it_still_names_the_other_surfaces() {
        let out = elsewhere(&[]);
        assert!(!out.contains("connected system"), "{out}");
        for expected in ["`grep`", "`web_search`", "ask them"] {
            assert!(out.contains(expected), "missing {expected}:\n{out}");
        }
    }

    #[test]
    fn a_long_server_list_is_summarised_rather_than_recited() {
        let servers: Vec<String> = (0..ELSEWHERE_SERVERS_NAMED + 3)
            .map(|i| format!("srv{i}"))
            .collect();
        let out = elsewhere(&servers);
        assert!(out.contains("(+3 more in <mcpservers>)"), "{out}");
        assert!(!out.contains("srv10"), "{out}");
    }
}

#[cfg(test)]
mod inline_budget_tests {
    use super::{INLINE_PAGE_CHARS, INLINE_TOTAL_CHARS, InlineBudget};

    #[test]
    fn a_page_comes_back_with_its_text_so_no_read_is_needed() {
        let mut budget = InlineBudget::new(Default::default());
        let out = budget.render(
            "devices/landing",
            "/vault/devices/landing.md",
            "Aqara P1 on the landing.",
        );
        assert!(
            out.contains("<page path=\"/vault/devices/landing.md\">"),
            "{out}"
        );
        assert!(out.contains("Aqara P1 on the landing."), "{out}");
        assert!(
            !out.contains("cut off"),
            "a short page arrives whole:\n{out}"
        );
    }

    /// The same page ranks first for two queries of one batch. Its text is worth sending
    /// once; the second hit only has to say where it went.
    #[test]
    fn a_page_hit_twice_in_one_call_is_sent_once() {
        let mut budget = InlineBudget::new(Default::default());
        budget.render("devices/landing", "/vault/devices/landing.md", "Aqara P1.");
        let again = budget.render("devices/landing", "/vault/devices/landing.md", "Aqara P1.");
        assert!(again.contains("already sent this turn"), "{again}");
    }

    /// The run, not the call, is the unit: a second search in the same turn that ranks
    /// the same page first has already put that text in the transcript.
    #[test]
    fn a_page_sent_by_an_earlier_call_in_the_run_is_not_sent_again() {
        let run = crate::memory::service::MemoryLookupLedger::default();
        InlineBudget::new(run.clone()).render("devices/landing", "/vault/l.md", "Aqara P1.");
        let later = InlineBudget::new(run).render("devices/landing", "/vault/l.md", "Aqara P1.");
        assert!(later.contains("already sent this turn"), "{later}");
    }

    #[test]
    fn one_long_page_is_cut_off_and_says_how_to_get_the_rest() {
        let mut budget = InlineBudget::new(Default::default());
        let out = budget.render("k", "/vault/k.md", &"x".repeat(INLINE_PAGE_CHARS * 2));
        assert!(out.contains("cut off — read(\"/vault/k.md\")"), "{out}");
        assert!(
            out.chars().count() < INLINE_PAGE_CHARS * 2,
            "the long tail is not sent:\n{out}"
        );
    }

    /// A batch of queries shares one budget, so a wide search degrades to what it always
    /// was - a list of paths - rather than answering with a library.
    #[test]
    fn the_budget_is_spent_across_a_whole_call_then_pages_fall_back_to_paths() {
        let mut budget = InlineBudget::new(Default::default());
        let page = "y".repeat(INLINE_PAGE_CHARS);
        for i in 0..(INLINE_TOTAL_CHARS / INLINE_PAGE_CHARS) {
            let out = budget.render(&format!("p{i}"), &format!("/vault/p{i}.md"), &page);
            assert!(out.contains("<page"), "page {i} still fits:\n{out}");
        }
        let out = budget.render("last", "/vault/last.md", &page);
        assert!(out.contains("text budget is spent"), "{out}");
        assert!(out.contains("read(paths=[…])"), "{out}");
    }

    #[test]
    fn a_page_with_no_prose_says_so_rather_than_sending_an_empty_block() {
        let mut budget = InlineBudget::new(Default::default());
        let out = budget.render("k", "/vault/k.md", "   \n  ");
        assert!(out.contains("frontmatter only"), "{out}");
        assert!(!out.contains("<page"), "{out}");
    }
}
