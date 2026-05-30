use frona::app::supervisor::AppSupervisor;
use frona::core::supervisor::Supervisor;

use super::*;

#[tokio::test]
async fn find_running_skips_static_apps() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "sup-test", "suptest@example.com", "password123").await;

    let app = build_app(state.clone());
    app.oneshot(auth_post_json(
        "/api/agents/default/chats",
        &token,
        serde_json::json!({}),
    ))
    .await
    .unwrap();

    let supervisor = AppSupervisor::new(state.clone());
    let running = supervisor.find_running().await.unwrap();
    assert!(
        running.is_empty(),
        "no apps should be running on a fresh state"
    );
}

#[tokio::test]
async fn label_returns_app() {
    let (state, _tmp) = test_app_state().await;
    let supervisor = AppSupervisor::new(state);
    assert_eq!(supervisor.label(), "app");
}

#[tokio::test]
async fn owner_of_missing_app_returns_not_found() {
    let (state, _tmp) = test_app_state().await;
    let supervisor = AppSupervisor::new(state);
    let result = supervisor.owner_of("nonexistent").await;
    assert!(matches!(
        result,
        Err(frona::core::error::AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn display_name_falls_back_to_id_when_missing() {
    let (state, _tmp) = test_app_state().await;
    let supervisor = AppSupervisor::new(state);
    let name = supervisor.display_name("unknown-id").await;
    assert_eq!(name, "unknown-id");
}

#[tokio::test]
async fn notification_data_has_correct_shape() {
    let (state, _tmp) = test_app_state().await;
    let supervisor = AppSupervisor::new(state);
    let data = supervisor.notification_data("app-123", "crash").await;
    match data {
        frona::notification::models::NotificationData::App { app_handle, action } => {
            // No app row exists for "app-123" in the test fixture, so the
            // supervisor falls back to using the id verbatim.
            assert_eq!(app_handle, "app-123");
            assert_eq!(action, "crash");
        }
        _ => panic!("expected App notification data"),
    }
}
