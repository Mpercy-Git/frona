mod helpers;

use chrono::Utc;
use frona::chat::models::Chat;
use frona::core::config::{CacheConfig, ModelProviderConfig, VoiceConfig};
use frona::core::repository::Repository;
use frona::db::repo::generic::SurrealRepo;
use frona::inference::transcription::TranscriptionService;
use frona::storage::Attachment;
use helpers::test_chat_service_with_db;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn seed_user(db: &surrealdb::Surreal<surrealdb::engine::local::Db>, id: &str) {
    let users =
        frona::auth::UserService::new(SurrealRepo::new(db.clone()), &CacheConfig::default());
    users
        .create(&frona::auth::User {
            id: id.into(),
            handle: frona::core::Handle::try_new(id).unwrap(),
            email: format!("{id}@example.com"),
            name: id.into(),
            password_hash: String::new(),
            timezone: None,
            groups: Vec::new(),
            deactivated_at: None,
            phone: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
}

fn audio(owner: &str, path: &str) -> Attachment {
    Attachment {
        filename: path.rsplit('/').next().unwrap().to_string(),
        content_type: "audio/webm".to_string(),
        size_bytes: 9,
        owner: owner.to_string(),
        path: path.to_string(),
        url: None,
        transcript: None,
    }
}

#[tokio::test]
async fn transcribes_only_voice_notes_in_the_chat_owners_storage() {
    let (mut chat_service, db) = test_chat_service_with_db().await;
    seed_user(&db, "owner").await;
    seed_user(&db, "other").await;
    let now = Utc::now();
    let chat: Chat = SurrealRepo::new(db.clone())
        .create(&Chat {
            id: frona::core::repository::new_id(),
            user_id: "owner".into(),
            space_id: None,
            task_id: None,
            agent_id: "agent-1".into(),
            title: Some("Voice".into()),
            archived_at: None,
            channel_id: None,
            channel_external_id: None,
            metadata: Default::default(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    let storage = chat_service.storage_service().clone();
    for (user, file) in [
        ("owner", "web.weba"),
        ("owner", "inbox/channel.ogg"),
        ("other", "theirs.weba"),
    ] {
        let path = storage
            .user_files_path(&frona::core::Handle::try_new(user).unwrap())
            .join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"fake opus").unwrap();
    }

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"text": "call mum"})),
        )
        .mount(&server)
        .await;
    let providers = [(
        frona::core::Handle::try_new("whisper").unwrap(),
        ModelProviderConfig {
            provider: Some("generic".into()),
            base_url: Some(format!("{}/v1", server.uri())),
            ..Default::default()
        },
    )]
    .into();
    let voice = VoiceConfig {
        transcription_provider: Some("whisper".into()),
        ..Default::default()
    };
    chat_service.set_transcription(TranscriptionService::from_config(&voice, &providers).unwrap());

    let saved = chat_service
        .create_stream_user_message(
            "owner",
            &chat.id,
            "",
            vec![
                // A web upload names its owner by id ...
                audio("user:owner", "web.weba"),
                // ... a channel download by handle.
                audio("user:owner", "inbox/channel.ogg"),
                // Another user's recording must not be read on this user's behalf,
                audio("user:other", "theirs.weba"),
                // nor may an unresolvable attachment fall back to a raw path,
                audio("nobody", "/etc/hostname"),
                // nor may a path climb out of the owner's storage.
                audio("user:owner", "../other/files/theirs.weba"),
            ],
            None,
        )
        .await
        .unwrap();

    let transcripts: Vec<Option<&str>> = saved
        .attachments
        .iter()
        .map(|a| a.transcript.as_deref())
        .collect();
    assert_eq!(
        transcripts,
        vec![Some("call mum"), Some("call mum"), None, None, None]
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn without_a_provider_voice_notes_are_saved_untouched() {
    let (chat_service, db) = test_chat_service_with_db().await;
    seed_user(&db, "owner").await;
    let now = Utc::now();
    let chat: Chat = SurrealRepo::new(db.clone())
        .create(&Chat {
            id: frona::core::repository::new_id(),
            user_id: "owner".into(),
            space_id: None,
            task_id: None,
            agent_id: "agent-1".into(),
            title: Some("Voice".into()),
            archived_at: None,
            channel_id: None,
            channel_external_id: None,
            metadata: Default::default(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    let saved = chat_service
        .create_stream_user_message(
            "owner",
            &chat.id,
            "",
            vec![audio("user:owner", "web.weba")],
            None,
        )
        .await
        .unwrap();
    assert_eq!(saved.attachments.len(), 1);
    assert!(saved.attachments[0].transcript.is_none());
}
