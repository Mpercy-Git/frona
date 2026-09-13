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

        if let [only] = queries.as_slice() {
            return Ok(ToolOutput::text(self.run_query(only, ctx, &vault).await?));
        }
        let mut out = String::new();
        for query in &queries {
            out.push_str(&format!("### {query}\n\n"));
            out.push_str(&self.run_query(query, ctx, &vault).await?);
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
                 Another search will not return anything new. Answer from what you \
                 have already read, or tell the user which value you could not find \
                 and ask them for it."
            ));
        }
        let hits = self.repo.search_entities(&ctx.user.id, query).await?;
        if hits.is_empty() {
            return Ok(match verdict {
                // Repeating a query that found nothing is the cheapest loop to fall
                // into, so the second identical miss closes the door rather than
                // inviting another reformulation.
                LookupVerdict::Repeat { .. } => {
                    "No pages matched - the same answer as the last time you ran this \
                     exact query in this turn. The KB does not model this. Stop \
                     searching for it: ask the user, or tell them it isn't in your \
                     knowledge base."
                }
                _ => {
                    "No pages matched. The KB doesn't model this yet - reformulate once \
                     with the specific name if another query could plausibly find it; \
                     otherwise ask the user instead of searching again."
                }
            }
            .to_string());
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
        // Emit the ABSOLUTE `.md` file path so the agent can `read(<path>)`
        // verbatim - no root-prepending or extension-guessing (both of which it
        // gets wrong, e.g. reading `.../me` before retrying `.../me.md`).
        // Internal (Memory) pages are directory-prefixed under the root; External
        // (User Vault) notes live at their own full vault path and are tagged
        // `[external]` (read-only - the agent may read/cite but never edit them).
        out.push_str("Top matches — read(paths=[…]) opens as many as you need in one call:\n\n");
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
                "- {}  [{tag}]\n  {}\n  {locator}\n\n",
                h.name, h.description
            ));
        }
        Ok(out)
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
