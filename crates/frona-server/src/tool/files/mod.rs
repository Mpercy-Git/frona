//! Typed file tools for in-workspace work. `produce_file` is the
//! publish-to-chat-as-attachment primitive; these tools are for working
//! with files inside the agent's workspace (and Cedar-permitted siblings).

pub mod edit;
pub mod glob;
pub mod grep;
pub mod read;
pub mod write;

pub use edit::EditTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use read::ReadTool;
pub use write::WriteTool;

/// Workspace root for the agent this tool call is running as.
///
/// Scoped to the agent **owner** (`ctx.agent_owner_handle`), not the runner
/// (`ctx.user.handle`). The two are equal for an owned agent and diverge for a
/// shared one — `ShareLevel::Use` lets a recipient run an agent they don't own,
/// and such a run executes against the owner's workspace. That is also exactly
/// what `SandboxManager::for_tool` mounts, so resolving under the runner would
/// put every relative path outside the mount and get it denied.
pub fn workspace_root(
    ctx: &super::InferenceContext,
    storage: &crate::storage::service::StorageService,
) -> std::path::PathBuf {
    storage.agent_workspace_path(&ctx.agent_owner_handle, &ctx.agent.handle)
}

/// Virtual-path URIs (`agent://`, `user://`) are deliberately rejected:
/// they have no slot for the owning user, so they cannot address an
/// agent owned by a specific user without separate context.
///
/// Takes the whole `InferenceContext` rather than a pair of loose handles: the
/// owner/agent pair has exactly one correct binding, and passing them
/// separately let call sites choose the wrong one.
pub fn resolve_path(
    input: &str,
    ctx: &super::InferenceContext,
    storage: &crate::storage::service::StorageService,
) -> Result<std::path::PathBuf, crate::core::error::AppError> {
    if input.is_empty() {
        return Err(crate::core::error::AppError::Validation(
            "path must not be empty".into(),
        ));
    }
    if input.starts_with("agent://") || input.starts_with("user://") {
        return Err(crate::core::error::AppError::Validation(format!(
            "virtual-path URIs are not supported here: {input}",
        )));
    }
    if input.starts_with('/') {
        return Ok(std::path::PathBuf::from(input));
    }
    crate::storage::path::validate_relative_path(input)?;
    Ok(workspace_root(ctx, storage).join(input))
}

/// Atomically writes `content` to `target` via a tempfile in the same parent.
/// Returns an `AppError` if mkdir, tempfile creation, write, or rename fails.
pub async fn atomic_write(target: &std::path::Path, content: &[u8]) -> Result<(), crate::core::error::AppError> {
    let parent = target.parent().ok_or_else(|| {
        crate::core::error::AppError::Validation(format!(
            "path has no parent directory: {}",
            target.display()
        ))
    })?;
    tokio::fs::create_dir_all(parent).await.map_err(|e| {
        crate::core::error::AppError::Internal(format!("mkdir {}: {e}", parent.display()))
    })?;
    let target = target.to_path_buf();
    let content = content.to_vec();
    tokio::task::spawn_blocking(move || -> Result<(), crate::core::error::AppError> {
        let mut tmp = tempfile::NamedTempFile::new_in(target.parent().unwrap()).map_err(|e| {
            crate::core::error::AppError::Internal(format!("tempfile: {e}"))
        })?;
        use std::io::Write;
        tmp.write_all(&content).map_err(|e| {
            crate::core::error::AppError::Internal(format!("tempfile write: {e}"))
        })?;
        tmp.persist(&target).map_err(|e| {
            crate::core::error::AppError::Internal(format!(
                "atomic persist to {}: {e}",
                target.display()
            ))
        })?;
        Ok(())
    })
    .await
    .map_err(|e| crate::core::error::AppError::Internal(format!("join: {e}")))??;
    Ok(())
}

#[cfg(test)]
mod resolve_path_tests {
    use super::*;
    use crate::core::Handle;
    use crate::core::config::Config;
    use crate::storage::service::StorageService;
    use std::path::PathBuf;

    fn test_storage(data_dir: &str) -> StorageService {
        let mut cfg = Config::default();
        cfg.storage.data_dir = data_dir.to_string();
        StorageService::new(&cfg)
    }

    fn handle(s: &str) -> Handle {
        Handle::try_new(s).unwrap()
    }

    /// Context for a run of `agent` owned by `owner` and driven by `runner`.
    /// Pass the same handle for both to model an ordinary owned agent.
    fn test_ctx(owner: &str, runner: &str, agent: &str) -> super::super::InferenceContext {
        let broadcast = crate::chat::broadcast::BroadcastService::new();
        let event_sender = broadcast.create_event_sender("runner-id", "test-chat", None);
        crate::inference::request::InferenceContext::new(
            crate::auth::User {
                id: "runner-id".into(),
                handle: handle(runner),
                email: "runner@test.com".into(),
                name: "Runner".into(),
                password_hash: String::new(),
                timezone: None,
                phone: None,
                groups: Vec::new(),
                deactivated_at: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            crate::agent::models::Agent {
                id: "test-agent".into(),
                user_id: "owner-id".into(),
                handle: handle(agent),
                name: "Test Agent".into(),
                description: String::new(),
                model_group: "primary".into(),
                enabled: true,
                skills: None,
                sandbox_limits: None,
                max_concurrent_tasks: None,
                avatar: None,
                voice_id: None,
                identity: Default::default(),
                prompt: None,
                heartbeat_interval: None,
                next_heartbeat_at: None,
                heartbeat_chat_id: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            crate::chat::models::Chat {
                id: "test-chat".into(),
                user_id: "runner-id".into(),
                space_id: None,
                task_id: None,
                agent_id: "test-agent".into(),
                title: None,
                archived_at: None,
                channel_id: None,
                channel_external_id: None,
                metadata: Default::default(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            event_sender,
            tokio_util::sync::CancellationToken::new(),
            tokio_util::sync::CancellationToken::new(),
        )
        .with_agent_owner_handle(handle(owner))
    }

    /// Owned-agent run: owner and runner are the same user.
    fn run(
        input: &str,
        user: &str,
        agent: &str,
        data_dir: &str,
    ) -> Result<PathBuf, crate::core::error::AppError> {
        resolve_path(input, &test_ctx(user, user, agent), &test_storage(data_dir))
    }

    #[tokio::test]
    async fn bare_path_resolves_to_calling_agents_workspace() {
        // Regression: agent="system", user="mina". A bare path must resolve
        // to `users/mina/agents/system/...`, NOT `users/system/agents/system/...`.
        assert_eq!(
            run("notes.md", "mina", "system", "/app/data").unwrap(),
            PathBuf::from("/app/data/users/mina/agents/system/notes.md")
        );
    }

    #[tokio::test]
    async fn bare_path_with_subdir() {
        assert_eq!(
            run("subdir/notes.md", "mina", "system", "/app/data").unwrap(),
            PathBuf::from("/app/data/users/mina/agents/system/subdir/notes.md")
        );
    }

    #[tokio::test]
    async fn absolute_path_passes_through() {
        assert_eq!(
            run("/etc/hostname", "mina", "system", "/app/data").unwrap(),
            PathBuf::from("/etc/hostname")
        );
    }

    #[tokio::test]
    async fn agent_uri_is_rejected() {
        assert!(run("agent://system/notes.md", "mina", "system", "/app/data").is_err());
    }

    #[tokio::test]
    async fn user_uri_is_rejected() {
        assert!(run("user://mina/notes.md", "mina", "system", "/app/data").is_err());
    }

    #[tokio::test]
    async fn empty_input_is_rejected() {
        assert!(run("", "mina", "system", "/app/data").is_err());
    }

    #[tokio::test]
    async fn traversal_in_bare_path_is_rejected() {
        assert!(run("../etc/passwd", "mina", "system", "/app/data").is_err());
        assert!(run("subdir/../../etc/passwd", "mina", "system", "/app/data").is_err());
    }

    #[tokio::test]
    async fn bare_path_does_not_collapse_user_into_agent_handle() {
        // Regression: user=alice, agent=researcher. Resolved path must contain
        // BOTH handles in their proper positions, never doubling the agent handle.
        let resolved = run("output.txt", "alice", "researcher", "/data").unwrap();
        let s = resolved.to_string_lossy();
        assert!(s.contains("/users/alice/"), "missing user: {s}");
        assert!(s.contains("/agents/researcher/"), "missing agent: {s}");
        assert!(!s.contains("/users/researcher/"), "user-as-agent: {s}");
    }

    #[tokio::test]
    async fn shared_agent_resolves_under_owner_not_runner() {
        // Regression: bob runs alice's shared agent (ShareLevel::Use). The
        // sandbox mounts `users/alice/agents/researcher`, so resolution must
        // land there too. Resolving under the runner produced
        // `users/bob/agents/researcher/...` — outside the mount, so every
        // relative-path read/write in a shared run was denied.
        let resolved = resolve_path(
            "notes.md",
            &test_ctx("alice", "bob", "researcher"),
            &test_storage("/data"),
        )
        .unwrap();
        assert_eq!(
            resolved,
            PathBuf::from("/data/users/alice/agents/researcher/notes.md")
        );
    }

    #[tokio::test]
    async fn workspace_root_is_owner_scoped() {
        // The default scope for grep/glob (no `path` argument) must agree with
        // resolve_path, or an unscoped search in a shared run walks the wrong
        // tree.
        assert_eq!(
            workspace_root(&test_ctx("alice", "bob", "researcher"), &test_storage("/data")),
            PathBuf::from("/data/users/alice/agents/researcher")
        );
        assert_eq!(
            workspace_root(&test_ctx("mina", "mina", "system"), &test_storage("/app/data")),
            PathBuf::from("/app/data/users/mina/agents/system")
        );
    }
}
