//! Per-invocation ephemeral token files. A short-lived JWT is written to a
//! uniquely-named file outside the sandbox workspace; the sandbox is granted
//! read access to only that file. The file is unlinked when the
//! [`EphemeralTokenGuard`] drops.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

use super::User;
use super::token::models::TokenType;
use super::token::service::{CreateTokenRequest, TokenService};
use crate::core::Principal;
use crate::core::error::AppError;
use crate::credential::keypair::service::KeyPairService;

pub struct EphemeralTokenGuard {
    path: PathBuf,
}

impl EphemeralTokenGuard {
    pub async fn issue(
        token_service: &TokenService,
        keypair_service: &KeyPairService,
        user: &User,
        principal: Principal,
        ttl_secs: u64,
        root: &Path,
    ) -> Result<Self, AppError> {
        tokio::fs::create_dir_all(root).await.map_err(|e| {
            AppError::Internal(format!(
                "Failed to create runtime tokens dir {}: {e}",
                root.display()
            ))
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = tokio::fs::set_permissions(
                root,
                std::fs::Permissions::from_mode(0o700),
            )
            .await;
        }

        // Sandbox drivers (syd, landlock) need absolute paths in allowlists.
        let root = tokio::fs::canonicalize(root).await.map_err(|e| {
            AppError::Internal(format!(
                "Failed to canonicalize runtime tokens dir {}: {e}",
                root.display()
            ))
        })?;

        let created = token_service
            .create_token(
                keypair_service,
                user,
                CreateTokenRequest {
                    token_type: TokenType::Ephemeral,
                    principal,
                    ttl_secs,
                    name: String::new(),
                    scopes: Vec::new(),
                    refresh_pair_id: None,
                    extensions: None,
                },
            )
            .await?;

        let path = root.join(&created.token_id);
        write_atomic_0600(&path, created.jwt.as_bytes()).await?;

        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for EphemeralTokenGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Sweeps stale token files left after a prior crash.
pub fn prepare_runtime_dir(users_root: &Path) {
    let Ok(entries) = std::fs::read_dir(users_root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let tokens_dir = entry.path().join("tokens");
        cleanup_stale(&tokens_dir);
    }
}

fn cleanup_stale(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

async fn write_atomic_0600(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| {
        AppError::Internal(format!("Token path has no parent: {}", path.display()))
    })?;

    // tempfile in the same directory → atomic rename on the same filesystem.
    let tmp_name = format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("token"),
        uuid::Uuid::new_v4()
    );
    let tmp_path = parent.join(tmp_name);

    {
        let mut opts = tokio::fs::OpenOptions::new();
        #[cfg(unix)]
        {
            opts.mode(0o600);
        }

        let mut file = opts
            .create_new(true)
            .write(true)
            .open(&tmp_path)
            .await
            .map_err(|e| {
                AppError::Internal(format!(
                    "Failed to create ephemeral token tempfile {}: {e}",
                    tmp_path.display()
                ))
            })?;
        file.write_all(bytes).await.map_err(|e| {
            AppError::Internal(format!(
                "Failed to write ephemeral token to {}: {e}",
                tmp_path.display()
            ))
        })?;
        file.flush().await.map_err(|e| {
            AppError::Internal(format!(
                "Failed to flush ephemeral token to {}: {e}",
                tmp_path.display()
            ))
        })?;
    }

    tokio::fs::rename(&tmp_path, path).await.map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        AppError::Internal(format!(
            "Failed to rename ephemeral token into place {}: {e}",
            path.display()
        ))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_stale_is_idempotent_on_missing_dir() {
        let missing = std::env::temp_dir().join("nonexistent-frona-ephemeral-xxx");
        let _ = std::fs::remove_dir_all(&missing);
        cleanup_stale(&missing);
    }

    #[tokio::test]
    async fn write_atomic_creates_0600_file() {
        let dir = std::env::temp_dir().join(format!(
            "frona-eph-test-{}",
            uuid::Uuid::new_v4()
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("tok");
        write_atomic_0600(&path, b"jwt-content").await.unwrap();

        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"jwt-content");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = tokio::fs::metadata(&path)
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "token file must be owner-only");
        }

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn cleanup_stale_removes_files() {
        let dir = std::env::temp_dir().join(format!(
            "frona-eph-cleanup-{}",
            uuid::Uuid::new_v4()
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("a"), b"x").await.unwrap();
        tokio::fs::write(dir.join("b"), b"y").await.unwrap();

        cleanup_stale(&dir);

        let remaining: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        assert!(remaining.is_empty(), "stale files should be removed");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
