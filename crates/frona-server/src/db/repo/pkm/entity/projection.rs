use super::*;

impl PkmRepo {
    /// Persist an entity's authored article `body` (the deterministic render source).
    /// Written by the author/playbook stages after (re)authoring; read by
    /// `reconcile_files` to re-render a missing file without an LLM call.
    pub async fn set_page_body(
        &self,
        user_id: &str,
        path: &str,
        body: &str,
    ) -> Result<(), AppError> {
        self.db
            .query("UPDATE knowledge_entity SET body = $body WHERE user_id = $uid AND path = $path")
            .bind(("body", body.to_string()))
            .bind(("uid", user_id.to_string()))
            .bind(("path", path.to_string()))
            .await
            .map_err(|e| Self::err("set_page_body", e))?;
        Ok(())
    }

    /// Stamp an entity's sync `rev` - the `sha256` of its rendered file bytes,
    /// recomputed every time the file is (re)written. The CAS token + manifest
    /// entry for stateless sync.
    pub async fn set_page_rev(&self, user_id: &str, path: &str, rev: &str) -> Result<(), AppError> {
        self.db
            .query(
                "UPDATE knowledge_entity SET rev = $rev, sync_content = NONE
                 WHERE user_id = $uid AND path = $path",
            )
            .bind(("rev", rev.to_string()))
            .bind(("uid", user_id.to_string()))
            .bind(("path", path.to_string()))
            .await
            .map_err(|e| Self::err("set_page_rev", e))?;
        Ok(())
    }

    /// Persist the exact canonical Markdown bytes and their public sync revision.
    /// The database commit happens before the filesystem mirror write.
    pub async fn set_page_projection(
        &self,
        user_id: &str,
        path: &str,
        content: &str,
        rev: &str,
    ) -> Result<(), AppError> {
        self.db
            .query(
                "UPDATE knowledge_entity SET rev = $rev, sync_content = $content
                 WHERE user_id = $uid AND path = $path",
            )
            .bind(("rev", rev.to_string()))
            .bind(("content", content.to_string()))
            .bind(("uid", user_id.to_string()))
            .bind(("path", path.to_string()))
            .await
            .and_then(|response| response.check())
            .map_err(|e| Self::err("set_page_projection", e))?;
        Ok(())
    }

    /// The `(clean_path, rev)` manifest of all **rendered** `Internal` entities - the
    /// stateless-sync pull primitive. Entities with no `rev` yet (never authored) are
    /// omitted. The sync layer prefixes the Memory directory to form vault paths and
    /// the client diffs this against its local rev map.
    pub async fn page_manifest(&self, user_id: &str) -> Result<Vec<(String, String)>, AppError> {
        let mut q = self
            .db
            .query(
                "SELECT path, rev FROM knowledge_entity
                 WHERE user_id = $uid AND origin = $origin AND rev != NONE",
            )
            .bind(("uid", user_id.to_string()))
            .bind(("origin", EntityOrigin::Internal))
            .await
            .map_err(|e| Self::err("page_manifest", e))?;
        #[derive(Deserialize, Serialize, SurrealValue)]
        #[surreal(crate = "surrealdb::types")]
        struct Row {
            path: String,
            rev: String,
        }
        let rows: Vec<Row> = q.take(0).map_err(|e| Self::err("page_manifest_take", e))?;
        Ok(rows.into_iter().map(|r| (r.path, r.rev)).collect())
    }
}
