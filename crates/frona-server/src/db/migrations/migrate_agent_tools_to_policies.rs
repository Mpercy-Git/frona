use surrealdb::Surreal;
use surrealdb::engine::local::Db;

use frona_derive::migration;

const NAMESPACE: &str = "Policy";

#[migration("2026-04-24T00:00:00Z")]
async fn migrate_agent_tools_to_policies(db: &Surreal<Db>) -> Result<(), surrealdb::Error> {
    let mut result = db
        .query("SELECT meta::id(id) as id, user_id, tools FROM agent WHERE tools IS NOT NONE AND array::len(tools) > 0")
        .await?;

    let agents: Vec<serde_json::Value> = result.take(0)?;

    for agent_val in agents {
        let Some(agent_id) = agent_val.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(user_id) = agent_val.get("user_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let tools: Vec<String> = agent_val
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if tools.is_empty() {
            continue;
        }

        let policy_name = format!("{agent_id}-migrated-tools");

        let existing: Option<serde_json::Value> = db
            .query("SELECT VALUE id FROM policy WHERE user_id = $user_id AND name = $name LIMIT 1")
            .bind(("user_id", user_id.to_string()))
            .bind(("name", policy_name.clone()))
            .await?
            .take(0)?;

        if existing.is_some() {
            continue;
        }

        let mut statements = Vec::new();
        for tool in &tools {
            statements.push(format!(
                "permit(\n  principal == {NAMESPACE}::Agent::\"{agent_id}\",\n  action == {NAMESPACE}::Action::\"invoke_tool\",\n  resource in {NAMESPACE}::ToolGroup::\"{tool}\"\n);"
            ));
        }

        let policy_text = format!(
            "@id(\"{policy_name}\")\n@description(\"Migrated tool permits for {agent_id}\")\n{}",
            statements.join("\n\n")
        );

        let now = chrono::Utc::now();
        db.query(
            "CREATE policy CONTENT {
                id: $id,
                user_id: $user_id,
                name: $name,
                description: $description,
                policy_text: $policy_text,
                enabled: true,
                created_at: $now,
                updated_at: $now,
            }"
        )
        .bind(("id", crate::core::repository::new_id()))
        .bind(("user_id", user_id.to_string()))
        .bind(("name", policy_name))
        .bind(("description", format!("Migrated tool permits for {agent_id}")))
        .bind(("policy_text", policy_text))
        .bind(("now", now))
        .await?;

        tracing::info!(agent_id, user_id, tools = ?tools, "Migrated agent tools to Cedar policy");
    }

    db.query("UPDATE agent SET tools = [] WHERE tools IS NOT NONE AND array::len(tools) > 0")
        .await?;
    tracing::info!("Cleared agent.tools field after policy migration");

    Ok(())
}
