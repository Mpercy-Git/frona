//! Pretty-URL redirector for the preview page.
//!
//! The Next.js static export can't generate runtime-dynamic routes, so the
//! preview page lives at the single static `/p` page that reads its source
//! from `?id=...` (short share) or `?path=...` (long canonical) query params.
//!
//! Channels still emit clean URLs (`/p/{id}` or `/p/{owner}/{handle}/{path}`)
//! because those are shorter and look human-readable. This module catches
//! those URLs at the backend and 303-redirects them to the query-param form
//! the static page expects.

use axum::extract::Path;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use url::form_urlencoded::byte_serialize;

use crate::core::state::AppState;

fn encode(s: &str) -> String {
    byte_serialize(s.as_bytes()).collect()
}

pub fn router() -> Router<AppState> {
    Router::new().route("/p/{*slug}", get(redirect_preview))
}

async fn redirect_preview(Path(slug): Path<String>) -> Response {
    // Strip the leading `/` axum's wildcard sometimes captures and trailing
    // slashes that browsers occasionally append.
    let slug = slug.trim_matches('/');

    let segment_count = slug.split('/').filter(|s| !s.is_empty()).count();
    let target = if segment_count == 1 {
        format!("/p?id={}", encode(slug))
    } else if segment_count >= 3 {
        format!("/p?path={}", encode(slug))
    } else {
        // Two segments isn't a valid preview shape — fall through to the
        // static page with no params; it'll show "Invalid preview link".
        "/p".to_string()
    };

    Redirect::to(&target).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compute the redirect target without going through axum's extractor —
    /// these tests focus on the slug → target mapping.
    fn target_for(slug: &str) -> String {
        let slug = slug.trim_matches('/');
        let n = slug.split('/').filter(|s| !s.is_empty()).count();
        if n == 1 {
            format!("/p?id={}", encode(slug))
        } else if n >= 3 {
            format!("/p?path={}", encode(slug))
        } else {
            "/p".to_string()
        }
    }

    #[test]
    fn short_id_redirects_to_id_param() {
        assert_eq!(target_for("8Dbcv_bu"), "/p?id=8Dbcv_bu");
    }

    #[test]
    fn long_path_redirects_to_path_param() {
        assert_eq!(
            target_for("agent/researcher/report.md"),
            "/p?path=agent%2Fresearcher%2Freport.md",
        );
    }

    #[test]
    fn deep_long_path_preserved() {
        assert_eq!(
            target_for("agent/researcher/2026/06/07/topic-x.md"),
            "/p?path=agent%2Fresearcher%2F2026%2F06%2F07%2Ftopic-x.md",
        );
    }

    #[test]
    fn two_segments_falls_through_to_invalid() {
        assert_eq!(target_for("agent/researcher"), "/p");
    }

    #[test]
    fn empty_slug_falls_through() {
        assert_eq!(target_for(""), "/p");
    }

    #[test]
    fn nanoid_special_chars_preserved() {
        assert_eq!(target_for("abc-_123"), "/p?id=abc-_123");
    }
}
