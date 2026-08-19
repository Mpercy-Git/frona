use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use crate::agent::skill::resolver::Skill;
use crate::agent::workspace::AgentPromptLoader;
use crate::core::Handle;
use crate::core::template::render_template;
use crate::storage::StorageService;

#[derive(Clone)]
pub struct PromptLoader {
    base_dir: PathBuf,
    defaults: HashMap<String, String>,
}

impl PromptLoader {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            defaults: HashMap::new(),
        }
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

    pub fn read_with_vars(&self, name: &str, vars: &[(&str, &str)]) -> Option<String> {
        let path = self.base_dir.join(name);
        let raw = std::fs::read_to_string(&path).ok()?;
        let merged = self.merge_vars(vars);
        let merged_refs: Vec<(&str, &str)> = merged
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        render_template(&raw, &merged_refs).ok()
    }

    pub fn read_raw(&self, name: &str) -> Option<String> {
        let path = self.base_dir.join(name);
        std::fs::read_to_string(&path).ok()
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

pub fn append_tagged_section(
    result: &mut String,
    tag: &str,
    header: Option<&str>,
    items: &[(String, String)],
) {
    if items.is_empty() {
        return;
    }
    result.push_str(&format!("\n\n<{tag}>\n"));
    if let Some(h) = header {
        let trimmed = h.trim();
        if !trimmed.is_empty() {
            result.push_str(trimmed);
            result.push('\n');
        }
    }
    for (key, value) in items {
        result.push_str(&format!("- {key}: {value}\n"));
    }
    result.push_str(&format!("</{tag}>"));
}

/// Assemble the agent's full system prompt (identity, agent prompt files,
/// skills, MCP, available agents, temporal context). The **memory** service
/// contributes only `memory_section` (its static `MEMORY.md`); the dynamic
/// memory blocks are appended later by `MemoryService::retrieve`. Ordered
/// static → almost-static → dynamic to maximise the cacheable prefix.
#[allow(clippy::too_many_arguments)]
pub fn build_augmented_system_prompt(
    base_prompt: &str,
    identity: &BTreeMap<String, String>,
    prompts: &PromptLoader,
    storage: &StorageService,
    user_handle: &Handle,
    agent_handle: &Handle,
    skills: &[Skill],
    agent_summaries: &[(String, String)],
    mcp_servers: &[(String, String)],
    user_timezone: &str,
) -> String {
    let mut result = base_prompt.to_string();

    // IDENTITY.md fallback - only when the agent has no core identity keys.
    const CORE_IDENTITY_KEYS: &[&str] = &["name", "creature", "vibe"];
    let has_core_identity = CORE_IDENTITY_KEYS
        .iter()
        .all(|core_key| identity.keys().any(|k| k.eq_ignore_ascii_case(core_key)));
    if !has_core_identity {
        let ws = storage.agent_workspace(user_handle, agent_handle);
        if let Some(identity_prompt) = AgentPromptLoader::new(&ws, prompts).read("IDENTITY.md") {
            result.push_str("\n\n");
            result.push_str(&identity_prompt);
        }
    }

    // Static agent prompt files. The memory backend's usage section is no longer
    // spliced here - `MemoryService::retrieve` prepends it ahead of its own
    // dynamic tags (so the constant part stays in the cacheable prefix).
    for name in ["WORKSPACE.md", "TOOLS.md", "SKILLS.md"] {
        if let Some(content) = prompts.read(name) {
            result.push_str("\n\n");
            result.push_str(&content);
        }
    }
    if let Some(content) = prompts.read("SCHEDULING.md") {
        result.push_str("\n\n");
        result.push_str(&content);
    }

    let skill_items: Vec<(String, String)> = skills
        .iter()
        .filter(|s| !s.disable_model_invocation)
        .map(|s| {
            (
                s.name.clone(),
                format!("{} (file: {}/SKILL.md)", s.description, s.path),
            )
        })
        .collect();
    append_tagged_section(&mut result, "available_skills", None, &skill_items);

    if !mcp_servers.is_empty() {
        if let Some(mcp_prompt) = prompts.read("MCP.md") {
            result.push_str("\n\n");
            result.push_str(&mcp_prompt);
        }
        append_tagged_section(&mut result, "mcpservers", None, mcp_servers);
    }

    append_tagged_section(
        &mut result,
        "available_agents",
        prompts.read("AVAILABLE_AGENTS.md").as_deref(),
        agent_summaries,
    );

    let identity_pairs: Vec<(String, String)> = identity
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    append_tagged_section(&mut result, "agent_identity", None, &identity_pairs);

    // Date-only keeps this byte-stable within a day so prefix caches stay warm.
    let tz: chrono_tz::Tz = user_timezone.parse().unwrap_or(chrono_tz::UTC);
    let now_local = chrono::Utc::now().with_timezone(&tz);
    let items = vec![
        (
            "current_date_local".to_string(),
            format!(
                "{} ({})",
                now_local.format("%Y-%m-%d"),
                now_local.format("%A")
            ),
        ),
        ("user_timezone".to_string(), user_timezone.to_string()),
    ];
    append_tagged_section(&mut result, "temporal_context", None, &items);

    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn assembler_places_base_prompt() {
        let prompts = PromptLoader::new(shared_prompts_dir());
        let storage = StorageService::new(&crate::core::config::Config::default());
        let mut identity = BTreeMap::new();
        for k in ["name", "creature", "vibe"] {
            identity.insert(k.to_string(), "x".to_string());
        }
        let prompt = build_augmented_system_prompt(
            "BASE_PROMPT_MARKER",
            &identity,
            &prompts,
            &storage,
            &crate::handle!("user"),
            &crate::handle!("agent"),
            &[],
            &[],
            &[],
            "UTC",
        );
        assert!(
            prompt.starts_with("BASE_PROMPT_MARKER"),
            "base prompt leads"
        );
        assert!(
            prompt.contains("<temporal_context>"),
            "dynamic temporal tail present"
        );
    }
}
