use axum::body::Body;
use axum::http::{Request, StatusCode};
use tokio::fs;
use tower::ServiceExt;

use super::super::*;


#[tokio::test]
async fn download_user_file_returns_content() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "dluser", "dluser@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("dluser").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("test.txt"), b"file content").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/files/user/dluser/test.txt", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"file content");
}

#[tokio::test]
async fn download_user_file_other_user_returns_403() {
    let (state, tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "dl-owner", "dlowner@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "dl-other", "dlother@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("dl-owner").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("secret.txt"), b"private").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/files/user/dl-owner/secret.txt", &token_b))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Owner can access
    let _ = token_a;
}

#[tokio::test]
async fn download_user_file_not_found_returns_404() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "dl-miss", "dlmiss@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/files/user/dl-miss/nonexistent.txt", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}


#[tokio::test]
async fn download_agent_file_returns_content() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "agentdl", "agentdl@example.com", "password123").await;
    let agent = create_agent(&state, &token, "DlAgent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let agent_handle = agent["handle"].as_str().unwrap();

    let agent_dir = tmp
        .path()
        .join("users")
        .join("agentdl")
        .join("agents")
        .join(agent_handle);
    fs::create_dir_all(&agent_dir).await.unwrap();
    fs::write(agent_dir.join("output.csv"), b"col1,col2").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(
            &format!("/api/files/agent/{agent_id}/output.csv"),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"col1,col2");
}

#[tokio::test]
async fn download_agent_file_other_user_returns_error() {
    let (state, tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "ag-owner", "agowner@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "ag-other", "agother@example.com", "password123").await;

    let agent = create_agent(&state, &token_a, "PrivAgent").await;
    let agent_id = agent["id"].as_str().unwrap();

    let agent_dir = tmp.path().join("users").join(agent_id).join("agents").join(agent_id);
    fs::create_dir_all(&agent_dir).await.unwrap();
    fs::write(agent_dir.join("data.txt"), b"secret").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(
            &format!("/api/files/agent/{agent_id}/data.txt"),
            &token_b,
        ))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND,
        "Expected 403 or 404, got {}",
        resp.status()
    );
}


#[tokio::test]
async fn delete_user_file_removes_file() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "delfile", "delfile@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("delfile").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("remove.txt"), b"bye").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_delete("/api/files/user/delfile/remove.txt", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!user_dir.join("remove.txt").exists());
}

#[tokio::test]
async fn delete_user_file_removes_directory() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "deldir", "deldir@example.com", "password123").await;

    let sub_dir = tmp.path().join("users").join("deldir").join("files").join("mydir");
    fs::create_dir_all(&sub_dir).await.unwrap();
    fs::write(sub_dir.join("inner.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_delete("/api/files/user/deldir/mydir", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!sub_dir.exists());
}

#[tokio::test]
async fn delete_user_file_not_found_returns_404() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "del-miss", "delmiss@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_delete("/api/files/user/del-miss/nope.txt", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_user_file_other_user_returns_403() {
    let (state, tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "del-own", "delown@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "del-oth", "deloth@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("del-own").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("mine.txt"), b"private").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_delete("/api/files/user/del-own/mine.txt", &token_b))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let _ = token_a;
}


#[tokio::test]
async fn list_user_files_empty_root() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "browse-empty", "browseempty@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/files/browse/user", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_user_files_returns_entries() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "browse-files", "browsefiles@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("browse-files").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("a.txt"), b"aaa").await.unwrap();
    fs::create_dir_all(user_dir.join("subdir")).await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/files/browse/user", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let entries = json.as_array().unwrap();
    assert_eq!(entries.len(), 2);

    let types: Vec<&str> = entries.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert!(types.contains(&"file"));
    assert!(types.contains(&"folder"));
}

#[tokio::test]
async fn list_user_files_subdirectory() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "browse-sub", "browsesub@example.com", "password123").await;

    let sub_dir = tmp.path().join("users").join("browse-sub").join("files").join("docs");
    fs::create_dir_all(&sub_dir).await.unwrap();
    fs::write(sub_dir.join("nested.md"), b"# Hello").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/files/browse/user/docs", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let entries = json.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "/docs/nested.md");
    assert_eq!(entries[0]["type"], "file");
    assert_eq!(entries[0]["parent"], "/docs");
}


#[tokio::test]
async fn list_agent_files_root() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "ag-browse", "agbrowse@example.com", "password123").await;
    let agent = create_agent(&state, &token, "BrowseAgent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let handle = agent["handle"].as_str().unwrap();

    let agent_dir = tmp.path().join("users").join("ag-browse").join("agents").join(handle);
    fs::create_dir_all(&agent_dir).await.unwrap();
    fs::write(agent_dir.join("file.txt"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(
            &format!("/api/files/browse/agent/{agent_id}"),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn list_agent_files_subdir() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "ag-sub", "agsub@example.com", "password123").await;
    let agent = create_agent(&state, &token, "SubAgent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let handle = agent["handle"].as_str().unwrap();

    let sub = tmp
        .path()
        .join("users")
        .join("ag-sub")
        .join("agents")
        .join(handle)
        .join("output");
    fs::create_dir_all(&sub).await.unwrap();
    fs::write(sub.join("result.json"), b"{}").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(
            &format!("/api/files/browse/agent/{agent_id}/output"),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let entries = json.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "/output/result.json");
}

#[tokio::test]
async fn list_agent_files_other_user_returns_error() {
    let (state, _tmp) = test_app_state().await;
    let (token_a, _) =
        register_user(&state, "ag-list-own", "aglistown@example.com", "password123").await;
    let (token_b, _) =
        register_user(&state, "ag-list-oth", "aglistoth@example.com", "password123").await;

    let agent = create_agent(&state, &token_a, "PrivList").await;
    let agent_id = agent["id"].as_str().unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(
            &format!("/api/files/browse/agent/{agent_id}"),
            &token_b,
        ))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND,
        "Expected 403 or 404, got {}",
        resp.status()
    );
}


#[tokio::test]
async fn search_files_finds_matching() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "searcher", "searcher@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("searcher").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("report.pdf"), b"pdf").await.unwrap();
    fs::write(user_dir.join("notes.txt"), b"txt").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/files/search?q=report", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let results = json.as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0]["id"].as_str().unwrap().contains("report"));
}

#[tokio::test]
async fn search_files_empty_query_returns_empty() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "search-empty", "searchempty@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/files/search?q=", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn search_files_user_scope() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "search-scope", "searchscope@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("search-scope").join("files");
    let sub = user_dir.join("docs");
    fs::create_dir_all(&sub).await.unwrap();
    fs::write(sub.join("readme.md"), b"hi").await.unwrap();
    fs::write(user_dir.join("readme.txt"), b"other").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(
            "/api/files/search?q=readme&scope=user:docs",
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let results = json.as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0]["id"].as_str().unwrap().contains("readme.md"));
}

#[tokio::test]
async fn search_files_agent_scope() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "search-agent", "searchagent@example.com", "password123").await;
    let agent = create_agent(&state, &token, "SearchAgent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let handle = agent["handle"].as_str().unwrap();

    let agent_dir = tmp.path().join("users").join("search-agent").join("agents").join(handle);
    fs::create_dir_all(&agent_dir).await.unwrap();
    fs::write(agent_dir.join("found.csv"), b"data").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get(
            &format!("/api/files/search?q=found&scope=agent:{agent_id}"),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let results = json.as_array().unwrap();
    assert_eq!(results.len(), 1);
}


#[tokio::test]
async fn search_files_default_scope_includes_agents() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "search-def", "searchdef@example.com", "password123").await;
    let agent = create_agent(&state, &token, "DefAgent").await;
    let handle = agent["handle"].as_str().unwrap();

    let user_dir = tmp.path().join("users").join("search-def").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("userfile.txt"), b"u").await.unwrap();

    let agent_dir = tmp.path().join("users").join("search-def").join("agents").join(handle);
    fs::create_dir_all(&agent_dir).await.unwrap();
    fs::write(agent_dir.join("agentfile.txt"), b"a").await.unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/files/search?q=file", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let results = json.as_array().unwrap();
    assert_eq!(results.len(), 2);

    let ids: Vec<&str> = results.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(ids.iter().any(|id| id.contains("userfile")));
    assert!(ids.iter().any(|id| id.contains("agentfile")));
}


#[tokio::test]
async fn download_user_file_with_presigned_url() {
    let (state, tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "pre-dl", "predl@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("pre-dl").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("presigned.txt"), b"presigned content")
        .await
        .unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/presign",
            &token,
            serde_json::json!({
                "owner": format!("user:{user_id}"),
                "path": "presigned.txt"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let url = json["url"].as_str().unwrap();

    let presign_token = url.split("presign=").nth(1).unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/files/user/pre-dl/presigned.txt?presign={presign_token}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"presigned content");
}

#[tokio::test]
async fn download_agent_file_with_presigned_url() {
    let (state, tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "pre-ag-dl", "preagdl@example.com", "password123").await;
    let agent = create_agent(&state, &token, "PreDlAgent").await;
    let agent_id = agent["id"].as_str().unwrap();
    let agent_handle = agent["handle"].as_str().unwrap();

    let agent_dir = tmp
        .path()
        .join("users")
        .join("pre-ag-dl")
        .join("agents")
        .join(agent_handle);
    fs::create_dir_all(&agent_dir).await.unwrap();
    fs::write(agent_dir.join("report.csv"), b"agent data")
        .await
        .unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/presign",
            &token,
            serde_json::json!({
                "owner": format!("agent:{agent_id}"),
                "path": "report.csv"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let url = json["url"].as_str().unwrap();
    let presign_token = url.split("presign=").nth(1).unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/files/agent/{agent_id}/report.csv?presign={presign_token}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"agent data");
}

#[tokio::test]
async fn download_user_file_presigned_wrong_path_returns_403() {
    let (state, tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "pre-wrong", "prewrong@example.com", "password123").await;

    let user_dir = tmp.path().join("users").join("pre-wrong").join("files");
    fs::create_dir_all(&user_dir).await.unwrap();
    fs::write(user_dir.join("real.txt"), b"real").await.unwrap();
    fs::write(user_dir.join("other.txt"), b"other").await.unwrap();

    let app = build_app(state.clone());
    let resp = app
        .oneshot(auth_post_json(
            "/api/files/presign",
            &token,
            serde_json::json!({
                "owner": format!("user:{user_id}"),
                "path": "real.txt"
            }),
        ))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let url = json["url"].as_str().unwrap();
    let presign_token = url.split("presign=").nth(1).unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/files/user/pre-wrong/other.txt?presign={presign_token}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}


#[tokio::test]
async fn browse_path_traversal_returns_400() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "trav-browse", "travbrowse@example.com", "password123").await;

    let app = build_app(state);
    let resp = app
        .oneshot(auth_get("/api/files/browse/user/../../../etc", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}


#[tokio::test]
async fn file_endpoints_reject_no_auth() {
    let (state, _tmp) = test_app_state().await;

    let cases: Vec<(&str, &str)> = vec![
        ("GET", "/api/files/browse/user"),
        ("GET", "/api/files/search?q=test"),
        ("POST", "/api/files/rename"),
        ("POST", "/api/files/copy"),
        ("POST", "/api/files/move"),
        ("POST", "/api/files/mkdir"),
        ("POST", "/api/files/presign"),
    ];

    for (method, uri) in cases {
        let app = build_app(state.clone());
        let body = if method == "POST" {
            Body::from("{}")
        } else {
            Body::empty()
        };
        let mut builder = Request::builder().method(method).uri(uri);
        if method == "POST" {
            builder = builder.header("content-type", "application/json");
        }
        let req = builder.body(body).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {uri} should return 401 without auth"
        );
    }
}
