use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::State;
use tokio::fs;

use crate::storage::{VirtualPath, validate_relative_path};

use super::super::super::error::ApiError;
use super::super::super::middleware::auth::AuthUser;
use super::models::{CopyMoveRequest, DeleteRequest, MkdirRequest, RenameRequest};
use crate::core::error::AppError;
use crate::core::state::AppState;

pub(crate) async fn rename_user_file(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<RenameRequest>,
) -> Result<(), ApiError> {
    reject_workspace_root(&req.path, "rename")?;
    let trimmed = req.path.trim_start_matches('/');
    let vpath = VirtualPath::user(&auth.handle, trimmed);
    let resolved = state
        .storage_service
        .resolve_virtual_path_for_user(&auth.handle, &vpath)?;

    if !resolved.exists() {
        return Err(ApiError(AppError::NotFound("File not found".into())));
    }

    if req.new_name.contains('/') || req.new_name.contains("..") || req.new_name.contains('\0') {
        return Err(ApiError(AppError::Validation("Invalid filename".into())));
    }

    let dest = resolved
        .parent()
        .ok_or_else(|| ApiError(AppError::Internal("No parent dir".into())))?
        .join(&req.new_name);

    if dest.exists() {
        return Err(ApiError(AppError::Validation(
            "A file with that name already exists".into(),
        )));
    }

    fs::rename(&resolved, &dest)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    Ok(())
}

fn resolve_file_virtual_path(
    path: &str,
    auth: &AuthUser,
    storage: &crate::storage::StorageService,
) -> Result<PathBuf, ApiError> {
    // Every branch resolves against the caller. The `user://` branch used to be the
    // only one that checked, and `agent://` went straight to the resolver - where an
    // agent namespace was rooted at the handle in the URI, so `agent://victim/…`
    // named the victim's workspace and copied out of it.
    let vpath = if path.starts_with("user://") || path.starts_with("agent://") {
        VirtualPath::parse(path)?
    } else {
        VirtualPath::user(&auth.handle, path.trim_start_matches('/'))
    };
    storage
        .resolve_virtual_path_for_user(&auth.handle, &vpath)
        .map_err(ApiError)
}

/// Refuse an operation aimed at a workspace root rather than something in it.
///
/// An empty relative path resolves to the workspace directory itself, so a
/// rename or delete of `""` would move or erase the whole tree. Callers name a
/// file; the root is never the intended target.
fn reject_workspace_root(path: &str, operation: &str) -> Result<(), ApiError> {
    let relative = if path.starts_with("user://") || path.starts_with("agent://") {
        VirtualPath::parse(path)?.relative
    } else {
        path.trim_start_matches('/').to_string()
    };
    if relative.is_empty() {
        return Err(ApiError(AppError::Validation(format!(
            "Cannot {operation} a workspace root"
        ))));
    }
    Ok(())
}

fn ensure_user_destination(path: &str) -> Result<(), ApiError> {
    if path.starts_with("agent://") {
        return Err(ApiError(AppError::Forbidden(
            "Cannot write to agent workspaces".into(),
        )));
    }
    Ok(())
}

pub(crate) async fn copy_files(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CopyMoveRequest>,
) -> Result<(), ApiError> {
    ensure_user_destination(&req.destination)?;

    let dest_dir = resolve_file_virtual_path(&req.destination, &auth, &state.storage_service)?;

    fs::create_dir_all(&dest_dir)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    for source in &req.sources {
        // `move_files` has refused agent sources all along; copy did not, which is
        // how a caller reached an agent workspace at all. Same rule, same reason:
        // these routes serve a user's own files.
        if source.starts_with("agent://") {
            return Err(ApiError(AppError::Forbidden(
                "Cannot copy from agent workspaces".into(),
            )));
        }
        let src = resolve_file_virtual_path(source, &auth, &state.storage_service)?;
        if !src.exists() {
            continue;
        }
        let name = src
            .file_name()
            .ok_or_else(|| ApiError(AppError::Internal("No filename".into())))?
            .to_string_lossy()
            .into_owned();
        let target = dest_dir.join(&name);
        if src.is_dir() {
            copy_dir_recursive(&src, &target).await?;
        } else {
            fs::copy(&src, &target)
                .await
                .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
        }
    }

    Ok(())
}

async fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), ApiError> {
    fs::create_dir_all(dest)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    let mut read_dir = fs::read_dir(src)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?
    {
        let target = dest.join(entry.file_name());
        if entry
            .metadata()
            .await
            .map_err(|e| ApiError(AppError::Internal(e.to_string())))?
            .is_dir()
        {
            Box::pin(copy_dir_recursive(&entry.path(), &target)).await?;
        } else {
            fs::copy(entry.path(), &target)
                .await
                .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
        }
    }

    Ok(())
}

pub(crate) async fn move_files(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CopyMoveRequest>,
) -> Result<(), ApiError> {
    ensure_user_destination(&req.destination)?;

    let dest_dir = resolve_file_virtual_path(&req.destination, &auth, &state.storage_service)?;

    fs::create_dir_all(&dest_dir)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    for source in &req.sources {
        if source.starts_with("agent://") {
            return Err(ApiError(AppError::Forbidden(
                "Cannot move from agent workspaces".into(),
            )));
        }
        let src = resolve_file_virtual_path(source, &auth, &state.storage_service)?;
        if !src.exists() {
            continue;
        }
        let name = src
            .file_name()
            .ok_or_else(|| ApiError(AppError::Internal("No filename".into())))?
            .to_string_lossy()
            .into_owned();
        let target = dest_dir.join(&name);
        fs::rename(&src, &target)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
    }

    Ok(())
}

pub(crate) async fn delete_files(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<DeleteRequest>,
) -> Result<(), ApiError> {
    if req.paths.is_empty() {
        return Err(ApiError(AppError::Validation(
            "At least one path is required".into(),
        )));
    }

    // Resolve and check every path before removing any, so a bad entry late in
    // the batch fails the request instead of leaving it half applied.
    let mut resolved_paths = Vec::with_capacity(req.paths.len());
    for path in &req.paths {
        reject_workspace_root(path, "delete")?;
        // Same rule as copy and move: these routes serve a user's own files.
        if path.starts_with("agent://") {
            return Err(ApiError(AppError::Forbidden(
                "Cannot delete from agent workspaces".into(),
            )));
        }
        let resolved = resolve_file_virtual_path(path, &auth, &state.storage_service)?;
        if !resolved.exists() {
            return Err(ApiError(AppError::NotFound(format!(
                "File not found: {path}"
            ))));
        }
        resolved_paths.push(resolved);
    }

    for resolved in resolved_paths {
        if resolved.is_dir() {
            fs::remove_dir_all(&resolved)
                .await
                .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
        } else {
            fs::remove_file(&resolved)
                .await
                .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
        }
    }

    Ok(())
}

pub(crate) async fn create_user_folder(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<MkdirRequest>,
) -> Result<(), ApiError> {
    let trimmed = req.path.trim_start_matches('/');
    validate_relative_path(trimmed)?;

    let vpath = VirtualPath::user(&auth.handle, trimmed);
    let resolved = state
        .storage_service
        .resolve_virtual_path_for_user(&auth.handle, &vpath)?;

    fs::create_dir_all(&resolved)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    Ok(())
}
