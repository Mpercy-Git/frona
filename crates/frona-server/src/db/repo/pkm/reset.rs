use crate::db::repo::pkm::*;

impl PkmRepo {
    pub async fn reset_user_derived_memory(&self, user_id: &str) -> Result<(), AppError> {
        let tx = self
            .db
            .clone()
            .begin()
            .await
            .map_err(|error| Self::err("reset_user_begin", error))?;
        let result = tx
            .query(
                "LET $chat_ids = SELECT VALUE meta::id(id) FROM chat WHERE user_id = $uid;
                 DELETE knowledge_consolidation_entity WHERE user_id = $uid;
                 DELETE knowledge_consolidation_record WHERE user_id = $uid;
                 DELETE knowledge_consolidation_watermark WHERE chat_id IN $chat_ids;
                 DELETE chat_summary WHERE user_id = $uid OR chat_id IN $chat_ids;
                 DELETE knowledge_entity_link WHERE user_id = $uid;
                 DELETE knowledge_entity_source WHERE user_id = $uid;
                 DELETE knowledge_entity WHERE user_id = $uid;
                 DELETE knowledge_memory WHERE user_id = $uid;
                 DELETE knowledge_ontology WHERE user_id = $uid;
                 UPDATE knowledge_short_memory SET validated = false WHERE user_id = $uid;",
            )
            .bind(("uid", user_id.to_string()))
            .await
            .and_then(|response| response.check());
        if let Err(error) = result {
            let _ = tx.cancel().await;
            return Err(Self::err("reset_user", error));
        }
        tx.commit()
            .await
            .map_err(|error| Self::err("reset_user_commit", error))?;
        Ok(())
    }
}
