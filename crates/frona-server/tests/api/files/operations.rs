use axum::http::StatusCode;
use tokio::fs;
use tower::ServiceExt;

use super::super::*;


#[tokio::test]
async fn rename_user_file_succeeds() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "renamer", "renamer@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("renamer").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("old.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/rename",
            &token,
            serde_json::json!({"path": "old.txt", "new_name": "new.txt"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!user_dir.join("old.txt").exists());
    assert!(user_dir.join("new.txt").exists());
}

#[tokio::test]
async fn rename_file_not_found_returns_404() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "rename-miss", "renamemiss@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/rename",
            &token,
            serde_json::json!({"path": "nonexistent.txt", "new_name": "x.txt"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rename_file_invalid_name_returns_400() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "rename-bad", "renamebad@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("rename-bad").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("file.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/rename",
            &token,
            serde_json::json!({"path": "file.txt", "new_name": "../escape.txt"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rename_file_destination_exists_returns_400() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "rename-dup", "renamedup@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("rename-dup").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("a.txt"), b"a").await.unwrap();
    fs::write(user_dir.join("b.txt"), b"b").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/rename",
            &token,
            serde_json::json!({"path": "a.txt", "new_name": "b.txt"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rename_path_traversal_returns_400() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "rename-trav", "renametrav@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("rename-trav").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("ok.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/rename",
            &token,
            serde_json::json!({"path": "ok.txt", "new_name": "sub/escape.txt"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}


#[tokio::test]
async fn copy_files_succeeds() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "copier", "copier@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("copier").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("src.txt"), b"source data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/copy",
            &token,
            serde_json::json!({
                "sources": ["/src.txt"],
                "destination": "/backup"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(user_dir.join("backup").join("src.txt").exists());
    // Original still exists
    assert!(user_dir.join("src.txt").exists());
}

#[tokio::test]
async fn copy_directory_recursive() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "copydir", "copydir@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("copydir").join("files");
    let src_dir = user_dir.join("project");
    fs::create_dir_all(src_dir.join("sub")).await.unwrap();
    fs::write(src_dir.join("root.txt"), b"root").await.unwrap();
    fs::write(src_dir.join("sub").join("deep.txt"), b"deep").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/copy",
            &token,
            serde_json::json!({
                "sources": ["/project"],
                "destination": "/copy-dest"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(user_dir.join("copy-dest").join("project").join("root.txt").exists());
    assert!(user_dir
        .join("copy-dest")
        .join("project")
        .join("sub")
        .join("deep.txt")
        .exists());
}

#[tokio::test]
async fn copy_to_agent_workspace_returns_403() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "copy-ag", "copyag@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("copy-ag").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("f.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/copy",
            &token,
            serde_json::json!({
                "sources": ["/f.txt"],
                "destination": "agent://some-agent/out"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn copy_from_other_user_via_prefix_returns_403() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "copy-own-a", "copyowna@example.com", "password123").await;
    let (_, _) =
        register_user(&state, "copy-own-b", "copyownb@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/copy",
            &token_a,
            serde_json::json!({
                "sources": ["user://copy-own-b/secret.txt"],
                "destination": "/stolen"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}


#[tokio::test]
async fn move_files_succeeds() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mover", "mover@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("mover").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("moveme.txt"), b"moving").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/move",
            &token,
            serde_json::json!({
                "sources": ["/moveme.txt"],
                "destination": "/archive"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!user_dir.join("moveme.txt").exists());
    assert!(user_dir.join("archive").join("moveme.txt").exists());
}

#[tokio::test]
async fn move_from_agent_workspace_returns_403() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "move-ag", "moveag@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/move",
            &token,
            serde_json::json!({
                "sources": ["agent://some-agent/file.txt"],
                "destination": "/stolen"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn move_to_agent_workspace_returns_403() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "move-ag2", "moveag2@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("move-ag2").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("f.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/move",
            &token,
            serde_json::json!({
                "sources": ["/f.txt"],
                "destination": "agent://some-agent/out"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn move_from_other_user_via_prefix_returns_403() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "move-own-a", "moveowna@example.com", "password123").await;
    let (_, _) =
        register_user(&state, "move-own-b", "moveownb@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/move",
            &token_a,
            serde_json::json!({
                "sources": ["user://move-own-b/secret.txt"],
                "destination": "/stolen"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}


#[tokio::test]
async fn create_folder_succeeds() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mkdir-user", "mkdiruser@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/mkdir",
            &token,
            serde_json::json!({"path": "new-folder/sub"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(tmp.path().join("users").join("mkdir-user").join("files").join("new-folder").join("sub").is_dir());
}

#[tokio::test]
async fn create_folder_path_traversal_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mkdir-trav", "mkdirtrav@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/mkdir",
            &token,
            serde_json::json!({"path": "../escape"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mkdir_null_char_in_path_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "mkdir-null", "mkdirnull@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/mkdir",
            &token,
            serde_json::json!({"path": "test\u{0000}dir"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
