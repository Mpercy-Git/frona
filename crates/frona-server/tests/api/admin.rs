use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

/// Bootstrap a fresh state with the built-in `admins` group seeded and one admin user.
async fn setup_admin() -> (AppState, tempfile::TempDir, String, String) {
    let (state, tmp) = test_app_state().await;
    state.user_group_service.seed_built_in().await.unwrap();
    let (token, user_id) =
        register_user(&state, "rootadmin", "root@example.com", "password123").await;
    // The first registered user is promoted to admin via ensure_admin_invariant.
    (state, tmp, token, user_id)
}

/// Register a second (non-admin) user, returning (token, user_id).
async fn register_member(state: &AppState, name: &str) -> (String, String) {
    register_user(state, name, &format!("{name}@example.com"), "password123").await
}

fn auth_post_json(uri: &str, token: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn auth_patch_json(uri: &str, token: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("PATCH")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn auth_delete(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn non_admin_list_users_returns_403() {
    let (state, _tmp, _admin_token, _) = setup_admin().await;
    let (member_token, _) = register_member(&state, "alice").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/admin/users", &member_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_list_users_returns_200() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    register_member(&state, "alice").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/admin/users", &admin_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let arr = json.as_array().unwrap();
    assert!(arr.len() >= 2);
}

#[tokio::test]
async fn admin_can_create_user_returns_201() {
    let (state, _tmp, admin_token, _) = setup_admin().await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/admin/users",
            &admin_token,
            serde_json::json!({
                "handle": "newby",
                "email": "newby@example.com",
                "name": "Newby",
                "password": "password123"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert_eq!(json["handle"], "newby");
    assert_eq!(json["groups"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn non_admin_create_user_returns_403() {
    let (state, _tmp, _admin_token, _) = setup_admin().await;
    let (member_token, _) = register_member(&state, "bob").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/admin/users",
            &member_token,
            serde_json::json!({
                "handle": "denied",
                "email": "denied@example.com",
                "name": "Denied",
                "password": "password123"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_create_user_with_admins_group_succeeds() {
    let (state, _tmp, admin_token, _) = setup_admin().await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/admin/users",
            &admin_token,
            serde_json::json!({
                "handle": "coadmin",
                "email": "coadmin@example.com",
                "name": "Co Admin",
                "password": "password123",
                "groups": ["admins"]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert!(
        json["groups"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g == "admins")
    );
}

#[tokio::test]
async fn create_user_with_unknown_group_returns_400() {
    let (state, _tmp, admin_token, _) = setup_admin().await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/admin/users",
            &admin_token,
            serde_json::json!({
                "handle": "ghost",
                "email": "ghost@example.com",
                "name": "Ghost",
                "password": "password123",
                "groups": ["nope-not-real"]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_can_promote_member_to_admin() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (_, member_id) = register_member(&state, "alice").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_patch_json(
            &format!("/api/admin/users/{member_id}"),
            &admin_token,
            serde_json::json!({"groups": ["admins"]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let updated = state
        .user_service
        .find_by_id(&member_id)
        .await
        .unwrap()
        .unwrap();
    assert!(updated.groups.iter().any(|g| g == "admins"));
}

#[tokio::test]
async fn cannot_demote_last_admin() {
    let (state, _tmp, admin_token, admin_id) = setup_admin().await;
    register_member(&state, "alice").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_patch_json(
            &format!("/api/admin/users/{admin_id}"),
            &admin_token,
            serde_json::json!({"groups": []}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap_or("").contains("last_admin"));
}

#[tokio::test]
async fn non_admin_cannot_self_promote_to_admin() {
    // Regression for the privilege-escalation footgun.
    let (state, _tmp, _admin_token, _) = setup_admin().await;
    let (member_token, member_id) = register_member(&state, "alice").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_patch_json(
            &format!("/api/admin/users/{member_id}"),
            &member_token,
            serde_json::json!({"groups": ["admins"]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn patch_user_with_unknown_group_returns_400() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (_, member_id) = register_member(&state, "alice").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_patch_json(
            &format!("/api/admin/users/{member_id}"),
            &admin_token,
            serde_json::json!({"groups": ["admins-typo"]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_can_deactivate_member() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (_, member_id) = register_member(&state, "alice").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/admin/users/{member_id}/deactivate"),
            &admin_token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let target = state
        .user_service
        .find_by_id(&member_id)
        .await
        .unwrap()
        .unwrap();
    assert!(target.deactivated_at.is_some());
}

#[tokio::test]
async fn cannot_deactivate_last_admin() {
    let (state, _tmp, admin_token, admin_id) = setup_admin().await;
    register_member(&state, "alice").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/admin/users/{admin_id}/deactivate"),
            &admin_token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap_or("").contains("last_admin"));
}

#[tokio::test]
async fn admin_can_reactivate_user() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (_, member_id) = register_member(&state, "alice").await;
    state.user_service.deactivate(&member_id).await.unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/admin/users/{member_id}/reactivate"),
            &admin_token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let target = state
        .user_service
        .find_by_id(&member_id)
        .await
        .unwrap()
        .unwrap();
    assert!(target.deactivated_at.is_none());
}

#[tokio::test]
async fn cannot_delete_last_admin() {
    let (state, _tmp, admin_token, admin_id) = setup_admin().await;
    register_member(&state, "alice").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_delete(
            &format!("/api/admin/users/{admin_id}"),
            &admin_token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap_or("").contains("last_admin"));
}

#[tokio::test]
async fn delete_user_cascades_owned_chats_and_agents() {
    use chrono::Utc;
    use frona::chat::models::Chat;
    use frona::core::repository::{Repository, new_id};
    use frona::db::repo::generic::SurrealRepo;

    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (_, member_id) = register_member(&state, "alice").await;

    // Seed a chat directly through the repo so we don't have to drive the
    // whole chat creation flow. Registration already auto-clones the built-in
    // agents, so the member starts with 4 agent rows + 1 chat row.
    let chat_repo: SurrealRepo<Chat> = SurrealRepo::new(state.db.clone());
    let chat = Chat {
        id: new_id(),
        user_id: member_id.clone(),
        space_id: None,
        task_id: None,
        agent_id: "agent-x".into(),
        channel_id: None,
        channel_external_id: None,
        title: Some("test".into()),
        archived_at: None,
        metadata: Default::default(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    chat_repo.create(&chat).await.unwrap();

    assert!(
        !state
            .agent_service
            .list(&member_id)
            .await
            .unwrap()
            .is_empty()
    );

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_delete(
            &format!("/api/admin/users/{member_id}"),
            &admin_token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    assert!(
        state
            .user_service
            .find_by_id(&member_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .agent_service
            .list(&member_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(chat_repo.find_by_id(&chat.id).await.unwrap().is_none());
}

#[tokio::test]
async fn list_groups_includes_seeded_admins() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/admin/groups", &admin_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let groups = json.as_array().unwrap();
    let admins = groups
        .iter()
        .find(|g| g["name"] == "admins")
        .expect("admins group present");
    assert_eq!(admins["built_in"], true);
}

fn auth_put_json(uri: &str, token: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn login_attempt(state: &AppState, identifier: &str, password: &str, from_ip: u8) -> StatusCode {
    let app = build_app(state.clone());
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "identifier": identifier, "password": password }).to_string(),
        ))
        .unwrap();
    with_connect_info_from(&mut req, from_ip);
    app.oneshot(req).await.unwrap().status()
}

#[tokio::test]
async fn admin_can_reset_a_user_password() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (_, member_id) = register_member(&state, "forgot").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/admin/users/{member_id}/password"),
            &admin_token,
            serde_json::json!({ "password": "issuedbyadmin" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        login_attempt(&state, "forgot@example.com", "password123", 1).await,
        StatusCode::UNAUTHORIZED,
        "the old password must stop working"
    );
    assert_eq!(
        login_attempt(&state, "forgot@example.com", "issuedbyadmin", 2).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn admin_password_reset_revokes_target_sessions() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (member_token, member_id) = register_member(&state, "kicked").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/admin/users/{member_id}/password"),
            &admin_token,
            serde_json::json!({ "password": "issuedbyadmin" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header("authorization", format!("Bearer {member_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_password_reset_clears_a_lockout() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (_, member_id) = register_member(&state, "stuck").await;

    for i in 0..5 {
        login_attempt(&state, "stuck@example.com", "wrong", i + 1).await;
    }
    assert_eq!(
        login_attempt(&state, "stuck@example.com", "password123", 6).await,
        StatusCode::TOO_MANY_REQUESTS
    );

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/admin/users/{member_id}/password"),
            &admin_token,
            serde_json::json!({ "password": "issuedbyadmin" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        login_attempt(&state, "stuck@example.com", "issuedbyadmin", 7).await,
        StatusCode::OK,
        "resetting the password must also lift the lockout"
    );
}

#[tokio::test]
async fn admin_can_unlock_without_touching_the_password() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (_, member_id) = register_member(&state, "locked").await;

    for i in 0..5 {
        login_attempt(&state, "locked@example.com", "wrong", i + 1).await;
    }
    assert_eq!(
        login_attempt(&state, "locked@example.com", "password123", 6).await,
        StatusCode::TOO_MANY_REQUESTS
    );

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/admin/users/{member_id}/unlock"),
            &admin_token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        login_attempt(&state, "locked@example.com", "password123", 7).await,
        StatusCode::OK,
        "the original password must still be the right one"
    );
}

#[tokio::test]
async fn unlock_by_handle_clears_the_handle_bucket() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (_, member_id) = register_member(&state, "handly").await;

    // Lock via the handle rather than the email address.
    for i in 0..5 {
        login_attempt(&state, "handly", "wrong", i + 1).await;
    }
    assert_eq!(
        login_attempt(&state, "handly", "password123", 6).await,
        StatusCode::TOO_MANY_REQUESTS
    );

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/admin/users/{member_id}/unlock"),
            &admin_token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        login_attempt(&state, "handly", "password123", 7).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn non_admin_cannot_reset_another_users_password() {
    let (state, _tmp, _admin_token, _) = setup_admin().await;
    let (member_token, _) = register_member(&state, "meddler").await;
    let (_, victim_id) = register_member(&state, "victim").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/admin/users/{victim_id}/password"),
            &member_token,
            serde_json::json!({ "password": "takenover" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    assert_eq!(
        login_attempt(&state, "victim@example.com", "password123", 1).await,
        StatusCode::OK,
        "the victim's password must be untouched"
    );
}

#[tokio::test]
async fn non_admin_cannot_unlock_another_user() {
    let (state, _tmp, _admin_token, _) = setup_admin().await;
    let (member_token, _) = register_member(&state, "nosy").await;
    let (_, other_id) = register_member(&state, "other").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/admin/users/{other_id}/unlock"),
            &member_token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_password_reset_rejects_a_short_password() {
    let (state, _tmp, admin_token, _) = setup_admin().await;
    let (_, member_id) = register_member(&state, "tooshort").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/admin/users/{member_id}/password"),
            &admin_token,
            serde_json::json!({ "password": "abc" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
