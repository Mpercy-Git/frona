use chrono::{DateTime, Utc};

use super::*;
use crate::memory::pkm::{ConsolidationStageState, IngestState};

pub(crate) async fn seed_reconciled_entity(
    repo: &PkmRepo,
    user_id: &str,
    path: &str,
    name: &str,
    description: &str,
    attributes: &serde_json::Value,
) -> Result<(), ()> {
    let existing = repo.entity_by_path(user_id, path).await.unwrap().unwrap();
    let name = name.trim();
    let renamed = !name.is_empty() && name != existing.name;
    let final_name = if renamed {
        name.to_string()
    } else {
        existing.name.clone()
    };
    let mut aliases = existing.aliases;
    if renamed {
        aliases.insert(existing.name);
    }
    let search_text = derive_search_text(&final_name, description, &aliases);
    repo.db
        .query(
            "UPDATE type::record('knowledge_entity', $id) SET
            name = $name, description = $description, search_text = $search_text,
            aliases = $aliases, attributes = $attributes",
        )
        .bind(("id", existing.id))
        .bind(("name", final_name))
        .bind(("description", description.to_string()))
        .bind(("search_text", search_text))
        .bind(("aliases", aliases))
        .bind(("attributes", attributes.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    Ok(())
}

pub(crate) async fn mark_entity_rendered(
    repo: &PkmRepo,
    user_id: &str,
    path: &str,
) -> Result<(), ()> {
    repo.db
        .query(
            "UPDATE knowledge_entity SET rendered_at = $now
         WHERE user_id = $user_id AND path = $path",
        )
        .bind(("now", Utc::now()))
        .bind(("user_id", user_id.to_string()))
        .bind(("path", path.to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
    Ok(())
}

pub(crate) async fn seed_asserted_entity_link(
    repo: &PkmRepo,
    user_id: &str,
    from: &str,
    to: &str,
    relation: &str,
) -> Result<(), ()> {
    let link = KnowledgeEntityLink {
        id: new_id(),
        user_id: user_id.to_string(),
        from_entity_path: from.to_string(),
        to_entity_path: to.to_string(),
        relation: relation.to_string(),
        source_memory_ids: Vec::new(),
        origin: LinkOrigin::Asserted,
        created_at: Utc::now(),
    };
    let _: Option<surrealdb::types::Value> = repo
        .db
        .create(("knowledge_entity_link", link.id.clone()))
        .content(link)
        .await
        .unwrap();
    Ok(())
}

pub(crate) async fn commit_checkpointed_extract_patch(
    repo: &PkmRepo,
    user_id: &str,
    batch: &IngestBatch,
    watermark: Option<(&str, DateTime<Utc>)>,
    short_memory_ids: &[String],
) -> Result<IngestCounts, AppError> {
    let now = Utc::now();
    let record = KnowledgeConsolidationRecord {
        id: new_id(),
        consolidation_id: new_id(),
        user_id: user_id.to_string(),
        state: ConsolidationStageState::Ingest(IngestState::default()),
        stats: Default::default(),
        attempts: 0,
        restart_count: 0,
        failure: None,
        next_attempt_at: now,
        updated_at: now,
    };
    let watermarks = watermark
        .map(|(chat_id, until)| (chat_id.to_string(), until))
        .into_iter()
        .collect::<Vec<_>>();
    repo.commit_extract_patch_with_checkpoint(
        user_id,
        batch,
        &watermarks,
        short_memory_ids,
        &record,
    )
    .await
}
