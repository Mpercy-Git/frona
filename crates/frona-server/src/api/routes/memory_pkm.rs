//! `/api/memory/pkm/*`, the Obsidian sync API (read side: manifest-diff pull).
//! Gated twice: the PAT must carry the `memory` scope, and PKM must be the
//! active memory backend (`state.pkm_sync` is `Some` only then).


use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::memory::pkm::sync::{EditOp, EditResult, PkmSyncService};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

const SCOPE: &str = "memory";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/memory/pkm/sync/config", get(config).post(set_config))
        .route("/api/memory/pkm/sync/manifest", get(manifest))
        .route("/api/memory/pkm/sync/pages", post(pages))
        .route("/api/memory/pkm/sync/edits", post(edits))
        .route("/api/memory/pkm/sync/rename", post(rename))
}

/// Resolve the sync engine, enforcing the `memory` scope (→ 403) and PKM being the
/// active backend (→ 404).
///
/// `state.pkm_sync` is `Some` only under PKM, so **presence is the gate**. No config
/// re-read, and no per-request assembly. It used to build a fresh `PkmSyncService` with its own
/// `PkmRepo` and `PkmStorage` on every call, which is how the sync engine ended up with a
/// rename and a CAS check independent of the main service instance.
fn require_sync<'a>(auth: &AuthUser, state: &'a AppState) -> Result<&'a PkmSyncService, AppError> {
    if !auth.has_scope(SCOPE) {
        return Err(AppError::Forbidden(format!("token lacks '{SCOPE}' scope")));
    }
    state.pkm_sync.as_ref().ok_or_else(|| {
        AppError::NotFound("memory sync is not enabled (PKM backend inactive)".into())
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigResponse {
    memory_directory: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetConfigRequest {
    memory_directory: String,
}

async fn config(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<ConfigResponse>, ApiError> {
    let sync = require_sync(&auth, &state)?;
    Ok(Json(ConfigResponse {
        memory_directory: sync.memory_directory(&auth.user_id).await?,
    }))
}

/// Set the Memory directory name (single clean segment). Returns the effective value.
async fn set_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SetConfigRequest>,
) -> Result<Json<ConfigResponse>, ApiError> {
    let sync = require_sync(&auth, &state)?;
    sync.set_memory_directory(&auth.user_id, &req.memory_directory).await?;
    Ok(Json(ConfigResponse {
        memory_directory: sync.memory_directory(&auth.user_id).await?,
    }))
}

#[derive(Serialize)]
struct ManifestEntryItem {
    path: String,
    rev: String,
}

#[derive(Serialize)]
struct ManifestResponse {
    pages: Vec<ManifestEntryItem>,
}

async fn manifest(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<ManifestResponse>, ApiError> {
    let sync = require_sync(&auth, &state)?;
    let pages = sync
        .manifest(&auth.user_id)
        .await?
        .into_iter()
        .map(|e| ManifestEntryItem {
            path: e.path,
            rev: e.rev,
        })
        .collect();
    Ok(Json(ManifestResponse { pages }))
}

#[derive(Deserialize)]
struct PagesRequest {
    paths: Vec<String>,
}

#[derive(Serialize)]
struct PageItem {
    path: String,
    rev: String,
    content: String,
}

#[derive(Serialize)]
struct PagesResponse {
    pages: Vec<PageItem>,
}

async fn pages(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<PagesRequest>,
) -> Result<Json<PagesResponse>, ApiError> {
    let sync = require_sync(&auth, &state)?;
    let pages = sync
        .get_pages(&auth.user_id, &auth.handle, &req.paths)
        .await?
        .into_iter()
        .map(|p| PageItem {
            path: p.path,
            rev: p.rev,
            content: p.content,
        })
        .collect();
    Ok(Json(PagesResponse { pages }))
}


#[derive(Deserialize)]
struct EditsRequest {
    edits: Vec<EditItem>,
}

/// Wire verb for an edit. Unknown values fall back to `Upsert`, preserving the
/// pre-typed behavior where only `delete` was special-cased.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum EditVerb {
    Delete,
    #[serde(other)]
    Upsert,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditItem {
    op: EditVerb,
    path: String,
    #[serde(default)]
    base_rev: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct EditResultItem {
    path: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head_rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head_content: Option<String>,
    /// Pages moved by a directory rename (`status = "renamed"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<usize>,
}

impl EditResultItem {
    fn from_result(path: String, r: EditResult) -> Self {
        match r {
            EditResult::Accepted { rev } => Self { path, status: "accepted", rev: Some(rev), ..Default::default() },
            EditResult::Created { rev } => Self { path, status: "created", rev: Some(rev), ..Default::default() },
            EditResult::Removed => Self { path, status: "removed", ..Default::default() },
            EditResult::Indexed => Self { path, status: "indexed", ..Default::default() },
            EditResult::Unchanged => Self { path, status: "unchanged", ..Default::default() },
            EditResult::Renamed { count } => Self { path, status: "renamed", count: Some(count), ..Default::default() },
            EditResult::Conflict { head_rev, head_content } => Self {
                path,
                status: "conflict",
                head_rev: Some(head_rev),
                head_content: Some(head_content),
                ..Default::default()
            },
        }
    }
}

#[derive(Serialize)]
struct EditsResponse {
    results: Vec<EditResultItem>,
}

async fn edits(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<EditsRequest>,
) -> Result<Json<EditsResponse>, ApiError> {
    let sync = require_sync(&auth, &state)?;
    let mut results = Vec::with_capacity(req.edits.len());
    for item in req.edits {
        let op = match item.op {
            EditVerb::Delete => EditOp::Delete { path: item.path.clone() },
            EditVerb::Upsert => EditOp::Upsert {
                path: item.path.clone(),
                base_rev: item.base_rev,
                content: item.content.unwrap_or_default(),
            },
        };
        let r = sync.apply_edit(&state.harness, &auth.user_id, &auth.handle, op).await?;
        results.push(EditResultItem::from_result(item.path, r));
    }
    Ok(Json(EditsResponse { results }))
}

#[derive(Deserialize)]
struct RenameRequest {
    from: String,
    to: String,
}

async fn rename(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<RenameRequest>,
) -> Result<Json<EditResultItem>, ApiError> {
    let sync = require_sync(&auth, &state)?;
    let r = sync.rename(&auth.user_id, &auth.handle, &req.from, &req.to).await?;
    Ok(Json(EditResultItem::from_result(req.to, r)))
}
