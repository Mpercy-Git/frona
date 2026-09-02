use axum::body::Body;
use axum::http::{Request, StatusCode};
use frona::api::routes::voice;
use frona::core::config::Config;
use frona::core::metrics::setup_metrics_recorder;
use frona::core::state::AppState;
use frona::db::init as db;
use frona::storage::StorageService;
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};
use tower::ServiceExt;

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

async fn test_app_state() -> (AppState, tempfile::TempDir) {
    let db = test_db().await;
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().to_string_lossy().to_string();
    let config = Config {
        auth: frona::core::config::AuthConfig {
            encryption_secret: "test-secret".to_string(),
            ..Default::default()
        },
        storage: frona::core::config::StorageConfig {
            data_dir: base.clone(),
            shared_config_dir: format!("{base}/config"),
            ..Default::default()
        },
        ..Default::default()
    };
    let storage = StorageService::new(&config);
    let resource_manager = std::sync::Arc::new(
        frona::tool::sandbox::driver::resource_monitor::SystemResourceManager::new(
            80.0, 80.0, 90.0, 90.0,
        ),
    );
    let metrics = setup_metrics_recorder();
    let state = AppState::new(
        db,
        &config,
        Some(frona::inference::config::ModelRegistryConfig::empty()),
        storage,
        metrics,
        resource_manager,
    );
    (state, tmp)
}

#[tokio::test]
async fn twilio_callback_invalid_token_returns_403() {
    let (state, _tmp) = test_app_state().await;
    let app = voice::router().with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/voice/twilio/callback?token=invalid.token.here")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn twilio_ws_invalid_token_returns_403() {
    let (state, _tmp) = test_app_state().await;
    let app = voice::router().with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/voice/twilio/ws?token=bad.token.here")
                .header("upgrade", "websocket")
                .header("connection", "upgrade")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .header("sec-websocket-version", "13")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// Everything the callback path needs: a state whose token service can
/// round-trip a voice token, plus the persisted user it is signed for.
async fn callback_test_fixture() -> (AppState, Surreal<Db>, frona::auth::User, tempfile::TempDir) {
    use frona::auth::User;
    use frona::core::repository::Repository;
    use frona::db::repo::generic::SurrealRepo;

    let (state, tmp) = test_app_state().await;
    let db = state.db.clone();

    // Persist the user so the AppState's token_service can round-trip the token
    // through the ApiToken DB row it creates for access tokens.
    let user = User {
        id: "user-123".to_string(),
        handle: frona::handle!("testuser"),
        email: "test@example.com".to_string(),
        name: "Test".to_string(),
        password_hash: String::new(),
        timezone: None,
        groups: Vec::new(),
        deactivated_at: None,
        phone: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let user_repo: SurrealRepo<User> = SurrealRepo::new(db.clone());
    user_repo.create(&user).await.unwrap();

    (state, db, user, tmp)
}

/// Mint the callback token an outbound leg carries, signed for `agent_id`.
async fn callback_token(state: &AppState, user: &frona::auth::User, agent_id: &str) -> String {
    use frona::auth::token::models::TokenType;
    use frona::auth::token::service::CreateTokenRequest;
    use frona::core::Principal;
    use frona::tool::voice::VoiceCallbackExtensions;

    let extensions = serde_json::to_value(VoiceCallbackExtensions {
        chat_id: "chat-456".to_string(),
        welcome_greeting: None,
        hints: None,
        contact_id: None,
        transfer_note: None,
    })
    .unwrap();

    state
        .token_service
        .create_token(
            &state.keypair_service,
            user,
            CreateTokenRequest {
                token_type: TokenType::Access,
                principal: Principal::agent(agent_id),
                ttl_secs: 300,
                name: "voice_callback".into(),
                scopes: Vec::new(),
                refresh_pair_id: None,
                extensions: Some(extensions),
            },
        )
        .await
        .unwrap()
        .jwt
}

async fn post_callback(state: AppState, token: &str) -> axum::response::Response {
    voice::router()
        .with_state(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/voice/twilio/callback?token={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn body_string(resp: axum::response::Response) -> String {
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[tokio::test]
async fn twilio_callback_valid_token_returns_xml() {
    let (state, _db, user, _tmp) = callback_test_fixture().await;
    let token = callback_token(&state, &user, "receptionist").await;

    let resp = post_callback(state, &token).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        ct.contains("application/xml"),
        "Expected XML content-type, got: {ct}"
    );

    let body_str = body_string(resp).await;
    assert!(
        body_str.contains("<ConversationRelay"),
        "Expected ConversationRelay in TwiML:\n{body_str}"
    );
}

/// An outbound leg — `transfer_call`'s callback among them — must speak in the
/// voice of the agent that placed it, not the server-level default. Without
/// this a transferred caller hears the target agent introduce itself in the
/// source agent's voice.
#[tokio::test]
async fn twilio_callback_uses_the_calling_agents_voice() {
    use frona::agent::models::Agent;
    use frona::core::repository::Repository;
    use frona::db::repo::generic::SurrealRepo;

    let (state, db, user, _tmp) = callback_test_fixture().await;

    let agent = Agent {
        id: "agent-with-voice".to_string(),
        user_id: user.id.clone(),
        handle: frona::handle!("specialist"),
        name: "Specialist".to_string(),
        description: "Takes transferred calls".to_string(),
        model_group: "primary".to_string(),
        enabled: true,
        sandbox_limits: None,
        max_concurrent_tasks: None,
        skills: None,
        avatar: None,
        voice_id: Some("en-GB-Standard-B".to_string()),
        identity: Default::default(),
        prompt: None,
        heartbeat_interval: None,
        next_heartbeat_at: None,
        heartbeat_chat_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let agent_repo: SurrealRepo<Agent> = SurrealRepo::new(db.clone());
    agent_repo.create(&agent).await.unwrap();

    let token = callback_token(&state, &user, &agent.id).await;
    let resp = post_callback(state, &token).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body_str = body_string(resp).await;
    assert!(
        body_str.contains(r#"voice="en-GB-Standard-B""#),
        "Expected the agent's own voice on the relay:\n{body_str}"
    );
}

/// An agent with no voice of its own leaves the attribute off, so the
/// server-level `voice.twilio_voice_id` still applies.
#[tokio::test]
async fn twilio_callback_omits_voice_when_the_agent_has_none() {
    let (state, _db, user, _tmp) = callback_test_fixture().await;
    // No agent row at all — the same fallback path as an agent without a voice.
    let token = callback_token(&state, &user, "agent-without-voice").await;

    let resp = post_callback(state, &token).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body_str = body_string(resp).await;
    assert!(
        !body_str.contains("voice="),
        "Expected no voice attribute when neither agent nor config sets one:\n{body_str}"
    );
}
