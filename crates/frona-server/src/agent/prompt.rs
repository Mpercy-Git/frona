use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use std::path::PathBuf;

use crate::core::template::render_template;

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
        let merged_refs: Vec<(&str, &str)> = merged.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
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
        let content = loader.read_with_vars("test.md", &[("name", "Override")]).unwrap();
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
        assert_eq!(loader.read("test.md").unwrap(), "first", "served from cache");
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
        let content = loader.read_with_vars("active_call.md", &[
            ("caller_name", "Alice"),
            ("phone_number", "+1234567890"),
            ("objective", "Schedule meeting"),
        ]).unwrap();
        assert_eq!(
            content,
            "[CALL_CONNECTED: Now speaking with Alice (+1234567890). Goal: Schedule meeting.]"
        );
    }
}
