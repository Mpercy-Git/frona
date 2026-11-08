use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::agent::workspace::AgentPromptLoader;
use crate::core::Handle;
use crate::core::template::render_template;
use crate::storage::StorageService;

#[derive(Clone)]
pub struct PromptLoader {
    base_dir: PathBuf,
    defaults: HashMap<String, String>,
    /// Raw file contents keyed by name, shared across clones of this loader.
    ///
    /// Prompts are read several times per agent turn (system prompt, tool docs,
    /// continuation prompts) from every concurrent task, and each read was a
    /// blocking `read_to_string` on the async runtime. These files ship with the
    /// deployment and don't change while the server runs, so they're cached for
    /// the process lifetime — editing one requires a restart to take effect.
    cache: Arc<RwLock<HashMap<String, Arc<str>>>>,
}

impl PromptLoader {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            defaults: HashMap::new(),
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Raw file contents, served from cache after the first read. A missing or
    /// unreadable file is not cached, so it isn't remembered as permanently
    /// absent.
    fn raw(&self, name: &str) -> Option<Arc<str>> {
        if let Ok(cache) = self.cache.read()
            && let Some(hit) = cache.get(name)
        {
            return Some(Arc::clone(hit));
        }

        let path = self.base_dir.join(name);
        let contents: Arc<str> = std::fs::read_to_string(&path).ok()?.into();

        if let Ok(mut cache) = self.cache.write() {
            cache.insert(name.to_string(), Arc::clone(&contents));
        }
        Some(contents)
    }

    pub fn with_var(mut self, key: &str, value: &str) -> Self {
        self.defaults.insert(key.to_lowercase(), value.to_string());
        self
    }

    pub fn defaults(&self) -> &HashMap<String, String> {
        &self.defaults
    }

    pub fn read(&self, name: &str) -> Option<String> {
        self.read_with_vars(name, &[])
    }

    /// Optional files may be absent, but unreadable or invalid files are errors.
    pub fn try_read(&self, name: &str) -> Result<Option<String>, crate::core::error::AppError> {
        self.try_read_with_vars(name, &[])
    }

    pub fn try_read_with_vars(
        &self,
        name: &str,
        vars: &[(&str, &str)],
    ) -> Result<Option<String>, crate::core::error::AppError> {
        let raw = match std::fs::read_to_string(self.base_dir.join(name)) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(crate::core::error::AppError::Internal(format!(
                    "Read prompt {name}: {e}"
                )));
            }
        };
        let merged = self.merge_vars(vars);
        let refs: Vec<_> = merged
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        render_template(&raw, &refs).map(Some).map_err(|e| {
            crate::core::error::AppError::Internal(format!("Render prompt {name}: {e}"))
        })
    }

    pub fn read_with_vars(&self, name: &str, vars: &[(&str, &str)]) -> Option<String> {
        let raw = self.raw(name)?;

        // The common `read()` path passes no vars; render straight from the
        // defaults rather than cloning the whole map to merge nothing into it.
        if vars.is_empty() {
            let refs: Vec<(&str, &str)> = self
                .defaults
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            return render_template(&raw, &refs).ok();
        }

        let merged = self.merge_vars(vars);
        let merged_refs: Vec<(&str, &str)> = merged
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        render_template(&raw, &merged_refs).ok()
    }

    pub fn read_raw(&self, name: &str) -> Option<String> {
        self.raw(name).map(|c| c.to_string())
    }

    fn merge_vars(&self, caller_vars: &[(&str, &str)]) -> Vec<(String, String)> {
        let mut merged: HashMap<String, String> = self.defaults.clone();
        for (k, v) in caller_vars {
            merged.insert(k.to_lowercase(), v.to_string());
        }
        merged.into_iter().collect()
    }

    pub fn list_dir(&self, dir: &str) -> Vec<String> {
        let mut paths = BTreeSet::new();

        let full_dir = self.base_dir.join(dir);
        if let Ok(entries) = std::fs::read_dir(&full_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                    && let Some(name) = entry.file_name().to_str()
                {
                    paths.insert(format!("{dir}/{name}"));
                }
            }
        }

        paths.into_iter().collect()
    }
}

/// The line `<available_agents>` carries when the delegation policy turned
/// agents down.
///
/// An agent that is shown nothing concludes there is nothing - and tells the
/// user they have no other agents, which is wrong whenever a policy is what hid
/// them. Naming them costs a line and turns an invisible denial into something
/// the user can act on. Deterministic in the order the agents were listed, so
/// the cacheable prefix stays byte-stable between turns.
pub(crate) fn denied_delegation_note(denied: &[String]) -> Option<String> {
    if denied.is_empty() {
        return None;
    }
    Some(format!(
        "Not delegable from here: {}. These agents exist, but the delegation policy \
         refuses them to you - an agent may only delegate to agents whose tools are a \
         subset of its own. Do not try; do the work yourself. If the user expected one \
         of them to take it, say plainly that policy blocks the handoff and that the \
         fix is to line up the two agents' tool permissions.",
        denied.join(", ")
    ))
}

/// Emit `<tag>`: an optional header, the items, an optional footer. Nothing is
/// written when there is nothing to say - but a footer alone counts as
/// something, so a section can report that its list is empty *and why*.
pub fn append_tagged_section(
    result: &mut String,
    tag: &str,
    header: Option<&str>,
    items: &[(String, String)],
    footer: Option<&str>,
) {
    let footer = footer.map(str::trim).filter(|f| !f.is_empty());
    if items.is_empty() && footer.is_none() {
        return;
    }
    result.push_str(&format!("\n\n<{tag}>\n"));
    // The header introduces the items, so it is dropped when there are none:
    // "delegate to these agents" above an empty list is an instruction the
    // model cannot follow, and the footer is what explains the emptiness.
    if let Some(h) = header.filter(|_| !items.is_empty()) {
        let trimmed = h.trim();
        if !trimmed.is_empty() {
            result.push_str(trimmed);
            result.push('\n');
        }
    }
    for (key, value) in items {
        result.push_str(&format!("- {key}: {value}\n"));
    }
    if let Some(f) = footer {
        result.push_str(f);
        result.push('\n');
    }
    result.push_str(&format!("</{tag}>"));
}

/// Cap on tool names spelled out for one server in `<mcpservers>`. A server with
/// more than this is summarised and the tail left to `--help`: the point is to
/// show what the server is *for*, which the first couple of dozen names do.
const MCP_TOOLS_LISTED: usize = 24;

/// How one running MCP server introduces itself in `<mcpservers>`.
///
/// The tool names are already in hand - the manager caches them when the server
/// starts - and leaving them out of the prompt was quietly costing calls. A model
/// weighing `mcpctl homeassistant --help` (a round-trip that answers nothing by
/// itself) against `memory_search` (a round-trip that might answer everything)
/// picks memory, and then answers a question about a live system from notes about
/// that system. Naming the tools puts the capability in front of it for free.
pub fn mcp_server_line(handle: &str, description: &str, tools: &[String]) -> String {
    let desc = description
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches('.');
    let mut line = if desc.is_empty() {
        String::new()
    } else {
        format!("{desc}. ")
    };
    if tools.is_empty() {
        line.push_str(&format!("Tools: run `mcpctl {handle} --help`"));
        return line;
    }
    let shown = tools.len().min(MCP_TOOLS_LISTED);
    line.push_str("Tools: ");
    line.push_str(&tools[..shown].join(", "));
    if tools.len() > shown {
        line.push_str(&format!(
            " (+{} more: `mcpctl {handle} --help`)",
            tools.len() - shown
        ));
    }
    line
}

/// Renders standing instructions and skill expansions.
/// Callers persist dynamic text once; replay never renders it again.
#[derive(Clone)]
pub struct AgentPromptBuilder {
    prompts: PromptLoader,
    storage: StorageService,
    memory: std::sync::Arc<dyn crate::memory::service::MemoryService>,
    skills: crate::agent::skill::service::SkillService,
}

impl AgentPromptBuilder {
    pub fn new(
        prompts: PromptLoader,
        storage: StorageService,
        memory: std::sync::Arc<dyn crate::memory::service::MemoryService>,
        skills: crate::agent::skill::service::SkillService,
    ) -> Self {
        Self {
            prompts,
            storage,
            memory,
            skills,
        }
    }

    pub async fn render_skill(
        &self,
        user: &crate::auth::models::User,
        agent: &crate::agent::models::Agent,
        name: &str,
        prompt: &str,
    ) -> Result<Option<String>, crate::core::error::AppError> {
        let skills = self
            .skills
            .try_list(&user.handle, &agent.handle, agent.skills.as_deref())
            .await?;
        crate::chat::command::render::try_render_skill(name, prompt, &skills)
    }

    pub async fn build(
        &self,
        base_prompt: &str,
        identity: &BTreeMap<String, String>,
        agent_handle: &Handle,
        user: &crate::auth::User,
        additional_instructions: &[&str],
    ) -> Result<String, crate::core::error::AppError> {
        let mut result = base_prompt.to_string();
        if !["name", "creature", "vibe"]
            .iter()
            .all(|key| identity.keys().any(|k| k.eq_ignore_ascii_case(key)))
        {
            let ws = self.storage.agent_workspace(&user.handle, agent_handle);
            if let Some(text) = AgentPromptLoader::new(&ws, &self.prompts).read("IDENTITY.md") {
                result.push_str("\n\n");
                result.push_str(&text);
            }
        }
        for name in ["WORKSPACE.md", "TOOLS.md", "SKILLS.md", "SCHEDULING.md"] {
            if let Some(text) = self.prompts.read(name) {
                result.push_str("\n\n");
                result.push_str(&text);
            }
        }
        append_tagged_section(
            &mut result,
            "agent_identity",
            None,
            &identity
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>(),
            None,
        );
        // The wrapper is stable even when the first server is installed.
        if let Some(instructions) = self.prompts.read("MCP.md") {
            result.push_str("\n\n");
            result.push_str(&instructions);
        }

        for text in additional_instructions {
            result.push_str("\n\n");
            result.push_str(text);
        }
        let memory = self.memory.instructions(user).await?;
        if !memory.is_empty() {
            result.push_str("\n\n");
            result.push_str(&memory);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn builder_keeps_instruction_order_and_scopes_memory_without_collecting_it() {
        use crate::agent::context::{AgentContextScope, AgentContextSection};
        use crate::core::error::AppError;
        use crate::memory::service::MemoryService;
        use std::sync::Arc;

        struct MemoryInstructions;
        #[async_trait::async_trait]
        impl MemoryService for MemoryInstructions {
            fn tools(&self) -> Vec<Arc<dyn crate::tool::AgentTool>> {
                vec![]
            }
            async fn instructions(&self, user: &crate::auth::User) -> Result<String, AppError> {
                if user.id == "unavailable" {
                    return Err(AppError::Internal("memory instructions unavailable".into()));
                }
                Ok(format!("Memory instructions for {}", user.id))
            }
            async fn collect(
                &self,
                _: &AgentContextScope<'_>,
            ) -> Result<Vec<AgentContextSection>, AppError> {
                panic!("Standing prompts must not collect dynamic memory");
            }
        }

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("WORKSPACE.md"), "Workspace instructions").unwrap();
        let storage = StorageService::new(&crate::core::config::Config::default());
        let builder = AgentPromptBuilder::new(
            PromptLoader::new(dir.path()),
            storage.clone(),
            Arc::new(MemoryInstructions),
            crate::agent::skill::service::SkillService::new(
                crate::agent::skill::registry::SkillRegistryClient::new(
                    reqwest::Client::new(),
                    dir.path(),
                ),
                crate::agent::skill::resolver::SkillResolver::new(
                    dir.path(),
                    storage.clone(),
                    dir.path(),
                ),
                storage,
                dir.path(),
                &crate::core::config::CacheConfig::default(),
            ),
        );
        let identity = BTreeMap::from([
            ("name".into(), "Agent".into()),
            ("creature".into(), "Assistant".into()),
            ("vibe".into(), "Helpful".into()),
        ]);
        let handle = Handle::try_new("agent").unwrap();
        let mut user = crate::auth::User {
            phone: None,
            id: "alice".into(),
            handle: crate::handle!("alice"),
            email: "alice@example.com".into(),
            name: "Alice".into(),
            password_hash: String::new(),
            timezone: None,
            groups: vec![],
            deactivated_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let first = builder
            .build(
                "Base instructions",
                &identity,
                &handle,
                &user,
                &["Task instructions", "Conversation instructions"],
            )
            .await
            .unwrap();
        assert!(first.starts_with("Base instructions\n\nWorkspace instructions"));
        assert!(first.ends_with(
            "\n\nTask instructions\n\nConversation instructions\n\nMemory instructions for alice"
        ));
        assert_eq!(first.matches("Memory instructions").count(), 1);
        user.id = "bob".into();
        let second = builder
            .build("Base instructions", &identity, &handle, &user, &[])
            .await
            .unwrap();
        assert!(second.ends_with("Memory instructions for bob"));
        assert!(!second.contains("alice"));
        assert!(!second.contains("Task instructions"));
        user.id = "unavailable".into();
        assert!(
            builder
                .build("Base instructions", &identity, &handle, &user, &[])
                .await
                .is_err()
        );
    }

    fn shared_prompts_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("resources")
            .join("prompts")
    }

    #[test]
    fn reads_prompt_from_base_dir() {
        let loader = PromptLoader::new(shared_prompts_dir());
        let content = loader.read("CHAT_COMPACTION.md");
        assert!(content.is_some());
        assert!(content.unwrap().contains("conversation summarizer"));
    }

    #[test]
    fn returns_none_for_missing_prompt() {
        let loader = PromptLoader::new("/nonexistent");
        assert!(loader.read("DOES_NOT_EXIST.md").is_none());
    }

    #[test]
    fn list_dir_returns_files() {
        let loader = PromptLoader::new(shared_prompts_dir());
        let files = loader.list_dir("tools");
        assert!(!files.is_empty(), "Expected tool files in dir");
        assert!(files.iter().any(|f| f.ends_with("shell.md")));
        assert!(files.iter().any(|f| f.ends_with("python.md")));
    }

    #[test]
    fn read_renders_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.md"), "Hello {{name}}!").unwrap();
        let loader = PromptLoader::new(dir.path()).with_var("name", "World");
        let content = loader.read("test.md").unwrap();
        assert_eq!(content, "Hello World!");
    }

    #[test]
    fn read_with_vars_overrides_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.md"), "Hello {{name}}!").unwrap();
        let loader = PromptLoader::new(dir.path()).with_var("name", "Default");
        let content = loader
            .read_with_vars("test.md", &[("name", "Override")])
            .unwrap();
        assert_eq!(content, "Hello Override!");
    }

    #[test]
    fn raw_contents_are_cached_after_first_read() {
        // Documents the tradeoff: prompt files are treated as fixed for the
        // process lifetime, so an edit needs a restart to be picked up.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "first").unwrap();
        let loader = PromptLoader::new(dir.path());
        assert_eq!(loader.read("test.md").unwrap(), "first");

        std::fs::write(&path, "second").unwrap();
        assert_eq!(
            loader.read("test.md").unwrap(),
            "first",
            "served from cache"
        );
    }

    #[test]
    fn missing_file_is_not_negatively_cached() {
        let dir = tempfile::tempdir().unwrap();
        let loader = PromptLoader::new(dir.path());
        assert!(loader.read("later.md").is_none());

        std::fs::write(dir.path().join("later.md"), "now here").unwrap();
        assert_eq!(loader.read("later.md").unwrap(), "now here");
    }

    #[test]
    fn cache_is_shared_across_clones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "original").unwrap();
        let loader = PromptLoader::new(dir.path());
        assert_eq!(loader.read("test.md").unwrap(), "original");

        // Clones (and `with_var` derivations) must not each re-read the file.
        let derived = loader.clone().with_var("unused", "x");
        std::fs::write(&path, "changed").unwrap();
        assert_eq!(derived.read("test.md").unwrap(), "original");
    }

    #[test]
    fn read_with_vars_renders_active_call_template() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("active_call.md"),
            "[CALL_CONNECTED: Now speaking with {{caller_name}} ({{phone_number}}). Goal: {{objective}}.]",
        ).unwrap();
        let loader = PromptLoader::new(dir.path());
        let content = loader
            .read_with_vars(
                "active_call.md",
                &[
                    ("caller_name", "Alice"),
                    ("phone_number", "+1234567890"),
                    ("objective", "Schedule meeting"),
                ],
            )
            .unwrap();
        assert_eq!(
            content,
            "[CALL_CONNECTED: Now speaking with Alice (+1234567890). Goal: Schedule meeting.]"
        );
    }

    #[test]
    fn a_server_names_its_tools_so_no_call_is_spent_discovering_them() {
        let line = mcp_server_line(
            "homeassistant",
            "Home Assistant",
            &["get_state".into(), "call_service".into()],
        );
        assert_eq!(line, "Home Assistant. Tools: get_state, call_service");
    }

    #[test]
    fn a_server_with_too_many_tools_shows_the_first_and_points_at_help() {
        let tools: Vec<String> = (0..MCP_TOOLS_LISTED + 5).map(|i| format!("t{i}")).collect();
        let line = mcp_server_line("github", "GitHub.", &tools);
        assert!(line.contains("t0, t1"), "{line}");
        assert!(
            line.ends_with("(+5 more: `mcpctl github --help`)"),
            "the tail is reachable, not lost:\n{line}"
        );
        assert!(!line.contains(&format!("t{}", MCP_TOOLS_LISTED)), "{line}");
    }

    /// A server whose cache is empty (just installed, never started) still has to
    /// say how to reach it - silence reads as "nothing here".
    #[test]
    fn a_server_with_no_cached_tools_still_points_at_help() {
        let line = mcp_server_line("weather", "Forecasts", &[]);
        assert_eq!(line, "Forecasts. Tools: run `mcpctl weather --help`");
    }

    /// A section with no items and nothing to explain stays out of the prompt.
    #[test]
    fn an_empty_section_with_no_footer_is_not_emitted() {
        let mut out = String::new();
        append_tagged_section(&mut out, "available_agents", Some("Delegate!"), &[], None);
        assert!(out.is_empty(), "{out}");
    }

    /// The regression this file exists to prevent: every colleague denied, so
    /// the list is empty - and the agent is told why instead of being shown
    /// nothing and concluding the user has no other agents.
    #[test]
    fn a_section_whose_items_were_all_denied_still_says_so() {
        let mut out = String::new();
        append_tagged_section(
            &mut out,
            "available_agents",
            Some("Delegate to these agents, always."),
            &[],
            denied_delegation_note(&["Researcher".into(), "Developer".into()]).as_deref(),
        );
        assert!(
            out.contains("<available_agents>"),
            "section present:\n{out}"
        );
        assert!(out.contains("Researcher, Developer"), "names both:\n{out}");
        assert!(
            !out.contains("Delegate to these agents, always."),
            "the header introduces items there aren't any of:\n{out}"
        );
        assert!(out.trim_end().ends_with("</available_agents>"), "{out}");
    }

    /// A partly-denied view keeps both halves: the reachable agents as items,
    /// the rest as the footer.
    #[test]
    fn a_partly_denied_section_lists_the_reachable_and_names_the_rest() {
        let mut out = String::new();
        append_tagged_section(
            &mut out,
            "available_agents",
            Some("Delegate to these agents, always."),
            &[("Developer".into(), "Writes code".into())],
            denied_delegation_note(&["Receptionist".into()]).as_deref(),
        );
        assert!(out.contains("Delegate to these agents, always."), "{out}");
        assert!(out.contains("- Developer: Writes code"), "{out}");
        assert!(
            out.contains("Not delegable from here: Receptionist."),
            "{out}"
        );
    }

    #[test]
    fn nothing_denied_means_no_note() {
        assert!(denied_delegation_note(&[]).is_none());
    }
}
