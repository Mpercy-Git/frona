# PKM prompt contracts

This directory holds prompts for isolated model calls. Repetition needed to make one call
self-contained is acceptable; conflicting ownership or output contracts are not.

## Stage ownership

| Stage | Owns | Must not take over |
|---|---|---|
| Ingest | Discover mentioned entities, emit grounded memories, and attach plain candidate attributes. It is the primary entity-discovery path. | Ontology classes, CURIE properties, literal-vs-edge decisions, identity merges. |
| Classify | Type one entity, map stated keys and relations to vocabulary, decide literal-vs-edge, propose provisional class keys/inverse-functional identity markers, and mint an attribute-valued entity only as a missed-entity fail-safe. | Persist the schema, reconcile memory history, merge entity identity. |
| Resolve | Decide whether a newly mentioned entity is the same individual as a compatible existing entity, using Classify markers as retrieval evidence rather than deterministic merge rules. | Typing, schema design, similarity-based merging without identity evidence. |
| Reconcile | Relate immutable memories, derive current entity state, and perform a post-entity-reconcile literal-to-existing-entity promotion fail-safe. | Independently mint entities, persist schema decisions, reclassify entities. |
| Assemble | Decide the entire pass's proposed vocabulary, then commit the accepted schema and entity types together. | Change reconciled facts or entity identity. |
| Playbook Resolve | Resolve reusable procedure identity and memory ownership. | Author a procedure body or invent commands. |
| Playbook Author | Create or update reusable procedure bodies from current procedural memories and captured invocations. | Change procedure identity or discard an existing procedure body during update. |
| Page Author | Render accepted current/history state as article prose. | Create facts, alter schema, or claim arbitrary absent fields are not recorded. |
| Writeback | Translate deliberate human page edits into canonical memory operations. | Re-render pages or infer changes outside the supplied diff. |

## Shared invariants

- Model outputs using structured inference are returned with the `submit` tool, never as
  raw JSON prose.
- A correction prompt must request every required top-level field in that stage's current
  output schema. Rejected submissions are not partial state.
- Extracted keys stay plain. Classify normally assigns CURIEs. Reconcile may propose a
  relation-specific CURIE only in its promotion fail-safe; Adjudicate decides whether any
  new `frona:` term persists.
- Evidence packs and entity candidates are advisory. They satisfy the requirement to search,
  but the model still judges whether a candidate fits.
- Entity paths are stable identity handles. A title rename and a path move are separate.
- Exact values are preserved; no stage fills missing values with defaults.

## Maintenance

When a prompt output changes:

1. Change its Rust schema and prompt together.
2. Update every reject, advisory, and bad-term prompt for that stage.
3. Update the semantic contract tests in `consolidation/prompt/mod.rs`.
4. Run the PKM unit and end-to-end suites without automatic formatting.
