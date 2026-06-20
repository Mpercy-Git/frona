//! Translate persisted `tool_call.tool_data` blobs into the typed
//! `tool_call.hitl` / `tool_call.task_event` shapes. Walks every `tool_call`
//! row, projects the `tool_data` JSON onto the new fields if present, and
//! clears `tool_data`. Idempotent — rows whose new fields are already
//! populated, or that never had `tool_data`, are skipped.

use surrealdb::Surreal;
use surrealdb::engine::local::Db;

use frona_derive::migration;

#[migration("2026-06-01T00:00:00Z")]
async fn translate_tool_data_to_hitl(db: &Surreal<Db>) -> Result<(), surrealdb::Error> {
    let mut result = db
        .query(
            "SELECT meta::id(id) as id, chat_id, tool_data \
             FROM tool_call \
             WHERE tool_data IS NOT NONE AND hitl IS NONE AND task_event IS NONE",
        )
        .await?;
    let rows: Vec<serde_json::Value> = result.take(0)?;

    for row in rows {
        let Some(id) = row.get("id").and_then(|v| v.as_str()).map(str::to_string) else {
            continue;
        };
        let chat_id = row
            .get("chat_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let Some(td) = row.get("tool_data") else { continue };
        let Some(tag) = td.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        let data = td.get("data").cloned().unwrap_or(serde_json::Value::Null);

        let url = format!("/chats/{chat_id}");

        match tag {
            "HumanInTheLoop" => {
                let reason = data
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let debugger_url = data
                    .get("debugger_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let status = data
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending");
                let response_text = data.get("response").and_then(|v| v.as_str());
                let hitl = serde_json::json!({
                    "prompt": if debugger_url.is_empty() {
                        reason.clone()
                    } else {
                        format!("{reason}\n\nTake over: {debugger_url}")
                    },
                    "url": url,
                    "request": {
                        "type": "Takeover",
                        "data": { "reason": reason, "debugger_url": debugger_url },
                    },
                    "status": status,
                    "response": response_text.map(|s| serde_json::json!({
                        "type": "Choice",
                        "data": s,
                    })),
                    "delivery": serde_json::Value::Null,
                });
                update_with_hitl(db, &id, hitl).await?;
            }
            "Question" => {
                let question = data
                    .get("question")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let options: Vec<String> = data
                    .get("options")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let status = data
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending");
                let response_text = data.get("response").and_then(|v| v.as_str());
                let hitl = serde_json::json!({
                    "prompt": question,
                    "url": url,
                    "request": { "type": "Question", "data": { "options": options } },
                    "status": status,
                    "response": response_text.map(|s| serde_json::json!({
                        "type": "Choice",
                        "data": s,
                    })),
                    "delivery": serde_json::Value::Null,
                });
                update_with_hitl(db, &id, hitl).await?;
            }
            "VaultApproval" => {
                let query = data
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let reason = data
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let status = data
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending");
                // Legacy `response` was free-text — can't be translated into the
                // typed VaultGrant cleanly. `te.result` carries the same text
                // from the LLM's perspective; the typed `hitl.response` stays
                // None. Legacy `env_var_prefix` is dropped — the binding shape
                // (Prefix or Single { field }) is chosen by the user at
                // resolution time on the new path.
                let hitl = serde_json::json!({
                    "prompt": format!("Allow access to credential: {query}"),
                    "url": url,
                    "request": {
                        "type": "Credential",
                        "data": { "query": query, "reason": reason },
                    },
                    "status": status,
                    "response": serde_json::Value::Null,
                    "delivery": serde_json::Value::Null,
                });
                update_with_hitl(db, &id, hitl).await?;
            }
            "ServiceApproval" => {
                let action = data
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("deploy")
                    .to_string();
                let manifest = data.get("manifest").cloned().unwrap_or(serde_json::Value::Null);
                let previous_manifest = data.get("previous_manifest").cloned();
                let handle = manifest
                    .get("handle")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                let status = data
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending");
                let response_text = data.get("response").and_then(|v| v.as_str());
                let typed_response = response_text.map(|s| {
                    let approved = matches!(
                        s.to_ascii_lowercase().as_str(),
                        "approved" | "approve" | "yes" | "true",
                    );
                    serde_json::json!({ "type": "Approval", "data": approved })
                });
                let hitl = serde_json::json!({
                    "prompt": format!("{action} `{handle}`?"),
                    "url": url,
                    "request": {
                        "type": "App",
                        "data": {
                            "action": action,
                            "manifest": manifest,
                            "previous_manifest": previous_manifest,
                        },
                    },
                    "status": status,
                    "response": typed_response,
                    "delivery": serde_json::Value::Null,
                });
                update_with_hitl(db, &id, hitl).await?;
            }
            "TaskCompletion" => {
                let task_event = serde_json::json!({
                    "type": "Completion",
                    "data": {
                        "task_id": data.get("task_id").cloned().unwrap_or(serde_json::Value::Null),
                        "chat_id": data.get("chat_id").cloned().unwrap_or(serde_json::Value::Null),
                        "status": data.get("status").cloned().unwrap_or(serde_json::json!("completed")),
                        "summary": data.get("summary").cloned().unwrap_or(serde_json::Value::Null),
                        "deliverables": data.get("deliverables").cloned().unwrap_or(serde_json::json!([])),
                    },
                });
                update_with_task_event(db, &id, task_event).await?;
            }
            "TaskDeferred" => {
                let task_event = serde_json::json!({
                    "type": "Deferred",
                    "data": {
                        "task_id": data.get("task_id").cloned().unwrap_or(serde_json::Value::Null),
                        "delay_minutes": data.get("delay_minutes").cloned().unwrap_or(serde_json::json!(0)),
                        "reason": data.get("reason").cloned().unwrap_or(serde_json::json!("")),
                    },
                });
                update_with_task_event(db, &id, task_event).await?;
            }
            _ => {
                // Unknown tag — leave the row alone (forward-compat for any
                // shape we haven't anticipated).
                continue;
            }
        }
    }

    Ok(())
}

async fn update_with_hitl(
    db: &Surreal<Db>,
    id: &str,
    hitl: serde_json::Value,
) -> Result<(), surrealdb::Error> {
    db.query("UPDATE type::record('tool_call', $id) SET hitl = $hitl, tool_data = NONE")
        .bind(("id", id.to_string()))
        .bind(("hitl", hitl))
        .await?
        .check()?;
    Ok(())
}

async fn update_with_task_event(
    db: &Surreal<Db>,
    id: &str,
    task_event: serde_json::Value,
) -> Result<(), surrealdb::Error> {
    db.query(
        "UPDATE type::record('tool_call', $id) \
         SET task_event = $task_event, tool_data = NONE",
    )
    .bind(("id", id.to_string()))
    .bind(("task_event", task_event))
    .await?
    .check()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::engine::local::Mem;

    async fn mem_db() -> Surreal<Db> {
        let db = Surreal::new::<Mem>(()).await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        db
    }

    async fn create_legacy(
        db: &Surreal<Db>,
        chat_id: &str,
        tool_data: serde_json::Value,
    ) -> String {
        let now = chrono::Utc::now();
        let mut res = db
            .query(
                "CREATE tool_call CONTENT {
                    chat_id: $chat_id,
                    message_id: 'msg-1',
                    turn: 0,
                    provider_call_id: 'pc-1',
                    name: 'legacy',
                    arguments: {},
                    result: '',
                    success: true,
                    duration_ms: 0,
                    tool_data: $tool_data,
                    created_at: $now,
                } RETURN AFTER",
            )
            .bind(("chat_id", chat_id.to_string()))
            .bind(("tool_data", tool_data))
            .bind(("now", now))
            .await
            .unwrap();
        let row: serde_json::Value = res.take::<Option<serde_json::Value>>(0).unwrap().unwrap();
        row.get("id").and_then(|v| v.as_str()).map(str::to_string)
            .unwrap_or_else(|| {
                // SurrealDB returns IDs as objects sometimes; extract id field.
                row.get("id")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_default()
            })
    }

    async fn fetch(db: &Surreal<Db>) -> Vec<serde_json::Value> {
        let mut res = db
            .query(
                "SELECT meta::id(id) as id, tool_data, hitl, task_event, chat_id \
                 FROM tool_call",
            )
            .await
            .unwrap();
        res.take(0).unwrap()
    }

    #[tokio::test]
    async fn translates_question_to_hitl() {
        let db = mem_db().await;
        let _ = create_legacy(
            &db,
            "chat-1",
            serde_json::json!({
                "type": "Question",
                "data": {
                    "question": "Which region?",
                    "options": ["us", "eu"],
                    "status": "pending",
                    "response": null,
                },
            }),
        )
        .await;

        translate_tool_data_to_hitl(&db).await.unwrap();

        let rows = fetch(&db).await;
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(row.get("tool_data").is_none_or(|v| v.is_null()));
        let hitl = row.get("hitl").unwrap();
        assert_eq!(hitl.get("status").and_then(|v| v.as_str()), Some("pending"));
        assert_eq!(hitl.get("prompt").and_then(|v| v.as_str()), Some("Which region?"));
        let req = hitl.get("request").unwrap();
        assert_eq!(req.get("type").and_then(|v| v.as_str()), Some("Question"));
        let opts = req.get("data").and_then(|d| d.get("options")).and_then(|v| v.as_array()).unwrap();
        assert_eq!(opts.len(), 2);
    }

    #[tokio::test]
    async fn translates_vault_approval_to_vault_pick() {
        let db = mem_db().await;
        let _ = create_legacy(
            &db,
            "chat-1",
            serde_json::json!({
                "type": "VaultApproval",
                "data": {
                    "query": "postgres-prod",
                    "reason": "ETL",
                    "env_var_prefix": "DB",
                    "status": "pending",
                    "response": null,
                },
            }),
        )
        .await;

        translate_tool_data_to_hitl(&db).await.unwrap();

        let rows = fetch(&db).await;
        let req = rows[0].get("hitl").unwrap().get("request").unwrap();
        assert_eq!(req.get("type").and_then(|v| v.as_str()), Some("Credential"));
        assert_eq!(
            req.get("data").and_then(|d| d.get("query")).and_then(|v| v.as_str()),
            Some("postgres-prod"),
        );
    }

    #[tokio::test]
    async fn translates_service_approval_with_resolved_status() {
        let db = mem_db().await;
        let _ = create_legacy(
            &db,
            "chat-1",
            serde_json::json!({
                "type": "ServiceApproval",
                "data": {
                    "action": "deploy",
                    "manifest": {"handle": "notes", "name": "Notes"},
                    "previous_manifest": null,
                    "status": "resolved",
                    "response": "Approved",
                },
            }),
        )
        .await;

        translate_tool_data_to_hitl(&db).await.unwrap();

        let rows = fetch(&db).await;
        let hitl = rows[0].get("hitl").unwrap();
        assert_eq!(hitl.get("status").and_then(|v| v.as_str()), Some("resolved"));
        let resp = hitl.get("response").unwrap();
        assert_eq!(resp.get("type").and_then(|v| v.as_str()), Some("Approval"));
        assert_eq!(resp.get("data").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn translates_human_in_the_loop_to_takeover() {
        let db = mem_db().await;
        let _ = create_legacy(
            &db,
            "chat-1",
            serde_json::json!({
                "type": "HumanInTheLoop",
                "data": {
                    "reason": "Solve the captcha",
                    "debugger_url": "https://debugger.example/abc",
                    "status": "pending",
                    "response": null,
                },
            }),
        )
        .await;

        translate_tool_data_to_hitl(&db).await.unwrap();

        let rows = fetch(&db).await;
        let req = rows[0].get("hitl").unwrap().get("request").unwrap();
        assert_eq!(req.get("type").and_then(|v| v.as_str()), Some("Takeover"));
        let prompt = rows[0]
            .get("hitl")
            .and_then(|h| h.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(prompt.contains("Solve the captcha"));
        assert!(prompt.contains("debugger.example"));
    }

    #[tokio::test]
    async fn translates_task_completion_to_task_event() {
        let db = mem_db().await;
        let _ = create_legacy(
            &db,
            "chat-1",
            serde_json::json!({
                "type": "TaskCompletion",
                "data": {
                    "task_id": "task-1",
                    "chat_id": "chat-1",
                    "status": "completed",
                    "summary": "Done!",
                    "deliverables": [],
                },
            }),
        )
        .await;

        translate_tool_data_to_hitl(&db).await.unwrap();

        let rows = fetch(&db).await;
        assert!(rows[0].get("tool_data").is_none_or(|v| v.is_null()));
        let ev = rows[0].get("task_event").unwrap();
        assert_eq!(ev.get("type").and_then(|v| v.as_str()), Some("Completion"));
        let inner = ev.get("data").unwrap();
        assert_eq!(inner.get("task_id").and_then(|v| v.as_str()), Some("task-1"));
        assert_eq!(inner.get("summary").and_then(|v| v.as_str()), Some("Done!"));
    }

    #[tokio::test]
    async fn translates_task_deferred_to_task_event() {
        let db = mem_db().await;
        let _ = create_legacy(
            &db,
            "chat-1",
            serde_json::json!({
                "type": "TaskDeferred",
                "data": {
                    "task_id": "task-1",
                    "delay_minutes": 15,
                    "reason": "Try later",
                },
            }),
        )
        .await;

        translate_tool_data_to_hitl(&db).await.unwrap();

        let rows = fetch(&db).await;
        let ev = rows[0].get("task_event").unwrap();
        assert_eq!(ev.get("type").and_then(|v| v.as_str()), Some("Deferred"));
        let inner = ev.get("data").unwrap();
        assert_eq!(inner.get("delay_minutes").and_then(|v| v.as_u64()), Some(15));
    }

    #[tokio::test]
    async fn is_idempotent() {
        let db = mem_db().await;
        let _ = create_legacy(
            &db,
            "chat-1",
            serde_json::json!({
                "type": "Question",
                "data": {
                    "question": "?",
                    "options": [],
                    "status": "pending",
                    "response": null,
                },
            }),
        )
        .await;

        translate_tool_data_to_hitl(&db).await.unwrap();
        translate_tool_data_to_hitl(&db).await.unwrap();

        let rows = fetch(&db).await;
        assert_eq!(rows.len(), 1);
        assert!(rows[0].get("hitl").is_some());
        assert!(rows[0].get("tool_data").is_none_or(|v| v.is_null()));
    }

    #[tokio::test]
    async fn leaves_rows_with_no_tool_data_alone() {
        let db = mem_db().await;
        // Row already has hitl set, no tool_data.
        let now = chrono::Utc::now();
        db.query(
            "CREATE tool_call CONTENT {
                chat_id: 'chat-1',
                message_id: 'msg-1',
                turn: 0,
                provider_call_id: 'pc-1',
                name: 'native',
                arguments: {},
                result: '',
                success: true,
                duration_ms: 0,
                hitl: {
                    prompt: 'Already migrated',
                    url: '/chats/chat-1',
                    request: { type: 'Question', data: { options: [] } },
                    status: 'pending',
                    response: NONE,
                    delivery: NONE,
                },
                created_at: $now,
            }",
        )
        .bind(("now", now))
        .await
        .unwrap();

        translate_tool_data_to_hitl(&db).await.unwrap();

        let rows = fetch(&db).await;
        assert_eq!(rows.len(), 1);
        let prompt = rows[0]
            .get("hitl")
            .and_then(|h| h.get("prompt"))
            .and_then(|v| v.as_str());
        assert_eq!(prompt, Some("Already migrated"));
    }
}
