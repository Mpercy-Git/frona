use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tracing::warn;

use crate::core::error::AppError;
use crate::db::repo::pkm::{IngestBatch, PendingEntity, PendingEntityUpdate, PendingMemory};
use crate::memory::pkm::consolidation::Verdict;
use crate::memory::pkm::consolidation::context::ConsolidationContext;
use crate::memory::pkm::consolidation::evidence::SearchToolEvidenceTool;
use crate::memory::pkm::consolidation::ingest::cleanup::{
    candidate_attribute_map, remove_multi_entity_candidate_attributes, terminal_cleanup_with_recall,
};
use crate::memory::pkm::consolidation::ingest::correction::{
    AgentEvidenceMetrics, GroundingCorrectionState, agent_contribution_count,
    apply_allowed_revision, canonical_failure_state, contribution_count, correction_memory_ids,
    count_tool_supports, render_memory_evidence, validate_entity_revision_identity,
};
use crate::memory::pkm::consolidation::ingest::evidence::{
    batch_without_failed_contributions, candidate_attribute_evidence, is_agent_evidence_failure,
    resolve_evidence, resolve_evidence_with_tools,
};
use crate::memory::pkm::consolidation::ingest::submission::Batch;
use crate::memory::pkm::consolidation::ingest::temporal::{
    extraction_submission_limit, resolve_episode,
};
use crate::memory::pkm::consolidation::ingest::validation::{
    distinct_assertion_message_count, research_coverage_stats, research_message_handles,
    validate_erroneous_memories, validate_extract_submission,
};
use crate::memory::pkm::consolidation::recall::ReadRecallResultTool;
use crate::memory::pkm::consolidation::{
    PendingPlaybookCandidate, PromptSpec, TranscriptEvidenceKind, prepare_ingest_batch,
};
use crate::memory::pkm::model::{Episode, MemoryKind};
use crate::memory::pkm::storage::normalize_path;
use crate::tool::{AgentTool, registry::ToolFilter};

/// Mine one transcript into memories and provisional, untyped mention entities.
/// Failure leaves the transcript watermark unchanged so the pass can retry it.
pub struct Ingest {
    ctx: Arc<ConsolidationContext>,
}

impl Ingest {
    pub fn new(ctx: Arc<ConsolidationContext>) -> Self {
        Self { ctx }
    }

    pub async fn run(&self, transcript: &str) -> Result<IngestBatch, AppError> {
        let mut batch = self.extract(transcript).await?.unwrap_or_default();
        prepare_ingest_batch(&mut batch);
        Ok(batch)
    }

    /// Ingest, then **check the result against the transcript** and give the model a
    /// chance to correct what it invented.
    ///
    /// A model can return content unrelated to its input - a memorized example, or
    /// another session entirely - and every stage downstream will faithfully classify,
    /// type, and assemble it. So each round is scored by [`Grounding`]: entity names and
    /// attribute values must appear in the conversation, facts must be mostly drawn
    /// from it. Anything unsupported is reported back (in plain terms - see below) and
    /// the model revises in the same dialogue.
    ///
    /// The feedback deliberately says *what* is unsupported, never *how* it was
    /// measured. Handing a model the scoring function invites it to satisfy the score
    /// instead of the task - it would start naming entities after whatever words happen
    /// to be in the transcript. (the Classify's `reject.md` can quote its checker's output
    /// precisely because that checker is an OWL reasoner: you cannot phrase a logical
    /// contradiction away.)
    ///
    /// After the turn budget, whatever is grounded is kept and the rest dropped. Entities
    /// and attributes lose invalid citations
    /// individually; a memory loses as a unit because partial provenance changes its claim.
    /// An extraction that yields nothing still
    /// returns `Ok`, so the watermark advances: the model has had its attempts, and
    /// re-running the identical transcript next sweep would only repeat them.
    ///
    /// No chat and no live knowledge tools. The only optional tool reads a stored result
    /// from this extraction window; it cannot query current state or outlive the request.
    /// Naming a chat would make the stage - the one whose failure holds the watermark -
    /// fail outright for a chat deleted between selection and mining. An ingest failure
    /// charges an attempt against the pass record, so repeated failure eventually abandons
    /// the pass instead of retrying forever.
    async fn extract_grounded(
        &self,
        system: String,
        input: String,
    ) -> Result<(Batch, usize, usize, usize, AgentEvidenceMetrics), AppError> {
        let transcript_turns = self
            .ctx
            .scope
            .evidence_sources
            .iter()
            .filter(|source| {
                matches!(
                    source.kind,
                    TranscriptEvidenceKind::UserMessage { .. }
                        | TranscriptEvidenceKind::AgentMessage { .. }
                )
            })
            .count();
        let max_submissions = extraction_submission_limit(transcript_turns);
        let max_tool_turns = max_submissions;
        let recall_lookups = Arc::new(AtomicUsize::new(0));
        let evidence_lookups = Arc::new(AtomicUsize::new(0));
        let searched_messages = Arc::new(Mutex::new(HashSet::new()));
        let erroneous_contents = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let evidence_assertions = self
            .ctx
            .scope
            .evidence_sources
            .iter()
            .filter_map(|source| {
                let TranscriptEvidenceKind::AgentMessage { message_id, .. } = &source.kind else {
                    return None;
                };
                Some((source.handle.clone(), message_id.clone()))
            })
            .collect::<HashMap<_, _>>();
        let research_messages = Arc::new(research_message_handles(
            &self.ctx.scope.evidence_sources,
            &self.ctx.scope.recall.evidence,
        ));
        let tools: Vec<Arc<dyn AgentTool>> = vec![
            Arc::new(ReadRecallResultTool {
                prompts: self.ctx.llm.prompts().clone(),
                projection: self.ctx.scope.recall.clone(),
                lookups: recall_lookups.clone(),
            }),
            Arc::new(SearchToolEvidenceTool {
                prompts: self.ctx.llm.prompts().clone(),
                projection: self.ctx.scope.recall.evidence.clone(),
                messages: evidence_assertions,
                token_cap: self.ctx.scope.recall.evidence.result_token_cap,
                lookups: evidence_lookups.clone(),
                max_lookups: max_submissions,
                searched_messages: searched_messages.clone(),
            }),
        ];
        const ALLOWED: &[&str] = &["read_recall_result", "search_tool_evidence"];
        let mut convo = self
            .ctx
            .llm
            .conversation::<Batch>(
                None,
                &self.ctx.scope.agent_id,
                system,
                input,
                &[ToolFilter::AllowList(ALLOWED)],
                &tools,
                max_tool_turns,
            )
            .await?;

        let failure_state = Mutex::new(GroundingCorrectionState::default());
        let reported_failures = Mutex::new(HashSet::new());
        let corrections = AtomicUsize::new(0);
        let no_tool_drops = AtomicUsize::new(0);
        let no_tool_items = Mutex::new(HashSet::new());
        let strong_matches = AtomicUsize::new(0);
        let fallback_reviews = AtomicUsize::new(0);
        let invalid_submissions = AtomicUsize::new(0);
        let citation_repairs = AtomicUsize::new(0);
        let coverage_memory_additions = AtomicUsize::new(0);
        let mixed_claim_splits = AtomicUsize::new(0);
        let (ctx, state, reported_ref, corrections_ref) =
            (&self.ctx, &failure_state, &reported_failures, &corrections);
        let searched_ref = &searched_messages;
        let research_ref = research_messages.clone();
        let erroneous_ref = &erroneous_contents;
        let (no_tool_ref, no_tool_items_ref, strong_ref, fallback_ref, invalid_ref) = (
            &no_tool_drops,
            &no_tool_items,
            &strong_matches,
            &fallback_reviews,
            &invalid_submissions,
        );
        let (citation_ref, additions_ref, splits_ref) = (
            &citation_repairs,
            &coverage_memory_additions,
            &mixed_claim_splits,
        );
        // The transcript is consumed on `Ok`, so a missing first candidate must not let
        // the watermark advance. Hence `?` rather than a fallback.
        let refined = convo
            .refine(max_submissions, move |mut batch: Batch| {
                let research_ref = research_ref.clone();
                async move {
                let (revision, revision_failures) = {
                    let state = state.lock().expect("grounding failure state poisoned");
                    state.retained.as_ref().map(|previous| {
                        let revision_failures = validate_entity_revision_identity(
                            previous, &batch, &state.failures,
                        );
                        let failed_memories = state.failures.iter().filter_map(|failure| {
                            failure.field_path.strip_prefix("memories[")?
                                .split_once(']')?.0.parse::<usize>().ok()
                        }).collect::<BTreeSet<_>>();
                        let coverage_repair = state.failures.iter()
                            .any(|failure| failure.reason == "research_message_unaccounted");
                        let merged = apply_allowed_revision(previous, &batch, &state.failures);
                        if coverage_repair {
                            additions_ref.fetch_add(
                                merged.memories.len().saturating_sub(previous.memories.len()),
                                Ordering::Relaxed,
                            );
                        }
                        if !failed_memories.is_empty() {
                            let retained = previous.memories.len().saturating_sub(failed_memories.len());
                            splits_ref.fetch_add(
                                merged.memories.len().saturating_sub(retained)
                                    .saturating_sub(failed_memories.len()),
                                Ordering::Relaxed,
                            );
                        }
                        (merged, revision_failures)
                    }).map_or((None, Vec::new()), |(merged, failures)| (Some(merged), failures))
                };
                if let Some(merged) = revision { batch = merged; }
                let supports_before = count_tool_supports(&batch);
                let mut failures = {
                    let searched = searched_ref.lock().expect("searched evidence messages poisoned");
                    validate_extract_submission(
                        &mut batch,
                        &ctx.scope.evidence_sources,
                        &ctx.scope.temporal_sources,
                        &ctx.scope.recall,
                        &searched,
                        &research_ref,
                        citation_ref,
                    )
                };
                failures.extend(validate_erroneous_memories(
                    &batch,
                    ctx,
                    erroneous_ref,
                ).await?);
                failures.extend(revision_failures);
                let newly_rejected_without_tool = {
                    let mut seen = no_tool_items_ref.lock().expect("no-tool evidence state poisoned");
                    failures.iter()
                        .filter(|failure| failure.reason == "agent_claim_without_tool_evidence")
                        .filter(|failure| seen.insert(failure.fingerprint()))
                        .count()
                };
                no_tool_ref.fetch_add(newly_rejected_without_tool, Ordering::Relaxed);
                strong_ref.fetch_add(count_tool_supports(&batch).saturating_sub(supports_before), Ordering::Relaxed);
                fallback_ref.fetch_add(failures.iter().filter(|failure| failure.reason == "agent_claim_needs_tool_evidence").count(), Ordering::Relaxed);
                invalid_ref.fetch_add(failures.iter().filter(|failure| matches!(failure.reason,
                    "tool_evidence_without_agent_source" | "unknown_tool_evidence" | "tool_evidence_quote_not_found"
                        | "tool_evidence_clause_mismatch" | "selected_evidence_missing_critical_values")).count(), Ordering::Relaxed);
                if failures.is_empty() {
                    // An empty result trivially has nothing unsupported. Accepting it
                    // would let a model escape the check by giving up, discarding an
                    // earlier round that did find grounded content.
                    let accepted_recall_omission = {
                        let state = state.lock().expect("grounding failure state poisoned");
                        state.retained.as_ref().is_some_and(|previous| {
                            state.failures.iter().any(|failure| is_agent_evidence_failure(failure.reason))
                                && serde_json::to_value(batch_without_failed_contributions(previous, &state.failures)).ok()
                                    == serde_json::to_value(&batch).ok()
                        })
                    };
                    return Ok(if batch.new_entities.is_empty()
                        && batch.existing_entity_updates.is_empty()
                        && batch.playbooks.is_empty()
                        && batch.memories.is_empty()
                        && batch.research_dispositions.is_empty()
                        && !accepted_recall_omission
                    {
                        Verdict::Abandon
                    } else {
                        Verdict::Accept(batch)
                    });
                }
                let fingerprints = canonical_failure_state(&failures);
                let streak = {
                    let mut state = state.lock().expect("grounding failure state poisoned");
                    if state.fingerprints == fingerprints { state.streak += 1 } else {
                        state.fingerprints = fingerprints.clone();
                        state.streak = 1;
                    }
                    state.retained = Some(batch.clone());
                    state.failures = failures.clone();
                    state.streak
                };
                let submission = corrections_ref.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::info!(
                    submission, max_submissions, failures = failures.len(), streak,
                    fingerprints = ?fingerprints, "pkm extract grounding rejected"
                );
                if submission >= max_submissions {
                    let reported = reported_ref.lock().expect("reported grounding state poisoned");
                    if fingerprints.iter().any(|fingerprint| !reported.contains(fingerprint)) {
                        return Err(AppError::Internal(
                            "extract correction budget ended with a validation error that was not sent to the model; the window will retry without advancing its watermark".into(),
                        ));
                    }
                    warn!(unsupported = failures.len(), submissions = submission,
                        "extract: grounding budget exhausted; applying terminal cleanup");
                    let retained = state.lock().expect("grounding failure state poisoned")
                        .retained.clone().unwrap_or(batch);
                    return Ok(Verdict::Stop(retained));
                }
                let rejected = failures.iter().map(|failure| {
                    failure.render_with_allowed(None)
                }).collect::<Vec<_>>();
                let (accepted_memories, memory_repairs) = correction_memory_ids(&batch, &failures);
                let accepted_memories = render_memory_evidence(&batch, &accepted_memories);
                let memory_repairs = render_memory_evidence(&batch, &memory_repairs);
                let feedback = ctx.llm.reject(
                    PromptSpec::INGEST,
                    &[
                        ("rejected", &rejected.join("\n")),
                        ("accepted_memories", &accepted_memories),
                        ("memory_repairs", &memory_repairs),
                    ],
                )?;
                reported_ref.lock().expect("reported grounding state poisoned")
                    .extend(fingerprints);
                let keep = state.lock().expect("grounding failure state poisoned")
                    .retained.clone().unwrap_or(batch);
                Ok(Verdict::Revise { feedback, keep: Some(keep) })
                }
            })
            .await?;

        let mut batch = refined.unwrap_or(Batch {
            new_entities: Vec::new(),
            existing_entity_updates: Vec::new(),
            playbooks: Vec::new(),
            memories: Vec::new(),
            research_dispositions: Vec::new(),
        });
        let agent_before_cleanup =
            agent_contribution_count(&batch, &self.ctx.scope.evidence_sources);
        let mut terminal_failures = {
            let searched = searched_messages
                .lock()
                .expect("searched evidence messages poisoned");
            validate_extract_submission(
                &mut batch,
                &self.ctx.scope.evidence_sources,
                &self.ctx.scope.temporal_sources,
                &self.ctx.scope.recall,
                &searched,
                &research_messages,
                &citation_repairs,
            )
        };
        terminal_failures
            .extend(validate_erroneous_memories(&batch, &self.ctx, &erroneous_contents).await?);
        let has_unreported_terminal_failure = {
            let reported = reported_failures
                .lock()
                .expect("reported grounding state poisoned");
            terminal_failures
                .iter()
                .any(|failure| !reported.contains(&failure.fingerprint()))
        };
        if has_unreported_terminal_failure {
            return Err(AppError::Internal(
                "extract terminal cleanup found a validation error that was not sent to the model; the window will retry without advancing its watermark".into(),
            ));
        }
        let newly_rejected_without_tool = {
            let mut seen = no_tool_items
                .lock()
                .expect("no-tool evidence state poisoned");
            terminal_failures
                .iter()
                .filter(|failure| failure.reason == "agent_claim_without_tool_evidence")
                .filter(|failure| seen.insert(failure.fingerprint()))
                .count()
        };
        no_tool_drops.fetch_add(newly_rejected_without_tool, Ordering::Relaxed);
        let before_failed_cleanup = contribution_count(&batch);
        batch = batch_without_failed_contributions(&batch, &terminal_failures);
        let failed_cleanup_drops = before_failed_cleanup.saturating_sub(contribution_count(&batch));
        let dropped = failed_cleanup_drops
            + terminal_cleanup_with_recall(
                &mut batch,
                &self.ctx.scope.evidence_sources,
                &self.ctx.scope.recall,
            );
        for memory in &batch.memories {
            let recall_ids = memory
                .sources
                .iter()
                .filter_map(|citation| {
                    let source = self
                        .ctx
                        .scope
                        .evidence_sources
                        .iter()
                        .find(|source| source.handle == citation.message)?;
                    let TranscriptEvidenceKind::AgentMessage { message_id, .. } = &source.kind
                    else {
                        return None;
                    };
                    Some(
                        self.ctx
                            .scope
                            .recall
                            .result_calls_for_message(message_id)
                            .iter()
                            .map(|call| call.local_id.as_str())
                            .collect::<Vec<_>>(),
                    )
                })
                .flatten()
                .collect::<Vec<_>>();
            if !recall_ids.is_empty() {
                tracing::info!(
                    memory = %memory.content,
                    recall_calls = ?recall_ids,
                    reason = "no attached recall result supported the complete retained claim",
                    "pkm extract: retained Agent-sourced candidate after recall check"
                );
            }
        }
        let strong = strong_matches.load(Ordering::Relaxed);
        let total_supports = count_tool_supports(&batch);
        let evidence_terminal_drops = agent_before_cleanup.saturating_sub(
            agent_contribution_count(&batch, &self.ctx.scope.evidence_sources),
        );
        let mut research_coverage =
            research_coverage_stats(&batch, &self.ctx.scope.evidence_sources, &research_messages);
        research_coverage.memories_added_by_repair =
            coverage_memory_additions.load(Ordering::Relaxed);
        research_coverage.citation_repairs = citation_repairs.load(Ordering::Relaxed);
        research_coverage.mixed_claim_splits = mixed_claim_splits.load(Ordering::Relaxed);
        Ok((
            batch,
            corrections.load(Ordering::Relaxed),
            dropped,
            recall_lookups.load(Ordering::Relaxed),
            AgentEvidenceMetrics {
                no_tool_drops: no_tool_drops.load(Ordering::Relaxed),
                strong_matches: strong,
                fallback_reviews: fallback_reviews.load(Ordering::Relaxed),
                fallback_retains: total_supports.saturating_sub(strong),
                invalid_submissions: invalid_submissions.load(Ordering::Relaxed),
                lookup_calls: evidence_lookups.load(Ordering::Relaxed),
                terminal_drops: evidence_terminal_drops,
                research_coverage,
            },
        ))
    }

    /// Turn the transcript into the rows it implies. Reads live state to decide what to
    /// suppress, but writes nothing - the commit is the caller's, in one transaction.
    async fn extract(&self, transcript: &str) -> Result<Option<IngestBatch>, AppError> {
        if transcript.trim().is_empty() {
            return Ok(None);
        }

        // Seed the owner's self-entity and pass the owner→path binding so self-facts
        // route there. (This is not resolution - just the reserved owner entity.)
        let self_path = self
            .ctx
            .repo
            .ensure_self_entity(&self.ctx.scope.user_id, &self.ctx.scope.user_name)
            .await
            .unwrap_or_else(|_| crate::memory::pkm::model::SELF_ENTITY_PATH.to_string());
        let owner_name = if self.ctx.scope.user_name.trim().is_empty() {
            "the account owner"
        } else {
            self.ctx.scope.user_name.as_str()
        };

        // A failed render is an `Err` (see `PromptSpec`), so the empty-prompt hazard is
        // handled for every stage now - but keep the transcript check below: it is about
        // *content*, not templating. A template that renders fine while dropping
        // `{{transcript}}` would still hand the model nothing to answer from.
        let mut existing_pages = self.ctx.repo.list_entities(&self.ctx.scope.user_id).await?;
        existing_pages.sort_by(|a, b| a.path.cmp(&b.path));
        let existing_entities = existing_pages
            .iter()
            .map(|entity| format!("- `{}` — {}", entity.path, entity.name))
            .collect::<Vec<_>>()
            .join("\n");
        let existing_entities = if existing_entities.is_empty() {
            "(none)".to_string()
        } else {
            existing_entities
        };
        let mut research_messages = research_message_handles(
            &self.ctx.scope.evidence_sources,
            &self.ctx.scope.recall.evidence,
        )
        .into_iter()
        .collect::<Vec<_>>();
        research_messages.sort();
        let research_messages = if research_messages.is_empty() {
            "(none)".to_string()
        } else {
            research_messages
                .into_iter()
                .map(|message| format!("- `{message}`"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let rendered = self.ctx.llm.render(
            PromptSpec::INGEST,
            &[
                ("owner_name", owner_name),
                ("handle", self.ctx.scope.vault.handle().as_str()),
                ("self_path", &self_path),
                ("existing_entities", &existing_entities),
                ("research_messages", &research_messages),
                ("transcript", transcript),
            ],
        )?;

        if !rendered.input.contains(transcript.trim()) {
            return Err(AppError::Internal(
                "extract: rendered prompt does not contain the transcript — refusing to \
                 call the model with no input"
                    .into(),
            ));
        }

        // A failure must propagate so the sweep does not advance the watermark.
        let (
            mut parsed,
            grounding_corrections,
            grounding_items_dropped,
            recall_result_lookups,
            agent_evidence,
        ) = self
            .extract_grounded(rendered.system, rendered.input)
            .await?;
        remove_multi_entity_candidate_attributes(&mut parsed);

        let mut out = IngestBatch {
            grounding_corrections,
            grounding_items_dropped,
            recall_result_lookups,
            agent_evidence_no_tool_drops: agent_evidence.no_tool_drops,
            agent_evidence_strong_matches: agent_evidence.strong_matches,
            agent_evidence_fallback_reviews: agent_evidence.fallback_reviews,
            agent_evidence_fallback_retains: agent_evidence.fallback_retains,
            agent_evidence_invalid_submissions: agent_evidence.invalid_submissions,
            agent_evidence_lookup_calls: agent_evidence.lookup_calls,
            agent_evidence_terminal_drops: agent_evidence.terminal_drops,
            research_coverage: agent_evidence.research_coverage,
            ..Default::default()
        };
        for entity in &parsed.new_entities {
            let Some(path) = normalize_path(&entity.path) else {
                // Only reachable for a proposed path with no alphanumeric content at all
                // once transliterated - punctuation, or symbols with no ASCII rendering.
                // Logged rather than skipped in silence: the transcript is consumed
                // once, so an entity dropped here is dropped for good.
                warn!(
                    proposed = %entity.path,
                    entity = %entity.name,
                    "pkm extract: proposed path has no usable segment, entity dropped"
                );
                continue;
            };
            // Attributes are the extractor's ONLY structured output about an entity, and
            // they stay free-text keyed: naming them against a vocabulary, and deciding
            // which of them are really edges to another entity needs the catalogue. Ingest
            // sees existing entity names only to reuse their identity, and has no ontology
            // vocabulary. The Classify stage decides a stage later - see `classify`'s
            // `AttributeMapping`.
            //
            // Carried as a map rather than flattened into `"{key}: {value}"` strings so
            // that decision has structure to work from instead of prose to re-parse.
            let attribute_evidence = candidate_attribute_evidence(
                &entity.candidate_attributes,
                &self.ctx.scope.evidence_sources,
                &self.ctx.scope.recall.evidence,
            );
            let mut attributes = candidate_attribute_map(&entity.candidate_attributes);
            attributes.retain(|key, _| attribute_evidence.contains_key(key));
            out.entities.push(PendingEntity {
                path,
                name: entity.name.clone(),
                description: entity.description.clone(),
                aliases: entity.aliases.clone(),
                identity_evidence: resolve_evidence(
                    &entity.sources,
                    &self.ctx.scope.evidence_sources,
                )
                .unwrap_or_default(),
                attributes: serde_json::Value::Object(attributes),
                attribute_evidence,
            });
        }

        let existing_paths: HashSet<&str> = existing_pages
            .iter()
            .map(|entity| entity.path.as_str())
            .collect();
        for update in &parsed.existing_entity_updates {
            let Some(path) = normalize_path(&update.path) else {
                continue;
            };
            if !existing_paths.contains(path.as_str()) {
                warn!(
                    proposed = %update.path,
                    "pkm extract: existing-entity update names an entity absent from the input"
                );
                continue;
            }
            let attribute_evidence = candidate_attribute_evidence(
                &update.candidate_attributes,
                &self.ctx.scope.evidence_sources,
                &self.ctx.scope.recall.evidence,
            );
            let mut attributes = candidate_attribute_map(&update.candidate_attributes);
            attributes.retain(|key, _| attribute_evidence.contains_key(key));
            if attributes.is_empty() {
                continue;
            }
            out.entity_updates.push(PendingEntityUpdate {
                path,
                attributes: serde_json::Value::Object(attributes),
                attribute_evidence,
            });
        }

        // Memories keep their mention paths until Resolve selects a canonical identity.
        // Re-learn suppression (a fact retired as `Erroneous` must not be re-minted) is
        // preserved, cached per entity across the loop. Read before the transaction: a
        // stale answer only means a fact the user retired moments ago is re-minted, which
        // the next pass retires again - not worth holding a transaction open for.
        let mut playbook_by_local: HashMap<String, usize> = HashMap::new();
        for candidate in parsed.playbooks {
            let local = candidate.id.trim();
            let name = candidate.name.trim();
            let description = candidate.description.trim();
            let path = normalize_path(&candidate.path).ok_or_else(|| {
                AppError::Internal(
                    "extract accepted a playbook candidate with an unusable path".into(),
                )
            })?;
            if local.is_empty()
                || name.is_empty()
                || description.is_empty()
                || playbook_by_local.contains_key(local)
            {
                return Err(AppError::Internal(
                    "extract accepted an invalid or duplicate playbook candidate".into(),
                ));
            }
            let index = out.playbook_candidates.len();
            out.playbook_candidates.push(PendingPlaybookCandidate {
                id: crate::core::repository::new_id(),
                path,
                name: name.to_string(),
                description: description.to_string(),
                source_memory_ids: Default::default(),
            });
            playbook_by_local.insert(local.to_string(), index);
        }

        for mem in parsed.memories {
            let kind = MemoryKind::parse(&mem.kind).ok_or_else(|| {
                AppError::Internal("extract accepted a memory with an invalid kind".into())
            })?;
            let playbook_index = match (kind == MemoryKind::Procedural, mem.playbook.as_deref()) {
                (true, Some(local)) => playbook_by_local.get(local.trim()).copied(),
                (true, None) | (false, Some(_)) => None,
                (false, None) => Some(usize::MAX),
            };
            if playbook_index.is_none() {
                return Err(AppError::Internal(
                    "extract accepted a memory with an invalid procedural playbook reference"
                        .into(),
                ));
            }
            if (kind == MemoryKind::Episodic) != mem.episode.is_some() {
                return Err(AppError::Internal(
                    "extract accepted a memory with invalid episodic metadata".into(),
                ));
            }
            let mut episode = mem.episode.map(|episode| Episode {
                status: episode.status,
                anchor: episode.anchor,
                duration: episode.duration,
                absolute: episode.absolute,
                resolved_start: None,
                resolved_end: None,
            });
            if episode
                .as_ref()
                .is_some_and(|e| e.duration.is_some() && e.absolute.is_some())
            {
                return Err(AppError::Internal(
                    "extract accepted an episode with both relative and absolute time".into(),
                ));
            }
            if let Some(episode) = &mut episode {
                resolve_episode(
                    episode,
                    &self.ctx.scope.temporal_sources,
                    &self.ctx.scope.timezone,
                );
            }
            let content = mem.content.trim();
            if content.is_empty() {
                return Err(AppError::Internal(
                    "extract accepted a memory with empty content".into(),
                ));
            }
            let paths: Vec<String> = mem
                .entities
                .iter()
                .filter_map(|p| normalize_path(p))
                .collect();
            if paths.is_empty() {
                return Err(AppError::Internal(
                    "extract accepted a memory without a usable entity".into(),
                ));
            }
            let Some(evidence) = resolve_evidence_with_tools(
                &mem.sources,
                &mem.tool_evidence,
                &self.ctx.scope.evidence_sources,
                &self.ctx.scope.recall.evidence,
            ) else {
                warn!(
                    memory = %content,
                    "pkm extract: accepted memory had unresolvable evidence; memory dropped"
                );
                out.grounding_items_dropped += 1;
                continue;
            };
            if kind == MemoryKind::Procedural && distinct_assertion_message_count(&evidence) != 1 {
                return Err(AppError::Internal(
                    "extract accepted a Procedural memory without exactly one User or Agent assertion source".into(),
                ));
            }
            let memory_id = crate::core::repository::new_id();
            out.memories.push(PendingMemory {
                id: memory_id.clone(),
                kind,
                evidence,
                episode,
                content: content.to_string(),
                paths,
            });
            if let Some(index) = playbook_index.filter(|index| *index != usize::MAX) {
                out.playbook_candidates[index]
                    .source_memory_ids
                    .insert(memory_id);
            }
        }
        out.playbook_candidates
            .retain(|candidate| !candidate.source_memory_ids.is_empty());
        Ok(Some(out))
    }
}
