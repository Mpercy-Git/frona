//! Web Push must not be usable to make the server request its own network.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use frona::core::config::PushConfig;
use frona::core::repository::Repository;
use frona::notification::models::{Notification, NotificationData, NotificationLevel, PushExtras};
use frona::notification::push_model::PushSubscription;
use frona::notification::push_sender::PushSender;
use tower::ServiceExt;

use super::*;

// A syntactically valid subscription key pair (from the web-push test suite),
// so the message is built and encrypted and the guard is what stops the send.
const P256DH: &str =
    "BLMbF9ffKBiWQLCKvTHb6LO8Nb6dcUh6TItC455vu2kElga6PQvUmaFyCdykxY2nOSSL3yKgfbmFLRTUaGv4yV8";
const AUTH: &str = "xS03Fi5ErfTNH_l9WHE9Ig";

fn subscribe(token: &str, endpoint: &str) -> Request<Body> {
    auth_post_json(
        "/api/push/subscribe",
        token,
        serde_json::json!({
            "endpoint": endpoint,
            "keys": { "p256dh": P256DH, "auth": AUTH },
        }),
    )
}

#[tokio::test]
async fn subscribing_to_an_internal_endpoint_is_refused() {
    let (state, _tmp) = test_app_state().await;
    let (token, user_id) =
        register_user(&state, "pusher", "pusher@example.com", "password123").await;

    for endpoint in [
        "https://127.0.0.1/hook",
        "https://localhost/hook",
        "https://169.254.169.254/latest/meta-data/",
        "https://10.1.2.3:8443/hook",
        "https://[::1]/hook",
        "https://[::ffff:192.168.0.1]/hook",
        "https://printer.local/hook",
        "https://intranet/hook",
        "https://user:pw@fcm.googleapis.com/hook",
        "http://fcm.googleapis.com/hook",
    ] {
        let resp = build_app(state.clone())
            .oneshot(subscribe(&token, endpoint))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{endpoint}");
    }
    assert!(
        state
            .push_subscription_repo
            .find_by_user_id(&user_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_real_push_service_endpoint_is_accepted() {
    let (state, _tmp) = test_app_state().await;
    let (token, _) = register_user(&state, "pushok", "pushok@example.com", "password123").await;

    let resp = build_app(state)
        .oneshot(subscribe(
            &token,
            "https://fcm.googleapis.com/fcm/send/abc123",
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

fn sender_with_generated_keys(state: &AppState, dir: &std::path::Path) -> PushSender {
    let mut config = PushConfig::default();
    frona::notification::vapid::ensure_keys(&mut config, &dir.to_string_lossy());
    PushSender::new(&config, state.push_subscription_repo.clone())
        .unwrap()
        .expect("VAPID keys were generated")
}

fn notification(user_id: &str) -> Notification {
    Notification {
        id: frona::core::repository::new_id(),
        user_id: user_id.to_string(),
        data: NotificationData::System {},
        level: NotificationLevel::Info,
        title: "t".into(),
        body: "b".into(),
        read: false,
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn the_sender_never_connects_to_a_stored_internal_endpoint() {
    let (state, tmp) = test_app_state().await;
    let (_token, user_id) =
        register_user(&state, "legacy", "legacy@example.com", "password123").await;
    let sender = sender_with_generated_keys(&state, tmp.path());

    // A TLS-less listener stands in for an internal service. If the sender
    // connected, it would accept.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // Stored directly, as a subscription from before the checks existed would be.
    state
        .push_subscription_repo
        .create(&PushSubscription {
            id: frona::core::repository::new_id(),
            user_id: user_id.clone(),
            endpoint: format!("https://127.0.0.1:{port}/internal"),
            expiration_time: None,
            p256dh_key: P256DH.into(),
            auth_secret: AUTH.into(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let report = sender
        .deliver_to_user(&user_id, &notification(&user_id), &PushExtras::default())
        .await;
    assert_eq!(report.attempted, 1);
    assert_eq!(report.delivered, 0);
    assert_eq!(report.failures.len(), 1);

    let accepted =
        tokio::time::timeout(std::time::Duration::from_millis(300), listener.accept()).await;
    assert!(
        accepted.is_err(),
        "the server connected to an internal address"
    );
}

#[tokio::test]
async fn delivery_is_rate_limited_per_user() {
    let (state, tmp) = test_app_state().await;
    let (_token, user_id) =
        register_user(&state, "limited", "limited@example.com", "password123").await;
    let sender = sender_with_generated_keys(&state, tmp.path());

    state
        .push_subscription_repo
        .create(&PushSubscription {
            id: frona::core::repository::new_id(),
            user_id: user_id.clone(),
            endpoint: "https://127.0.0.1/blocked".into(),
            expiration_time: None,
            p256dh_key: P256DH.into(),
            auth_secret: AUTH.into(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let mut limited = false;
    for _ in 0..130 {
        let report = sender
            .deliver_to_user(&user_id, &notification(&user_id), &PushExtras::default())
            .await;
        if report
            .failures
            .iter()
            .any(|f| f.reason.contains("Too many"))
        {
            limited = true;
            break;
        }
    }
    assert!(limited, "130 pushes in a row were never throttled");
}
