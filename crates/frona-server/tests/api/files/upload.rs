use axum::body::Body;
use axum::http::{Request, StatusCode};
use tokio::fs;
use tower::ServiceExt;

use super::super::*;


#[tokio::test]
async fn upload_file_returns_attachment() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "uploader", "uploader@example.com", "password123").await;

    let att = upload_test_file(&state, &token, "hello.txt", b"hello world").await;
    assert_eq!(att["filename"], "hello.txt");
    assert_eq!(att["content_type"], "text/plain");
    assert_eq!(att["size_bytes"], 11);
    assert!(att["owner"].as_str().unwrap().starts_with("user:"));
    assert_eq!(att["path"], "hello.txt");
}

#[tokio::test]
async fn upload_file_without_auth_returns_401() {
    let (state, _tmp) = test_app_state().await;
    let app = build_app(state);
    let boundary = "----testboundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.txt\"\r\nContent-Type: text/plain\r\n\r\ndata\r\n--{boundary}--\r\n"
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/files")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn upload_file_with_relative_path() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "pathuser", "pathuser@example.com", "password123").await;

    let app = build_app(state);
    let req = multipart_upload_with_path(&token, "doc.txt", b"content", "docs/readme/doc.txt");
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["filename"], "doc.txt");
    assert!(json["path"].as_str().unwrap().starts_with("docs/readme/"));
}

#[tokio::test]
async fn upload_file_missing_file_field_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "nofield", "nofield@example.com", "password123").await;

    let app = build_app(state);
    let boundary = "----testboundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"path\"\r\n\r\nsome/path\r\n--{boundary}--\r\n"
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/files")
        .header("authorization", format!("Bearer {token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upload_deduplicates_filename() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "dedup", "dedup@example.com", "password123").await;

    let first = upload_test_file(&state, &token, "dup.txt", b"first").await;
    assert_eq!(first["filename"], "dup.txt");

    let second = upload_test_file(&state, &token, "dup.txt", b"second").await;
    assert_eq!(second["filename"], "dup-1.txt");
}


#[tokio::test]
async fn upload_with_traversal_path_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "trav-upload", "travupload@example.com", "password123").await;

    let app = build_app(state);
    let req =
        multipart_upload_with_path(&token, "evil.txt", b"data", "../../etc/evil.txt");
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upload_absolute_path_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "abs-path", "abspath@example.com", "password123").await;

    let app = build_app(state);
    let req = multipart_upload_with_path(&token, "file.txt", b"data", "/absolute/path.txt");
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upload_with_unknown_field_only_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "unk-field", "unkfield@example.com", "password123").await;

    let boundary = "----testboundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"unknown\"\r\n\r\nvalue\r\n--{boundary}--\r\n"
    );

    let app = build_app(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/files")
        .header("authorization", format!("Bearer {token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}


#[tokio::test]
async fn presign_file_returns_url() {
    let (state, tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "presigner", "presigner@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("presigner").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("doc.pdf"), b"pdf content").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/presign",
            &token,
            serde_json::json!({
                "owner": format!("user:{user_id}"),
                "path": "doc.pdf"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let url = json["url"].as_str().unwrap();
    assert!(url.contains("presign="));
    assert!(url.contains("doc.pdf"));
}

#[tokio::test]
async fn presign_other_user_returns_403() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "pre-own", "preown@example.com", "password123").await;
    let (_, user_id_b) =
        register_user(&state, "pre-oth", "preoth@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/presign",
            &token_a,
            serde_json::json!({
                "owner": format!("user:{user_id_b}"),
                "path": "file.txt"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn presign_invalid_owner_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "pre-bad", "prebad@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/presign",
            &token,
            serde_json::json!({
                "owner": "invalid:prefix",
                "path": "file.txt"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}


#[tokio::test]
async fn presign_agent_file_returns_url() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "pre-agent", "preagent@example.com", "password123").await;
    let agent = create_agent(&state, &token, "PreAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let agent_dir = tmp.path().join("users").join(agent_id).join("agents").join(agent_id);
    fs::create_dir_all(&agent_dir).await.unwrap();
    fs::write(agent_dir.join("out.csv"), b"csv data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/presign",
            &token,
            serde_json::json!({
                "owner": format!("agent:{agent_id}"),
                "path": "out.csv"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let url = json["url"].as_str().unwrap();
    assert!(url.contains("presign="));
    assert!(url.contains("out.csv"));
}
