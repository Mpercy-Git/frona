//! The background consolidation sweep: choosing which chats to mine, windowing them
//! against the message clock, and assembling the transcript the extract stage reads.
//!
//! Everything between "the scheduler fired" and "a transcript exists". The pass itself
//! lives in [`consolidation`](super::consolidation); this decides *what* it runs on.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::StreamExt;

use crate::agent::service::AgentService;
use crate::chat::message::models::{MessageEvent, MessageResponse, MessageRole};
use crate::chat::service::ChatService;
use crate::contact::service::ContactService;
use crate::core::error::AppError;
use crate::db::repo::tool_calls::ToolCallRepository;

use super::PkmService;
use super::consolidation::{
    ConsolidationScope, ConsolidationStageState, ConsolidationStats, IngestState,
};
use super::model::KnowledgeShortMemory;
use super::vault::VaultScope;

struct ExtractionWrite {
    batch: crate::db::repo::pkm::IngestBatch,
    watermark: Option<(String, DateTime<Utc>)>,
    short_memory_ids: Vec<String>,
    done: tokio::sync::oneshot::Sender<Result<crate::db::repo::pkm::IngestCounts, String>>,
}

struct PreparedExtractionWindow {
    scope: ConsolidationScope,
    transcript: String,
    watermark: Option<(String, DateTime<Utc>)>,
    short_memory_ids: Vec<String>,
    new_messages: usize,
}

struct MinedExtractionWindow {
    prepared: PreparedExtractionWindow,
    batch: crate::db::repo::pkm::IngestBatch,
}

fn completed_task_result_links(
    messages: &[MessageResponse],
) -> std::collections::HashMap<String, Vec<String>> {
    let mut ordered = messages.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|message| message.created_at);
    let mut pending = Vec::new();
    let mut links = std::collections::HashMap::<String, Vec<String>>::new();
    for message in ordered {
        if let Some(MessageEvent::TaskCompletion {
            task_id, status, ..
        }) = &message.event
            && *status == crate::agent::task::models::TaskStatus::Completed
        {
            pending.push(task_id.clone());
            continue;
        }
        if message.role == MessageRole::Agent && !pending.is_empty() {
            links
                .entry(message.id.clone())
                .or_default()
                .append(&mut pending);
        }
    }
    links
}

fn task_target_at(task: &crate::agent::task::models::Task) -> Option<DateTime<Utc>> {
    match &task.kind {
        crate::agent::task::models::TaskKind::Cron { next_run_at, .. } => *next_run_at,
        crate::agent::task::models::TaskKind::CronRun { fire_at, .. } => Some(*fire_at),
        _ => task.run_at,
    }
}

fn render_task_lifecycle(
    lifecycle: &str,
    title: &str,
    event_at: DateTime<Utc>,
    target_at: Option<DateTime<Utc>>,
) -> String {
    use chrono::SecondsFormat;

    let event_at = event_at.to_rfc3339_opts(SecondsFormat::Millis, true);
    match target_at {
        Some(target_at) => format!(
            "[task {lifecycle} event_at={} target_at={}] {}",
            event_at,
            target_at.to_rfc3339_opts(SecondsFormat::Millis, true),
            title.trim(),
        ),
        None => format!("[task {lifecycle} event_at={event_at}] {}", title.trim()),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SweepMode {
    Full,
    ExtractOnly,
}

#[derive(Clone, Copy)]
struct SweepServices<'a> {
    chat: &'a ChatService,
    contact: &'a ContactService,
    agent: &'a AgentService,
    harness: &'a Arc<crate::agent::harness::Harness>,
}

struct TranscriptInput<'a> {
    messages: &'a [MessageResponse],
    short: &'a [KnowledgeShortMemory],
    contact_service: &'a ContactService,
    task_service: &'a crate::agent::task::service::TaskService,
    tool_calls: &'a [crate::inference::tool_call::ToolCall],
    agent_message_ids: &'a [String],
    assertion_message_ids: &'a [String],
    task_evidence:
        &'a std::collections::HashMap<String, Vec<crate::inference::tool_call::ToolCall>>,
    user_id: &'a str,
    vault: &'a VaultScope,
}

#[derive(Clone)]
struct ExtractionSchedule {
    sender: tokio::sync::mpsc::Sender<ExtractionWrite>,
    slots: Arc<tokio::sync::Semaphore>,
    per_chat_concurrency: usize,
}

async fn collect_task_tree_tool_calls(
    root_task_ids: &[String],
    task_service: &crate::agent::task::service::TaskService,
    tool_calls: &crate::db::repo::tool_calls::SurrealToolCallRepo,
) -> Result<Vec<crate::inference::tool_call::ToolCall>, AppError> {
    let mut pending = std::collections::VecDeque::from(root_task_ids.to_vec());
    let mut visited_tasks = std::collections::HashSet::new();
    let mut visited_chats = std::collections::HashSet::new();
    let mut calls = Vec::new();
    while let Some(task_id) = pending.pop_front() {
        if !visited_tasks.insert(task_id.clone()) {
            continue;
        }
        let Some(task) = task_service.find_by_id(&task_id).await? else {
            continue;
        };
        let Some(chat_id) = task.chat_id else {
            continue;
        };
        if visited_chats.insert(chat_id.clone()) {
            calls.extend(tool_calls.find_by_chat_id(&chat_id).await?);
        }
        for child in task_service.find_by_source_chat_id(&chat_id).await? {
            pending.push_back(child.id);
        }
    }
    calls.sort_by(|left, right| {
        (left.created_at, &left.chat_id, left.turn, &left.id).cmp(&(
            right.created_at,
            &right.chat_id,
            right.turn,
            &right.id,
        ))
    });
    calls.dedup_by(|left, right| left.id == right.id);
    Ok(calls)
}

impl PkmService {
    pub async fn run_consolidation_sweep(
        &self,
        chat_service: &ChatService,
        contact_service: &ContactService,
        agent_service: &AgentService,
        harness: &Arc<crate::agent::harness::Harness>,
    ) -> Result<(), AppError> {
        self.run_sweep(
            SweepServices {
                chat: chat_service,
                contact: contact_service,
                agent: agent_service,
                harness,
            },
            SweepMode::Full,
        )
        .await
    }

    /// Mine eligible chat windows and stop after Extract. Intended for diagnostics and
    /// benchmarks that need to inspect raw extraction without downstream mutation.
    pub async fn run_extraction_sweep(
        &self,
        chat_service: &ChatService,
        contact_service: &ContactService,
        agent_service: &AgentService,
        harness: &Arc<crate::agent::harness::Harness>,
    ) -> Result<(), AppError> {
        self.run_sweep(
            SweepServices {
                chat: chat_service,
                contact: contact_service,
                agent: agent_service,
                harness,
            },
            SweepMode::ExtractOnly,
        )
        .await
    }

    async fn run_sweep(
        &self,
        services: SweepServices<'_>,
        mode: SweepMode,
    ) -> Result<(), AppError> {
        let reset_users = self.spawn_pending_reset_requests().await?;
        // The ontology catalogue is the one thing here that can legitimately be absent:
        // it ships in the image, so a checkout or a stripped install has none. Skipping
        // the tick is the right response rather than failing - the sweep is periodic, so
        // it simply resumes once a catalogue is in place. Running without one would
        // classify every entity against an empty TBox and write those types back, which is
        // far worse than doing nothing.
        if mode == SweepMode::Full && !self.ontology_manager.is_ready() {
            tracing::warn!("pkm consolidation skipped: no ontology catalogue loaded");
            return Ok(());
        }

        let idle_cutoff = Utc::now()
            - chrono::Duration::seconds(self.memory_config.pkm_consolidate_idle_secs as i64);
        let chat_ids = self.repo.chats_needing_consolidation(idle_cutoff).await?;

        // Work is partitioned by user, because a pass is per user: one record, one
        // agent, one vault, one ontology delta. A user whose pass is wedged then costs
        // only their own memory, not everyone's.
        let mut by_user: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for (chat_id, user_id) in self.repo.chat_owners(&chat_ids).await? {
            by_user.entry(user_id).or_default().push(chat_id);
        }
        // A pass that failed at, say, author has no chats left to mine - so it would
        // never be seen again if the only work source were eligible chats.
        for user_id in self.repo.users_with_open_consolidation().await? {
            by_user.entry(user_id).or_default();
        }
        tracing::debug!(
            chats = chat_ids.len(),
            users = by_user.len(),
            "pkm consolidation sweep"
        );

        for (user_id, mut chats) in by_user {
            if reset_users.contains(&user_id) {
                continue;
            }
            let Some(operation) = self.operations.try_begin_consolidation(&user_id) else {
                tracing::debug!(user = %user_id, "pkm consolidation skipped: user operation active");
                continue;
            };
            let cancel_token = operation.cancellation();
            chats.sort();
            if let Err(e) = self
                .consolidate_user(&user_id, &chats, services, mode, cancel_token)
                .await
            {
                tracing::warn!(error = %e, user = %user_id, "pkm consolidation: pass failed");
            }
        }
        Ok(())
    }

    /// At most one pass is live per user. If theirs is still backing off from a failure,
    /// the tick is skipped entirely - including mining, so a wedged pass does not keep
    /// accumulating unreconciled entities underneath itself.
    async fn consolidate_user(
        &self,
        user_id: &str,
        chat_ids: &[String],
        services: SweepServices<'_>,
        mode: SweepMode,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> Result<(), AppError> {
        if let Some(open) = self
            .repo
            .latest_consolidation_record(user_id)
            .await?
            .filter(|r| !r.state.is_done())
            && open.next_attempt_at > Utc::now()
        {
            tracing::debug!(
                user = %user_id,
                stage = open.state.label(),
                attempts = open.attempts,
                "pkm consolidation: backing off"
            );
            return Ok(());
        }

        // The record is opened **before** mining, not after. Two things depend on it:
        // a chat this pass already mined must not be mined again on a resume, and an
        // ingest failure has to charge an attempt - otherwise it never reaches the
        // backoff that suppresses the next tick, and a chat that fails at the same point
        // every time re-mints the same memories on every sweep, forever.
        let mut record = self.open_record(user_id).await?;
        let (extraction_tx, mut extraction_rx) = tokio::sync::mpsc::channel::<ExtractionWrite>(32);
        let extraction_repo = self.repo.clone();
        let extraction_user = user_id.to_string();
        let mut writer_record = record.clone();
        let extraction_writer = tokio::spawn(async move {
            while let Some(first) = extraction_rx.recv().await {
                // Give concurrently finishing chats one scheduling turn to reach the
                // channel, then drain a bounded patch without a timer or another task.
                tokio::task::yield_now().await;
                let mut patch = vec![first];
                while patch.len() < 16 {
                    match extraction_rx.try_recv() {
                        Ok(write) => patch.push(write),
                        Err(_) => break,
                    }
                }
                let Some(_state) = (match &mut writer_record.state {
                    ConsolidationStageState::Ingest(state) => Some(state),
                    _ => None,
                }) else {
                    for write in patch {
                        let _ = write
                            .done
                            .send(Err("extraction writer is no longer in ingest".into()));
                    }
                    continue;
                };
                let mut combined = crate::db::repo::pkm::IngestBatch::default();
                let mut watermarks = Vec::new();
                let mut short_memory_ids = Vec::new();
                for write in &mut patch {
                    combined.merge_from(&mut write.batch);
                    if let Some(watermark) = write.watermark.take() {
                        watermarks.push(watermark);
                    }
                    short_memory_ids.append(&mut write.short_memory_ids);
                }
                // These counters describe the same windows as `combined`. Bank them in
                // the checkpoint passed to the transaction instead of reconstructing
                // them after commit: the watermark, extracted rows, pending state, and
                // grounding diagnostics then survive (or roll back) together.
                writer_record.stats.absorb_ingest_batch(&combined);
                match extraction_repo
                    .commit_extract_patch_with_checkpoint(
                        &extraction_user,
                        &combined,
                        &watermarks,
                        &short_memory_ids,
                        &writer_record,
                    )
                    .await
                {
                    Ok(counts) => {
                        let mut counts = Some(counts);
                        for write in patch {
                            let _ = write.done.send(Ok(counts.take().unwrap_or_default()));
                        }
                    }
                    Err(error) => {
                        let message = error.to_string();
                        for write in patch {
                            let _ = write.done.send(Err(message.clone()));
                        }
                        return Err(message);
                    }
                }
            }
            Ok(writer_record)
        });
        let already_mined = match &record.state {
            ConsolidationStageState::Ingest(st) => st.mined.clone(),
            // Past ingest: the mining half of this pass is done, whatever is left is
            // the consolidator's. Re-mining now would add entities the later stages have already
            // walked past.
            _ => chat_ids.iter().cloned().collect(),
        };
        let chat_ids: Vec<String> = chat_ids
            .iter()
            .filter(|c| !already_mined.contains(*c))
            .cloned()
            .collect();

        // Mine every chat before consolidating. The user-scoped stages that follow read the
        // whole dirty entity set, so running them per chat would repeat them - and their
        // LLM calls - once per chat that touched an entity.
        //
        // Chats are mined concurrently: extract is chat-scoped and the LLM call
        // dominates its cost. Bounded, because a fresh install replaying its history
        // would otherwise open one request per chat at once. Two chats naming the same
        // entity race on `upsert_entity_skeleton`, which recovers from a lost insert
        // rather than failing.
        let concurrency = self.memory_config.pkm_consolidation_concurrency.max(1);
        let extraction_slots = Arc::new(tokio::sync::Semaphore::new(concurrency));
        type IngestOutcome = Result<Option<(ConsolidationScope, ConsolidationStats)>, AppError>;
        // Boxed rather than bare `async move` blocks, for the same reason the author
        // stage boxes its jobs: an inline future borrowing `&self` here is only provably
        // `Send` for one specific lifetime, which breaks the scheduler's `Send` bound
        // several layers up.
        let jobs: Vec<futures::future::BoxFuture<'_, (String, IngestOutcome)>> = chat_ids
            .iter()
            .map(|chat_id| {
                let schedule = ExtractionSchedule {
                    sender: extraction_tx.clone(),
                    slots: extraction_slots.clone(),
                    per_chat_concurrency: concurrency,
                };
                let chat_cancel = cancel_token.clone();
                Box::pin(async move {
                    let outcome = self
                        .drain_chat(chat_id, services, schedule, chat_cancel)
                        .await;
                    (chat_id.clone(), outcome)
                }) as futures::future::BoxFuture<'_, (String, IngestOutcome)>
            })
            .collect();
        let collection = futures::stream::iter(jobs)
            .buffer_unordered(concurrency)
            .collect::<Vec<(String, IngestOutcome)>>();
        tokio::pin!(collection);
        let mut results = tokio::select! {
            results = &mut collection => results,
            _ = cancel_token.cancelled() => {
                drop(extraction_tx);
                extraction_writer.abort();
                let _ = extraction_writer.await;
                return Err(AppError::Internal("PKM consolidation was cancelled for reset".into()));
            }
        };
        drop(extraction_tx);
        record = extraction_writer
            .await
            .map_err(|error| AppError::Internal(format!("extraction writer task failed: {error}")))?
            .map_err(AppError::Internal)?;

        // Completion order is nondeterministic; sort so the scope chosen (and therefore
        // the agent the consolidation runs as) does not vary run to run.
        results.sort_by(|a, b| a.0.cmp(&b.0));
        let mut mined: Option<ConsolidationScope> = None;
        let mut ingested = ConsolidationStats::default();
        let mut done: std::collections::BTreeSet<String> = already_mined;
        let mut failure: Option<AppError> = None;
        for (chat_id, outcome) in results {
            match outcome {
                // The lowest-ordered chat supplies the agent/handle/directory the
                // consolidation runs under. Counts, unlike the scope, ACCUMULATE because
                // the record's stats cover the whole pass, mining included.
                Ok(Some((scope, stats))) => {
                    mined.get_or_insert(scope);
                    ingested.merge(stats);
                    done.insert(chat_id);
                }
                // Skipped - a heartbeat chat, or nothing new past the watermark. Banked
                // all the same: there is nothing here to come back for.
                Ok(None) => {
                    done.insert(chat_id);
                }
                Err(e) => {
                    tracing::warn!(error = %e, chat = %chat_id, "pkm ingest failed");
                    failure.get_or_insert(e);
                }
            }
        }

        // Bank what mined before deciding what to do about what didn't, so a retry does
        // not re-read a transcript this pass has already consumed.
        if matches!(&record.state, ConsolidationStageState::Ingest(_)) {
            record.state = ConsolidationStageState::Ingest(IngestState { mined: done });
            record.stats.merge(ingested.clone());
            if let Err(e) = self.repo.save_consolidation_record(&record).await {
                tracing::warn!(error = %e, user = %user_id, "pkm consolidation: checkpoint failed");
            }
        }

        // A chat that would not mine parks the pass exactly as a failed consolidation
        // stage does - charge the attempt, back off, and leave the rest for the retry.
        // Consolidating a half-mined window would bake an incomplete picture into the entities.
        if let Some(e) = failure {
            self.record_failure(&mut record, &e).await;
            return Ok(());
        }

        if mode == SweepMode::ExtractOnly {
            tracing::debug!(user = %user_id, ?ingested, "pkm extracted user");
            return Ok(());
        }

        let scope = match mined {
            Some(s) => s,
            None => match self.resume_scope(user_id, services.agent).await? {
                Some(s) => s,
                None => return Ok(()),
            },
        };

        if cancel_token.is_cancelled() {
            return Err(AppError::Internal(
                "PKM consolidation was cancelled for reset".into(),
            ));
        }
        let stats = self
            .consolidate_with_cancel(scope, services.harness.clone(), cancel_token)
            .await?;
        tracing::debug!(user = %user_id, ?stats, "pkm consolidated user");
        Ok(())
    }

    /// A scope for a pass with nothing left to mine - the resume case, where the record
    /// is mid-flight but every chat is already consolidated. Runs under the user's
    /// first available agent, since no chat is supplying one.
    async fn resume_scope(
        &self,
        user_id: &str,
        agent_service: &AgentService,
    ) -> Result<Option<ConsolidationScope>, AppError> {
        let open = self
            .repo
            .latest_consolidation_record(user_id)
            .await?
            .filter(|r| !r.state.is_done());
        if open.is_none() {
            return Ok(None);
        }
        let Some(user) = self.user_service.find_by_id(user_id).await? else {
            return Ok(None);
        };
        let Some(agent) = agent_service.list(user_id).await?.into_iter().next() else {
            return Ok(None);
        };
        let vault =
            VaultScope::resolve(&self.user_service, &self.storage, user_id, &user.handle).await?;
        Ok(Some(ConsolidationScope {
            user_id: user_id.to_string(),
            user_name: user.name,
            agent_id: agent.id,
            chat_id: None,
            vault,
            temporal_sources: Vec::new(),
            evidence_sources: Vec::new(),
            recall: Default::default(),
            timezone: user.timezone.unwrap_or_else(|| "UTC".to_string()),
        }))
    }

    /// Freeze every currently eligible extraction window for one chat. Mining may run
    /// these snapshots concurrently because extraction does not read pending output
    /// from an earlier window; only the ordered commit below mutates checkpoint state.
    async fn prepare_chat_windows(
        &self,
        chat_id: &str,
        services: SweepServices<'_>,
    ) -> Result<Vec<PreparedExtractionWindow>, AppError> {
        let Some(chat) = services.chat.find_chat(chat_id).await? else {
            return Ok(Vec::new());
        };
        if let Ok(Some(agent)) = services.agent.find_by_id(&chat.agent_id).await
            && agent.heartbeat_chat_id.as_deref() == Some(chat_id)
        {
            return Ok(Vec::new());
        }
        let Some(user) = self.user_service.find_by_id(&chat.user_id).await? else {
            return Ok(Vec::new());
        };

        let watermark = self
            .repo
            .consolidation_watermark(chat_id)
            .await?
            .unwrap_or(DateTime::<Utc>::MIN_UTC);
        let all_messages = services.chat.list_messages(&chat.user_id, chat_id).await?;
        let task_result_links = completed_task_result_links(&all_messages);
        let agent_message_ids = all_messages
            .iter()
            .filter(|message| message.role == MessageRole::Agent)
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        let chat_tool_calls = self
            .tool_calls
            .find_by_chat_id(chat_id)
            .await
            .unwrap_or_default();
        let mut windows = consolidation_windows(
            all_messages,
            watermark,
            self.memory_config.pkm_extract_max_tokens.max(1),
            self.memory_config.pkm_extract_max_messages.max(1),
            &chat_tool_calls,
        )?;
        let short = self.repo.unconsolidated_short_memories(chat_id).await?;
        if windows.is_empty() {
            if short.is_empty() {
                return Ok(Vec::new());
            }
            windows.push((Vec::new(), None));
        }
        let vault = VaultScope::resolve(
            &self.user_service,
            &self.storage,
            &chat.user_id,
            &user.handle,
        )
        .await?;
        let short_ids: Vec<String> = short.iter().map(|memory| memory.id.clone()).collect();
        let mut prepared = Vec::with_capacity(windows.len());
        for (ordinal, (new_messages, advance_to)) in windows.into_iter().enumerate() {
            let window_short = if ordinal == 0 { short.as_slice() } else { &[] };
            let assertion_message_ids = new_messages
                .iter()
                .filter(|message| message.role == MessageRole::Agent)
                .map(|message| message.id.clone())
                .collect::<Vec<_>>();
            let mut task_evidence = std::collections::HashMap::new();
            for assertion in &assertion_message_ids {
                let Some(task_ids) = task_result_links.get(assertion) else {
                    continue;
                };
                let calls = collect_task_tree_tool_calls(
                    task_ids,
                    &services.harness.task_service,
                    &self.tool_calls,
                )
                .await?;
                if !calls.is_empty() {
                    task_evidence.insert(assertion.clone(), calls);
                }
            }
            let (transcript, temporal_sources, evidence_sources, recall) = self
                .build_transcript(TranscriptInput {
                    messages: &new_messages,
                    short: window_short,
                    contact_service: services.contact,
                    task_service: &services.harness.task_service,
                    tool_calls: &chat_tool_calls,
                    agent_message_ids: &agent_message_ids,
                    assertion_message_ids: &assertion_message_ids,
                    task_evidence: &task_evidence,
                    user_id: &chat.user_id,
                    vault: &vault,
                })
                .await;
            prepared.push(PreparedExtractionWindow {
                scope: ConsolidationScope {
                    user_id: chat.user_id.clone(),
                    user_name: user.name.clone(),
                    agent_id: chat.agent_id.clone(),
                    chat_id: Some(chat_id.to_string()),
                    vault: vault.clone(),
                    temporal_sources,
                    evidence_sources,
                    recall,
                    timezone: user.timezone.clone().unwrap_or_else(|| "UTC".to_string()),
                },
                transcript,
                watermark: advance_to.map(|until| (chat_id.to_string(), until)),
                short_memory_ids: if ordinal == 0 {
                    short_ids.clone()
                } else {
                    Vec::new()
                },
                new_messages: new_messages.len(),
            });
        }
        Ok(prepared)
    }

    /// Drain every currently eligible extraction window for one chat before the
    /// user-scoped stages begin.
    ///
    /// Model requests overlap, but `buffered` yields them in source order. Each yielded
    /// window then commits its rows, short-memory flags, and watermark atomically through
    /// the single writer. A failed earlier window therefore prevents a completed later
    /// window from advancing the durable state past it.
    async fn drain_chat(
        &self,
        chat_id: &str,
        services: SweepServices<'_>,
        schedule: ExtractionSchedule,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> Result<Option<(ConsolidationScope, ConsolidationStats)>, AppError> {
        let windows = self.prepare_chat_windows(chat_id, services).await?;
        if windows.is_empty() {
            return Ok(None);
        }
        let mut mined: Option<ConsolidationScope> = None;
        let mut stats = ConsolidationStats::default();

        let jobs = windows.into_iter().map(|prepared| {
            let extraction_slots = schedule.slots.clone();
            let harness = services.harness.clone();
            let cancel_token = cancel_token.clone();
            async move {
                let _permit = tokio::select! {
                    permit = extraction_slots.acquire_owned() => permit
                        .map_err(|_| AppError::Internal("extraction scheduler stopped".into()))?,
                    _ = cancel_token.cancelled() => {
                        return Err(AppError::Internal("PKM extraction was cancelled for reset".into()));
                    }
                };
                let batch = self
                    .mine_window_with_cancel(
                        prepared.scope.clone(),
                        &prepared.transcript,
                        harness,
                        cancel_token,
                    )
                    .await?;
                Ok::<_, AppError>(MinedExtractionWindow { prepared, batch })
            }
        });
        let mut completed =
            futures::stream::iter(jobs).buffered(schedule.per_chat_concurrency.max(1));
        while let Some(result) = completed.next().await {
            let window = result?;
            let MinedExtractionWindow { prepared, batch } = window;
            let (done, committed) = tokio::sync::oneshot::channel();
            schedule
                .sender
                .send(ExtractionWrite {
                    batch,
                    watermark: prepared.watermark,
                    short_memory_ids: prepared.short_memory_ids,
                    done,
                })
                .await
                .map_err(|_| AppError::Internal("extraction writer stopped".into()))?;
            let counts = committed
                .await
                .map_err(|_| AppError::Internal("extraction writer dropped commit result".into()))?
                .map_err(AppError::Internal)?;
            let mut window_stats = ConsolidationStats::default();
            window_stats.absorb_ingest_counts(&counts);
            tracing::debug!(
                chat = %chat_id,
                new_messages = prepared.new_messages,
                ?window_stats,
                "pkm ingested chat"
            );
            if !prepared.transcript.trim().is_empty() {
                mined.get_or_insert(prepared.scope);
                stats.merge(window_stats);
            }
        }

        Ok(mined.map(|scope| (scope, stats)))
    }

    /// Render new messages + un-consolidated short memories into one speaker-labeled
    /// transcript, interleaved by `created_at`: contact names resolved for group
    /// chats, `System` dropped, short memories as `[remembered: …]`.
    async fn build_transcript(
        &self,
        input: TranscriptInput<'_>,
    ) -> (
        String,
        Vec<super::consolidation::TemporalSource>,
        Vec<super::consolidation::TranscriptEvidenceSource>,
        super::consolidation::RecallProjection,
    ) {
        let TranscriptInput {
            messages,
            short,
            contact_service,
            task_service,
            tool_calls,
            agent_message_ids,
            assertion_message_ids,
            task_evidence,
            user_id,
            vault,
        } = input;
        let current_message_ids = messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let current_tool_calls = tool_calls
            .iter()
            .filter(|call| current_message_ids.contains(call.message_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let is_memory_path = |value: &str| {
            self.storage.is_user_pkm_path(vault.handle(), value)
                || vault.page_from_any(value).is_some()
                || value.split_whitespace().any(|token| {
                    let token = token.trim_matches(|character: char| {
                        matches!(character, '"' | '\'' | ',' | ')' | ']' | '}')
                    });
                    let token = token.strip_prefix("path=").unwrap_or(token);
                    self.storage.is_user_pkm_path(vault.handle(), token)
                        || vault.page_from_any(token).is_some()
                })
        };
        let evidence = super::consolidation::ToolEvidenceProjection::new_with_task_evidence(
            tool_calls,
            agent_message_ids,
            assertion_message_ids,
            task_evidence,
            self.memory_config
                .pkm_extract_agent_evidence_lookback_messages,
            self.memory_config
                .pkm_extract_agent_evidence_result_token_cap,
            is_memory_path,
        );
        let recall =
            super::consolidation::RecallProjection::new(&current_tool_calls, is_memory_path)
                .with_evidence(evidence);
        tracing::debug!(
            recall_calls = recall.len(),
            recall_preview_chars = recall.preview_chars(),
            "pkm extract recall projection"
        );
        enum Item<'a> {
            Msg(&'a MessageResponse),
            Mem(&'a KnowledgeShortMemory),
            Task(&'a crate::inference::tool_call::ToolCall),
            Hitl(&'a crate::inference::tool_call::ToolCall),
        }
        let mut items: Vec<(DateTime<Utc>, Item)> = Vec::new();
        for m in messages {
            items.push((m.created_at, Item::Msg(m)));
        }
        for s in short {
            items.push((s.created_at, Item::Mem(s)));
        }
        for call in tool_calls {
            if !current_message_ids.contains(call.message_id.as_str()) {
                continue;
            }
            if call.success && matches!(call.name.as_str(), "create_task" | "create_recurring_task")
            {
                items.push((call.created_at, Item::Task(call)));
            }
            if call.hitl.as_ref().is_some_and(|hitl| {
                hitl.status == crate::inference::tool_call::ToolStatus::Resolved
                    && matches!(
                        &hitl.response,
                        Some(
                            crate::inference::hitl::HitlResponse::Choice(_)
                                | crate::inference::hitl::HitlResponse::Approval(_)
                        )
                    )
            }) {
                items.push((call.created_at, Item::Hitl(call)));
            }
        }
        items.sort_by_key(|(t, _)| *t);

        let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut out = String::new();
        let mut temporal_sources = Vec::new();
        let mut evidence_sources = Vec::new();
        let mut next_handle = 1usize;
        for (created_at, item) in items {
            match item {
                Item::Msg(m) => {
                    if m.role == MessageRole::TaskCompletion {
                        let Some(MessageEvent::TaskCompletion {
                            task_id, status, ..
                        }) = &m.event
                        else {
                            continue;
                        };
                        let Ok(Some(task)) = task_service.find_by_id(task_id).await else {
                            continue;
                        };
                        let lifecycle = match status {
                            crate::agent::task::models::TaskStatus::Completed => "completed",
                            crate::agent::task::models::TaskStatus::Failed => "failed",
                            crate::agent::task::models::TaskStatus::Cancelled => "cancelled",
                            _ => continue,
                        };
                        let target_at = task_target_at(&task);
                        let text =
                            render_task_lifecycle(lifecycle, &task.title, created_at, target_at);
                        let handle = format!("m{next_handle}");
                        next_handle += 1;
                        super::consolidation::transcript::push_task(&mut out, &handle, &text);
                        evidence_sources.push(super::consolidation::TranscriptEvidenceSource {
                            handle: handle.clone(),
                            text: text.clone(),
                            kind: super::consolidation::TranscriptEvidenceKind::TaskLifecycle {
                                message_id: m.id.clone(),
                                chat_id: m.chat_id.clone(),
                                task_id: task_id.clone(),
                            },
                        });
                        temporal_sources.push(super::consolidation::TemporalSource {
                            handle,
                            text,
                            created_at,
                            task_event_at: Some(created_at),
                            task_target_at: target_at,
                        });
                        continue;
                    }
                    let agent_text = if m.role == MessageRole::Agent {
                        Some(super::consolidation::transcript::message_text(
                            &m.id, &m.content, tool_calls,
                        ))
                    } else {
                        None
                    };
                    let text = agent_text
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(|| m.content.trim().to_string());
                    if text.is_empty() {
                        continue;
                    }
                    let speaker = match m.role {
                        MessageRole::User => "User".to_string(),
                        MessageRole::Agent => "Agent".to_string(),
                        MessageRole::Contact => match &m.contact_id {
                            Some(cid) => {
                                if let Some(n) = names.get(cid) {
                                    n.clone()
                                } else {
                                    let n = contact_service
                                        .get(user_id, cid)
                                        .await
                                        .map(|c| c.name)
                                        .unwrap_or_else(|_| "Contact".to_string());
                                    names.insert(cid.clone(), n.clone());
                                    n
                                }
                            }
                            None => "Contact".to_string(),
                        },
                        _ => continue,
                    };
                    let handle = format!("m{next_handle}");
                    next_handle += 1;
                    if let Some(agent_text) = &agent_text {
                        super::consolidation::transcript::push_agent_message(
                            &mut out,
                            &handle,
                            agent_text,
                            &recall.render_for_message(&m.id, &handle),
                        );
                    } else {
                        super::consolidation::transcript::push_message(
                            &mut out, &handle, &speaker, &text,
                        );
                    }
                    let kind = match m.role {
                        MessageRole::User => {
                            Some(super::consolidation::TranscriptEvidenceKind::UserMessage {
                                message_id: m.id.clone(),
                                chat_id: m.chat_id.clone(),
                            })
                        }
                        MessageRole::Agent => {
                            Some(super::consolidation::TranscriptEvidenceKind::AgentMessage {
                                message_id: m.id.clone(),
                                agent_id: m.agent_id.clone().unwrap_or_default(),
                                chat_id: m.chat_id.clone(),
                            })
                        }
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        evidence_sources.push(super::consolidation::TranscriptEvidenceSource {
                            handle: handle.clone(),
                            text: text.clone(),
                            kind,
                        });
                    }
                    temporal_sources.push(super::consolidation::TemporalSource {
                        handle,
                        text,
                        created_at,
                        task_event_at: None,
                        task_target_at: None,
                    });
                }
                Item::Mem(s) => {
                    super::consolidation::transcript::push_remembered(&mut out, &s.content)
                }
                Item::Task(call) => {
                    let Some(title) = call.arguments.get("title").and_then(|value| value.as_str())
                    else {
                        continue;
                    };
                    let title = title.trim();
                    if title.is_empty() {
                        continue;
                    }
                    let Some(task_id) = serde_json::from_str::<serde_json::Value>(&call.result)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("task_id")
                                .or_else(|| value.get("id"))
                                .and_then(|id| id.as_str())
                                .map(str::to_string)
                        })
                    else {
                        continue;
                    };
                    let target_at = match task_service.find_by_id(&task_id).await {
                        Ok(task) => task.as_ref().and_then(task_target_at),
                        Err(error) => {
                            tracing::warn!(%task_id, %error, "pkm extract could not load scheduled task time");
                            None
                        }
                    };
                    let text = render_task_lifecycle("scheduled", title, created_at, target_at);
                    let handle = format!("m{next_handle}");
                    next_handle += 1;
                    super::consolidation::transcript::push_task(&mut out, &handle, &text);
                    evidence_sources.push(super::consolidation::TranscriptEvidenceSource {
                        handle: handle.clone(),
                        text: text.clone(),
                        kind: super::consolidation::TranscriptEvidenceKind::TaskLifecycle {
                            message_id: call.message_id.clone(),
                            chat_id: call.chat_id.clone(),
                            task_id,
                        },
                    });
                    temporal_sources.push(super::consolidation::TemporalSource {
                        handle,
                        text,
                        created_at,
                        task_event_at: Some(created_at),
                        task_target_at: target_at,
                    });
                }
                Item::Hitl(call) => {
                    let Some(hitl) = &call.hitl else { continue };
                    let text = match &hitl.response {
                        Some(crate::inference::hitl::HitlResponse::Choice(value)) => value.clone(),
                        Some(crate::inference::hitl::HitlResponse::Approval(value)) => {
                            if *value {
                                "approved".to_string()
                            } else {
                                "denied".to_string()
                            }
                        }
                        _ => continue,
                    };
                    let handle = format!("m{next_handle}");
                    next_handle += 1;
                    super::consolidation::transcript::push_message(
                        &mut out,
                        &handle,
                        "User",
                        text.trim(),
                    );
                    evidence_sources.push(super::consolidation::TranscriptEvidenceSource {
                        handle: handle.clone(),
                        text: text.clone(),
                        kind: super::consolidation::TranscriptEvidenceKind::UserMessage {
                            message_id: call.message_id.clone(),
                            chat_id: call.chat_id.clone(),
                        },
                    });
                    temporal_sources.push(super::consolidation::TemporalSource {
                        handle,
                        text,
                        created_at,
                        task_event_at: None,
                        task_target_at: None,
                    });
                }
            }
        }
        (out, temporal_sources, evidence_sources, recall)
    }
}

mod window;

use window::consolidation_windows;

#[cfg(test)]
mod tests;
