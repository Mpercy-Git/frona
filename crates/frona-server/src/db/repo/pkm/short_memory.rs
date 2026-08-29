use super::*;

impl PkmRepo {
    pub async fn remember(
        &self,
        user_id: &str,
        chat_id: &str,
        content: &str,
    ) -> Result<(), AppError> {
        let now = Utc::now();
        let row = KnowledgeShortMemory {
            id: new_id(),
            user_id: user_id.to_string(),
            content: content.to_string(),
            created_at: now,
            last_accessed_at: now,
            source_chat_id: Some(chat_id.to_string()),
            validated: false,
        };
        let _: Option<surrealdb::types::Value> = self
            .db
            .create(("knowledge_short_memory", row.id.clone()))
            .content(row)
            .await
            .map_err(|e| Self::err("remember", e))?;
        Ok(())
    }

    pub async fn list_short_memory(
        &self,
        user_id: &str,
    ) -> Result<Vec<KnowledgeShortMemory>, AppError> {
        let mut q = self
            .db
            .query(format!(
                "{SELECT} FROM knowledge_short_memory WHERE user_id = $uid"
            ))
            .bind(("uid", user_id.to_string()))
            .await
            .map_err(|e| Self::err("list_sm", e))?;
        q.take(0).map_err(|e| Self::err("list_sm_take", e))
    }

    pub async fn delete_short_memory(&self, id: &str) -> Result<(), AppError> {
        let _: Option<surrealdb::types::Value> = self
            .db
            .delete(("knowledge_short_memory", id))
            .await
            .map_err(|e| Self::err("delete_sm", e))?;
        Ok(())
    }

    /// Short memories from a chat not yet consolidated into the wiki (fed into the
    /// next consolidation pass).
    pub async fn unconsolidated_short_memories(
        &self,
        chat_id: &str,
    ) -> Result<Vec<KnowledgeShortMemory>, AppError> {
        let mut q = self
            .db
            .query(format!(
                "{SELECT} FROM knowledge_short_memory WHERE source_chat_id = $chat AND validated = false"
            ))
            .bind(("chat", chat_id.to_string()))
            .await
            .map_err(|e| Self::err("unconsolidated_sm", e))?;
        q.take(0)
            .map_err(|e| Self::err("unconsolidated_sm_take", e))
    }

    /// The chat's consolidation watermark (last consolidated message time), if any.
    pub async fn consolidation_watermark(
        &self,
        chat_id: &str,
    ) -> Result<Option<DateTime<Utc>>, AppError> {
        let mut q = self
            .db
            .query("SELECT VALUE consolidated_until FROM knowledge_consolidation_watermark WHERE chat_id = $chat LIMIT 1")
            .bind(("chat", chat_id.to_string()))
            .await
            .map_err(|e| Self::err("watermark", e))?;
        let rows: Vec<DateTime<Utc>> = q.take(0).map_err(|e| Self::err("watermark_take", e))?;
        Ok(rows.into_iter().next())
    }

    /// Chats the consolidation sweep should process: idle (settled) chats that
    /// have unconsolidated content. Keyed on the **message clock**, not
    /// `chat.updated_at` - the latter is bumped by unrelated chat-level ops (async
    /// title generation, metadata, archive), so it has no ordering relationship
    /// with the message-derived watermark and mis-selects in both directions.
    ///
    /// A chat qualifies when it is non-archived / non-task, has **no** message since
    /// `idle_cutoff` (settled), and either has a **terminal** message past its
    /// watermark or an **unvalidated short memory**. In-flight (`executing`/`paused`)
    /// messages never trigger selection; `consolidate_chat` windows below them.
    pub async fn chats_needing_consolidation(
        &self,
        idle_cutoff: DateTime<Utc>,
    ) -> Result<Vec<String>, AppError> {
        let epoch = DateTime::<Utc>::MIN_UTC;

        // Chats with a terminal message newer than their watermark.
        let by_msg: Vec<String> = {
            let mut q = self
                .db
                .query(
                    "SELECT VALUE chat_id FROM message
                     WHERE (status = NONE OR status IN [$completed, $failed, $cancelled])
                       AND created_at > ( (SELECT VALUE consolidated_until FROM knowledge_consolidation_watermark
                                           WHERE chat_id = $parent.chat_id LIMIT 1)[0] ?? $epoch )",
                )
                .bind(("epoch", epoch))
                .bind(("completed", MessageStatus::Completed))
                .bind(("failed", MessageStatus::Failed))
                .bind(("cancelled", MessageStatus::Cancelled))
                .await
                .map_err(|e| Self::err("chats_needing_msg", e))?;
            q.take(0)
                .map_err(|e| Self::err("chats_needing_msg_take", e))?
        };

        // Chats with an unvalidated short memory (may have no new messages at all,
        // so they must trigger selection on their own).
        let by_short: Vec<String> = {
            let mut q = self
                .db
                .query(
                    "SELECT VALUE source_chat_id FROM knowledge_short_memory
                     WHERE validated = false AND source_chat_id != NONE",
                )
                .await
                .map_err(|e| Self::err("chats_needing_short", e))?;
            q.take(0)
                .map_err(|e| Self::err("chats_needing_short_take", e))?
        };

        // Still-active chats (a message since the idle cutoff) - excluded.
        let active: std::collections::HashSet<String> = {
            let mut q = self
                .db
                .query("SELECT VALUE chat_id FROM message WHERE created_at >= $cutoff")
                .bind(("cutoff", idle_cutoff))
                .await
                .map_err(|e| Self::err("chats_needing_active", e))?;
            let v: Vec<String> = q
                .take(0)
                .map_err(|e| Self::err("chats_needing_active_take", e))?;
            v.into_iter().collect()
        };

        // Live (non-archived, non-task) chats - a candidate must be one of these.
        let live: std::collections::HashSet<String> = {
            let mut q = self
                .db
                .query(
                    "SELECT VALUE meta::id(id) FROM chat WHERE archived_at = NONE AND task_id = NONE",
                )
                .await
                .map_err(|e| Self::err("chats_needing_live", e))?;
            let v: Vec<String> = q
                .take(0)
                .map_err(|e| Self::err("chats_needing_live_take", e))?;
            v.into_iter().collect()
        };

        let mut out: Vec<String> = by_msg.into_iter().chain(by_short).collect();
        out.sort();
        out.dedup();
        out.retain(|c| live.contains(c) && !active.contains(c));
        Ok(out)
    }

    /// `(chat_id, user_id)` for the given chats - how the sweep partitions its work by
    /// user before opening a pass. A chat that no longer exists is simply absent.
    pub async fn chat_owners(
        &self,
        chat_ids: &[String],
    ) -> Result<Vec<(String, String)>, AppError> {
        if chat_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut q = self
            .db
            .query(
                "SELECT meta::id(id) AS chat_id, user_id FROM chat
                 WHERE meta::id(id) IN $ids",
            )
            .bind(("ids", chat_ids.to_vec()))
            .await
            .map_err(|e| Self::err("chat_owners", e))?;
        #[derive(Deserialize, Serialize, SurrealValue)]
        #[surreal(crate = "surrealdb::types")]
        struct Row {
            chat_id: String,
            user_id: String,
        }
        let rows: Vec<Row> = q.take(0).map_err(|e| Self::err("chat_owners_take", e))?;
        Ok(rows.into_iter().map(|r| (r.chat_id, r.user_id)).collect())
    }
}
