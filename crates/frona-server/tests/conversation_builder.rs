use chrono::Utc;
use frona::auth::UserService;
use frona::chat::message::models::{Message, MessageRole, MessageStatus};
use frona::db::init as db;
use frona::db::repo::generic::SurrealRepo;
use frona::inference::conversation::{
    ConversationBuilder, ConversationContext, DefaultConversationBuilder,
};
use frona::inference::provider::ModelRef;
use frona::inference::tool_call::ToolCall;
use frona::storage::StorageService;
use rig_core::completion::message::UserContent;
use rig_core::completion::{AssistantContent, Message as RigMessage};
use surrealdb::engine::local::{Db, Mem};
use surrealdb::Surreal;

async fn test_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db::setup_schema(&db).await.unwrap();
    db
}

fn test_builder(db: &Surreal<Db>) -> DefaultConversationBuilder {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().to_string_lossy().to_string();
    let config = frona::core::config::Config {
        storage: frona::core::config::StorageConfig {
            data_dir: base.clone(),
            shared_config_dir: format!("{base}/config"),
            ..Default::default()
        },
        ..Default::default()
    };
    DefaultConversationBuilder {
        user_service: UserService::new(SurrealRepo::new(db.clone()), &config.cache),
        storage_service: StorageService::new(&config),
    }
}

fn test_ctx() -> ConversationContext {
    ConversationContext {
        agent_id: "test-agent".into(),
        model_ref: ModelRef {
            provider: "mock".into(),
            model_id: "test-model".into(),
            additional_params: None,
        },
        user_id: "test-user".into(),
    }
}

fn user_message(chat_id: &str, content: &str) -> Message {
    Message::builder(chat_id, MessageRole::User, content.to_string()).build()
}

fn agent_message(chat_id: &str, content: &str, status: Option<MessageStatus>) -> Message {
    let mut msg = Message::builder(chat_id, MessageRole::Agent, content.to_string())
        .agent_id("test-agent".to_string())
        .build();
    msg.status = status;
    msg
}

fn tool_call(chat_id: &str, message_id: &str, turn: u32, name: &str) -> ToolCall {
    ToolCall {
        id: frona::core::repository::new_id(),
        chat_id: chat_id.to_string(),
        message_id: message_id.to_string(),
        turn,
        provider_call_id: format!("call-{}", frona::core::repository::new_id()),
        name: name.to_string(),
        arguments: serde_json::json!({"query": "test"}),
        result: "tool output".to_string(),
        success: true,
        duration_ms: 100,
        tool_data: None,
        system_prompt: None,
        description: None,
        turn_text: None,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn agent_with_tool_calls_single_turn() {
    let db = test_db().await;
    let builder = test_builder(&db);
    let ctx = test_ctx();

    let agent_msg = agent_message("chat-1", "Here's the answer", Some(MessageStatus::Completed));
    let agent_msg_id = agent_msg.id.clone();

    let messages = vec![user_message("chat-1", "Search for Rust"), agent_msg];

    let te1 = tool_call("chat-1", &agent_msg_id, 0, "search_web");
    let te2 = tool_call("chat-1", &agent_msg_id, 0, "browse_page");
    let tool_calls = vec![te1, te2];

    let result = builder.build(&messages, &tool_calls, &ctx).await;

    // user msg, assistant(tool_calls x2), user(tool_results x2), assistant(final text)
    assert_eq!(result.len(), 4);
    assert!(matches!(&result[0], RigMessage::User { .. }));

    // Assistant with tool calls
    if let RigMessage::Assistant { content, .. } = &result[1] {
        let items: Vec<_> = content.iter().collect();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|c| matches!(c, AssistantContent::ToolCall(_))));
    } else {
        panic!("Expected assistant message with tool calls");
    }

    // User with tool results
    if let RigMessage::User { content } = &result[2] {
        let items: Vec<_> = content.iter().collect();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|c| matches!(c, UserContent::ToolResult(_))));
    } else {
        panic!("Expected user message with tool results");
    }

    // Final text
    if let RigMessage::Assistant { content, .. } = &result[3] {
        let items: Vec<_> = content.iter().collect();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], AssistantContent::Text(t) if t.text == "Here's the answer"));
    } else {
        panic!("Expected assistant message with final text");
    }
}

#[tokio::test]
async fn agent_with_tool_calls_multi_turn() {
    let db = test_db().await;
    let builder = test_builder(&db);
    let ctx = test_ctx();

    let agent_msg = agent_message("chat-1", "Final answer", Some(MessageStatus::Completed));
    let agent_msg_id = agent_msg.id.clone();

    let messages = vec![user_message("chat-1", "Help me"), agent_msg];

    let te_turn0 = tool_call("chat-1", &agent_msg_id, 0, "search");
    let te_turn1 = tool_call("chat-1", &agent_msg_id, 1, "browse");
    let tool_calls = vec![te_turn0, te_turn1];

    let result = builder.build(&messages, &tool_calls, &ctx).await;

    // user, assistant(tc t0), user(tr t0), assistant(tc t1), user(tr t1), assistant(final)
    assert_eq!(result.len(), 6);
}

#[tokio::test]
async fn agent_executing_status_no_final_text() {
    let db = test_db().await;
    let builder = test_builder(&db);
    let ctx = test_ctx();

    let agent_msg = agent_message("chat-1", "", Some(MessageStatus::Executing));
    let agent_msg_id = agent_msg.id.clone();

    let messages = vec![user_message("chat-1", "Do something"), agent_msg];

    let te = tool_call("chat-1", &agent_msg_id, 0, "web_search");
    let tool_calls = vec![te];

    let result = builder.build(&messages, &tool_calls, &ctx).await;

    // user, assistant(tool_call), user(tool_result) — no final text
    assert_eq!(result.len(), 3);
    assert!(matches!(&result[0], RigMessage::User { .. }));
    assert!(matches!(&result[1], RigMessage::Assistant { .. }));
    assert!(matches!(&result[2], RigMessage::User { .. }));
}

#[tokio::test]
async fn agent_without_tool_calls_unchanged() {
    let db = test_db().await;
    let builder = test_builder(&db);
    let ctx = test_ctx();

    let messages = vec![
        user_message("chat-1", "Hello"),
        agent_message("chat-1", "Hi there!", None),
    ];
    let tool_calls = vec![];

    let result = builder.build(&messages, &tool_calls, &ctx).await;

    assert_eq!(result.len(), 2);
    assert!(matches!(&result[0], RigMessage::User { .. }));
    if let RigMessage::Assistant { content, .. } = &result[1] {
        let items: Vec<_> = content.iter().collect();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], AssistantContent::Text(t) if t.text == "Hi there!"));
    } else {
        panic!("Expected assistant message");
    }
}

#[tokio::test]
async fn turn_text_appears_in_reconstructed_history() {
    let db = test_db().await;
    let builder = test_builder(&db);
    let ctx = test_ctx();

    let agent_msg = agent_message("chat-1", "Done", Some(MessageStatus::Completed));
    let agent_msg_id = agent_msg.id.clone();

    let messages = vec![user_message("chat-1", "Search for Rust"), agent_msg];

    let mut te = tool_call("chat-1", &agent_msg_id, 0, "search_web");
    te.turn_text = Some("Here's what I found:".into());
    let tool_calls = vec![te];

    let result = builder.build(&messages, &tool_calls, &ctx).await;

    // user, assistant(turn_text + tool_call), user(tool_result), assistant(final text)
    assert_eq!(result.len(), 4);

    // Assistant message should contain turn text + tool call
    if let RigMessage::Assistant { content, .. } = &result[1] {
        let items: Vec<_> = content.iter().collect();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], AssistantContent::Text(t) if t.text == "Here's what I found:"));
        assert!(matches!(items[1], AssistantContent::ToolCall(_)));
    } else {
        panic!("Expected assistant message with turn text and tool call");
    }
}

#[tokio::test]
async fn turn_text_empty_string_omitted() {
    let db = test_db().await;
    let builder = test_builder(&db);
    let ctx = test_ctx();

    let agent_msg = agent_message("chat-1", "Done", Some(MessageStatus::Completed));
    let agent_msg_id = agent_msg.id.clone();

    let messages = vec![user_message("chat-1", "Do it"), agent_msg];

    let mut te = tool_call("chat-1", &agent_msg_id, 0, "search_web");
    te.turn_text = Some(String::new());
    let tool_calls = vec![te];

    let result = builder.build(&messages, &tool_calls, &ctx).await;

    // Assistant message should only have the tool call, no empty text
    if let RigMessage::Assistant { content, .. } = &result[1] {
        let items: Vec<_> = content.iter().collect();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], AssistantContent::ToolCall(_)));
    } else {
        panic!("Expected assistant message with tool call only");
    }
}
