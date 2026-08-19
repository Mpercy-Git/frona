use super::*;
use crate::memory::pkm::model::PendingEntityContribution;

#[test]
fn variant_is_the_stage_and_advances_in_pipeline_order() {
    let mut s = ConsolidationStageState::Ingest(IngestState::default());
    let mut seen = vec![s.label()];
    for _ in 0..9 {
        s = s.next();
        seen.push(s.label());
    }
    assert_eq!(
        seen,
        [
            "ingest",
            "classify",
            "resolve",
            "reconcile",
            "assemble",
            "playbook_resolve",
            "playbook_author",
            "page_author",
            "cleanup",
            "done",
        ]
    );
    assert!(s.is_done());
    assert!(s.next().is_done(), "Done is terminal");
}

/// A stage carries a payload only when its completion is invisible in the live
/// tables. Page Author and Cleanup are already covered - Page Author by the
/// `rendered_at` it stamps with each article, cleanup by being idempotent - so a
/// payload appearing on either would be bookkeeping nothing reads.
#[test]
fn only_the_stages_that_need_state_carry_it() {
    let has_payload = |s: &ConsolidationStageState| {
        !matches!(
            s,
            ConsolidationStageState::PlaybookAuthor
                | ConsolidationStageState::PageAuthor
                | ConsolidationStageState::Cleanup
                | ConsolidationStageState::Done
        )
    };
    let mut s = ConsolidationStageState::Ingest(IngestState::default());
    let mut stateful = Vec::new();
    for _ in 0..10 {
        if has_payload(&s) {
            stateful.push(s.label());
        }
        s = s.next();
    }
    assert_eq!(
        stateful,
        [
            "ingest",
            "classify",
            "resolve",
            "reconcile",
            "assemble",
            "playbook_resolve"
        ]
    );
}

fn pending(
    name: &str,
    description: &str,
    memory_id: &str,
) -> crate::memory::pkm::model::KnowledgeConsolidationEntity {
    let contribution = PendingEntityContribution {
        name: name.into(),
        description: description.into(),
        aliases: Default::default(),
        attributes: serde_json::json!({"key": "value"}),
        attribute_evidence: Default::default(),
        source_memory_ids: [memory_id.to_string()].into_iter().collect(),
        existing_only: false,
        occurrence_count: 1,
    };
    crate::memory::pkm::model::KnowledgeConsolidationEntity::pending(
        "test-run",
        "user",
        "topics/example",
        crate::memory::pkm::model::EntityCategory::Concept,
        vec![contribution],
        [memory_id.to_string()].into_iter().collect(),
    )
}

#[test]
fn same_path_keeps_semantically_different_contributions() {
    let mut entity = pending("Example", "first account", "m1");
    let incoming = pending("Example", "different account", "m2");
    entity.merge_contribution(incoming.contributions.into_iter().next().unwrap());
    assert_eq!(entity.contributions.len(), 2);
    assert_eq!(entity.source_memory_ids.len(), 2);
}

#[test]
fn exact_contributions_collapse_without_losing_provenance() {
    let mut entity = pending("Example", "same account", "m1");
    let incoming = pending("Example", "same account", "m2");
    entity.merge_contribution(incoming.contributions.into_iter().next().unwrap());
    assert_eq!(entity.contributions.len(), 1);
    assert_eq!(entity.contributions[0].occurrence_count, 2);
    assert_eq!(entity.contributions[0].source_memory_ids.len(), 2);
}

#[test]
fn staged_attributes_expose_conflicting_values_from_every_contribution() {
    let mut former = pending("", "", "former-employer-memory");
    former.contributions[0].attributes = serde_json::json!({
        "employer": "Former Corp",
        "name": "Casey Owner"
    });

    let mut current = pending("", "", "current-employer-memory");
    current.contributions[0].attributes = serde_json::json!({
        "employer": "Example Corp",
        "current employer": "Example Corp"
    });
    former.merge_contribution(current.contributions.into_iter().next().unwrap());

    assert_eq!(
        former.staged_attributes(),
        serde_json::json!({
            "employer": ["Former Corp", "Example Corp"],
            "name": "Casey Owner",
            "current employer": "Example Corp"
        })
    );
}
