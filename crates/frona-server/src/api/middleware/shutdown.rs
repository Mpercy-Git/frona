use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::core::state::AppState;

pub async fn shutdown_gate(
    State(state): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    if state.is_shutting_down() && !is_healthcheck(&req) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "Server is shutting down"})),
        )
            .into_response();
    }
    next.run(req).await
}

fn is_healthcheck(req: &axum::http::Request<axum::body::Body>) -> bool {
    let path = req.uri().path();
    path == "/api/system/health" || path == "/healthz"
}
