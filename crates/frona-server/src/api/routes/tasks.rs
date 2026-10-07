use crate::agent::task::models::{CreateTaskRequest, TaskResponse, UpdateTaskRequest};
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tasks", get(list_active_tasks).post(create_task))
        .route(
            "/api/tasks/{id}",
            get(get_task).put(update_task).delete(delete_task),
        )
        .route("/api/tasks/{id}/cancel", axum::routing::post(cancel_task))
        .route("/api/tasks/{id}/runs", get(list_cron_runs))
}

async fn get_task(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TaskResponse>, ApiError> {
    let task = state
        .task_service
        .find_by_id(&id)
        .await?
        .ok_or_else(|| crate::core::error::AppError::NotFound("Task not found".into()))?;

    if task.user_id != auth.user_id {
        return Err(crate::core::error::AppError::Forbidden("Not your task".into()).into());
    }

    Ok(Json(task.into()))
}

async fn create_task(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateTaskRequest>,
) -> Result<Json<TaskResponse>, ApiError> {
    let due_now = req.run_at.is_none_or(|at| at <= chrono::Utc::now());
    let response = state.task_service.create(&auth.user_id, req).await?;

    // Start a task that is due now instead of leaving it for the scheduler's
    // sweep, which runs only every `scheduler.poll_secs` (60 s by default). A
    // task created by an outside system (a doorbell ring, a webhook) is
    // usually one somebody is waiting on. The sweep still re-drives it if a
    // concurrency cap keeps `run_task` from starting it here.
    if due_now {
        let exec = state.task_executor.clone();
        let task_id = response.id.clone();
        tokio::spawn(async move {
            if let Err(e) = exec.run_task_by_id(&task_id).await {
                tracing::warn!(error = %e, task_id = %task_id, "Failed to start task created through the API");
            }
        });
    }

    Ok(Json(response))
}

async fn list_active_tasks(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskResponse>>, ApiError> {
    let tasks = state.task_service.list_all(&auth.user_id).await?;
    Ok(Json(tasks))
}

async fn update_task(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTaskRequest>,
) -> Result<Json<TaskResponse>, ApiError> {
    let task = state.task_service.update(&auth.user_id, &id, req).await?;
    Ok(Json(task))
}

async fn delete_task(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(), ApiError> {
    // Verify ownership before cancelling so another user cannot abort a
    // foreign task by guessing its ID.
    let task = state
        .task_service
        .find_by_id(&id)
        .await?
        .ok_or_else(|| crate::core::error::AppError::NotFound("Task not found".into()))?;
    if task.user_id != auth.user_id {
        return Err(crate::core::error::AppError::Forbidden("Not your task".into()).into());
    }
    // Fire cancellation token so in-flight inference unwinds cleanly.
    state.task_executor.cancel_task(&id).await;
    state.task_service.delete(&auth.user_id, &id).await?;
    Ok(())
}

async fn cancel_task(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TaskResponse>, ApiError> {
    let task = state.task_service.cancel(&auth.user_id, &id).await?;

    state.task_executor.cancel_task(&id).await;

    Ok(Json(task.into()))
}

async fn list_cron_runs(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TaskResponse>>, ApiError> {
    let template = state
        .task_service
        .find_by_id(&id)
        .await?
        .ok_or_else(|| crate::core::error::AppError::NotFound("Task not found".into()))?;
    if template.user_id != auth.user_id {
        return Err(crate::core::error::AppError::Forbidden("Not your task".into()).into());
    }

    let runs = state.task_service.find_runs_by_cron(&id).await?;
    Ok(Json(runs.into_iter().map(Into::into).collect()))
}
