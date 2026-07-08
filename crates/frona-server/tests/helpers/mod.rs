// Each integration test file compiles as its own binary and pulls in this
// helpers module; helpers used only by other test files surface as dead-code
// warnings in every binary that doesn't reference them. Silencing module-wide.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use frona::core::metrics;
use frona::db::repo::generic::SurrealRepo;
use frona::inference::config::{ModelGroup, RetryConfig};
use frona::inference::error::InferenceError;
use frona::inference::provider::{ModelProvider, ModelRef};
use frona::inference::registry::ModelProviderRegistry;
use frona::inference::Usage;
use frona::policy::service::PolicyService;
use frona::tool::manager::ToolManager;
use frona::tool::{AgentTool, InferenceContext, ToolDefinition, ToolOutput};
use surrealdb::Surreal;
use surrealdb::engine::local::Db;

/// Test-only UsageService — empty model catalog, fresh broadcast,
/// real DB table. Sufficient to satisfy constructor signatures; in tests that
/// don't assert against the inference_usage table this is a complete stub.
pub fn test_usage_service(db: &Surreal<Db>) -> frona::inference::usage::UsageService {
    frona::inference::usage::UsageService::new(
        frona::inference::metadata::ModelCatalogStore::new(
            frona::inference::metadata::ModelCatalogSnapshot::empty(),
        ),
        SurrealRepo::new(db.clone()),
        frona::chat::broadcast::BroadcastService::new(),
    )
}

/// Test-only UsageContext for fixtures that only need to satisfy a signature.
pub fn test_usage_ctx() -> frona::inference::usage::UsageContext {
    frona::inference::usage::UsageContext::new(
        frona::inference::usage::InferenceKind::Title {
            agent_id: "test-agent".to_string(),
            chat_id: "test-chat".to_string(),
        },
        "test-user",
        "primary".to_string(),
    )
}

pub fn test_policy_service(db: &Surreal<Db>) -> PolicyService {
    let schema = frona::policy::schema::build_schema();
    let repo: Arc<dyn frona::policy::repository::PolicyRepository> =
        Arc::new(SurrealRepo::<frona::policy::models::Policy>::new(db.clone()));
    let tool_manager = Arc::new(ToolManager::new(false));
    let storage = frona::storage::StorageService::new(&frona::core::config::Config::default());
    let user_service = frona::auth::UserService::new(
        SurrealRepo::new(db.clone()),
        &frona::core::config::CacheConfig::default(),
    );
    PolicyService::new(repo, schema, tool_manager, storage, user_service)
}
use rig_core::completion::request::ToolDefinition as RigToolDefinition;
use rig_core::completion::{AssistantContent, Message as RigMessage};
use rig_core::completion::message::{ToolCall, ToolFunction};
use serde_json::Value;
use tokio::sync::mpsc;

pub enum MockResponse {
    Text(String),
    TextWithReasoning(String, String),
    ToolCalls(Vec<(String, String, Value)>),
    Error(InferenceError),
}

pub struct MockModelProvider {
    responses: Mutex<Vec<MockResponse>>,
    pub call_count: Mutex<usize>,
}

impl MockModelProvider {
    pub fn new(responses: Vec<MockResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            call_count: Mutex::new(0),
        }
    }

    fn next_response(&self) -> MockResponse {
        let mut responses = self.responses.lock().unwrap();
        *self.call_count.lock().unwrap() += 1;
        if responses.is_empty() {
            MockResponse::Text("default response".into())
        } else {
            responses.remove(0)
        }
    }

    pub fn calls(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

#[async_trait]
impl ModelProvider for MockModelProvider {
    async fn inference(
        &self,
        _model_id: &str,
        _system_prompt: &str,
        _chat_history: Vec<RigMessage>,
        _tools: Vec<RigToolDefinition>,
        _max_tokens: Option<u64>,
        _temperature: Option<f64>,
        _additional_params: Option<serde_json::Value>,
    ) -> Result<frona::inference::provider::InferenceOutput, InferenceError> {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            cached_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning_tokens: 0,
        };
        let content = match self.next_response() {
            MockResponse::Text(t) => vec![AssistantContent::text(&t)],
            MockResponse::TextWithReasoning(text, reasoning) => vec![
                AssistantContent::Reasoning(rig_core::completion::message::Reasoning::new(&reasoning)),
                AssistantContent::text(&text),
            ],
            MockResponse::ToolCalls(calls) => calls
                .into_iter()
                .map(|(id, name, args)| {
                    AssistantContent::ToolCall(ToolCall::new(id, ToolFunction::new(name, args)))
                })
                .collect(),
            MockResponse::Error(e) => return Err(e),
        };
        Ok(frona::inference::provider::InferenceOutput::new(content, usage))
    }

    async fn stream_inference(
        &self,
        _model_id: &str,
        _system_prompt: &str,
        _chat_history: Vec<RigMessage>,
        _tools: Vec<RigToolDefinition>,
        token_tx: mpsc::Sender<frona::inference::provider::StreamToken>,
        _max_tokens: Option<u64>,
        _temperature: Option<f64>,
        _additional_params: Option<serde_json::Value>,
    ) -> Result<frona::inference::provider::InferenceOutput, InferenceError> {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            cached_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning_tokens: 0,
        };
        let content = match self.next_response() {
            MockResponse::Text(t) => {
                let _ = token_tx.send(frona::inference::provider::StreamToken::Text(t.clone())).await;
                vec![AssistantContent::text(t)]
            }
            MockResponse::TextWithReasoning(text, reasoning) => {
                let _ = token_tx.send(frona::inference::provider::StreamToken::Reasoning(reasoning.clone())).await;
                let _ = token_tx.send(frona::inference::provider::StreamToken::Text(text.clone())).await;
                vec![
                    AssistantContent::Reasoning(rig_core::completion::message::Reasoning::new(&reasoning)),
                    AssistantContent::text(text),
                ]
            }
            MockResponse::ToolCalls(calls) => calls
                .into_iter()
                .map(|(id, name, args)| {
                    AssistantContent::ToolCall(ToolCall::new(id, ToolFunction::new(name, args)))
                })
                .collect(),
            MockResponse::Error(e) => return Err(e),
        };
        Ok(frona::inference::provider::InferenceOutput::new(content, usage))
    }

    async fn structured_inference(
        &self,
        _model_id: &str,
        _system_prompt: &str,
        _chat_history: Vec<RigMessage>,
        _schema: serde_json::Value,
        _max_tokens: Option<u64>,
        _temperature: Option<f64>,
        _additional_params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, InferenceError> {
        match self.next_response() {
            MockResponse::ToolCalls(mut calls) => {
                let (_id, _name, args) = calls
                    .pop()
                    .ok_or_else(|| InferenceError::InferenceFailed("mock: empty ToolCalls".into()))?;
                Ok(args)
            }
            MockResponse::Error(e) => Err(e),
            _ => Err(InferenceError::InferenceFailed(
                "mock structured_inference: queue head is not a ToolCalls response".into(),
            )),
        }
    }
}

pub struct MockInternalTool {
    pub tool_name: String,
    responses: Mutex<Vec<String>>,
}

impl MockInternalTool {
    pub fn new(name: &str, responses: Vec<String>) -> Self {
        Self {
            tool_name: name.to_string(),
            responses: Mutex::new(responses),
        }
    }
}

#[async_trait]
impl AgentTool for MockInternalTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            id: self.tool_name.clone(),
            provider_id: self.tool_name.clone(),
            description: format!("Mock tool {}", self.tool_name),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }]
    }

    async fn execute(
        &self,
        _tool_name: &str,
        _arguments: Value,
        _ctx: &InferenceContext,
    ) -> Result<ToolOutput, frona::core::error::AppError> {
        let mut responses = self.responses.lock().unwrap();
        let text = if responses.is_empty() {
            "mock result".to_string()
        } else {
            responses.remove(0)
        };
        Ok(ToolOutput::text(text))
    }
}

pub struct MockAttachmentTool {
    pub tool_name: String,
    pub attachment: frona::storage::Attachment,
}

impl MockAttachmentTool {
    pub fn new(name: &str, attachment: frona::storage::Attachment) -> Self {
        Self {
            tool_name: name.to_string(),
            attachment,
        }
    }
}

#[async_trait]
impl AgentTool for MockAttachmentTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            id: self.tool_name.clone(),
            provider_id: self.tool_name.clone(),
            description: format!("Attachment tool {}", self.tool_name),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }]
    }

    async fn execute(
        &self,
        _tool_name: &str,
        _arguments: Value,
        _ctx: &InferenceContext,
    ) -> Result<ToolOutput, frona::core::error::AppError> {
        Ok(ToolOutput::text("file produced").with_attachment(self.attachment.clone()))
    }
}

pub struct MockExternalTool {
    pub tool_name: String,
}

impl MockExternalTool {
    pub fn new(name: &str) -> Self {
        Self {
            tool_name: name.to_string(),
        }
    }
}

#[async_trait]
impl AgentTool for MockExternalTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            id: self.tool_name.clone(),
            provider_id: self.tool_name.clone(),
            description: format!("External tool {}", self.tool_name),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }]
    }

    async fn execute(
        &self,
        _tool_name: &str,
        _arguments: Value,
        _ctx: &InferenceContext,
    ) -> Result<ToolOutput, frona::core::error::AppError> {
        Ok(ToolOutput::text("external result").as_pending_external())
    }
}

pub struct MockFailingTool {
    pub tool_name: String,
}

impl MockFailingTool {
    pub fn new(name: &str) -> Self {
        Self {
            tool_name: name.to_string(),
        }
    }
}

#[async_trait]
impl AgentTool for MockFailingTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            id: self.tool_name.clone(),
            provider_id: self.tool_name.clone(),
            description: format!("Failing tool {}", self.tool_name),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }]
    }

    async fn execute(
        &self,
        _tool_name: &str,
        _arguments: Value,
        _ctx: &InferenceContext,
    ) -> Result<ToolOutput, frona::core::error::AppError> {
        Err(frona::core::error::AppError::Tool("tool failed".into()))
    }
}

pub fn mock_context() -> InferenceContext {
    let broadcast = frona::chat::broadcast::BroadcastService::new();
    let event_sender = broadcast.create_event_sender("test-user", "test-chat", None);
    InferenceContext::new(
        frona::auth::User {
            id: "test-user".into(),
            handle: frona::handle!("testuser"),
            email: "test@test.com".into(),
            name: "Test".into(),
            password_hash: String::new(),
            timezone: None,
            groups: Vec::new(),
            deactivated_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        frona::agent::models::Agent {
            id: "test-agent".into(),
            user_id: "test-user".into(),
            handle: frona::handle!("test-agent"),
            name: "Test Agent".into(),
            description: String::new(),
            model_group: "primary".into(),
            enabled: true,
            skills: None,
            sandbox_limits: None,
            max_concurrent_tasks: None,
            avatar: None,
            identity: Default::default(),
            prompt: None,
            heartbeat_interval: None,
            next_heartbeat_at: None,
            heartbeat_chat_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        frona::chat::models::Chat {
            id: "test-chat".into(),
            user_id: "test-user".into(),
            space_id: None,
            task_id: None,
            agent_id: "test-agent".into(),
            title: None,
            archived_at: None,
            channel_id: None,
            channel_external_id: None,
            metadata: Default::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        event_sender,
        tokio_util::sync::CancellationToken::new(),
        tokio_util::sync::CancellationToken::new(),
    )
}

pub fn test_model_group() -> ModelGroup {
    ModelGroup {
        name: "test".into(),
        main: ModelRef { provider: "mock".into(), model_id: "test-model".into(), additional_params: None },
        fallbacks: vec![],
        max_tokens: Some(4096),
        temperature: None,
        context_window: 128_000,
        retry: RetryConfig {
            max_retries: 1,
            initial_backoff_ms: 1,
            backoff_multiplier: 1.0,
            max_backoff_ms: 10,
        },
        inference: Default::default(),
    }
}

pub fn test_model_group_with_fallback(fallback_provider: &str, fallback_model: &str) -> ModelGroup {
    let mut group = test_model_group();
    group.fallbacks.push(ModelRef {
        provider: fallback_provider.into(),
        model_id: fallback_model.into(),
        additional_params: None,
    });
    group
}

/// Backwards-compatible name kept for the many call sites that pre-date the
/// service refactor. Returns an `UsageService` backed by a
/// process-wide in-memory DB so tests that don't assert on the table just work.
///
/// The DB is created on a **separate worker thread** so we don't trip
/// tokio's "cannot start a runtime from within a runtime" guard — every
/// `#[tokio::test]` call site already lives inside a runtime, and nested
/// `block_on` panics there.
pub fn test_metrics_ctx() -> frona::inference::usage::UsageService {
    use std::sync::OnceLock;
    static TEST_DB: OnceLock<Surreal<Db>> = OnceLock::new();
    let db = TEST_DB.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("test runtime");
            let db = rt.block_on(async {
                let db = Surreal::new::<surrealdb::engine::local::Mem>(())
                    .await
                    .expect("test db");
                frona::db::init::setup_schema(&db).await.expect("schema");
                db
            });
            tx.send(db).expect("send db back");
        });
        rx.recv().expect("recv db")
    });
    test_usage_service(db)
}

pub fn test_registry_with_provider(
    name: &str,
    provider: Arc<dyn ModelProvider>,
) -> ModelProviderRegistry {
    let mut providers = HashMap::new();
    providers.insert(name.to_string(), provider);
    let model_groups = HashMap::new();
    ModelProviderRegistry::for_testing(providers, model_groups)
}

pub fn init_metrics() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        metrics::setup_metrics_recorder();
    });
}

/// An SSE frame received from the broadcast dispatcher, parsed back into
/// event name + JSON data for assertion.
pub struct SseFrame {
    pub event: String,
    pub data: Value,
}

/// Convert an axum SSE `Event` to its wire-format string by running it
/// through a one-shot Sse body, the same way axum itself serializes events.
async fn event_to_string(event: axum::response::sse::Event) -> String {
    use axum::response::sse::Sse;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    let stream = futures::stream::once(async { Ok::<_, std::convert::Infallible>(event) });
    let sse = Sse::new(stream);
    let response = sse.into_response();
    let body = response.into_body();
    let collected = body.collect().await.unwrap();
    String::from_utf8(collected.to_bytes().to_vec()).unwrap()
}

/// Parse an SSE wire-format string into field name/value pairs, using the
/// same approach as axum's own test suite.
fn parse_sse_text(payload: &str) -> Option<SseFrame> {
    let mut event_name = String::new();
    let mut data_parts = Vec::new();

    for line in payload.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim_start();
            match key {
                "event" => event_name = value.to_string(),
                "data" => data_parts.push(value.to_string()),
                _ => {}
            }
        }
    }

    if event_name.is_empty() {
        return None;
    }

    let joined = data_parts.join("\n");
    let data: Value = serde_json::from_str(&joined).unwrap_or(Value::Null);

    Some(SseFrame { event: event_name, data })
}

/// Parse a single axum SSE `Event` into an `SseFrame`.
pub async fn parse_sse_frame(event: axum::response::sse::Event) -> Option<SseFrame> {
    let text = event_to_string(event).await;
    parse_sse_text(&text)
}

/// Drain all pending SSE events from a receiver, parse each into `SseFrame`.
pub async fn drain_sse_frames(
    rx: &mut mpsc::UnboundedReceiver<Result<axum::response::sse::Event, std::convert::Infallible>>,
) -> Vec<SseFrame> {
    let mut frames = Vec::new();
    while let Ok(Ok(event)) = rx.try_recv() {
        if let Some(frame) = parse_sse_frame(event).await {
            frames.push(frame);
        }
    }
    frames
}

/// Create a minimal ChatService backed by an in-memory SurrealDB for tool loop tests.
pub async fn test_chat_service() -> frona::chat::service::ChatService {
    use frona::db::repo::generic::SurrealRepo;
    use surrealdb::engine::local::Mem;
    use surrealdb::Surreal;

    let db = Surreal::new::<Mem>(()).await.unwrap();
    frona::db::init::setup_schema(&db).await.unwrap();

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

    let storage = frona::storage::StorageService::new(&config);
    let resource_manager = std::sync::Arc::new(
        frona::tool::sandbox::driver::resource_monitor::SystemResourceManager::new(80.0, 80.0, 90.0, 90.0),
    );
    let user_service = frona::auth::UserService::new(
        SurrealRepo::new(db.clone()),
        &config.cache,
    );
    let agent_service = frona::agent::service::AgentService::new(
        SurrealRepo::new(db.clone()),
        &config.cache,
        resource_manager.clone(),
        test_policy_service(&db),
        user_service.clone(),
    );
    let provider_registry = frona::inference::registry::ModelProviderRegistry::for_testing(
        HashMap::new(),
        HashMap::new(),
    );

    let usage_service = test_usage_service(&db);
    let memory_service = frona::memory::service::MemoryService::new(
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        std::sync::Arc::new(provider_registry.clone()),
        frona::agent::prompt::PromptLoader::new(&base),
        storage.clone(),
        usage_service.clone(),
    );

    let keypair_repo: SurrealRepo<frona::credential::keypair::models::KeyPair> =
        SurrealRepo::new(db.clone());
    let keypair_service = frona::credential::keypair::service::KeyPairService::new(
        &config.auth.encryption_secret,
        std::sync::Arc::new(keypair_repo),
    );
    let presign_service = frona::credential::presign::PresignService::new(
        keypair_service,
        user_service.clone(),
        "http://localhost:0".to_string(),
        300,
    );

    frona::chat::service::ChatService::new(
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        SurrealRepo::new(db.clone()),
        agent_service,
        provider_registry,
        storage,
        user_service,
        memory_service,
        frona::agent::prompt::PromptLoader::new(&base),
        frona::chat::broadcast::BroadcastService::new(),
        presign_service,
        usage_service,
    )
}

/// Create an `EventSender` backed by a real `BroadcastService` with a
/// registered SSE session, returning both the sender and the SSE receiver.
/// This exercises the full production path: serialize → dispatch → fan-out.
pub async fn test_event_sender() -> (
    frona::chat::broadcast::EventSender,
    mpsc::UnboundedReceiver<Result<axum::response::sse::Event, std::convert::Infallible>>,
    frona::chat::broadcast::BroadcastService,
) {
    let broadcast = frona::chat::broadcast::BroadcastService::new();
    let event_sender = broadcast.create_event_sender("test-user", "test-chat", None);

    let (tx, rx) = mpsc::unbounded_channel();
    broadcast.register_session("test-user", tx).await;

    (event_sender, rx, broadcast)
}
