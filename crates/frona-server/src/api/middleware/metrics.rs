use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::IntoResponse;
use metrics::{counter, histogram};

use crate::core::metrics::{HTTP_REQUESTS_TOTAL, HTTP_REQUEST_DURATION_SECONDS};

pub async fn track_http_metrics(req: Request, next: Next) -> impl IntoResponse {
    let method = req.method().to_string();
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());

    let start = Instant::now();
    let response = next.run(req).await;
    let duration = start.elapsed();
    let status = response.status().as_u16().to_string();

    let labels = [
        ("method", method),
        ("path", path),
        ("status", status),
    ];
    counter!(HTTP_REQUESTS_TOTAL, &labels).increment(1);
    histogram!(HTTP_REQUEST_DURATION_SECONDS, &labels).record(duration.as_secs_f64());

    response
}
