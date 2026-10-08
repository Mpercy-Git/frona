use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn list_apps_empty() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "apps-empty", "appsempty@example.com", "password123").await;

    let app = build_app(state);
    let resp = app.oneshot(auth_get("/api/apps", &token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_apps_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/apps")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn get_app_not_found() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(
        &state,
        "apps-notfound",
        "appsnotfound@example.com",
        "password123",
    )
    .await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/apps/nonexistent-id", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_app_not_found() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "apps-del", "appsdel@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_delete("/api/apps/nonexistent-id", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ---- app isolation (separate apps origin) ----

const MAIN_HOST: &str = "frona.example.test";
const APPS_HOST: &str = "apps.example.test";

fn with_apps_origin(mut state: AppState) -> AppState {
    let mut config = (*state.config).clone();
    config.server.base_url = Some(format!("https://{MAIN_HOST}"));
    config.server.apps_url = Some(format!("https://{APPS_HOST}"));
    state.config = std::sync::Arc::new(config);
    state
}

fn guarded_app(state: &AppState) -> axum::Router {
    build_app(state.clone()).layer(axum::middleware::from_fn_with_state(
        state.clone(),
        routes::apps::apps_origin_guard,
    ))
}

fn host_get(host: &str, uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("GET")
        .uri(uri)
        .header("host", host);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    builder.body(Body::empty()).unwrap()
}

fn location(resp: &axum::http::Response<Body>) -> String {
    resp.headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

fn set_cookie_value(resp: &axum::http::Response<Body>, name: &str) -> Option<String> {
    resp.headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{name}=")))
        .map(|v| v.split(';').next().unwrap().to_string())
}

/// Registers a user and returns `(refresh cookie, user id)`.
async fn register_for_refresh_cookie(state: &AppState, handle: &str) -> (String, String) {
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "handle": handle,
                "email": format!("{handle}@example.com"),
                "name": handle,
                "password": "password123",
            })
            .to_string(),
        ))
        .unwrap();
    with_connect_info(&mut req);
    let resp = build_app(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let cookie = set_cookie_value(&resp, "refresh_token").expect("refresh cookie");
    let json = body_json(resp).await;
    (cookie, json["user"]["id"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn app_credentials_are_not_account_api_tokens() {
    let (state, _tmp) = test_app_state().await;
    let (_cookie, user_id) = register_for_refresh_cookie(&state, "appcred").await;
    let user = state
        .user_service
        .find_by_id(&user_id)
        .await
        .unwrap()
        .unwrap();

    let session = state
        .token_service
        .create_app_session(&state.keypair_service, &user)
        .await
        .unwrap();
    let code = state
        .token_service
        .create_app_gate_code(&state.keypair_service, &user)
        .await
        .unwrap();

    for token in [&session, &code] {
        let resp = build_app(state.clone())
            .oneshot(auth_get("/api/auth/me", token))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
    // And each is only good for its own purpose.
    assert!(
        state
            .token_service
            .validate_app_session(&state.keypair_service, &code)
            .await
            .is_err()
    );
    assert!(
        state
            .token_service
            .validate_app_gate_code(&state.keypair_service, &session)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn gate_hands_a_code_to_the_apps_origin_and_callback_sets_the_session() {
    let (state, _tmp) = test_app_state().await;
    let state = with_apps_origin(state);
    let (refresh, _) = register_for_refresh_cookie(&state, "gateuser").await;

    // Main origin: no cookie is set, a code goes to the apps origin.
    let resp = guarded_app(&state)
        .oneshot(host_get(
            MAIN_HOST,
            "/api/auth/apps?redirect=%2Fapps%2Fdemo%2F%3Fa%3D1%26b%3D2",
            Some(&refresh),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    assert!(resp.headers().get("set-cookie").is_none());
    let to = location(&resp);
    assert!(
        to.starts_with(&format!("https://{APPS_HOST}/api/auth/apps/callback?code=")),
        "{to}"
    );

    // The callback only exists on the apps origin.
    let path = to.trim_start_matches(&format!("https://{APPS_HOST}"));
    let resp = guarded_app(&state)
        .oneshot(host_get(MAIN_HOST, path, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = guarded_app(&state)
        .oneshot(host_get(APPS_HOST, path, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(location(&resp), "/apps/demo/?a=1&b=2");
    let session = set_cookie_value(&resp, "app_session").expect("app_session cookie");

    // With the session the proxy lets the request through to the (absent) app;
    // without it the user is sent back to the main origin's gate.
    let resp = guarded_app(&state)
        .oneshot(host_get(APPS_HOST, "/apps/demo/", Some(&session)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = guarded_app(&state)
        .oneshot(host_get(APPS_HOST, "/apps/demo/", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    assert!(
        location(&resp).starts_with(&format!("https://{MAIN_HOST}/api/auth/apps?redirect=")),
        "{}",
        location(&resp)
    );
}

#[tokio::test]
async fn callback_rejects_a_missing_or_wrong_code() {
    let (state, _tmp) = test_app_state().await;
    let state = with_apps_origin(state);
    let (refresh, user_id) = register_for_refresh_cookie(&state, "badcode").await;
    let user = state
        .user_service
        .find_by_id(&user_id)
        .await
        .unwrap()
        .unwrap();
    let access = state
        .token_service
        .create_app_session(&state.keypair_service, &user)
        .await
        .unwrap();
    let _ = refresh;

    for uri in [
        "/api/auth/apps/callback".to_string(),
        "/api/auth/apps/callback?code=nope&redirect=%2Fapps%2Fx%2F".to_string(),
        // A session is not a code.
        format!("/api/auth/apps/callback?code={access}&redirect=%2Fapps%2Fx%2F"),
    ] {
        let resp = guarded_app(&state)
            .oneshot(host_get(APPS_HOST, &uri, None))
            .await
            .unwrap();
        assert!(
            resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::FORBIDDEN,
            "{uri} -> {}",
            resp.status()
        );
        assert!(resp.headers().get("set-cookie").is_none(), "{uri}");
    }
}

#[tokio::test]
async fn apps_origin_serves_only_apps_and_main_origin_redirects_to_it() {
    let (state, _tmp) = test_app_state().await;
    let state = with_apps_origin(state);

    for path in [
        "/api/auth/me",
        "/api/auth/refresh",
        "/api/apps",
        "/",
        "/login",
    ] {
        let resp = guarded_app(&state)
            .oneshot(host_get(APPS_HOST, path, None))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "apps origin served {path}"
        );
    }

    let resp = guarded_app(&state)
        .oneshot(host_get(MAIN_HOST, "/apps/demo/page?x=1", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        location(&resp),
        format!("https://{APPS_HOST}/apps/demo/page?x=1")
    );

    let resp = guarded_app(&state)
        .oneshot(host_get(MAIN_HOST, "/api/apps", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gate_refuses_open_redirects() {
    let (state, _tmp) = test_app_state().await;
    let separate = with_apps_origin(state.clone());
    let (refresh, _) = register_for_refresh_cookie(&separate, "openredir").await;

    for redirect in [
        "%2F%2Fevil.example",
        "https%3A%2F%2Fevil.example",
        "%2F%5Cevil.example",
        "%2Fother",
    ] {
        let resp = guarded_app(&separate)
            .oneshot(host_get(
                MAIN_HOST,
                &format!("/api/auth/apps?redirect={redirect}"),
                Some(&refresh),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{redirect}");
        assert!(resp.headers().get("location").is_none(), "{redirect}");
    }
}

#[tokio::test]
async fn refresh_is_refused_from_an_app_page() {
    let (state, _tmp) = test_app_state().await;
    let (refresh, _) = register_for_refresh_cookie(&state, "refreshref").await;

    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/refresh")
        .header("cookie", &refresh)
        .header("referer", "https://frona.example.test/apps/demo/index.html")
        .body(Body::empty())
        .unwrap();
    with_connect_info(&mut req);
    let resp = build_app(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // The frontend itself still refreshes.
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/refresh")
        .header("cookie", &refresh)
        .header("referer", "https://frona.example.test/chat")
        .body(Body::empty())
        .unwrap();
    with_connect_info(&mut req);
    let resp = build_app(state).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
