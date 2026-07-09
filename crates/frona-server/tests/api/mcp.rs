use axum::http::StatusCode;
use tower::ServiceExt;

use super::*;

fn sample_manifest() -> serde_json::Value {
    serde_json::json!({
        "name": "io.test/echo-mcp",
        "description": "A test MCP server",
        "version": "1.0.0",
        "packages": [{
            "registry_type": "npm",
            "identifier": "@test/echo-mcp",
            "version": "1.0.0",
            "transport": { "type": "stdio" },
            "environment_variables": []
        }]
    })
}

fn sample_manifest_with_secret() -> serde_json::Value {
    serde_json::json!({
        "name": "io.test/secret-mcp",
        "description": "Needs a secret",
        "version": "1.0.0",
        "packages": [{
            "registry_type": "npm",
            "identifier": "@test/secret-mcp",
            "version": "1.0.0",
            "transport": { "type": "stdio" },
            "environment_variables": [{
                "name": "API_KEY",
                "is_required": true,
                "is_secret": true
            }]
        }]
    })
}

async fn install_via_manifest(
    state: &AppState,
    token: &str,
    manifest: serde_json::Value,
) -> serde_json::Value {
    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/mcp/servers",
            token,
            serde_json::json!({
                "manifest": manifest,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "install should succeed"
    );
    body_json(resp).await
}

#[tokio::test]
async fn list_servers_empty() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mcp-list", "mcplist@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/mcp/servers", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_servers_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/api/mcp/servers")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn install_server_with_manifest_no_secrets() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mcp-install", "mcpinstall@example.com", "password123").await;

    let server = install_via_manifest(&state, &token, sample_manifest()).await;
    assert!(server["id"].is_string());
    assert_eq!(server["display_name"], "echo-mcp");
    assert_eq!(server["status"], "installed");
    assert_eq!(server["command"], "npx");

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/mcp/servers", &token))
        .await
        .unwrap();
    let list = body_json(resp).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn install_server_allows_missing_secret_binding() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mcp-nobind", "mcpnobind@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/mcp/servers",
            &token,
            serde_json::json!({
                "manifest": sample_manifest_with_secret(),
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn uninstall_server() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mcp-uninstall", "mcpuninstall@example.com", "password123").await;

    let server = install_via_manifest(&state, &token, sample_manifest()).await;
    let id = server["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_delete(&format!("/api/mcp/servers/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/mcp/servers", &token))
        .await
        .unwrap();
    let list = body_json(resp).await;
    assert_eq!(list.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn uninstall_nonexistent_returns_404() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mcp-del404", "mcpdel404@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_delete("/api/mcp/servers/nonexistent-id", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_server_extra_env() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mcp-update", "mcpupdate@example.com", "password123").await;

    let server = install_via_manifest(&state, &token, sample_manifest()).await;
    let id = server["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_patch_json(
            &format!("/api/mcp/servers/{id}"),
            &token,
            serde_json::json!({
                "extra_env": { "LOG_LEVEL": "debug" }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["restart_required"], false);
}

#[tokio::test]
async fn update_wrong_owner_returns_403() {
    let (state, _tmp) = test_app_state().await;
    let (owner_token, _) =
        register_user(&state, "mcp-owner", "mcpowner@example.com", "password123").await;
    let (attacker_token, _) =
        register_user(&state, "mcp-attacker", "mcpattacker@example.com", "password123").await;

    let server = install_via_manifest(&state, &owner_token, sample_manifest()).await;
    let id = server["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_patch_json(
            &format!("/api/mcp/servers/{id}"),
            &attacker_token,
            serde_json::json!({ "extra_env": {} }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn stop_installed_server_returns_ok() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mcp-stop", "mcpstop@example.com", "password123").await;

    let server = install_via_manifest(&state, &token, sample_manifest()).await;
    let id = server["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            &format!("/api/mcp/servers/{id}/stop"),
            &token,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
