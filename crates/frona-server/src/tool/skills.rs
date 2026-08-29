//! Skill discovery and installation.
//!
//! The agent only ever sees the skills that are already installed, rendered
//! into `<available_skills>`. Everything else in the registry is invisible to
//! it, so a task it has no skill for looks like a task nobody wrote a skill
//! for. These two tools close that gap: `search_skills` reads the registry,
//! and `add_skill` proposes an install that only happens once the user
//! approves — the same shape as `request_credentials`, where the agent can
//! ask for a secret but never reads one without a grant.

use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::agent::skill::service::SkillService;
use crate::core::error::AppError;
use crate::inference::hitl::{
    Hitl, HitlOutcome, HitlRequest, HitlResponse, SkillCandidate, SkillInstallScope,
};
use crate::inference::tool_call::ToolStatus;

use frona_derive::agent_tool;

use super::{InferenceContext, ToolOutput, active_chat};

/// Registry search returns names + repos only; a repo listing carries
/// descriptions. Cap both so a broad query can't flood the context window.
const MAX_SEARCH_RESULTS: usize = 20;
const MAX_REPO_SKILLS: usize = 60;
/// Descriptions in SKILL.md frontmatter can run to several paragraphs of
/// trigger rules. One line each is enough to choose from.
const MAX_DESCRIPTION_CHARS: usize = 240;

pub struct SkillsTool {
    skill_service: SkillService,
    prompts: PromptLoader,
    public_base_url: String,
}

impl SkillsTool {
    pub fn new(skill_service: SkillService, prompts: PromptLoader, public_base_url: String) -> Self {
        Self {
            skill_service,
            prompts,
            public_base_url,
        }
    }

    /// Names of the skills this agent can already use — the same resolution
    /// the system prompt's `<available_skills>` block uses, so "installed"
    /// in a search result means "you already have it", not "it exists
    /// somewhere on this server".
    async fn available_names(&self, ctx: &InferenceContext) -> Vec<String> {
        self.skill_service
            .list(
                &ctx.agent_owner_handle,
                &ctx.agent.handle,
                ctx.agent.skills.as_deref(),
            )
            .await
            .into_iter()
            .map(|s| s.name)
            .collect()
    }

    async fn handle_search(
        &self,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let query = str_arg(&arguments, "query");
        let repo = str_arg(&arguments, "repo");

        match (query, repo) {
            (_, Some(repo)) => self.browse_repo(&repo, ctx).await,
            (Some(query), None) => self.search_registry(&query, ctx).await,
            (None, None) => Err(AppError::Validation(
                "Provide `query` to search the skill registry, or `repo` (\"owner/repo\") to list the skills in one repository.".into(),
            )),
        }
    }

    async fn search_registry(
        &self,
        query: &str,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let results = self.skill_service.search(query).await?;
        if results.is_empty() {
            return Ok(ToolOutput::text(format!(
                "No skills in the registry match '{query}'. Try a broader term, or list a known repository with `repo`."
            )));
        }

        let available = self.available_names(ctx).await;
        let mut lines = vec![format!("Registry matches for '{query}':")];
        for r in results.into_iter().take(MAX_SEARCH_RESULTS) {
            let marker = if available.contains(&r.name) {
                " [already available to you]"
            } else {
                ""
            };
            lines.push(format!(
                "- {} — {} ({} installs){marker}",
                r.name, r.repo, r.installs
            ));
        }
        lines.push(String::new());
        lines.push(
            "Descriptions live in the repositories: call search_skills again with `repo` set to one of the repos above to see what each skill actually does, then add_skill to install."
                .to_string(),
        );

        Ok(ToolOutput::text(lines.join("\n")))
    }

    async fn browse_repo(&self, repo: &str, ctx: &InferenceContext) -> Result<ToolOutput, AppError> {
        let repo = normalize_repo(repo)?;
        let browse = self.skill_service.get_skills(&repo).await?;
        if browse.skills.is_empty() {
            return Ok(ToolOutput::text(format!(
                "No SKILL.md files found in {repo}."
            )));
        }

        let available = self.available_names(ctx).await;
        let total = browse.skills.len();
        let mut lines = vec![format!("Skills in {repo}:")];
        for s in browse.skills.into_iter().take(MAX_REPO_SKILLS) {
            let marker = if available.contains(&s.name) {
                " [already available to you]"
            } else {
                ""
            };
            lines.push(format!(
                "- {}: {}{marker}",
                s.name,
                truncate(&s.description, MAX_DESCRIPTION_CHARS)
            ));
        }
        if total > MAX_REPO_SKILLS {
            lines.push(format!(
                "... and {} more (narrow with `query`).",
                total - MAX_REPO_SKILLS
            ));
        }
        lines.push(String::new());
        lines.push(format!(
            "Install with add_skill(repo: \"{repo}\", skills: [...], reason: \"...\") — the user approves before anything is written."
        ));

        Ok(ToolOutput::text(lines.join("\n")))
    }

    async fn handle_add(
        &self,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        // A shared agent's skills resolve under its owner, so installing from
        // a recipient's run would silently edit someone else's agent. Refuse
        // rather than write into an account that never asked for it.
        if ctx.agent.user_id != ctx.user.id {
            return Ok(ToolOutput::error(format!(
                "'{}' was shared with you, so its skills can only be changed by its owner. Ask them to add the skill, or use one that's already available.",
                ctx.agent.name
            )));
        }

        let repo = normalize_repo(&str_arg(&arguments, "repo").ok_or_else(|| {
            AppError::Validation("Missing required parameter: repo (\"owner/repo\")".into())
        })?)?;
        let requested = parse_skill_names(&arguments)?;
        let reason = str_arg(&arguments, "reason").ok_or_else(|| {
            AppError::Validation(
                "Missing required parameter: reason (shown to the user in the approval prompt)"
                    .into(),
            )
        })?;
        let scope = parse_scope(&arguments)?;

        let browse = self.skill_service.get_skills(&repo).await?;
        let available = self.available_names(ctx).await;

        let mut items: Vec<SkillCandidate> = Vec::new();
        let mut already: Vec<String> = Vec::new();
        for name in &requested {
            let Some(found) = browse.skills.iter().find(|s| s.name == *name) else {
                let known: Vec<&str> = browse.skills.iter().map(|s| s.name.as_str()).collect();
                return Ok(ToolOutput::error(format!(
                    "'{name}' is not in {repo}. Skills there: {}.",
                    if known.is_empty() {
                        "none".to_string()
                    } else {
                        known.join(", ")
                    }
                )));
            };
            if available.contains(name) {
                already.push(name.clone());
                continue;
            }
            items.push(SkillCandidate {
                name: found.name.clone(),
                repo: repo.clone(),
                description: truncate(&found.description, MAX_DESCRIPTION_CHARS),
            });
        }

        if items.is_empty() {
            return Ok(ToolOutput::text(format!(
                "Already available to you — nothing to install: {}. Use them directly; their SKILL.md paths are listed in <available_skills>.",
                already.join(", ")
            )));
        }

        Ok(ToolOutput::text("").with_hitl(Hitl {
            prompt: approval_prompt(&items, scope, &reason),
            url: format!("{}/chat?id={}", self.public_base_url, active_chat(ctx)?.id),
            request: HitlRequest::Skills {
                items,
                scope,
                reason,
            },
            status: ToolStatus::Pending,
            response: None,
            delivery: None,
        }))
    }

    /// Perform the approved install and describe the result well enough that
    /// the agent can use the skill on the very next turn — the resumed turn
    /// rebuilds the system prompt, so the paths reported here match the ones
    /// in `<available_skills>`.
    async fn install_approved(
        &self,
        items: &[SkillCandidate],
        scope: SkillInstallScope,
        ctx: &InferenceContext,
    ) -> Result<String, AppError> {
        // One registry repo per approval in practice, but group anyway so a
        // future multi-repo proposal installs correctly rather than silently
        // dropping the odd one out.
        let mut by_repo: Vec<(String, Vec<String>)> = Vec::new();
        for item in items {
            match by_repo.iter_mut().find(|(repo, _)| *repo == item.repo) {
                Some((_, names)) => names.push(item.name.clone()),
                None => by_repo.push((item.repo.clone(), vec![item.name.clone()])),
            }
        }

        let mut installed: Vec<String> = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        for (repo, names) in &by_repo {
            let result = match scope {
                SkillInstallScope::Agent => {
                    self.skill_service
                        .install_batch(repo, names, Some((&ctx.agent_owner_handle, &ctx.agent.handle)))
                        .await
                }
                SkillInstallScope::User => {
                    self.skill_service
                        .install_batch_for_user(&ctx.user.handle, repo, names)
                        .await
                }
            };
            match result {
                Ok(written) => installed.extend(written.into_iter().map(|i| i.name)),
                Err(e) => failures.push(format!("{repo}: {e}")),
            }
        }

        if installed.is_empty() {
            return Ok(format!(
                "Skill install failed — nothing was written. {}",
                failures.join("; ")
            ));
        }

        // Re-resolve so the reported paths are the ones the next turn's
        // `<available_skills>` block will carry.
        let resolved = self
            .skill_service
            .list(
                &ctx.agent_owner_handle,
                &ctx.agent.handle,
                ctx.agent.skills.as_deref(),
            )
            .await;

        let mut lines = vec![format!(
            "Installed {} skill(s) ({} scope):",
            installed.len(),
            scope.as_str()
        )];
        let mut invisible: Vec<String> = Vec::new();
        for name in &installed {
            match resolved.iter().find(|s| s.name == *name) {
                Some(skill) => lines.push(format!("- {name} (file: {}/SKILL.md)", skill.path)),
                None => {
                    lines.push(format!("- {name}"));
                    invisible.push(name.clone());
                }
            }
        }
        if !failures.is_empty() {
            lines.push(format!("Failed: {}", failures.join("; ")));
        }
        if !invisible.is_empty() {
            // Only reachable for a user-scope install on an agent with an
            // explicit skill allowlist that doesn't name the new skill.
            lines.push(format!(
                "Installed but not enabled for this agent: {}. The agent's skill list has to include them before they can be used — tell the user.",
                invisible.join(", ")
            ));
        }
        lines.push(
            "Read a skill's SKILL.md before using it; they are also listed in <available_skills> from the next turn on."
                .to_string(),
        );

        Ok(lines.join("\n"))
    }
}

#[agent_tool(name = "skills", files("search_skills", "add_skill"))]
impl SkillsTool {
    async fn execute(
        &self,
        tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        match tool_name {
            "search_skills" => self.handle_search(arguments, ctx).await,
            "add_skill" => self.handle_add(arguments, ctx).await,
            other => Err(AppError::Tool(format!("Unknown skills tool: {other}"))),
        }
    }

    async fn on_resume(
        &self,
        _tool_name: &str,
        request: &HitlRequest,
        response: HitlResponse,
        ctx: &InferenceContext,
    ) -> Result<HitlOutcome, AppError> {
        let HitlRequest::Skills { items, scope, .. } = request else {
            return Err(AppError::Validation(
                "add_skill on_resume: expected Skills request".into(),
            ));
        };

        match response {
            HitlResponse::Approval(true) => {
                // An install that fails after approval resolves with the
                // failure text rather than erroring: the user already
                // answered, so the turn must resume and let the agent react.
                Ok(HitlOutcome::Resolved(
                    self.install_approved(items, *scope, ctx).await?,
                ))
            }
            HitlResponse::Approval(false) => Ok(HitlOutcome::Denied(format!(
                "User declined to install: {}. Continue without those skills.",
                names_of(items).join(", ")
            ))),
            _ => Err(AppError::Validation(
                "add_skill on_resume: expected Approval response".into(),
            )),
        }
    }
}

fn str_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn names_of(items: &[SkillCandidate]) -> Vec<String> {
    items.iter().map(|i| i.name.clone()).collect()
}

/// Accepts `owner/repo`, a full GitHub URL, or either with a trailing slash or
/// `.git` — the model reads repo strings off web pages and copies them whole.
fn normalize_repo(raw: &str) -> Result<String, AppError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("github.com/")
        .trim_end_matches('/')
        .trim_end_matches(".git");

    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() != 2 {
        return Err(AppError::Validation(format!(
            "Invalid repo '{raw}'. Use the \"owner/repo\" form, e.g. \"anthropics/skills\"."
        )));
    }
    Ok(format!("{}/{}", parts[0], parts[1]))
}

/// `skills: ["a", "b"]` is the documented shape; a bare `name: "a"` is
/// accepted too because a single-skill install reads naturally that way and
/// the model writes it that way often enough to be worth handling.
fn parse_skill_names(arguments: &Value) -> Result<Vec<String>, AppError> {
    let mut names: Vec<String> = Vec::new();
    let mut push = |raw: &str| {
        let name = raw.trim();
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    };

    if let Some(arr) = arguments.get("skills").and_then(|v| v.as_array()) {
        for el in arr {
            if let Some(s) = el.as_str() {
                push(s);
            }
        }
    }
    if let Some(s) = arguments.get("skills").and_then(|v| v.as_str()) {
        push(s);
    }
    if let Some(s) = arguments.get("name").and_then(|v| v.as_str()) {
        push(s);
    }

    if names.is_empty() {
        return Err(AppError::Validation(
            "Missing required parameter: skills (a non-empty array of skill names from the repository)".into(),
        ));
    }
    Ok(names)
}

fn parse_scope(arguments: &Value) -> Result<SkillInstallScope, AppError> {
    match arguments.get("scope").and_then(|v| v.as_str()) {
        None => Ok(SkillInstallScope::Agent),
        Some(s) => match s.trim().to_ascii_lowercase().as_str() {
            "" | "agent" => Ok(SkillInstallScope::Agent),
            "user" => Ok(SkillInstallScope::User),
            other => Err(AppError::Validation(format!(
                "Invalid scope '{other}'. Use \"agent\" (this agent only) or \"user\" (all of the user's agents)."
            ))),
        },
    }
}

fn truncate(s: &str, max: usize) -> String {
    let flattened = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.chars().count() <= max {
        return flattened;
    }
    let cut: String = flattened.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

fn approval_prompt(items: &[SkillCandidate], scope: SkillInstallScope, reason: &str) -> String {
    let where_to = match scope {
        SkillInstallScope::Agent => "this agent",
        SkillInstallScope::User => "all your agents",
    };
    let lines: Vec<String> = items
        .iter()
        .map(|i| {
            if i.description.is_empty() {
                format!("• {} — {}", i.name, i.repo)
            } else {
                format!("• {} ({}) — {}", i.name, i.repo, i.description)
            }
        })
        .collect();

    if items.len() == 1 {
        format!(
            "Install the '{}' skill for {where_to}?\n\n{reason}\n\n{}",
            items[0].name,
            lines.join("\n"),
        )
    } else {
        format!(
            "Install {} skills for {where_to}?\n\n{reason}\n\n{}",
            items.len(),
            lines.join("\n"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::AgentTool;
    use serde_json::json;
    use std::path::PathBuf;

    fn prompts_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("resources")
            .join("prompts")
    }

    fn test_tool() -> SkillsTool {
        let storage = crate::storage::StorageService::new(&crate::core::config::Config::default());
        let resolver = crate::agent::skill::resolver::SkillResolver::new(
            "/tmp/frona-test-config",
            storage.clone(),
        );
        let service = SkillService::new(
            crate::agent::skill::registry::SkillRegistryClient::default(),
            resolver,
            storage,
            "/tmp/frona-test-skills",
            &crate::core::config::CacheConfig::default(),
        );
        SkillsTool::new(
            service,
            PromptLoader::new(prompts_dir()),
            "https://frona.example".to_string(),
        )
    }

    /// `load_tool_definition` returns `None` on a frontmatter typo, which
    /// would drop the tool from every agent's schema silently. Pin both
    /// definitions and the parameters the tool code actually reads.
    #[test]
    fn both_tool_definitions_load_from_prompts() {
        let defs = test_tool().definitions();
        let ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["search_skills", "add_skill"]);

        for def in &defs {
            assert_eq!(def.provider_id, "skills");
            assert!(!def.description.trim().is_empty(), "{} has no body", def.id);
        }

        let search = defs.iter().find(|d| d.id == "search_skills").unwrap();
        let props = &search.parameters["properties"];
        assert!(props.get("query").is_some());
        assert!(props.get("repo").is_some());
        // Either parameter alone is enough — encoded as anyOf, not `required`.
        assert!(search.parameters.get("required").is_none());
        assert!(search.parameters.get("anyOf").is_some());

        let add = defs.iter().find(|d| d.id == "add_skill").unwrap();
        assert_eq!(
            add.parameters["required"],
            json!(["repo", "skills", "reason"])
        );
        assert_eq!(add.parameters["properties"]["skills"]["type"], "array");
        assert_eq!(
            add.parameters["properties"]["scope"]["enum"],
            json!(["agent", "user"])
        );
    }

    #[test]
    fn normalize_repo_accepts_owner_repo() {
        assert_eq!(normalize_repo("anthropics/skills").unwrap(), "anthropics/skills");
        assert_eq!(normalize_repo("  anthropics/skills  ").unwrap(), "anthropics/skills");
    }

    #[test]
    fn normalize_repo_strips_github_url_forms() {
        for raw in [
            "https://github.com/anthropics/skills",
            "https://github.com/anthropics/skills/",
            "http://www.github.com/anthropics/skills.git",
            "github.com/anthropics/skills",
        ] {
            assert_eq!(normalize_repo(raw).unwrap(), "anthropics/skills", "failed for {raw}");
        }
    }

    #[test]
    fn normalize_repo_rejects_non_repo_strings() {
        for raw in ["skills", "a/b/c", ""] {
            assert!(normalize_repo(raw).is_err(), "should reject {raw}");
        }
    }

    #[test]
    fn parse_skill_names_reads_array_and_dedupes() {
        let args = json!({"skills": ["pdf", "xlsx", "pdf", " docx "]});
        assert_eq!(parse_skill_names(&args).unwrap(), vec!["pdf", "xlsx", "docx"]);
    }

    #[test]
    fn parse_skill_names_accepts_singular_forms() {
        assert_eq!(parse_skill_names(&json!({"name": "pdf"})).unwrap(), vec!["pdf"]);
        assert_eq!(parse_skill_names(&json!({"skills": "pdf"})).unwrap(), vec!["pdf"]);
    }

    #[test]
    fn parse_skill_names_rejects_empty() {
        assert!(parse_skill_names(&json!({})).is_err());
        assert!(parse_skill_names(&json!({"skills": []})).is_err());
        assert!(parse_skill_names(&json!({"skills": ["  "]})).is_err());
    }

    #[test]
    fn parse_scope_defaults_to_agent() {
        assert_eq!(parse_scope(&json!({})).unwrap(), SkillInstallScope::Agent);
        assert_eq!(parse_scope(&json!({"scope": "USER"})).unwrap(), SkillInstallScope::User);
        assert!(parse_scope(&json!({"scope": "server"})).is_err());
    }

    #[test]
    fn truncate_flattens_and_caps() {
        let long = "a ".repeat(200);
        let out = truncate(&long, 20);
        assert!(out.chars().count() <= 21, "got {out:?}");
        assert!(out.ends_with('…'));
        assert_eq!(truncate("one\n  two", 40), "one two");
    }

    #[test]
    fn approval_prompt_names_every_skill_and_the_scope() {
        let items = vec![
            SkillCandidate {
                name: "pdf".into(),
                repo: "anthropics/skills".into(),
                description: "Fill PDF forms.".into(),
            },
            SkillCandidate {
                name: "xlsx".into(),
                repo: "anthropics/skills".into(),
                description: String::new(),
            },
        ];
        let prompt = approval_prompt(&items, SkillInstallScope::User, "The user asked for a form.");
        assert!(prompt.contains("Install 2 skills for all your agents?"));
        assert!(prompt.contains("The user asked for a form."));
        assert!(prompt.contains("pdf (anthropics/skills) — Fill PDF forms."));
        assert!(prompt.contains("• xlsx — anthropics/skills"));
    }

    #[test]
    fn approval_prompt_singular_names_the_skill() {
        let items = vec![SkillCandidate {
            name: "pdf".into(),
            repo: "anthropics/skills".into(),
            description: "Fill PDF forms.".into(),
        }];
        let prompt = approval_prompt(&items, SkillInstallScope::Agent, "Because.");
        assert!(prompt.starts_with("Install the 'pdf' skill for this agent?"));
    }
}
