use surrealdb::Surreal;
use surrealdb::engine::local::{Db, RocksDb};
use surrealdb::types::RecordId;
use tracing::info;

use crate::agent::config::parse_frontmatter;
use crate::agent::service::AgentService;
use crate::storage::StorageService;

pub async fn setup_schema(db: &Surreal<Db>) -> Result<(), surrealdb::Error> {
    db.use_ns("frona").use_db("frona").await?;

    db.query(
        "
        DEFINE TABLE IF NOT EXISTS user SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS unique_email ON TABLE user COLUMNS email UNIQUE;
        DEFINE INDEX IF NOT EXISTS unique_username ON TABLE user COLUMNS username UNIQUE;

        DEFINE TABLE IF NOT EXISTS user_group SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_user_group_name ON TABLE user_group COLUMNS name UNIQUE;

        DEFINE TABLE IF NOT EXISTS agent SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_agent_user ON TABLE agent COLUMNS user_id;

        DEFINE TABLE IF NOT EXISTS space SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_space_user ON TABLE space COLUMNS user_id;

        DEFINE TABLE IF NOT EXISTS chat SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_chat_user ON TABLE chat COLUMNS user_id;
        DEFINE INDEX IF NOT EXISTS idx_chat_space ON TABLE chat COLUMNS space_id;
        DEFINE INDEX IF NOT EXISTS idx_chat_channel ON TABLE chat COLUMNS channel_id;
        DEFINE INDEX IF NOT EXISTS idx_chat_channel_thread ON TABLE chat COLUMNS channel_id, channel_external_id UNIQUE;

        DEFINE TABLE IF NOT EXISTS message SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_message_chat ON TABLE message COLUMNS chat_id;
        DEFINE INDEX IF NOT EXISTS idx_message_delivery_due ON TABLE message COLUMNS delivery.state, delivery.next_attempt_at;

        DEFINE TABLE IF NOT EXISTS task SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_task_user ON TABLE task COLUMNS user_id;
        DEFINE INDEX IF NOT EXISTS idx_task_agent ON TABLE task COLUMNS agent_id;
        DEFINE INDEX IF NOT EXISTS idx_task_status ON TABLE task COLUMNS status;

        DEFINE TABLE IF NOT EXISTS credential SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_credential_user ON TABLE credential COLUMNS user_id;
        DEFINE INDEX IF NOT EXISTS idx_credential_user_provider ON TABLE credential COLUMNS user_id, provider;

        DEFINE TABLE IF NOT EXISTS memory SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_memory_source ON TABLE memory COLUMNS source_type, source_id;

        DEFINE TABLE IF NOT EXISTS memory_entry SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_memory_entry_agent ON TABLE memory_entry COLUMNS agent_id;
        DEFINE INDEX IF NOT EXISTS idx_memory_entry_user ON TABLE memory_entry COLUMNS user_id;

        DEFINE TABLE IF NOT EXISTS keypair SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_keypair_owner ON TABLE keypair COLUMNS owner UNIQUE;

        DEFINE TABLE IF NOT EXISTS api_token SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_api_token_user ON TABLE api_token COLUMNS user_id;
        DEFINE INDEX IF NOT EXISTS idx_api_token_pair ON TABLE api_token COLUMNS refresh_pair_id;

        DEFINE TABLE IF NOT EXISTS contact SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_contact_user ON TABLE contact COLUMNS user_id;
        DEFINE INDEX IF NOT EXISTS idx_contact_phone ON TABLE contact COLUMNS user_id, phone;
        DEFINE INDEX IF NOT EXISTS idx_contact_addresses ON TABLE contact COLUMNS user_id, addresses.provider, addresses.address UNIQUE;

        DEFINE TABLE IF NOT EXISTS oauth_identity SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_oauth_identity_sub ON TABLE oauth_identity COLUMNS external_sub UNIQUE;
        DEFINE INDEX IF NOT EXISTS idx_oauth_identity_user ON TABLE oauth_identity COLUMNS user_id;

        DEFINE TABLE IF NOT EXISTS notification SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_notification_user ON TABLE notification COLUMNS user_id;

        DEFINE TABLE IF NOT EXISTS policy SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_policy_user ON TABLE policy COLUMNS user_id;
        DEFINE INDEX IF NOT EXISTS idx_policy_user_name ON TABLE policy COLUMNS user_id, name UNIQUE;

        DEFINE TABLE IF NOT EXISTS call SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_call_chat ON TABLE call COLUMNS chat UNIQUE;

        DEFINE TABLE IF NOT EXISTS app SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_app_agent ON TABLE app COLUMNS agent_id;
        DEFINE INDEX IF NOT EXISTS idx_app_user ON TABLE app COLUMNS user_id;
        DEFINE INDEX IF NOT EXISTS idx_app_status ON TABLE app COLUMNS status;

        DEFINE TABLE IF NOT EXISTS mcp_server SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_mcp_server_user ON TABLE mcp_server COLUMNS user_id;
        DEFINE INDEX IF NOT EXISTS idx_mcp_server_status ON TABLE mcp_server COLUMNS status;
        DEFINE INDEX IF NOT EXISTS idx_mcp_server_user_slug ON TABLE mcp_server COLUMNS user_id, slug UNIQUE;

        DEFINE TABLE IF NOT EXISTS channel SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_channel_user ON TABLE channel COLUMNS user_id;
        DEFINE INDEX IF NOT EXISTS idx_channel_space ON TABLE channel COLUMNS space_id;
        DEFINE INDEX IF NOT EXISTS idx_channel_status ON TABLE channel COLUMNS status;
        DEFINE INDEX IF NOT EXISTS idx_channel_space_unique ON TABLE channel COLUMNS space_id UNIQUE;

        DEFINE TABLE IF NOT EXISTS vault_connection SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_vault_connection_user ON TABLE vault_connection COLUMNS user_id;

        DEFINE TABLE IF NOT EXISTS vault_grant SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_vault_grant_user ON TABLE vault_grant COLUMNS user_id;
        DEFINE INDEX IF NOT EXISTS idx_vault_grant_user_principal ON TABLE vault_grant COLUMNS user_id, principal.kind, principal.id;

        DEFINE TABLE IF NOT EXISTS principal_credential_binding SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_pcb_user_principal ON TABLE principal_credential_binding COLUMNS user_id, principal.kind, principal.id;
        DEFINE INDEX IF NOT EXISTS idx_pcb_lookup ON TABLE principal_credential_binding COLUMNS user_id, principal.kind, principal.id, query;
        DEFINE INDEX IF NOT EXISTS idx_pcb_chat ON TABLE principal_credential_binding COLUMNS scope.Chat.chat_id;

        DEFINE TABLE IF NOT EXISTS tool_call SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_tool_call_chat ON TABLE tool_call COLUMNS chat_id;
        DEFINE INDEX IF NOT EXISTS idx_tool_call_message ON TABLE tool_call COLUMNS message_id;

        DEFINE TABLE IF NOT EXISTS vault_access_log SCHEMALESS;
        DEFINE INDEX IF NOT EXISTS idx_vault_access_log_chat ON TABLE vault_access_log COLUMNS chat_id;
        DEFINE INDEX IF NOT EXISTS idx_vault_access_log_user ON TABLE vault_access_log COLUMNS user_id;

        DEFINE TABLE IF NOT EXISTS runtime_config SCHEMAFULL;
        DEFINE FIELD IF NOT EXISTS `key` ON runtime_config TYPE string;
        DEFINE FIELD IF NOT EXISTS `value` ON runtime_config TYPE string;
        DEFINE FIELD IF NOT EXISTS updated_at ON runtime_config TYPE datetime;
        DEFINE INDEX IF NOT EXISTS idx_runtime_config_key ON runtime_config FIELDS `key` UNIQUE;

        DEFINE EVENT IF NOT EXISTS cascade_delete_chat_messages ON TABLE chat
          WHEN $event = 'DELETE'
          THEN (DELETE FROM message WHERE chat_id = meta::id($before.id));

        DEFINE EVENT IF NOT EXISTS cascade_delete_chat_bindings ON TABLE chat
          WHEN $event = 'DELETE'
          THEN (DELETE FROM principal_credential_binding
                  WHERE scope.Chat.chat_id = meta::id($before.id));

        DEFINE EVENT IF NOT EXISTS cascade_delete_chat_tool_calls ON TABLE chat
          WHEN $event = 'DELETE'
          THEN (DELETE FROM tool_call WHERE chat_id = meta::id($before.id));

        DEFINE EVENT IF NOT EXISTS cascade_delete_task_chat ON TABLE task
          WHEN $event = 'DELETE' AND $before.chat_id IS NOT NONE
          THEN (DELETE type::record('chat', $before.chat_id));
        ",
    )
    .await?;

    Ok(())
}

pub async fn seed_config_agents(db: &Surreal<Db>, agent_service: &AgentService, storage: &StorageService) -> Result<(), surrealdb::Error> {
    let agent_ids = agent_service.builtin_agent_ids();
    info!(agents = ?agent_ids, "Builtin agent IDs from config");
    for agent_id in agent_ids {
        let rid = RecordId::new("agent", agent_id.as_str());
        let mut result = db
            .query("SELECT meta::id(id) as id FROM agent WHERE id = $id LIMIT 1")
            .bind(("id", rid))
            .await?;

        let existing: Option<serde_json::Value> = result.take(0)?;
        if existing.is_some() {
            continue;
        }

        let ws = storage.agent_workspace(&agent_id);
        let (description, model_group) = ws
            .read("AGENT.md")
            .map(|content| {
                let entry = parse_frontmatter(&content);
                let desc = entry.metadata.get("description").cloned().unwrap_or_default();
                let mg = entry.metadata.get("model_group").cloned().unwrap_or_else(|| "primary".to_string());
                (desc, mg)
            })
            .unwrap_or_default();

        db.query(
            "CREATE type::record('agent', $id) SET
                name = $id,
                description = $description,
                model_group = $model_group,
                enabled = true,
                skills = [],
                identity = {},
                created_at = time::now(),
                updated_at = time::now()"
        )
        .bind(("id", agent_id.clone()))
        .bind(("description", description))
        .bind(("model_group", model_group))
        .await?;

        info!(agent_id = %agent_id, "Seeded config agent into database");
    }

    Ok(())
}

pub async fn init(path: &str) -> Result<Surreal<Db>, surrealdb::Error> {
    info!("Initializing SurrealDB at {path}");

    let timeout = std::time::Duration::from_secs(60);
    let interval = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();

    let db = loop {
        match Surreal::new::<RocksDb>(path).await {
            Ok(db) => break db,
            Err(e) => {
                let elapsed = start.elapsed();
                if elapsed >= timeout {
                    tracing::error!("Failed to open database after {elapsed:.0?}: {e}");
                    std::process::exit(1);
                }
                tracing::warn!(
                    "Database locked, retrying ({:.0?} elapsed): {e}",
                    elapsed
                );
                tokio::time::sleep(interval).await;
            }
        }
    };

    setup_schema(&db).await?;
    crate::db::migrations::run_migrations(&db).await?;

    info!("SurrealDB schema initialized");
    Ok(db)
}
