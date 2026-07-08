use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};

pub async fn setup_redirect(
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = req.uri().path().trim_end_matches('/');

    if path.starts_with("/api")
        || path.starts_with("/_next")
        || req.uri().path().contains('.')
        || path == "/register"
        || path == "/setup"
    {
        return next.run(req).await;
    }

    Redirect::temporary("/register").into_response()
}
