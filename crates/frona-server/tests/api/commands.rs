use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn commands_discovery_includes_newly_created_agents() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "cmduser", "cmds@example.com", "password123").await;

    let agent_a = create_agent(&state, &token, "InitialAgent").await;
    let agent_a_id = agent_a["id"].as_str().unwrap();
    let chat = create_chat(&state, &token, agent_a_id, None).await;
    let chat_id = chat["id"].as_str().unwrap();
    let uri = format!("/api/chats/{chat_id}/commands");

    let resp = build_app(state.clone())
        .oneshot(auth_get(&uri, &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let before = body_json(resp).await;
    let initial_names: Vec<String> = before["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str().map(String::from))
        .collect();
    assert!(
        !initial_names.iter().any(|n| n == "freshlymade"),
        "test precondition: agent not yet created",
    );

    create_agent(&state, &token, "FreshlyMade").await;

    let resp = build_app(state)
        .oneshot(auth_get(&uri, &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let after = body_json(resp).await;
    let after_names: Vec<String> = after["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str().map(String::from))
        .collect();
    assert!(
        after_names.iter().any(|n| n == "freshlymade"),
        "newly-created agent missing from discovery: {after_names:?}",
    );

    let entry = after["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "freshlymade")
        .expect("freshlymade entry");
    assert_eq!(entry["display_name"], "FreshlyMade");
    assert_eq!(entry["argument_hint"], "[prompt]");
}

/// Static commands (`/clear`) shadow agents with the same handle —
/// precedence is enforced at the discovery layer, not just dispatch.
#[tokio::test]
async fn commands_discovery_filters_handle_collisions() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) =
        register_user(&state, "collide", "collide@example.com", "password123").await;

    let host = create_agent(&state, &token, "Host").await;
    let chat = create_chat(&state, &token, host["id"].as_str().unwrap(), None).await;
    let chat_id = chat["id"].as_str().unwrap();

    create_agent(&state, &token, "clear").await;

    let resp = build_app(state)
        .oneshot(auth_get(
            &format!("/api/chats/{chat_id}/commands"),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let clears: Vec<&serde_json::Value> = json["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == "clear")
        .collect();
    assert_eq!(clears.len(), 1, "duplicate 'clear' entries: {clears:?}");
}
