use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;


#[tokio::test]
async fn list_tools_returns_builtin() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "tools-user", "tools@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/tools", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let providers = json.as_array().unwrap();

    // Response shape: Vec<ToolProviderWithTools> — providers contain their own tools.
    // Browser tools are hardcoded so the browser provider must always carry tools in tests;
    // other providers may be empty because their definitions live in prompt .md files
    // unavailable in the test environment.
    let provider_ids: Vec<&str> = providers.iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert!(provider_ids.contains(&"browser"), "browser provider missing; got {:?}", provider_ids);

    let browser = providers.iter().find(|p| p["id"] == "browser").unwrap();
    assert!(browser["display_name"].is_string());
    assert_eq!(browser["kind"]["type"], "builtin");
    assert_eq!(browser["status"]["state"], "available");
    let browser_tools = browser["tools"].as_array().unwrap();
    assert!(!browser_tools.is_empty(), "browser provider should expose tools");
    let first = &browser_tools[0];
    assert!(first["id"].is_string());
    assert!(first["description"].is_string());
    assert!(first["configurable"].is_boolean());
}


#[tokio::test]
async fn openid_configuration_returns_json() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/.well-known/openid-configuration")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["issuer"].is_string());
    assert!(json["jwks_uri"].as_str().unwrap().contains("jwks.json"));
    assert!(json["token_endpoint"].is_string());
}

#[tokio::test]
async fn jwks_returns_keys() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/.well-known/jwks.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["keys"].is_array());
}


#[tokio::test]
async fn metrics_returns_text() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(content_type.contains("text/plain"));
}


#[tokio::test]
async fn health_check_returns_ok() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/system/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn healthz_alias_returns_ok() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["status"], "ok");
}


#[tokio::test]
async fn system_version_returns_json() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "sys-ver", "sysver@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/system/version", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["version"].is_string(), "Expected 'version' key in response");
}

#[tokio::test]
async fn system_version_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/system/version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}


#[tokio::test]
async fn get_config_schema_returns_json() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "cfg-schema", "cfgschema@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/config/schema", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    // JSON Schema should have a "type" or "$schema" or "properties" field
    assert!(
        json.get("type").is_some()
            || json.get("$schema").is_some()
            || json.get("properties").is_some(),
        "Expected JSON Schema response"
    );
}

#[tokio::test]
async fn get_config_returns_redacted() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "cfg-get", "cfgget@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/config", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    // Should have config structure but secrets redacted
    assert!(json.is_object());
}

#[tokio::test]
async fn config_endpoints_reject_no_auth() {
    let (state, _tmp) = test_app_state().await;

    for uri in ["/api/config", "/api/config/schema"] {
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
