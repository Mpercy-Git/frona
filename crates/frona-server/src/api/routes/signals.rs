use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::agent::signal::CandidateEvent;
use crate::core::error::AppError;
use crate::core::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/signals/evaluate", post(evaluate_signal))
}

#[derive(Debug, Deserialize)]
pub struct EvaluateSignalRequest {
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub connector_id: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub contact_id: Option<String>,
    #[serde(default)]
    pub sender: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub space_id: Option<String>,
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct EvaluateSignalResponse {
    pub fired_watches: Vec<String>,
}

const MAX_TAGS: usize = 32;
const MAX_CONTENT_BYTES: usize = 64 * 1024;

async fn evaluate_signal(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<EvaluateSignalRequest>,
) -> Result<Json<EvaluateSignalResponse>, ApiError> {
    if req.tags.is_empty() {
        return Err(AppError::Validation(
            "tags must contain at least one entry".into(),
        )
        .into());
    }
    if req.tags.len() > MAX_TAGS {
        return Err(AppError::Validation(format!(
            "tags must contain at most {MAX_TAGS} entries"
        ))
        .into());
    }
    if req.content.len() > MAX_CONTENT_BYTES {
        return Err(AppError::Validation(format!(
            "content must be at most {MAX_CONTENT_BYTES} bytes"
        ))
        .into());
    }

    let candidate = CandidateEvent {
        user_id: auth.user_id.clone(),
        space_id: req.space_id,
        chat_id: req.chat_id,
        message_id: req.message_id,
        connector_id: req.connector_id,
        channel_id: req.channel_id,
        contact_id: req.contact_id,
        sender: req.sender,
        tags: req.tags,
        summary: req.summary,
        content: req.content,
    };

    let signal_service = state.signal_service().ok_or_else(|| {
        AppError::Internal("Signal service not initialized".into())
    })?;
    let fired_watches = signal_service.evaluate(&auth.user_id, candidate).await?;
    Ok(Json(EvaluateSignalResponse { fired_watches }))
}
