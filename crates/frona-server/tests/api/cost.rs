//! Authorization on the admin cost surface.
//!
//! These routes read every user's spend, so the thing worth pinning is who is
//! turned away: `view_usage_analytics` is granted to `admins` by the default
//! policy, and an ordinary member must not reach any of it.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

async fn setup() -> (AppState, tempfile::TempDir, String, String) {
    let (state, tmp) = test_app_state().await;
    state.user_group_service.seed_built_in().await.unwrap();
    // The first registered user is promoted to `admins` by
    // `ensure_admin_invariant` during registration.
    let (admin_token, _) =
        register_user(&state, "costadmin", "costadmin@example.com", "password123").await;
    let (member_token, _) = register_user(
        &state,
        "costmember",
        "costmember@example.com",
        "password123",
    )
    .await;
    (state, tmp, admin_token, member_token)
}

fn auth_post_empty(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap()
}

#[tokio::test]
async fn admin_reads_instance_wide_usage() {
    let (state, _tmp, admin_token, _) = setup().await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/admin/usage", &admin_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    // The split that gives the whole feature its point must be present even on
    // an instance with no traffic yet.
    assert!(json.get("metered_cost_usd").is_some(), "{json}");
    assert!(json.get("subscription_cost_usd").is_some(), "{json}");
    assert!(json.get("subscription_list_value_usd").is_some(), "{json}");
    assert_eq!(json["totals"]["calls"], 0);
}

#[tokio::test]
async fn a_member_cannot_read_anyone_elses_spend() {
    let (state, _tmp, _, member_token) = setup().await;

    for uri in [
        "/api/admin/usage",
        "/api/admin/cost-reports",
        "/api/admin/cost-reports/some-id",
    ] {
        let app = build_app(state.clone());
        let resp = app.oneshot(auth_get(uri, &member_token)).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "GET {uri} should be forbidden for a non-admin"
        );
    }

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_empty(
            "/api/admin/cost-reports/run",
            &member_token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn cost_routes_reject_no_auth() {
    let (state, _tmp, _, _) = setup().await;

    for uri in ["/api/admin/usage", "/api/admin/cost-reports"] {
        let app = build_app(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "GET {uri} should return 401 without auth"
        );
    }
}

#[tokio::test]
async fn reports_list_is_empty_before_any_run() {
    let (state, _tmp, admin_token, _) = setup().await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/admin/cost-reports", &admin_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_missing_report_is_not_found_rather_than_a_server_error() {
    let (state, _tmp, admin_token, _) = setup().await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/admin/cost-reports/nope", &admin_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// The admin-only built-in must exist for an admin and for nobody else, so the
/// ad-hoc run has an agent to hand the task to — and a member does not get an
/// agent they could never use.
#[tokio::test]
async fn the_cost_analyst_is_provisioned_for_admins_only() {
    let (state, _tmp, _, _) = setup().await;

    let admin = state
        .user_service
        .find_by_handle(&frona::core::Handle::try_new("costadmin".to_string()).unwrap())
        .await
        .unwrap()
        .expect("admin exists");
    let member = state
        .user_service
        .find_by_handle(&frona::core::Handle::try_new("costmember".to_string()).unwrap())
        .await
        .unwrap()
        .expect("member exists");

    assert!(
        state
            .agent_service
            .find_by_handle(&admin.id, frona::agent::models::COST_ANALYST_AGENT_HANDLE)
            .await
            .unwrap()
            .is_some(),
        "an admin should have the cost analyst"
    );
    assert!(
        state
            .agent_service
            .find_by_handle(&member.id, frona::agent::models::COST_ANALYST_AGENT_HANDLE)
            .await
            .unwrap()
            .is_none(),
        "a non-admin must not be given an agent whose tools would always refuse"
    );

    // Ordinary built-ins are still cloned for everyone.
    assert!(
        state
            .agent_service
            .find_by_handle(&member.id, frona::agent::models::SYSTEM_AGENT_HANDLE)
            .await
            .unwrap()
            .is_some(),
        "the group gate must not affect unrestricted built-ins"
    );
}
