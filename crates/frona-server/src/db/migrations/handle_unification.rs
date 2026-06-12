//! Unifies the `Handle` identity model across User, Agent, App, McpServer,
//! and Channel: renames `user.username` → `user.handle`, materializes per-user
//! clones of built-in agents, backfills handles on App/Mcp/Channel, rewrites
//! Cedar policy text from UUIDs to `{user_handle}/{entity_handle}`, and moves
//! on-disk state into `{data_dir}/users/{user_handle}/{subsystem}/...`.
//!
//! The built-in handle set is frozen here so the migration is reproducible
//! regardless of later changes to `resources/agents/`.

use std::collections::{HashMap, HashSet};

use surrealdb::Surreal;
use surrealdb::engine::local::Db;

use frona_derive::migration;

use crate::core::Handle;

const BUILTIN_HANDLES: &[&str] = &["developer", "researcher", "receptionist", "system"];

#[migration("2026-05-26T12:00:00Z")]
async fn handle_unification(db: &Surreal<Db>) -> Result<(), surrealdb::Error> {
    rename_user_username_to_handle(db).await?;
    let user_handle_by_id = collect_user_handles(db).await?;
    let agents = clone_builtin_agents_per_user(db).await?;
    rewrite_agent_fks(db, &agents).await;
    let (app_rows, app_handle_by_id) = backfill_app_handles(db).await?;
    let (mcp_rows, mcp_handle_by_id) = backfill_mcp_handles(db).await?;
    let (channel_rows, channel_handle_by_id) = backfill_channel_handles(db).await?;
    drop_mcp_slug_column(db).await;
    assert_unique_handles(db, "agent").await?;
    assert_unique_handles(db, "app").await?;
    assert_unique_handles(db, "mcp_server").await?;
    assert_unique_handles(db, "channel").await?;
    rewrite_cedar_policy_text(
        db,
        &user_handle_by_id,
        &app_rows,
        &app_handle_by_id,
        &mcp_rows,
        &mcp_handle_by_id,
        &channel_rows,
        &channel_handle_by_id,
    )
    .await?;
    rewrite_notifications(db, &app_handle_by_id).await;
    relocate_per_user_dirs_to_new_layout(db, &user_handle_by_id, &mcp_rows, &mcp_handle_by_id).await;
    relocate_channel_dirs(&user_handle_by_id, &channel_rows, &channel_handle_by_id).await;
    prune_empty_legacy_dirs();
    drop_app_manifest_id(db, &user_handle_by_id).await?;
    normalize_agent_null_options(db).await?;

    Ok(())
}

async fn normalize_agent_null_options(db: &Surreal<Db>) -> Result<(), surrealdb::Error> {
    for field in ["sandbox_limits", "prompt", "skills"] {
        let stmt = format!("UPDATE agent SET {field} = NONE WHERE {field} = NULL");
        db.query(stmt).await?.check()?;
    }
    Ok(())
}

async fn rename_user_username_to_handle(db: &Surreal<Db>) -> Result<(), surrealdb::Error> {
    db.query(
        "UPDATE user SET handle = username, username = NONE \
         WHERE username IS NOT NONE AND (handle IS NONE OR handle = '')",
    )
    .await?
    .check()?;

    db.query("REMOVE INDEX IF EXISTS unique_username ON TABLE user")
        .await?
        .check()?;

    let mut leftover_result = db
        .query("SELECT count() AS count FROM user WHERE handle IS NONE OR handle = '' GROUP ALL")
        .await?;
    let leftover_rows: Vec<serde_json::Value> = leftover_result.take(0)?;
    let leftover = leftover_rows
        .first()
        .and_then(|v| v.get("count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if leftover > 0 {
        return Err(surrealdb::Error::thrown(format!(
            "Migration left {leftover} user rows without a `handle`; aborting"
        )));
    }
    Ok(())
}

async fn collect_user_handles(
    db: &Surreal<Db>,
) -> Result<HashMap<String, String>, surrealdb::Error> {
    let mut user_result = db
        .query("SELECT meta::id(id) AS id, handle FROM user")
        .await?;
    let user_rows: Vec<serde_json::Value> = user_result.take(0)?;
    Ok(user_rows
        .iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?.to_string();
            let handle = row.get("handle")?.as_str()?.to_string();
            Some((id, handle))
        })
        .collect())
}

/// `(user_id, builtin_handle) → agent.id` for every user post-migration.
/// First active user inherits the shared row (id stays as the handle string);
/// later users get fresh clones with UUID ids.
type AgentByOwner = HashMap<(String, String), String>;

async fn clone_builtin_agents_per_user(
    db: &Surreal<Db>,
) -> Result<AgentByOwner, surrealdb::Error> {
    // SurrealDB 3.x `IS NONE` is broken — use `type::is_none(…)` to cover both NULL and NONE.
    db.query("UPDATE agent SET handle = meta::id(id) WHERE type::is_none(handle)")
        .await?
        .check()?;

    let user_rows: Vec<serde_json::Value> = db
        .query(
            "SELECT meta::id(id) AS id, created_at FROM user \
             WHERE deactivated_at IS NONE ORDER BY created_at ASC",
        )
        .await?
        .take(0)?;
    let user_ids: Vec<String> = user_rows
        .into_iter()
        .filter_map(|r| r.get("id").and_then(|v| v.as_str().map(String::from)))
        .collect();

    let mut map: AgentByOwner = HashMap::new();
    let now = chrono::Utc::now();

    for handle in BUILTIN_HANDLES {
        let mut owners = user_ids.iter();

        // First active user inherits the shared row to avoid FK churn.
        let shared_id: Option<String> = db
            .query("SELECT VALUE meta::id(id) FROM agent \
                    WHERE handle = $h AND type::is_none(user_id) LIMIT 1")
            .bind(("h", handle.to_string()))
            .await?
            .take(0)?;
        if let Some(shared_id) = shared_id
            && let Some(first) = owners.next()
        {
            db.query("UPDATE type::record('agent', $id) \
                      SET user_id = $u, updated_at = $now")
                .bind(("id", shared_id.clone()))
                .bind(("u", first.clone()))
                .bind(("now", now))
                .await?
                .check()?;
            map.insert((first.clone(), handle.to_string()), shared_id);
        }

        for user_id in owners {
            if let Some(existing) = db
                .query("SELECT VALUE meta::id(id) FROM agent \
                        WHERE handle = $h AND user_id = $u LIMIT 1")
                .bind(("h", handle.to_string()))
                .bind(("u", user_id.clone()))
                .await?
                .take::<Option<String>>(0)?
            {
                map.insert((user_id.clone(), handle.to_string()), existing);
                continue;
            }
            let new_id = clone_builtin_for_user(db, user_id, handle, now).await?;
            map.insert((user_id.clone(), handle.to_string()), new_id);
        }
    }

    Ok(map)
}

async fn clone_builtin_for_user(
    db: &Surreal<Db>,
    user_id: &str,
    handle: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, surrealdb::Error> {
    fn opt(row: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
        row.get(key).cloned().filter(|v| !v.is_null())
    }

    // Shared template was promoted to first user; copy fields off any row with the same handle.
    let donor: Option<serde_json::Value> = db
        .query("SELECT * FROM agent WHERE handle = $h LIMIT 1")
        .bind(("h", handle.to_string()))
        .await?
        .take(0)?;
    let (description, model_group, skills, prompt, identity, sandbox_limits) = match donor {
        Some(row) => (
            row.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            row.get("model_group").and_then(|v| v.as_str()).unwrap_or("primary").to_string(),
            opt(&row, "skills"),
            opt(&row, "prompt"),
            row.get("identity").cloned().unwrap_or_else(|| serde_json::json!({})),
            opt(&row, "sandbox_limits"),
        ),
        None => (String::new(), "primary".into(), None, None, serde_json::json!({}), None),
    };

    let new_id = crate::core::repository::new_id();
    db.query(
        "CREATE agent CONTENT { \
            id: $id, user_id: $user_id, handle: $handle, name: $handle, \
            description: $description, model_group: $model_group, enabled: true, \
            skills: $skills, prompt: $prompt, identity: $identity, \
            sandbox_limits: $sandbox_limits, created_at: $now, updated_at: $now }",
    )
    .bind(("id", new_id.clone()))
    .bind(("user_id", user_id.to_string()))
    .bind(("handle", handle.to_string()))
    .bind(("description", description))
    .bind(("model_group", model_group))
    .bind(("skills", skills))
    .bind(("prompt", prompt))
    .bind(("identity", identity))
    .bind(("sandbox_limits", sandbox_limits))
    .bind(("now", now))
    .await?
    .check()?;
    tracing::info!(user_id, handle, "Cloned built-in agent for additional user");
    Ok(new_id)
}

/// For each (user_id, builtin_handle) where the user got a fresh clone, redirect
/// rows that still point at the legacy shared id (the handle string itself).
/// First-user inherited rows are no-ops because the id didn't change.
async fn rewrite_agent_fks(db: &Surreal<Db>, agents: &AgentByOwner) {
    const DIRECT: &[&str] = &["app", "chat", "task", "memory_entry", "channel"];

    for ((user_id, handle), new_id) in agents {
        if new_id == handle {
            continue;
        }
        for table in DIRECT {
            let _ = db
                .query(format!(
                    "UPDATE {table} SET agent_id = $new \
                     WHERE agent_id = $old AND user_id = $u"
                ))
                .bind(("new", new_id.clone()))
                .bind(("old", handle.clone()))
                .bind(("u", user_id.clone()))
                .await;
        }
        let _ = db
            .query(
                "UPDATE message SET agent_id = $new \
                 WHERE agent_id = $old AND chat_id IN \
                 (SELECT VALUE meta::id(id) FROM chat WHERE user_id = $u)",
            )
            .bind(("new", new_id.clone()))
            .bind(("old", handle.clone()))
            .bind(("u", user_id.clone()))
            .await;
    }
}


async fn backfill_app_handles(
    db: &Surreal<Db>,
) -> Result<(Vec<serde_json::Value>, HashMap<String, String>), surrealdb::Error> {
    let mut app_result = db
        .query("SELECT meta::id(id) AS id, user_id, name, handle, created_at \
                FROM app ORDER BY created_at ASC")
        .await?;
    let app_rows: Vec<serde_json::Value> = app_result.take(0)?;

    let mut taken_by_user: HashMap<String, HashSet<String>> = HashMap::new();
    for row in &app_rows {
        if let (Some(user_id), Some(handle)) = (
            row.get("user_id").and_then(|v| v.as_str()),
            row.get("handle").and_then(|v| v.as_str()),
        ) {
            taken_by_user
                .entry(user_id.to_string())
                .or_default()
                .insert(handle.to_string());
        }
    }

    let mut app_handle_by_id: HashMap<String, String> = HashMap::new();
    for row in &app_rows {
        let Some(app_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        if let Some(existing) = row.get("handle").and_then(|v| v.as_str())
            && !existing.is_empty()
        {
            app_handle_by_id.insert(app_id.to_string(), existing.to_string());
            continue;
        }
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("app");

        let base = crate::tool::mcp::sanitize_to_handle(name);
        let taken = taken_by_user.entry(user_id.to_string()).or_default();
        let final_handle = unique_with_suffix(base, taken);

        db.query("UPDATE type::record('app', $id) SET handle = $handle")
            .bind(("id", app_id.to_string()))
            .bind(("handle", final_handle.as_str().to_string()))
            .await?
            .check()?;
        tracing::warn!(
            app_id, user_id, name, handle = final_handle.as_str(),
            "backfilled app handle from name"
        );
        taken.insert(final_handle.as_str().to_string());
        app_handle_by_id.insert(app_id.to_string(), final_handle.as_str().to_string());
    }
    Ok((app_rows, app_handle_by_id))
}

async fn backfill_mcp_handles(
    db: &Surreal<Db>,
) -> Result<(Vec<serde_json::Value>, HashMap<String, String>), surrealdb::Error> {
    let mut mcp_result = db
        .query("SELECT meta::id(id) AS id, user_id, slug, handle, workspace_dir, installed_at \
                FROM mcp_server ORDER BY installed_at ASC")
        .await?;
    let mcp_rows: Vec<serde_json::Value> = mcp_result.take(0).unwrap_or_default();

    let mut taken_by_user: HashMap<String, HashSet<String>> = HashMap::new();
    for row in &mcp_rows {
        if let (Some(user_id), Some(handle)) = (
            row.get("user_id").and_then(|v| v.as_str()),
            row.get("handle").and_then(|v| v.as_str()),
        ) {
            taken_by_user
                .entry(user_id.to_string())
                .or_default()
                .insert(handle.to_string());
        }
    }

    let mut mcp_handle_by_id: HashMap<String, String> = HashMap::new();
    for row in &mcp_rows {
        let Some(mcp_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        if let Some(existing) = row.get("handle").and_then(|v| v.as_str())
            && !existing.is_empty()
        {
            mcp_handle_by_id.insert(mcp_id.to_string(), existing.to_string());
            continue;
        }
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let slug = row.get("slug").and_then(|v| v.as_str()).unwrap_or("mcp");

        let base = crate::tool::mcp::sanitize_to_handle(slug);
        let taken = taken_by_user.entry(user_id.to_string()).or_default();
        let final_handle = unique_with_suffix(base, taken);

        db.query("UPDATE type::record('mcp_server', $id) SET handle = $handle")
            .bind(("id", mcp_id.to_string()))
            .bind(("handle", final_handle.as_str().to_string()))
            .await?
            .check()?;
        tracing::warn!(
            mcp_id, user_id, old_slug = slug, handle = final_handle.as_str(),
            "backfilled mcp handle from slug"
        );
        taken.insert(final_handle.as_str().to_string());
        mcp_handle_by_id.insert(mcp_id.to_string(), final_handle.as_str().to_string());
    }
    Ok((mcp_rows, mcp_handle_by_id))
}

async fn drop_mcp_slug_column(db: &Surreal<Db>) {
    db.query("REMOVE FIELD slug ON mcp_server").await.ok();
}

async fn backfill_channel_handles(
    db: &Surreal<Db>,
) -> Result<(Vec<serde_json::Value>, HashMap<String, String>), surrealdb::Error> {
    let mut result = db
        .query(
            "SELECT meta::id(id) AS id, user_id, provider, space_id, handle, created_at \
             FROM channel ORDER BY created_at ASC",
        )
        .await?;
    let rows: Vec<serde_json::Value> = result.take(0).unwrap_or_default();

    let mut taken_by_user: HashMap<String, HashSet<String>> = HashMap::new();
    for row in &rows {
        if let (Some(user_id), Some(handle)) = (
            row.get("user_id").and_then(|v| v.as_str()),
            row.get("handle").and_then(|v| v.as_str()),
        ) {
            taken_by_user
                .entry(user_id.to_string())
                .or_default()
                .insert(handle.to_string());
        }
    }

    let mut handle_by_id: HashMap<String, String> = HashMap::new();
    for row in &rows {
        let Some(channel_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        if let Some(existing) = row.get("handle").and_then(|v| v.as_str())
            && !existing.is_empty()
        {
            handle_by_id.insert(channel_id.to_string(), existing.to_string());
            continue;
        }
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let provider = row.get("provider").and_then(|v| v.as_str()).unwrap_or("channel");

        let base = Handle::try_new(provider).unwrap_or_else(|_| {
            crate::tool::mcp::sanitize_to_handle(provider)
        });
        let taken = taken_by_user.entry(user_id.to_string()).or_default();
        let final_handle = unique_with_suffix(base, taken);

        db.query("UPDATE type::record('channel', $id) SET handle = $handle")
            .bind(("id", channel_id.to_string()))
            .bind(("handle", final_handle.as_str().to_string()))
            .await?
            .check()?;
        tracing::warn!(
            channel_id, user_id, provider, handle = final_handle.as_str(),
            "backfilled channel handle from provider"
        );
        taken.insert(final_handle.as_str().to_string());
        handle_by_id.insert(channel_id.to_string(), final_handle.as_str().to_string());
    }
    Ok((rows, handle_by_id))
}

fn prune_empty_legacy_dirs() {
    let data_dir = std::path::PathBuf::from(
        std::env::var("FRONA_SERVER_DATA_DIR").unwrap_or_else(|_| "data".into()),
    );
    let candidates = ["files", "channels", "workspaces"];
    for name in candidates {
        prune_empty_recursive(&data_dir.join(name));
    }
}

fn prune_empty_recursive(path: &std::path::Path) {
    if !path.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                prune_empty_recursive(&entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir(path);
}

async fn relocate_channel_dirs(
    user_handle_by_id: &HashMap<String, String>,
    channel_rows: &[serde_json::Value],
    channel_handle_by_id: &HashMap<String, String>,
) {
    let data_dir = std::path::PathBuf::from(
        std::env::var("FRONA_SERVER_DATA_DIR").unwrap_or_else(|_| "data".into()),
    );
    let users_root = data_dir.join("users");

    for row in channel_rows {
        let Some(channel_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let Some(provider) = row.get("provider").and_then(|v| v.as_str()) else { continue; };
        let Some(space_id) = row.get("space_id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_handle) = user_handle_by_id.get(user_id) else { continue; };
        let Some(channel_handle) = channel_handle_by_id.get(channel_id) else { continue; };

        let from = users_root
            .join(user_handle)
            .join("channels")
            .join(provider)
            .join(space_id);
        let to = users_root.join(user_handle).join("channels").join(channel_handle);
        if from == to {
            continue;
        }
        move_dir_contents(&from, &to);
    }
}

async fn assert_unique_handles(
    db: &Surreal<Db>,
    table: &str,
) -> Result<(), surrealdb::Error> {
    let query = format!(
        "SELECT user_id, handle, count() AS n FROM {table} \
         GROUP BY user_id, handle"
    );
    let mut result = db.query(&query).await?;
    let rows: Vec<serde_json::Value> = result.take(0).unwrap_or_default();
    for row in rows {
        let n = row.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
        if n > 1 {
            return Err(surrealdb::Error::thrown(format!(
                "{table} migration left duplicate (user_id, handle): {row}"
            )));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn rewrite_cedar_policy_text(
    db: &Surreal<Db>,
    user_handle_by_id: &HashMap<String, String>,
    app_rows: &[serde_json::Value],
    app_handle_by_id: &HashMap<String, String>,
    mcp_rows: &[serde_json::Value],
    mcp_handle_by_id: &HashMap<String, String>,
    channel_rows: &[serde_json::Value],
    channel_handle_by_id: &HashMap<String, String>,
) -> Result<(), surrealdb::Error> {
    // Legacy `Policy::Agent::"<template_handle>"` literals are rewritten per
    // policy owner below so multi-user setups don't collapse onto one clone.
    let mut agent_result = db
        .query("SELECT meta::id(id) AS id, user_id, handle FROM agent")
        .await?;
    let agent_rows: Vec<serde_json::Value> = agent_result.take(0)?;

    let mut agent_uid_by_key: HashMap<String, String> = HashMap::new();
    for row in &agent_rows {
        let Some(agent_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let Some(agent_handle) = row.get("handle").and_then(|v| v.as_str()) else { continue; };
        let Some(user_handle) = user_handle_by_id.get(user_id) else { continue; };
        agent_uid_by_key.insert(agent_id.to_string(), format!("{user_handle}/{agent_handle}"));
    }

    let mut app_uid_by_uuid: HashMap<String, String> = HashMap::new();
    for row in app_rows {
        let Some(app_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_handle) = user_handle_by_id.get(user_id) else { continue; };
        let Some(app_handle) = app_handle_by_id.get(app_id) else { continue; };
        app_uid_by_uuid.insert(app_id.to_string(), format!("{user_handle}/{app_handle}"));
    }
    let mut mcp_uid_by_uuid: HashMap<String, String> = HashMap::new();
    for row in mcp_rows {
        let Some(mcp_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_handle) = user_handle_by_id.get(user_id) else { continue; };
        let Some(mcp_handle) = mcp_handle_by_id.get(mcp_id) else { continue; };
        mcp_uid_by_uuid.insert(mcp_id.to_string(), format!("{user_handle}/{mcp_handle}"));
    }
    let mut channel_uid_by_uuid: HashMap<String, String> = HashMap::new();
    for row in channel_rows {
        let Some(channel_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_handle) = user_handle_by_id.get(user_id) else { continue; };
        let Some(channel_handle) = channel_handle_by_id.get(channel_id) else { continue; };
        channel_uid_by_uuid.insert(channel_id.to_string(), format!("{user_handle}/{channel_handle}"));
    }

    // Each policy is rewritten in its OWNER's handle context — UUID needles
    // use global maps, legacy bare-handle needles get the owner's prefix.
    let mut policy_result = db
        .query("SELECT meta::id(id) AS id, user_id, policy_text FROM policy")
        .await?;
    let policy_rows: Vec<serde_json::Value> = policy_result.take(0)?;
    let now = chrono::Utc::now();

    for row in &policy_rows {
        let Some(policy_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        let Some(policy_user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let Some(policy_user_handle) = user_handle_by_id.get(policy_user_id) else { continue; };
        let Some(original) = row.get("policy_text").and_then(|v| v.as_str()) else { continue; };

        let mut updated = original.to_string();
        for (uuid, new_uid) in &agent_uid_by_key {
            updated = updated.replace(
                &format!("Policy::Agent::\"{uuid}\""),
                &format!("Policy::Agent::\"{new_uid}\""),
            );
        }
        for (uuid, new_uid) in &app_uid_by_uuid {
            updated = updated.replace(
                &format!("Policy::App::\"{uuid}\""),
                &format!("Policy::App::\"{new_uid}\""),
            );
        }
        for (uuid, new_uid) in &mcp_uid_by_uuid {
            updated = updated.replace(
                &format!("Policy::Mcp::\"{uuid}\""),
                &format!("Policy::Mcp::\"{new_uid}\""),
            );
        }
        for (uuid, new_uid) in &channel_uid_by_uuid {
            updated = updated.replace(
                &format!("Policy::Channel::\"{uuid}\""),
                &format!("Policy::Channel::\"{new_uid}\""),
            );
        }
        for handle in BUILTIN_HANDLES {
            updated = updated.replace(
                &format!("Policy::Agent::\"{handle}\""),
                &format!("Policy::Agent::\"{policy_user_handle}/{handle}\""),
            );
        }

        if updated != original {
            db.query("UPDATE type::record('policy', $id) SET policy_text = $text, updated_at = $now")
                .bind(("id", policy_id.to_string()))
                .bind(("text", updated))
                .bind(("now", now))
                .await?;
            tracing::info!(policy_id, "Rewrote principal UIDs in policy_text");
        }
    }
    Ok(())
}

async fn rewrite_notifications(db: &Surreal<Db>, _app_handle_by_id: &HashMap<String, String>) {
    // App notifications carry `app_id` (now `app_handle`); drop them as
    // transient UI alerts rather than rewriting the externally-tagged shape.
    db.query("DELETE notification WHERE data.App IS NOT NONE")
        .await
        .ok();
}

/// Best-effort FS move into `{data_dir}/users/{user_handle}/{subsystem}/...`.
/// Skips missing sources and logs (does not abort) so operators can re-sync.
async fn relocate_per_user_dirs_to_new_layout(
    db: &Surreal<Db>,
    user_handle_by_id: &HashMap<String, String>,
    mcp_rows: &[serde_json::Value],
    mcp_handle_by_id: &HashMap<String, String>,
) {
    let data_dir = std::path::PathBuf::from(
        std::env::var("FRONA_SERVER_DATA_DIR").unwrap_or_else(|_| "data".into()),
    );
    let users_root = data_dir.join("users");

    for (user_id, user_handle) in user_handle_by_id {
        let user_root = users_root.join(user_handle);

        let old_files = data_dir.join("files").join(user_handle);
        let new_files = user_root.join("files");
        move_dir_contents(&old_files, &new_files);

        let old_channels = data_dir.join("channels");
        if old_channels.is_dir()
            && let Ok(provider_dirs) = std::fs::read_dir(&old_channels)
        {
            for entry in provider_dirs.flatten() {
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let provider = entry.file_name();
                let from = entry.path().join(user_handle);
                let to = user_root.join("channels").join(&provider);
                move_dir_contents(&from, &to);
            }
        }

        // User-managed vault was UUID-keyed under data/files/{user_id}/.
        let old_vault = data_dir.join("files").join(user_id);
        let new_vault = user_root.join("vault");
        move_dir_contents(&old_vault, &new_vault);
    }

    for row in mcp_rows {
        let Some(mcp_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_handle) = user_handle_by_id.get(user_id) else { continue; };
        let Some(mcp_handle) = mcp_handle_by_id.get(mcp_id) else { continue; };
        let new_dir = users_root.join(user_handle).join("mcps").join(mcp_handle);

        let old_dir = row.get("workspace_dir").and_then(|v| v.as_str());
        if let Some(old_dir) = old_dir
            && std::path::Path::new(old_dir).exists()
            && std::path::Path::new(old_dir) != new_dir
        {
            if let Some(parent) = new_dir.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::rename(old_dir, &new_dir) {
                Ok(()) => tracing::info!(
                    mcp_id, old_dir, new_dir = %new_dir.display(),
                    "moved mcp workspace dir to unified layout"
                ),
                Err(e) => tracing::warn!(
                    mcp_id, old_dir, new_dir = %new_dir.display(), error = %e,
                    "failed to move mcp workspace dir; operator must re-sync"
                ),
            }
        }

        // Always rewrite the persisted path, even if the FS move was skipped.
        let new_dir_str = new_dir.to_string_lossy().into_owned();
        let _ = db
            .query("UPDATE type::record('mcp_server', $id) SET workspace_dir = $dir")
            .bind(("id", mcp_id.to_string()))
            .bind(("dir", new_dir_str))
            .await;
    }

    let agent_rows: Vec<serde_json::Value> = db
        .query("SELECT meta::id(id) AS id, user_id, handle FROM agent")
        .await
        .ok()
        .and_then(|mut r| r.take(0).ok())
        .unwrap_or_default();
    for row in &agent_rows {
        let Some(agent_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()) else { continue; };
        let Some(handle) = row.get("handle").and_then(|v| v.as_str()) else { continue; };
        let Some(user_handle) = user_handle_by_id.get(user_id) else { continue; };

        // Only the inherited shared row (id == handle) owns the legacy
        // `workspaces/{handle}` dir; additional users' clones start empty.
        if agent_id != handle {
            continue;
        }

        let from = data_dir.join("workspaces").join(handle);
        let to = users_root.join(user_handle).join("agents").join(handle);
        move_dir_contents(&from, &to);
    }
}

fn move_dir_contents(from: &std::path::Path, to: &std::path::Path) {
    if !from.is_dir() {
        return;
    }
    if let Some(parent) = to.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if to.exists() {
        if let Ok(entries) = std::fs::read_dir(from) {
            for entry in entries.flatten() {
                let dest = to.join(entry.file_name());
                if dest.exists() {
                    tracing::warn!(
                        from = %entry.path().display(),
                        to = %dest.display(),
                        "destination exists; skipping move"
                    );
                    continue;
                }
                if let Err(e) = std::fs::rename(entry.path(), &dest) {
                    tracing::warn!(
                        from = %entry.path().display(),
                        to = %dest.display(),
                        error = %e,
                        "failed to move dir contents"
                    );
                }
            }
        }
        let _ = std::fs::remove_dir(from);
    } else {
        match std::fs::rename(from, to) {
            Ok(()) => tracing::info!(from = %from.display(), to = %to.display(), "relocated dir"),
            Err(e) => tracing::warn!(
                from = %from.display(),
                to = %to.display(),
                error = %e,
                "failed to relocate dir"
            ),
        }
    }
}

/// Strips the legacy `manifest.id` (now redundant with `handle`) and renames
/// `apps/{id}/` → `apps/{handle}/`. Leaving `manifest.id` would trigger
/// spurious re-approval prompts in `manage_app` diff comparison.
async fn drop_app_manifest_id(
    db: &Surreal<Db>,
    user_handle_by_id: &HashMap<String, String>,
) -> Result<(), surrealdb::Error> {
    let agent_handle_by_id = {
        let mut result = db
            .query("SELECT meta::id(id) AS id, handle FROM agent")
            .await?;
        let rows: Vec<serde_json::Value> = result.take(0).unwrap_or_default();
        rows.into_iter()
            .filter_map(|r| {
                let id = r.get("id")?.as_str()?.to_string();
                let handle = r.get("handle")?.as_str()?.to_string();
                Some((id, handle))
            })
            .collect::<HashMap<String, String>>()
    };

    let app_rows: Vec<serde_json::Value> = db
        .query("SELECT meta::id(id) AS id, user_id, agent_id, handle, manifest FROM app")
        .await?
        .take(0)
        .unwrap_or_default();

    let data_dir = std::path::PathBuf::from(
        std::env::var("FRONA_SERVER_DATA_DIR").unwrap_or_else(|_| "data".into()),
    );
    let users_root = data_dir.join("users");

    for row in &app_rows {
        let Some(app_id) = row.get("id").and_then(|v| v.as_str()) else { continue; };
        let Some(handle) = row.get("handle").and_then(|v| v.as_str()) else { continue; };
        let manifest_id = row
            .get("manifest")
            .and_then(|m| m.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !manifest_id.is_empty()
            && manifest_id != handle
            && let (Some(user_id), Some(agent_id)) = (
                row.get("user_id").and_then(|v| v.as_str()),
                row.get("agent_id").and_then(|v| v.as_str()),
            )
            && let (Some(user_handle), Some(agent_handle)) = (
                user_handle_by_id.get(user_id),
                agent_handle_by_id.get(agent_id),
            )
        {
            let agent_apps = users_root
                .join(user_handle)
                .join("agents")
                .join(agent_handle)
                .join("apps");
            let from = agent_apps.join(manifest_id);
            let to = agent_apps.join(handle);
            if from.exists() && from != to && !to.exists() {
                match std::fs::rename(&from, &to) {
                    Ok(()) => tracing::info!(
                        app_id,
                        from = %from.display(),
                        to = %to.display(),
                        "moved app source dir to handle-based layout"
                    ),
                    Err(e) => tracing::warn!(
                        app_id,
                        from = %from.display(),
                        to = %to.display(),
                        error = %e,
                        "failed to move app source dir; operator can re-sync"
                    ),
                }
            } else if to.exists() && from.exists() {
                tracing::warn!(
                    app_id,
                    from = %from.display(),
                    to = %to.display(),
                    "skipping app fs move: destination already exists"
                );
            }
        }

        // Inject `handle` so the supervisor restore path can deserialize
        // pre-PR rows into the now-required `AppManifest.handle` field.
        let Some(manifest) = row.get("manifest").cloned() else { continue; };
        let serde_json::Value::Object(mut obj) = manifest else { continue; };
        let removed_id = obj.remove("id").is_some();
        let inserted_handle = obj
            .insert("handle".into(), serde_json::Value::String(handle.to_string()))
            .is_none();
        if !removed_id && !inserted_handle {
            continue;
        }
        db.query("UPDATE type::record('app', $id) SET manifest = $manifest")
            .bind(("id", app_id.to_string()))
            .bind(("manifest", serde_json::Value::Object(obj)))
            .await?
            .check()?;
    }
    Ok(())
}

/// Find the smallest `-N` suffix that produces a handle not in `taken`,
/// truncated to fit Handle's 32-char limit.
fn unique_with_suffix(base: Handle, taken: &HashSet<String>) -> Handle {
    if !taken.contains(base.as_str()) {
        return base;
    }
    let base_str = base.as_str();
    for i in 2..1000u32 {
        let suffix = format!("-{i}");
        let max_base = 32usize.saturating_sub(suffix.len());
        let trimmed = base_str
            .get(..max_base.min(base_str.len()))
            .unwrap_or(base_str)
            .trim_end_matches(['-', '_']);
        let candidate = format!("{trimmed}{suffix}");
        if !taken.contains(&candidate)
            && let Ok(h) = Handle::try_new(&candidate)
        {
            return h;
        }
    }
    let uuid = crate::core::repository::new_id();
    let short = uuid.chars().take(6).collect::<String>();
    Handle::try_new(format!("mcp-{short}")).expect("uuid fallback always valid")
}

#[cfg(test)]
mod tests {
    use surrealdb::engine::local::Mem;
    use surrealdb::Surreal;

    async fn seeded_db() -> Surreal<surrealdb::engine::local::Db> {
        let db = Surreal::new::<Mem>(()).await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();

        let now = chrono::Utc::now();

        for (id, username) in [("u1", "alice"), ("u2", "bob")] {
            db.query(
                "CREATE type::record('user', $id) CONTENT { \
                    username: $username, email: $email, name: $username, \
                    password_hash: '', timezone: NONE, groups: [], \
                    deactivated_at: NONE, created_at: $now, updated_at: $now }",
            )
            .bind(("id", id.to_string()))
            .bind(("username", username.to_string()))
            .bind(("email", format!("{username}@x.test")))
            .bind(("now", now))
            .await
            .unwrap()
            .check()
            .unwrap();
        }

        db.query(
            "CREATE type::record('agent', 'developer') CONTENT { \
                user_id: NONE, name: 'developer', \
                description: 'shared developer template', model_group: 'primary', \
                enabled: true, identity: {}, \
                created_at: $now, updated_at: $now }",
        )
        .bind(("now", now))
        .await
        .unwrap()
        .check()
        .unwrap();

        db.query(
            "CREATE type::record('agent', 'custom-a') CONTENT { \
                user_id: 'u1', name: 'custom', \
                description: '', model_group: 'primary', enabled: true, \
                identity: {}, created_at: $now, updated_at: $now }",
        )
        .bind(("now", now))
        .await
        .unwrap()
        .check()
        .unwrap();

        for (id, user_id, name, ts) in [
            ("a1", "u1", "Notes", "2026-01-01T00:00:00Z"),
            ("a2", "u1", "Notes", "2026-01-02T00:00:00Z"),
            ("a3", "u2", "Dashboard", "2026-01-03T00:00:00Z"),
        ] {
            let ts: chrono::DateTime<chrono::Utc> = ts
                .parse::<chrono::DateTime<chrono::FixedOffset>>()
                .unwrap()
                .into();
            db.query(
                "CREATE type::record('app', $id) CONTENT { \
                    agent_id: 'agent-x', user_id: $user_id, name: $name, \
                    description: NONE, kind: 'service', command: 'true', \
                    static_dir: NONE, port: NONE, status: 'stopped', pid: NONE, \
                    manifest: { id: 'm', name: $name }, chat_id: 'c1', \
                    crash_fix_attempts: 0, last_accessed_at: NONE, \
                    created_at: $ts, updated_at: $ts }",
            )
            .bind(("id", id.to_string()))
            .bind(("user_id", user_id.to_string()))
            .bind(("name", name.to_string()))
            .bind(("ts", ts))
            .await
            .unwrap()
            .check()
            .unwrap();
        }

        for (id, user_id, slug, ts) in [
            ("m1", "u1", "_", "2026-02-01T00:00:00Z"),
            ("m2", "u1", "2024_tool", "2026-02-02T00:00:00Z"),
            (
                "m3",
                "u2",
                "io.github.taylorwilsdon/google_workspace_mcp",
                "2026-02-03T00:00:00Z",
            ),
        ] {
            let ts: chrono::DateTime<chrono::Utc> = ts
                .parse::<chrono::DateTime<chrono::FixedOffset>>()
                .unwrap()
                .into();
            db.query(
                "CREATE type::record('mcp_server', $id) CONTENT { \
                    user_id: $user_id, slug: $slug, display_name: $slug, \
                    description: NONE, repository_url: NONE, registry_id: NONE, \
                    server_info: NONE, package: { runtime: 'npm', name: 'x', version: '1' }, \
                    command: 'x', args: [], transports: [], active_transport: 'stdio', \
                    env: {}, status: 'stopped', tool_cache: [], \
                    workspace_dir: 'data/mcp-test/' + $slug, \
                    installed_at: $ts, last_started_at: NONE, updated_at: $ts }",
            )
            .bind(("id", id.to_string()))
            .bind(("user_id", user_id.to_string()))
            .bind(("slug", slug.to_string()))
            .bind(("ts", ts))
            .await
            .unwrap()
            .check()
            .unwrap();
        }

        for (id, name, text) in [
            (
                "p1",
                "agent-template",
                r#"permit(principal == Policy::Agent::"developer", action == Policy::Action::"invoke_tool", resource);"#,
            ),
            (
                "p2",
                "app-policy",
                r#"permit(principal == Policy::App::"a1", action == Policy::Action::"read", resource == Policy::Path::"/x");"#,
            ),
            (
                "p3",
                "mcp-policy",
                r#"permit(principal == Policy::Mcp::"m1", action == Policy::Action::"connect", resource);"#,
            ),
        ] {
            db.query(
                "CREATE type::record('policy', $id) CONTENT { \
                    user_id: 'u1', name: $name, policy_text: $text, \
                    enabled: true, created_at: $now, updated_at: $now }",
            )
            .bind(("id", id.to_string()))
            .bind(("name", name.to_string()))
            .bind(("text", text.to_string()))
            .bind(("now", now))
            .await
            .unwrap()
            .check()
            .unwrap();
        }

        db
    }

    #[tokio::test]
    async fn migration_renames_user_clones_builtins_backfills_handles_and_rewrites_cedar() {
        let db = seeded_db().await;
        super::handle_unification(&db).await.unwrap();

        let mut r = db
            .query("SELECT meta::id(id) AS id, handle, username FROM user ORDER BY id ASC")
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = r.take(0).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("handle").unwrap().as_str().unwrap(), "alice");
        assert!(rows[0].get("username").is_none() || rows[0].get("username").unwrap().is_null());

        let mut r = db
            .query("SELECT count() FROM agent WHERE user_id IS NONE GROUP ALL")
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = r.take(0).unwrap();
        let shared = rows.first().and_then(|v| v.get("count")).and_then(|v| v.as_u64()).unwrap_or(0);
        assert_eq!(shared, 0, "shared rows should be deleted");

        let mut r = db
            .query("SELECT VALUE handle FROM agent WHERE user_id = 'u1' ORDER BY handle ASC")
            .await
            .unwrap();
        let u1_handles: Vec<String> = r.take(0).unwrap();
        assert!(u1_handles.contains(&"developer".into()));
        assert!(u1_handles.contains(&"researcher".into()));
        assert!(u1_handles.contains(&"receptionist".into()));
        assert!(u1_handles.contains(&"system".into()));
        assert!(u1_handles.contains(&"custom-a".into()), "user-created agent kept its id as handle: {u1_handles:?}");

        let mut r = db
            .query("SELECT meta::id(id) AS id, handle, created_at FROM app ORDER BY created_at ASC")
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = r.take(0).unwrap();
        assert_eq!(rows[0].get("handle").unwrap().as_str().unwrap(), "notes");
        assert_eq!(rows[1].get("handle").unwrap().as_str().unwrap(), "notes-2");
        assert_eq!(rows[2].get("handle").unwrap().as_str().unwrap(), "dashboard");

        let mut r = db
            .query("SELECT meta::id(id) AS id, handle, installed_at FROM mcp_server ORDER BY installed_at ASC")
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = r.take(0).unwrap();
        let h0 = rows[0].get("handle").unwrap().as_str().unwrap();
        assert!(h0.starts_with("mcp-"), "expected uuid fallback, got {h0}");
        assert_eq!(rows[1].get("handle").unwrap().as_str().unwrap(), "m-2024_tool");
        let h2 = rows[2].get("handle").unwrap().as_str().unwrap();
        assert!(h2.len() <= 32);
        assert!(h2.starts_with("taylorwilsdon-google_workspace"), "got: {h2}");

        let mut r = db
            .query("SELECT policy_text FROM policy WHERE id = type::record('policy', 'p1')")
            .await.unwrap();
        let rows: Vec<serde_json::Value> = r.take(0).unwrap();
        let t = rows[0].get("policy_text").unwrap().as_str().unwrap();
        assert!(t.contains(r#"Policy::Agent::"alice/developer""#), "got: {t}");
        assert!(!t.contains(r#"Policy::Agent::"developer""#));

        let mut r = db
            .query("SELECT policy_text FROM policy WHERE id = type::record('policy', 'p2')")
            .await.unwrap();
        let rows: Vec<serde_json::Value> = r.take(0).unwrap();
        let t = rows[0].get("policy_text").unwrap().as_str().unwrap();
        assert!(t.contains(r#"Policy::App::"alice/notes""#), "got: {t}");

        let mut r = db
            .query("SELECT policy_text FROM policy WHERE id = type::record('policy', 'p3')")
            .await.unwrap();
        let rows: Vec<serde_json::Value> = r.take(0).unwrap();
        let t = rows[0].get("policy_text").unwrap().as_str().unwrap();
        assert!(t.contains(&format!(r#"Policy::Mcp::"alice/{h0}""#)), "got: {t}");
    }

    #[tokio::test]
    async fn migration_is_reentrant() {
        let db = seeded_db().await;
        super::handle_unification(&db).await.unwrap();
        super::handle_unification(&db).await.unwrap();
        let mut r = db.query("SELECT count() FROM app GROUP ALL").await.unwrap();
        let rows: Vec<serde_json::Value> = r.take(0).unwrap();
        assert_eq!(rows[0].get("count").unwrap().as_u64().unwrap(), 3);
    }
}
