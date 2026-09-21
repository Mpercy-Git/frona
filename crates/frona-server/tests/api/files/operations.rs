use axum::http::StatusCode;
use tokio::fs;
use tower::ServiceExt;

use super::super::*;

#[tokio::test]
async fn rename_user_file_succeeds() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "renamer", "renamer@example.com", "password123").await;

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
    let (token, _) = register_user(
        &state,
        "rename-miss",
        "renamemiss@example.com",
        "password123",
    )
    .await;

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
    let (token, _) = register_user(
        &state,
        "rename-trav",
        "renametrav@example.com",
        "password123",
    )
    .await;

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
    let (token, _) = register_user(&state, "copier", "copier@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("copier").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("src.txt"), b"source data")
        .await
        .unwrap();

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
    let (token, _) = register_user(&state, "copydir", "copydir@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("copydir").join("files");
    let src_dir = user_dir.join("project");
    fs::create_dir_all(src_dir.join("sub")).await.unwrap();
    fs::write(src_dir.join("root.txt"), b"root").await.unwrap();
    fs::write(src_dir.join("sub").join("deep.txt"), b"deep")
        .await
        .unwrap();

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
    assert!(
        user_dir
            .join("copy-dest")
            .join("project")
            .join("root.txt")
            .exists()
    );
    assert!(
        user_dir
            .join("copy-dest")
            .join("project")
            .join("sub")
            .join("deep.txt")
            .exists()
    );
}

#[tokio::test]
async fn copy_to_an_agent_the_caller_does_not_own_is_refused() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "copy-ag", "copyag@example.com", "password123").await;

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
    assert!(resp.status().is_client_error(), "status {}", resp.status());
}

#[tokio::test]
async fn copy_from_other_user_via_prefix_returns_403() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "copy-own-a", "copyowna@example.com", "password123").await;
    let (_, _) = register_user(&state, "copy-own-b", "copyownb@example.com", "password123").await;

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
    let (token, _) = register_user(&state, "mover", "mover@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("mover").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("moveme.txt"), b"moving")
        .await
        .unwrap();

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
async fn move_from_an_agent_the_caller_does_not_own_is_refused() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "move-ag", "moveag@example.com", "password123").await;

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
    assert!(resp.status().is_client_error(), "status {}", resp.status());
}

#[tokio::test]
async fn move_to_an_agent_the_caller_does_not_own_is_refused() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "move-ag2", "moveag2@example.com", "password123").await;

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
    assert!(resp.status().is_client_error(), "status {}", resp.status());
}

#[tokio::test]
async fn move_from_other_user_via_prefix_returns_403() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "move-own-a", "moveowna@example.com", "password123").await;
    let (_, _) = register_user(&state, "move-own-b", "moveownb@example.com", "password123").await;

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
    assert!(
        tmp.path()
            .join("users")
            .join("mkdir-user")
            .join("files")
            .join("new-folder")
            .join("sub")
            .is_dir()
    );
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

/// The `user://` branch of `resolve_file_virtual_path` checks the handle in the
/// path against the caller. The `agent://` branch checked nothing, and an agent
/// namespace resolves under `data/users/<name>/agents/<name>/` - so the name in
/// the URI picked the victim's tree and any authenticated user could copy out of
/// it. Reported upstream as `fix: make file operations ownership safe`.
#[tokio::test]
async fn copying_from_another_users_agent_workspace_is_refused() {
    let (state, tmp) = test_app_state().await;
    let (_victim_token, _) =
        register_user(&state, "victim", "victim@example.com", "password123").await;
    let (thief_token, _) = register_user(&state, "thief", "thief@example.com", "password123").await;

    // A file in the victim's own agent workspace.
    let victim_agent_dir = tmp
        .path()
        .join("users")
        .join("victim")
        .join("agents")
        .join("victim");
    fs::create_dir_all(&victim_agent_dir).await.unwrap();
    fs::write(victim_agent_dir.join("secrets.env"), b"API_KEY=hunter2")
        .await
        .unwrap();

    let thief_dir = tmp.path().join("users").join("thief").join("files");
    fs::create_dir_all(&thief_dir).await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/copy",
            &thief_token,
            serde_json::json!({
                "sources": ["agent://victim/secrets.env"],
                "destination": "",
            }),
        ))
        .await
        .unwrap();

    // The material harm first: whatever the status code, the bytes must not move.
    let leaked = thief_dir.join("secrets.env").exists();
    let status = resp.status();
    assert!(
        !leaked,
        "the victim's file was copied into the caller's directory (status {status})"
    );
    // 404 rather than 403: the ownership check looks the agent up among the
    // caller's own, so one that isn't theirs is indistinguishable from one that
    // does not exist. Answering 403 here would confirm the victim's agent by
    // name, which is an enumeration oracle the caller should not get.
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another user's workspace must not resolve"
    );
}

#[tokio::test]
async fn delete_removes_a_file() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "deleter", "deleter@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("deleter").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("gone.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/delete",
            &token,
            serde_json::json!({"paths": ["gone.txt"]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!user_dir.join("gone.txt").exists());
}

#[tokio::test]
async fn delete_removes_a_directory_and_its_contents() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "dirdel", "dirdel@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("dirdel").join("files");
    let nested = user_dir.join("project").join("src");
    fs::create_dir_all(&nested).await.unwrap();
    fs::write(nested.join("main.rs"), b"fn main() {}")
        .await
        .unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/delete",
            &token,
            serde_json::json!({"paths": ["project"]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!user_dir.join("project").exists());
}

#[tokio::test]
async fn delete_with_no_paths_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "emptydel", "emptydel@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/delete",
            &token,
            serde_json::json!({"paths": []}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_missing_path_leaves_the_rest_of_the_batch_alone() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "batchdel", "batchdel@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("batchdel").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("keep.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/delete",
            &token,
            serde_json::json!({"paths": ["keep.txt", "absent.txt"]}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert!(
        user_dir.join("keep.txt").exists(),
        "the whole batch must fail before anything is removed"
    );
}

#[tokio::test]
async fn delete_rejects_the_workspace_root() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "rootdel", "rootdel@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("rootdel").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("keep.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/delete",
            &token,
            serde_json::json!({"paths": [""]}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(user_dir.join("keep.txt").exists());
    assert!(user_dir.exists(), "the workspace root itself must survive");
}

#[tokio::test]
async fn delete_from_an_agent_the_caller_does_not_own_is_refused() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "agentdel", "agentdel@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/delete",
            &token,
            serde_json::json!({"paths": ["agent://agentdel/notes.txt"]}),
        ))
        .await
        .unwrap();
    assert!(resp.status().is_client_error(), "status {}", resp.status());
}

#[tokio::test]
async fn rename_rejects_the_workspace_root() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "rootren", "rootren@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("rootren").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/rename",
            &token,
            serde_json::json!({"path": "", "new_name": "stolen"}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(user_dir.exists(), "the workspace root itself must survive");
}

#[tokio::test]
async fn copy_into_an_owned_agent_workspace_persists() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "wsowner", "wsowner@example.com", "password123").await;
    let agent = create_agent(&state, &token, "Helper").await;
    let agent_handle = agent["handle"].as_str().unwrap().to_string();

    let user_dir = tmp.path().join("users").join("wsowner").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("brief.md"), b"the brief")
        .await
        .unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/copy",
            &token,
            serde_json::json!({
                "sources": ["/brief.md"],
                "destination": format!("agent://{agent_handle}/"),
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let landed = tmp
        .path()
        .join("users")
        .join("wsowner")
        .join("agents")
        .join(&agent_handle)
        .join("brief.md");
    assert!(
        landed.exists(),
        "the file should be in the agent's workspace"
    );
    assert!(
        user_dir.join("brief.md").exists(),
        "a copy leaves the source"
    );
}

#[tokio::test]
async fn move_out_of_an_owned_agent_workspace_persists() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "wsmover", "wsmover@example.com", "password123").await;
    let agent = create_agent(&state, &token, "Runner").await;
    let agent_handle = agent["handle"].as_str().unwrap().to_string();

    let agent_dir = tmp
        .path()
        .join("users")
        .join("wsmover")
        .join("agents")
        .join(&agent_handle);
    fs::create_dir_all(&agent_dir).await.unwrap();
    fs::write(agent_dir.join("result.csv"), b"a,b")
        .await
        .unwrap();
    let user_dir = tmp.path().join("users").join("wsmover").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/move",
            &token,
            serde_json::json!({
                "sources": [format!("agent://{agent_handle}/result.csv")],
                "destination": "",
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        user_dir.join("result.csv").exists(),
        "the file should have moved out"
    );
    assert!(
        !agent_dir.join("result.csv").exists(),
        "a move leaves nothing behind"
    );
}

// `files/` and `agents/<a>/` are siblings under a user's root, so a source of
// `user://<self>/` is not the no-op that moving a directory into itself would
// be: it relocates the user's entire tree into the workspace. `rename` and
// `delete` were guarded when the empty path was first noticed; `copy` and
// `move` take the same path and were not.
#[tokio::test]
async fn move_rejects_a_workspace_root_source() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "rootmv", "rootmv@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("rootmv").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("keep.txt"), b"data").await.unwrap();

    let app = build_app(state);
    for source in ["", "/", ".", "./", "user://rootmv/", "user://rootmv/."] {
        let resp = app
            .clone()
            .oneshot(auth_post_json(
                "/api/files/move",
                &token,
                serde_json::json!({
                    "sources": [source],
                    "destination": "agent://rootmv/"
                }),
            ))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "source {source:?}");
        assert!(
            user_dir.join("keep.txt").exists(),
            "the tree must survive a move of {source:?}"
        );
        assert!(user_dir.exists(), "the workspace root itself must survive");
    }
}

#[tokio::test]
async fn copy_rejects_a_workspace_root_source() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "rootcp", "rootcp@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("rootcp").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("keep.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/copy",
            &token,
            serde_json::json!({
                "sources": ["user://rootcp/"],
                "destination": "/backup"
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(user_dir.join("keep.txt").exists());
    assert!(
        !user_dir.join("backup").exists(),
        "nothing may be created for a rejected batch"
    );
}

/// The whole batch fails before anything moves, so one bad source cannot leave
/// the request half applied - the order `delete_files` already took.
#[tokio::test]
async fn a_workspace_root_source_fails_the_whole_batch() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "batchmv", "batchmv@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("batchmv").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("first.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/move",
            &token,
            serde_json::json!({
                "sources": ["/first.txt", ""],
                "destination": "/archive"
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(
        user_dir.join("first.txt").exists(),
        "the good source must not move when a later one is refused"
    );
}

/// The guard is on sources only. A workspace root is an ordinary *destination*
/// - copying a file to the top of My Files is exactly that request.
#[tokio::test]
async fn a_workspace_root_is_still_a_valid_destination() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "rootdst", "rootdst@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("rootdst").join("files");
    fs::create_dir_all(user_dir.join("nested")).await.unwrap();
    fs::write(user_dir.join("nested").join("note.txt"), b"data")
        .await
        .unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/copy",
            &token,
            serde_json::json!({
                "sources": ["/nested/note.txt"],
                "destination": ""
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(user_dir.join("note.txt").exists());
}

// The same root reached by a different spelling. `Path::join` keeps the `.` in
// the string while every reader of the path skips it, so `"."` names the
// workspace directory exactly as `""` does - and the guard tested only the
// empty spelling, leaving `delete` able to take a user's whole tree.
#[tokio::test]
async fn a_dot_path_is_the_workspace_root_too() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "dotpath", "dotpath@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("dotpath").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("keep.txt"), b"data").await.unwrap();

    let app = build_app(state);
    for path in [".", "./", "./."] {
        let resp = app
            .clone()
            .oneshot(auth_post_json(
                "/api/files/delete",
                &token,
                serde_json::json!({"paths": [path]}),
            ))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "delete {path:?}");
        assert!(
            user_dir.join("keep.txt").exists(),
            "the tree must survive a delete of {path:?}"
        );

        let resp = app
            .clone()
            .oneshot(auth_post_json(
                "/api/files/rename",
                &token,
                serde_json::json!({"path": path, "new_name": "taken.txt"}),
            ))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "rename {path:?}");
        assert!(user_dir.exists(), "the workspace root itself must survive");
    }
}

/// A `.` inside a path still names a file, so the guard must not read every
/// dot as the root.
#[tokio::test]
async fn a_dot_segment_within_a_path_still_names_a_file() {
    let (state, tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "dotseg", "dotseg@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("dotseg").join("files");
    fs::create_dir_all(user_dir.join("nested")).await.unwrap();
    fs::write(user_dir.join("nested").join("note.txt"), b"data")
        .await
        .unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/delete",
            &token,
            serde_json::json!({"paths": ["./nested/note.txt"]}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!user_dir.join("nested").join("note.txt").exists());
    assert!(user_dir.join("nested").exists());
}
