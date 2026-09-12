//! Where a user's PKM files live.
//!
//! The user's handle, Memory directory name, and absolute vault root are resolved together
//! once per pass or request.

use std::path::{Path, PathBuf};

use crate::auth::user_service::UserService;
use crate::core::Handle;
use crate::core::error::AppError;

use super::storage::PkmStorage;

#[derive(Debug, Clone)]
pub struct VaultScope {
    handle: Handle,
    /// The Memory directory name (default `Memory`) - the only directory that surfaces
    /// in vault paths and wikilinks. Validated on construction.
    directory: String,
    /// Absolute `<data>/users/<handle>/pkm` - the agent's navigation root.
    root: PathBuf,
}

impl VaultScope {
    /// Resolve for a user. **The single place `shared_vault_directory` is read.**
    pub async fn resolve(
        users: &UserService,
        storage: &PkmStorage,
        user_id: &str,
        handle: &Handle,
    ) -> Result<Self, AppError> {
        let directory = users
            .user_config(user_id)
            .await?
            .memory
            .shared_vault_directory;
        storage.vault_scope(handle.clone(), &directory)
    }

    /// Build from an already-known directory name and root. Pure - no I/O, no config
    /// lookup; [`PkmStorage::vault_scope`] is the convenience that supplies the root.
    pub fn new(handle: Handle, directory: &str, root: PathBuf) -> Result<Self, AppError> {
        let directory = PkmStorage::validate_directory(directory)?.to_string();
        Ok(Self {
            handle,
            directory,
            root,
        })
    }

    /// A scope whose directory bypasses validation - for tests that must prove the
    /// storage-layer backstop still refuses a bad value that reached it anyway.
    #[cfg(test)]
    pub fn new_unchecked(handle: Handle, directory: &str, root: PathBuf) -> Self {
        Self {
            handle,
            directory: directory.to_string(),
            root,
        }
    }

    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    pub fn directory(&self) -> &str {
        &self.directory
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<directory>/<path>` - the vault-relative form used in wikilinks and search
    /// results. Pure string projection; `path` stays clean in the DB.
    pub fn vault_path(&self, page_path: &str) -> String {
        format!("{}/{}", self.directory, page_path)
    }

    /// `<root>/<directory>/<path>.md` - the absolute file the agent can `read` verbatim.
    ///
    /// The one home for a rule that matters: `memory_search` emits this so the agent never
    /// has to reconstruct it, because it gets that wrong (reading `.../me` before retrying
    /// `.../me.md`).
    pub fn abs_page_file(&self, page_path: &str) -> String {
        format!(
            "{}.md",
            self.root.join(self.vault_path(page_path)).display()
        )
    }

    /// The absolute file for a path already in vault-relative form (`<directory>/…`),
    /// used where the caller has an External note's own full path rather than a Memory
    /// page path.
    pub fn abs_vault_file(&self, vault_path: &str) -> String {
        format!("{}.md", self.root.join(vault_path).display())
    }

    /// Every entity path a path the agent cited could mean, most likely first.
    ///
    /// `memory_search` hands out two shapes of absolute path, because the two page
    /// origins are addressed differently: an Internal page at
    /// `<root>/<directory>/services/postgres.md` is the entity `services/postgres`,
    /// while an External note at `<root>/Work Notes/standup.md` *is* the entity
    /// `Work Notes/standup` - its identity is its location. [`page_from_any`] only
    /// knows the first, so citing a User Vault note used to fail with "unknown entity
    /// path" no matter what the agent passed, and the advice to "use the path
    /// `memory_search` returned" sent it round the same search-then-cite loop forever.
    /// Both spellings are offered here and the caller lets the database pick. Every
    /// candidate is vault-relative and cannot climb out of the vault, so an absolute
    /// path from somewhere else on the filesystem yields none.
    pub fn entity_path_candidates(&self, input: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut push = |candidate: String| {
            if !candidate.is_empty()
                && !candidate.split('/').any(|seg| seg == "..")
                && !out.contains(&candidate)
            {
                out.push(candidate);
            }
        };
        if let Some(page) = self.page_from_any(input) {
            push(page);
        }
        if let Some(note) = self.vault_relative_from_any(input) {
            push(note);
        }
        out
    }

    /// The vault-relative form of a path under the mirror root - an External note's
    /// own identity (`Work Notes/standup`). `None` if `input` is not under the root
    /// (a bare relative path is returned unchanged, since that is already the form).
    fn vault_relative_from_any(&self, input: &str) -> Option<String> {
        let trimmed = input.trim().trim_end_matches(".md");
        let abs_root = std::path::absolute(&self.root).unwrap_or_else(|_| self.root.clone());
        for root in [&self.root, &abs_root] {
            if let Ok(rel) = Path::new(trimmed).strip_prefix(root) {
                let cleaned = rel.to_string_lossy().trim_matches('/').to_string();
                return (!cleaned.is_empty()).then_some(cleaned);
            }
        }
        // An absolute path that is not under this user's root is not addressable.
        (!trimmed.starts_with('/')).then(|| trimmed.trim_matches('/').to_string())
    }

    /// Recover a clean page path from whatever the agent supplied - an absolute path
    /// (`<root>/<directory>/people/bob.md`) or a vault-relative one
    /// (`<directory>/people/bob`). `None` if it isn't under the Memory directory.
    pub fn page_from_any(&self, input: &str) -> Option<String> {
        let trimmed = input.trim().trim_end_matches(".md");
        let base = self.root.join(&self.directory);
        // `memory_search` absolutizes via `memory_root`, so when `data_dir` is configured
        // relative the emitted path won't share a prefix with an un-absolutized base - try
        // both spellings.
        let abs_base = std::path::absolute(&base).unwrap_or_else(|_| base.clone());
        for b in [&base, &abs_base] {
            if let Ok(rel) = Path::new(trimmed).strip_prefix(b) {
                let cleaned = rel.to_string_lossy();
                let cleaned = cleaned.trim_matches('/');
                if !cleaned.is_empty() {
                    return Some(cleaned.to_string());
                }
            }
        }
        let prefix = format!("{}/", self.directory);
        let cleaned = trimmed.trim_start_matches('/').strip_prefix(&prefix)?;
        let cleaned = cleaned.trim_matches('/');
        (!cleaned.is_empty()).then(|| cleaned.to_string())
    }
}
