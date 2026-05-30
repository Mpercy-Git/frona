use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

async fn create_contact(
    state: &AppState,
    token: &str,
    name: &str,
) -> serde_json::Value {
    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/contacts",
            token,
            serde_json::json!({
                "name": name,
                "phone": "+1234567890",
                "email": "contact@example.com",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "create_contact({name}) failed");
    body_json(resp).await
}

#[tokio::test]
async fn create_contact_returns_json() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "ct-user", "ctuser@example.com", "password123").await;

    let contact = create_contact(&state, &token, "Alice").await;
    assert!(contact["id"].is_string());
    assert_eq!(contact["name"], "Alice");
    assert_eq!(contact["phone"], "+1234567890");
    assert_eq!(contact["email"], "contact@example.com");
}

#[tokio::test]
async fn contact_metadata_and_space_id_round_trip() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "ct-md", "ctmd@example.com", "password123").await;
    let space = create_space(&state, &token, "ChannelSpace").await;
    let space_id = space["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/contacts",
            &token,
            serde_json::json!({
                "name": "Bob",
                "space_id": space_id,
                "metadata": {"channel:external_user_id": "tg:42"},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["space_id"], space_id);
    assert_eq!(json["metadata"]["channel:external_user_id"], "tg:42");
    let id = json["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/contacts/{id}"),
            &token,
            serde_json::json!({
                "metadata": {"channel:external_user_id": "tg:99", "extra": null},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["metadata"]["channel:external_user_id"], "tg:99");
    assert!(json["metadata"].get("extra").is_none());
}

#[tokio::test]
async fn create_contact_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/contacts")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name": "X"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_contacts_returns_only_own() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "ct-a", "cta@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "ct-b", "ctb@example.com", "password123").await;

    create_contact(&state, &token_a, "Alice").await;
    create_contact(&state, &token_b, "Bob").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/contacts", &token_a))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let contacts = json.as_array().unwrap();
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0]["name"], "Alice");
}

#[tokio::test]
async fn update_contact() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "ct-update", "ctupdate@example.com", "password123").await;
    let contact = create_contact(&state, &token, "Before").await;
    let id = contact["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_put_json(
            &format!("/api/contacts/{id}"),
            &token,
            serde_json::json!({"name": "After", "company": "Acme"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["name"], "After");
    assert_eq!(json["company"], "Acme");
}

#[tokio::test]
async fn delete_contact() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "ct-delete", "ctdelete@example.com", "password123").await;
    let contact = create_contact(&state, &token, "Gone").await;
    let id = contact["id"].as_str().unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_delete(&format!("/api/contacts/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/contacts", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 0);
}
