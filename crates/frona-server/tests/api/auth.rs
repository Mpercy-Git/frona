use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn register_returns_201_with_token() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handle": "alice",
                "email": "alice@example.com",
                "name": "Alice",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let cookie = resp
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cookie.contains("refresh_token="));

    let json = body_json(resp).await;
    assert!(json["token"].is_string());
    assert!(json["user"]["id"].is_string());
    assert_eq!(json["user"]["handle"], "alice");
}

#[tokio::test]
async fn register_duplicate_email_returns_400() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "user1", "dup@example.com", "password123").await;

    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handle": "user2",
                "email": "dup@example.com",
                "name": "User2",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn register_duplicate_handle_returns_400() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "dupuser", "first@example.com", "password123").await;

    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handle": "dupuser",
                "email": "second@example.com",
                "name": "Dup",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn register_short_password_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handle": "shortpw",
                "email": "short@example.com",
                "name": "Short",
                "password": "abc",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn login_returns_token() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "loginuser", "login@example.com", "password123").await;

    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "identifier": "login@example.com",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["token"].is_string());
    assert_eq!(json["user"]["handle"], "loginuser");
}

#[tokio::test]
async fn login_by_handle() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "namelogin", "namelogin@example.com", "password123").await;

    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "identifier": "namelogin",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["user"]["handle"], "namelogin");
}

#[tokio::test]
async fn login_wrong_password_returns_401() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "wrongpw", "wrong@example.com", "password123").await;

    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "identifier": "wrong@example.com",
                "password": "badpassword",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_returns_user_info() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "meuser", "me@example.com", "password123").await;

    let app = build_app(state);
    let resp = app.oneshot(auth_get("/api/auth/me", &token)).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["handle"], "meuser");
    assert_eq!(json["email"], "me@example.com");
}

#[tokio::test]
async fn me_without_token_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_with_invalid_token_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/auth/me", "not.a.real.token"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_returns_needs_setup_false_after_setup_completed() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "setupuser", "setup@example.com", "password123").await;

    state
        .set_runtime_config("setup_completed", "true")
        .await
        .unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/auth/me", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.get("needs_setup").is_none());
}

#[tokio::test]
async fn invalid_authorization_format_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header("authorization", "NotBearer some-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_invalidates_session() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "logoutuser", "logout@example.com", "password123").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/auth/me", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_returns_new_token() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state.clone());
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handle": "refreshuser",
                "email": "refresh@example.com",
                "name": "Refresh",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    let cookie_header = resp
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let cookie_value = cookie_header.split(';').next().unwrap();

    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/refresh")
        .header("cookie", cookie_value)
        .body(Body::empty())
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let new_cookie = resp.headers().get("set-cookie").unwrap().to_str().unwrap();
    assert!(new_cookie.contains("refresh_token="));
    let json = body_json(resp).await;
    assert!(json["token"].is_string());
}

#[tokio::test]
async fn refresh_without_cookie_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/refresh")
        .body(Body::empty())
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}


#[tokio::test]
async fn change_handle_succeeds() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "oldname", "chname@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            "/api/auth/handle",
            &token,
            serde_json::json!({"handle": "newname"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["user"]["handle"], "newname");
    assert!(json["token"].is_string());
}

#[tokio::test]
async fn change_handle_duplicate_returns_400() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "taken-name", "taken@example.com", "password123").await;
    let (token, _) =
        register_user(&state, "changer", "changer@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            "/api/auth/handle",
            &token,
            serde_json::json!({"handle": "taken-name"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn change_handle_same_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "samename", "same@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            "/api/auth/handle",
            &token,
            serde_json::json!({"handle": "samename"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}


#[tokio::test]
async fn create_pat_returns_201() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "patuser", "patuser@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/auth/tokens",
            &token,
            serde_json::json!({"name": "my-token", "expires_in_days": 30}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert!(json["token"].is_string());
    assert_eq!(json["name"], "my-token");
    assert!(json["prefix"].is_string());
    assert!(json["expires_at"].is_string());
}

#[tokio::test]
async fn list_pats_returns_created_tokens() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "listpat", "listpat@example.com", "password123").await;

    for name in ["token-a", "token-b"] {
        let app = build_app(state.clone());
        let resp = app
            .oneshot(auth_post_json(
                "/api/auth/tokens",
                &token,
                serde_json::json!({"name": name}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/auth/tokens", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn delete_pat_returns_204() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "delpat", "delpat@example.com", "password123").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/auth/tokens",
            &token,
            serde_json::json!({"name": "to-delete"}),
        ))
        .await
        .unwrap();
    let pat = body_json(resp).await;
    let pat_id = pat["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_delete(&format!("/api/auth/tokens/{pat_id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/auth/tokens", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn pat_can_authenticate() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "patauth", "patauth@example.com", "password123").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/auth/tokens",
            &token,
            serde_json::json!({"name": "auth-pat"}),
        ))
        .await
        .unwrap();
    let pat = body_json(resp).await;
    let pat_token = pat["token"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/auth/me", pat_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["handle"], "patauth");
}

#[tokio::test]
async fn pat_cannot_create_another_pat() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "pat-no-create", "patnc@example.com", "password123").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/auth/tokens",
            &token,
            serde_json::json!({"name": "first-pat"}),
        ))
        .await
        .unwrap();
    let pat = body_json(resp).await;
    let pat_token = pat["token"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/auth/tokens",
            pat_token,
            serde_json::json!({"name": "second-pat"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn logout_with_pat_deletes_token() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "patlogout", "patlogout@example.com", "password123").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/auth/tokens",
            &token,
            serde_json::json!({"name": "logout-pat"}),
        ))
        .await
        .unwrap();
    let pat = body_json(resp).await;
    let pat_token = pat["token"].as_str().unwrap().to_string();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("authorization", format!("Bearer {pat_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/auth/me", &pat_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}


#[tokio::test]
async fn auth_config_returns_defaults() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["sso"]["enabled"], false);
    assert_eq!(json["sso"]["disable_local_auth"], false);
    assert_eq!(json["allow_registration"], true);
}

#[tokio::test]
async fn sso_authorize_without_provider_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/sso/authorize")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sso_callback_without_provider_redirects_with_error() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/sso/callback?code=test&state=test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(location.starts_with("/login?sso_error="));
}


fn build_disabled_registration_state(state: &AppState) -> AppState {
    let mut new_state = state.clone();
    let mut config = (*new_state.config).clone();
    config.auth.allow_registration = false;
    new_state.config = std::sync::Arc::new(config);
    new_state
}

#[tokio::test]
async fn register_returns_403_when_disabled() {
    let (state, _tmp) = test_app_state().await;
    // Seed a user so we're not in the "no users" startup-precondition case.
    register_user(&state, "first", "first@example.com", "password123").await;

    let disabled = build_disabled_registration_state(&state);
    let app = build_app(disabled);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handle": "newuser",
                "email": "new@example.com",
                "name": "New",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn register_returns_403_when_disabled_even_with_no_users() {
    let (state, _tmp) = test_app_state().await;
    let disabled = build_disabled_registration_state(&state);
    let app = build_app(disabled);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handle": "newuser",
                "email": "new@example.com",
                "name": "New",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    // No bootstrap escape — even with zero users, the route refuses.
    // (The startup precondition is what gates whether the server reaches this state at all.)
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}


fn build_sso_only_state(state: &AppState) -> AppState {
    let mut sso_state = state.clone();
    let mut config = (*sso_state.config).clone();
    config.sso.disable_local_auth = true;
    sso_state.config = std::sync::Arc::new(config);
    sso_state
}

#[tokio::test]
async fn register_with_sso_only_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let sso_state = build_sso_only_state(&state);
    let app = build_app(sso_state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handle": "ssouser",
                "email": "sso@example.com",
                "name": "SSO",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn login_with_sso_only_returns_400() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "ssologin", "ssologin@example.com", "password123").await;

    let sso_state = build_sso_only_state(&state);
    let app = build_app(sso_state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "identifier": "ssologin@example.com",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}


#[tokio::test]
async fn register_with_https_base_url_sets_secure_cookie() {
    let (state, _tmp) = test_app_state().await;
    let mut https_state = state.clone();
    let mut config = (*https_state.config).clone();
    config.server.base_url = Some("https://example.com".to_string());
    https_state.config = std::sync::Arc::new(config);

    let app = build_app(https_state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handle": "secureuser",
                "email": "secure@example.com",
                "name": "Secure",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let cookie = resp
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cookie.contains("Secure"));
}


#[tokio::test]
async fn first_password_register_becomes_admin() {
    let (state, _tmp) = test_app_state().await;
    // Seed the admins group (normally done at server startup).
    state.user_group_service.seed_built_in().await.unwrap();

    let (_, user_id) = register_user(&state, "firstadmin", "first@example.com", "password123").await;

    let user = state.user_service.find_by_id(&user_id).await.unwrap().unwrap();
    assert!(
        user.groups.iter().any(|g| g == "admins"),
        "first registered user should be promoted to admins, got groups: {:?}",
        user.groups
    );
}

#[tokio::test]
async fn second_register_is_not_admin() {
    let (state, _tmp) = test_app_state().await;
    state.user_group_service.seed_built_in().await.unwrap();

    register_user(&state, "firstadmin", "first@example.com", "password123").await;
    let (_, second_id) = register_user(&state, "second", "second@example.com", "password123").await;

    let user = state.user_service.find_by_id(&second_id).await.unwrap().unwrap();
    assert!(
        !user.groups.iter().any(|g| g == "admins"),
        "second registered user should not be admin, got groups: {:?}",
        user.groups
    );
}

#[tokio::test]
async fn ensure_admin_invariant_is_idempotent() {
    let (state, _tmp) = test_app_state().await;
    state.user_group_service.seed_built_in().await.unwrap();
    register_user(&state, "onlyone", "only@example.com", "password123").await;

    // Run twice in a row; second call should be a no-op (idempotent).
    state.user_service.ensure_admin_invariant().await.unwrap();
    state.user_service.ensure_admin_invariant().await.unwrap();

    let users = state.user_service.list_all(true).await.unwrap();
    let admin_count = users
        .iter()
        .filter(|u| u.deactivated_at.is_none() && u.groups.iter().any(|g| g == "admins"))
        .count();
    assert_eq!(admin_count, 1);
}

#[tokio::test]
async fn startup_promotes_oldest_active_user_when_no_admin() {
    use chrono::Utc;
    use frona::core::repository::new_id;
    use frona::auth::User as UserModel;

    let (state, _tmp) = test_app_state().await;
    state.user_group_service.seed_built_in().await.unwrap();

    // Insert two users via the repo (bypass the register flow so neither is auto-admin).
    let now = Utc::now();
    let older = UserModel {
        id: new_id(),
        handle: frona::handle!("older"),
        email: "older@example.com".into(),
        name: "Older".into(),
        password_hash: "x".into(),
        timezone: None,
        groups: Vec::new(),
        deactivated_at: None,
        phone: None,
        created_at: now,
        updated_at: now,
    };
    let newer = UserModel {
        id: new_id(),
        handle: frona::handle!("newer"),
        email: "newer@example.com".into(),
        name: "Newer".into(),
        password_hash: "x".into(),
        timezone: None,
        groups: Vec::new(),
        deactivated_at: None,
        phone: None,
        created_at: now + chrono::Duration::seconds(1),
        updated_at: now + chrono::Duration::seconds(1),
    };
    state.user_service.create(&older).await.unwrap();
    state.user_service.create(&newer).await.unwrap();

    // Pre-condition: no admin.
    let users_before = state.user_service.list_all(true).await.unwrap();
    assert!(
        users_before
            .iter()
            .all(|u| !u.groups.iter().any(|g| g == "admins")),
        "expected no admins before repair"
    );

    // Run the invariant repair (this is what main.rs does at boot).
    state.user_service.ensure_admin_invariant().await.unwrap();

    let promoted = state.user_service.find_by_id(&older.id).await.unwrap().unwrap();
    assert!(promoted.groups.iter().any(|g| g == "admins"));
    let untouched = state.user_service.find_by_id(&newer.id).await.unwrap().unwrap();
    assert!(!untouched.groups.iter().any(|g| g == "admins"));
}

#[tokio::test]
async fn login_refuses_deactivated_user() {
    let (state, _tmp) = test_app_state().await;
    state.user_group_service.seed_built_in().await.unwrap();
    // First user becomes admin via bootstrap; the DB events refuse deactivating the
    // last admin, so we need a second non-admin user to deactivate.
    register_user(&state, "firstadmin", "admin@example.com", "password123").await;
    let (_, user_id) =
        register_user(&state, "tobedisabled", "disabled@example.com", "password123").await;

    state.user_service.deactivate(&user_id).await.unwrap();

    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "identifier": "disabled@example.com",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap_or("").to_lowercase().contains("deactivated"));
}

#[tokio::test]
async fn refresh_refuses_deactivated_user() {
    let (state, _tmp) = test_app_state().await;
    state.user_group_service.seed_built_in().await.unwrap();
    // First user is auto-admin; we need a second non-admin to deactivate.
    register_user(&state, "firstadmin", "admin@example.com", "password123").await;
    let (_, user_id) = register_user(&state, "refreshuser", "refresh@example.com", "password123").await;

    // Capture the refresh cookie from registration.
    let app = build_app(state.clone());
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "identifier": "refresh@example.com",
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let login_resp = app.oneshot(req).await.unwrap();
    assert_eq!(login_resp.status(), StatusCode::OK);
    let cookie = login_resp.headers().get("set-cookie").unwrap().to_str().unwrap().to_string();
    let refresh_cookie = cookie.split(';').next().unwrap().to_string();

    // Deactivate the user, then try to refresh with the captured cookie.
    state.user_service.deactivate(&user_id).await.unwrap();

    let app = build_app(state);
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/refresh")
        .header("cookie", refresh_cookie)
        .body(Body::empty())
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// Login lockout
// ---------------------------------------------------------------------------

/// The tracker is built from config inside `AppState::new`, so swapping the
/// config alone wouldn't change it — the tracker has to be replaced too.
fn build_state_with_lockout(state: &AppState, max_attempts: u32, minutes: u64) -> AppState {
    let mut new_state = state.clone();
    let mut config = (*new_state.config).clone();
    config.auth.max_login_attempts = max_attempts;
    config.auth.lockout_minutes = minutes;
    new_state.config = std::sync::Arc::new(config);
    new_state.login_tracker =
        frona::auth::lockout::LoginAttemptTracker::new(max_attempts, minutes);
    new_state
}

/// One login attempt from a distinct source address, so the per-IP governor
/// never fires and the identifier lockout is what's actually under test.
async fn attempt_login(
    state: &AppState,
    identifier: &str,
    password: &str,
    from_ip: u8,
) -> axum::http::Response<Body> {
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
    app.oneshot(req).await.unwrap()
}

#[tokio::test]
async fn login_locks_out_after_max_attempts() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "locky", "locky@example.com", "password123").await;

    for i in 0..5 {
        let resp = attempt_login(&state, "locky@example.com", "wrong", i + 1).await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "attempt {i} should be a plain rejection"
        );
    }

    let resp = attempt_login(&state, "locky@example.com", "wrong", 6).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = resp
        .headers()
        .get("retry-after")
        .expect("Retry-After header")
        .to_str()
        .unwrap()
        .parse::<u64>()
        .expect("Retry-After is a number");
    assert!(retry_after > 0 && retry_after <= 15 * 60 + 1);
}

#[tokio::test]
async fn lockout_refuses_the_correct_password() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "stilllocked", "still@example.com", "password123").await;

    for i in 0..5 {
        attempt_login(&state, "still@example.com", "wrong", i + 1).await;
    }

    let resp = attempt_login(&state, "still@example.com", "password123", 6).await;
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "a lockout must hold even against the right password"
    );
}

#[tokio::test]
async fn successful_login_clears_the_failure_count() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "reset", "reset@example.com", "password123").await;

    for i in 0..4 {
        attempt_login(&state, "reset@example.com", "wrong", i + 1).await;
    }
    let resp = attempt_login(&state, "reset@example.com", "password123", 5).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Budget is back to full: four more failures still must not lock.
    for i in 0..4 {
        let resp = attempt_login(&state, "reset@example.com", "wrong", i + 6).await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "counter should have reset after the successful login"
        );
    }
}

#[tokio::test]
async fn lockout_ignores_identifier_casing_and_padding() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "casey", "casey@example.com", "password123").await;

    // Spread across spellings that all resolve to the same account.
    let spellings = [
        "casey@example.com",
        "Casey@Example.com",
        "  casey@example.com  ",
        "CASEY@EXAMPLE.COM",
        "cAsEy@ExAmPlE.cOm",
    ];
    for (i, spelling) in spellings.iter().enumerate() {
        let resp = attempt_login(&state, spelling, "wrong", i as u8 + 1).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    let resp = attempt_login(&state, "casey@example.com", "wrong", 6).await;
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "casing variants must share one budget, not get one each"
    );
}

#[tokio::test]
async fn deactivated_account_does_not_accrue_lockout() {
    let (state, _tmp) = test_app_state().await;
    state.user_group_service.seed_built_in().await.unwrap();
    register_user(&state, "keepadmin", "keepadmin@example.com", "password123").await;
    let (_, user_id) =
        register_user(&state, "gonner", "gonner@example.com", "password123").await;
    state.user_service.deactivate(&user_id).await.unwrap();

    // Six attempts with the right password: rejected for being deactivated,
    // never converted into a lockout.
    for i in 0..6 {
        let resp = attempt_login(&state, "gonner@example.com", "password123", i + 1).await;
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "deactivation is not a failed credential guess"
        );
    }
}

#[tokio::test]
async fn zero_max_attempts_disables_lockout() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "unlimited", "unlimited@example.com", "password123").await;
    let state = build_state_with_lockout(&state, 0, 15);

    for i in 0..8 {
        let resp = attempt_login(&state, "unlimited@example.com", "wrong", i + 1).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "attempt {i}");
    }
}

// ---------------------------------------------------------------------------
// Self-service password change
// ---------------------------------------------------------------------------

fn change_password_req(token: &str, current: &str, new: &str) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri("/api/auth/password")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "current_password": current, "new_password": new }).to_string(),
        ))
        .unwrap()
}

#[tokio::test]
async fn change_password_swaps_the_credential() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "swapper", "swap@example.com", "password123").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(change_password_req(&token, "password123", "newpassword456"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["token"].is_string(), "a fresh access token is returned");

    let resp = attempt_login(&state, "swap@example.com", "password123", 1).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "old password is dead");

    let resp = attempt_login(&state, "swap@example.com", "newpassword456", 2).await;
    assert_eq!(resp.status(), StatusCode::OK, "new password works");
}

#[tokio::test]
async fn change_password_requires_the_current_one() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "wrongcur", "wrongcur@example.com", "password123").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(change_password_req(&token, "notmypassword", "newpassword456"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let resp = attempt_login(&state, "wrongcur@example.com", "password123", 1).await;
    assert_eq!(resp.status(), StatusCode::OK, "password must be unchanged");
}

#[tokio::test]
async fn change_password_rejects_a_short_new_password() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "shorty", "shorty@example.com", "password123").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(change_password_req(&token, "password123", "short"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn change_password_revokes_other_sessions() {
    let (state, _tmp) = test_app_state().await;
    let (first_session, _) =
        register_user(&state, "multi", "multi@example.com", "password123").await;

    // A second device signs in, then the first device changes the password.
    let resp = attempt_login(&state, "multi@example.com", "password123", 1).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let second_session = body_json(resp).await["token"].as_str().unwrap().to_string();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(change_password_req(&first_session, "password123", "newpassword456"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header("authorization", format!("Bearer {second_session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "the other device's session must be gone"
    );
}

#[tokio::test]
async fn change_password_clears_an_existing_lockout() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "lockedout", "lockedout@example.com", "password123").await;

    for i in 0..5 {
        attempt_login(&state, "lockedout@example.com", "wrong", i + 1).await;
    }
    assert_eq!(
        attempt_login(&state, "lockedout@example.com", "password123", 6)
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );

    let app = build_app(state.clone());
    let resp = app
        .oneshot(change_password_req(&token, "password123", "newpassword456"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = attempt_login(&state, "lockedout@example.com", "newpassword456", 7).await;
    assert_eq!(resp.status(), StatusCode::OK, "lockout should be cleared");
}

// ---------------------------------------------------------------------------
// Emailed password reset
// ---------------------------------------------------------------------------

fn reset_password_req(token: &str, new_password: &str) -> Request<Body> {
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/reset-password")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "token": token, "new_password": new_password }).to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    req
}

#[tokio::test]
async fn reset_password_consumes_the_token_and_sets_the_password() {
    let (state, _tmp) = test_app_state().await;
    let (_, user_id) =
        register_user(&state, "forgetful", "forgetful@example.com", "password123").await;

    let secret = state.password_reset_service.issue(&user_id).await.unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(reset_password_req(&secret, "brandnewpass"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = attempt_login(&state, "forgetful@example.com", "brandnewpass", 1).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn reset_token_is_single_use() {
    let (state, _tmp) = test_app_state().await;
    let (_, user_id) = register_user(&state, "onceonly", "once@example.com", "password123").await;
    let secret = state.password_reset_service.issue(&user_id).await.unwrap();

    let app = build_app(state.clone());
    assert_eq!(
        app.oneshot(reset_password_req(&secret, "brandnewpass"))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );

    let app = build_app(state.clone());
    assert_eq!(
        app.oneshot(reset_password_req(&secret, "yetanotherpass"))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST,
        "a spent token must not work twice"
    );
}

#[tokio::test]
async fn reset_password_rejects_an_unknown_token() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "nobody", "nobody@example.com", "password123").await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(reset_password_req("not-a-real-token", "brandnewpass"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reset_password_keeps_the_token_when_the_new_password_is_rejected() {
    let (state, _tmp) = test_app_state().await;
    let (_, user_id) = register_user(&state, "typo", "typo@example.com", "password123").await;
    let secret = state.password_reset_service.issue(&user_id).await.unwrap();

    let app = build_app(state.clone());
    assert_eq!(
        app.oneshot(reset_password_req(&secret, "short"))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    // The user gets to try again with the same link rather than being sent back
    // to square one over a typo.
    let app = build_app(state.clone());
    assert_eq!(
        app.oneshot(reset_password_req(&secret, "properlength"))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn reset_password_revokes_existing_sessions() {
    let (state, _tmp) = test_app_state().await;
    let (session, user_id) =
        register_user(&state, "kickme", "kickme@example.com", "password123").await;
    let secret = state.password_reset_service.issue(&user_id).await.unwrap();

    let app = build_app(state.clone());
    assert_eq!(
        app.oneshot(reset_password_req(&secret, "brandnewpass"))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );

    let app = build_app(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header("authorization", format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn changing_the_password_invalidates_an_outstanding_reset_link() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "racer", "racer@example.com", "password123").await;

    // An attacker requests a reset; the real user then changes their password.
    let secret = state.password_reset_service.issue(&user_id).await.unwrap();
    let app = build_app(state.clone());
    assert_eq!(
        app.oneshot(change_password_req(&token, "password123", "chosenbyowner"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    let app = build_app(state.clone());
    assert_eq!(
        app.oneshot(reset_password_req(&secret, "attackerchosen"))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST,
        "the pre-existing reset link must be dead"
    );
}

#[tokio::test]
async fn forgot_password_is_refused_when_mail_is_unconfigured() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "nomail", "nomail@example.com", "password123").await;

    let app = build_app(state.clone());
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/forgot-password")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "email": "nomail@example.com" }).to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn auth_config_reports_password_reset_availability() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(
        json["password_reset_enabled"], false,
        "no SMTP configured in tests, so the flow must advertise as unavailable"
    );
}
