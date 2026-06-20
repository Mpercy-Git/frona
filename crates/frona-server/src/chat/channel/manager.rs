use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::http::Request;
use axum::response::Response;
use chrono::Utc;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::agent::models::Agent;
use crate::chat::broadcast::{BroadcastEvent, BroadcastEventKind, EntityAction};
use crate::chat::message::models::{DeliveryState, Message, MessageRole, MessageStatus};
use crate::chat::channel::service::ChannelService;
use crate::chat::message::repository::MessageRepository;
use crate::chat::service::ChatService;
use crate::contact::models::Contact;
use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::inference::conversation::ChannelConversationBuilder;
use crate::inference::tool_loop::InferenceEventKind;
use crate::policy::models::{PolicyAction, PolicyContact};

use super::models::{
    Channel, ChannelAdapter, ChannelCtx, ChannelStatus, ChatType, DispatchMode, ExternalMessage,
};

const INBOUND_BUFFER: usize = 64;

const DELIVERY_MAX_ATTEMPTS: u32 = 5;

const DELIVERY_BACKOFF: &[Duration] = &[
    Duration::from_secs(5),
    Duration::from_secs(25),
    Duration::from_secs(120),
    Duration::from_secs(600),
];

const DELIVERY_RETRY_BATCH: u32 = 50;

fn backoff_for(attempts: u32) -> Duration {
    if attempts == 0 {
        return Duration::from_secs(0);
    }
    let idx = (attempts as usize - 1).min(DELIVERY_BACKOFF.len() - 1);
    DELIVERY_BACKOFF[idx]
}

pub(super) fn channel_data_dir(
    storage: &crate::storage::service::StorageService,
    user_handle: &crate::core::Handle,
    channel_handle: &crate::core::Handle,
) -> std::path::PathBuf {
    storage.channel_data_path(user_handle, channel_handle)
}

// Lifecycle-only writes must not trigger a restart loop via the watcher.
fn channel_needs_restart(prior: &Channel, next: &Channel) -> bool {
    prior.provider != next.provider
        || prior.agent_id != next.agent_id
        || prior.dispatch_mode != next.dispatch_mode
        || prior.config != next.config
}

pub enum CarrierStatus {
    Delivered,
    Failed { error: String },
}


pub(super) struct ChannelTask {
    cancel: CancellationToken,
    pub(super) adapter: Arc<dyn ChannelAdapter>,
    pub(super) ctx: ChannelCtx,
}

pub struct ChannelManager {
    tasks: Arc<Mutex<HashMap<String, ChannelTask>>>,
    retry_cancels: Arc<Mutex<HashMap<String, CancellationToken>>>,
    message_repo: Arc<dyn MessageRepository>,
    chat_service: ChatService,
    channel_service: Arc<ChannelService>,
    harness: Arc<crate::agent::harness::Harness>,
    task_executor: Arc<crate::agent::task::executor::TaskExecutor>,
}

impl ChannelManager {
    pub fn new(
        message_repo: Arc<dyn MessageRepository>,
        chat_service: ChatService,
        channel_service: Arc<ChannelService>,
        harness: Arc<crate::agent::harness::Harness>,
        task_executor: Arc<crate::agent::task::executor::TaskExecutor>,
    ) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            retry_cancels: Arc::new(Mutex::new(HashMap::new())),
            message_repo,
            chat_service,
            channel_service,
            harness,
            task_executor,
        }
    }

    pub async fn report_failure(&self, channel_id: &str, reason: String) {
        if let Err(e) = self
            .channel_service
            .mark_status(channel_id, ChannelStatus::Failed, Some(reason))
            .await
        {
            tracing::warn!(
                channel_id = %channel_id,
                error = %e,
                "report_failure: could not persist Failed status",
            );
        }
    }

    /// Manager does NOT restart the channel; adapter must keep running across the transition.
    pub async fn report_setup_complete(&self, channel_id: &str) {
        let pair = {
            let tasks = self.tasks.lock().await;
            tasks.get(channel_id).map(|t| (t.adapter.clone(), t.ctx.clone()))
        };
        if let Some((adapter, ctx)) = pair
            && let Err(e) = adapter.on_setup_complete(&ctx).await
        {
            tracing::warn!(
                channel_id = %channel_id,
                error = %e,
                "on_setup_complete failed; continuing with state transition",
            );
        }
        if let Err(e) = self
            .channel_service
            .complete_setup(channel_id)
            .await
        {
            tracing::warn!(
                channel_id = %channel_id,
                error = %e,
                "report_setup_complete: could not persist Connected status",
            );
        }
    }

    /// Idempotent: no-op if a retry-loop is already in flight. To force a
    /// restart, callers must `stop_channel` first to clear the retry slot.
    pub fn start_with_retry(self: Arc<Self>, state: AppState, channel_id: String) {
        let cancel = state.shutdown_token.child_token();
        let manager = self.clone();
        let id_for_task = channel_id.clone();
        let cancel_for_task = cancel.clone();
        tokio::spawn(async move {
            let inserted = {
                let mut map = manager.retry_cancels.lock().await;
                if map.contains_key(&id_for_task) {
                    false
                } else {
                    map.insert(id_for_task.clone(), cancel_for_task.clone());
                    true
                }
            };
            if !inserted {
                return;
            }
            manager
                .clone()
                .retry_loop(state, id_for_task.clone(), cancel_for_task)
                .await;
            let mut map = manager.retry_cancels.lock().await;
            map.remove(&id_for_task);
        });
    }

    async fn retry_loop(
        self: Arc<Self>,
        state: AppState,
        channel_id: String,
        cancel: CancellationToken,
    ) {
        let mut attempt: u32 = 0;
        loop {
            let channel = match self.channel_service.find_by_id(&channel_id).await {
                Ok(c) => c,
                Err(_) => return,
            };
            // Excludes Connected so post-restart re-starts pick up zombie rows.
            if matches!(channel.status, ChannelStatus::Setup | ChannelStatus::Pairing) {
                return;
            }

            let retry_cfg = channel
                .retry
                .clone()
                .unwrap_or_else(|| state.config.channel.retry.clone());
            if attempt > 0 && attempt > retry_cfg.max_retries {
                tracing::warn!(
                    channel_id = %channel_id,
                    attempts = %attempt,
                    "channel retry exhausted; leaving Failed for operator",
                );
                return;
            }

            match self.start_channel(&state, &channel).await {
                Ok(()) => return,
                Err(e) => {
                    if retry_cfg.max_retries == 0 {
                        tracing::warn!(
                            channel_id = %channel_id,
                            error = %e,
                            "channel retry disabled; one-shot start failed",
                        );
                        return;
                    }
                    attempt = attempt.saturating_add(1);
                    let factor = retry_cfg
                        .backoff_multiplier
                        .powi(attempt.saturating_sub(1) as i32);
                    let delay = (retry_cfg.initial_backoff_ms as f64 * factor)
                        .min(retry_cfg.max_backoff_ms as f64) as u64;
                    tracing::info!(
                        channel_id = %channel_id,
                        attempt = %attempt,
                        delay_ms = %delay,
                        "channel start failed; retrying after backoff",
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_millis(delay)) => {},
                        _ = cancel.cancelled() => return,
                    }
                }
            }
        }
    }

    pub async fn start(self: Arc<Self>, state: AppState) -> Result<(), AppError> {
        if let Err(e) = state.channel_service.revert_orphaned_pairings().await {
            tracing::warn!(error = %e, "ChannelManager: failed to revert orphaned pairings");
        }
        let channels = state.channel_service.find_active().await.unwrap_or_default();
        for channel in channels {
            self.clone().start_with_retry(state.clone(), channel.id);
        }
        self.clone().spawn_broadcast_watcher(state);
        Ok(())
    }

    pub async fn start_channel(
        &self,
        state: &AppState,
        channel: &Channel,
    ) -> Result<(), AppError> {
        self.stop_running_task(&channel.id).await;

        // Don't mark Setup here — the watcher would loop. `Setup` is owned by service.create/update.
        let missing = state.channel_service.missing_required(channel).await?;
        if !missing.is_empty() {
            return Err(AppError::Validation(format!(
                "missing required field(s): {}",
                missing.join(", ")
            )));
        }

        let setup = async {
            let factory = state
                .channel_registry
                .get_factory(&channel.provider)
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "no in-process factory registered for provider {:?}",
                        channel.provider,
                    ))
                })?;
            let config = state.channel_service.resolve_config(channel).await?;
            let adapter: Arc<dyn ChannelAdapter> = Arc::from(factory.create(config)?);
            let space = state
                .space_service
                .find_by_id(&channel.space_id)
                .await?
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "channel {:?} references missing space {:?}",
                        channel.id, channel.space_id,
                    ))
                })?;
            Ok::<_, AppError>((adapter, space))
        }
        .await;
        let (adapter, space) = match setup {
            Ok(v) => v,
            Err(e) => {
                state
                    .channel_service
                    .mark_status(&channel.id, ChannelStatus::Failed, Some(e.to_string()))
                    .await
                    .ok();
                return Err(e);
            }
        };

        let (emit, rx) = mpsc::channel::<ExternalMessage>(INBOUND_BUFFER);

        let webhook_base = state.config.server.external_or_local_base_url();
        let webhook_url = format!(
            "{}{}/{}/{}",
            webhook_base.trim_end_matches('/'),
            super::WEBHOOK_PATH_PREFIX,
            channel.provider,
            channel.id,
        );

        let handle = state
            .user_service
            .find_by_id(&channel.user_id)
            .await?
            .map(|u| u.handle)
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "channel {:?} references missing user {:?}",
                    channel.id, channel.user_id,
                ))
            })?;
        let data_dir = channel_data_dir(&state.storage_service, &handle, &channel.handle);
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            return Err(AppError::Internal(format!(
                "could not create channel data dir {}: {e}",
                data_dir.display(),
            )));
        }

        let cancel = state.shutdown_token.child_token();

        let ctx = ChannelCtx {
            space,
            channel: channel.clone(),
            emit,
            webhook_url,
            channel_manager: state.channel_manager.clone(),
            storage_service: state.storage_service.clone(),
            user_service: state.user_service.clone(),
            chat_service: state.chat_service.clone(),
            data_dir,
            base_url: state.config.server.external_or_local_base_url(),
            share_service: state.share_service.clone(),
            share_ttl_secs: state.config.share.ttl_secs,
            cancel: cancel.clone(),
        };

        // Register before setup/connect: those hooks call back into the manager.
        {
            let mut tasks = self.tasks.lock().await;
            tasks.insert(
                channel.id.clone(),
                ChannelTask {
                    cancel: cancel.clone(),
                    adapter: adapter.clone(),
                    ctx: ctx.clone(),
                },
            );
        }

        // Pipelines spawn before setup/connect so adapters can emit from those hooks.
        {
            let adapter = adapter.clone();
            let ctx = ctx.clone();
            let state = state.clone();
            let cancel = cancel.clone();
            let channel_id = channel.id.clone();
            tokio::spawn(async move {
                if let Err(e) = run_outbound(adapter, ctx, state, cancel).await {
                    tracing::warn!(channel_id = %channel_id, error = %e, "channel outbound task exited with error");
                }
            });
        }
        {
            let ctx = ctx.clone();
            let state = state.clone();
            let cancel = cancel.clone();
            let channel_id = channel.id.clone();
            tokio::spawn(async move {
                if let Err(e) = run_inbound_pipeline(ctx, state, rx, cancel).await {
                    tracing::warn!(channel_id = %channel_id, error = %e, "channel inbound pipeline exited with error");
                }
            });
        }

        match adapter.on_setup_begin(&ctx).await {
            Ok(Some(setup)) => {
                state
                    .channel_service
                    .begin_setup(&channel.id, setup)
                    .await?;
                return Ok(());
            }
            Ok(None) => {}
            Err(e) => {
                state
                    .channel_service
                    .mark_status(&channel.id, ChannelStatus::Failed, Some(e.to_string()))
                    .await?;
                return Err(AppError::Internal(e.to_string()));
            }
        }

        let connect_result = adapter.on_connect(&ctx).await;
        let connect_error = connect_result.as_ref().err().map(|e| e.to_string());
        if let Some(err) = &connect_error {
            tracing::warn!(channel_id = %channel.id, error = %err, "channel.on_connect failed");
        }
        let connected = connect_result.is_ok();

        if connected {
            state
                .channel_service
                .mark_status(&channel.id, ChannelStatus::Connected, None)
                .await?;
            if let Err(e) = self.resume_deliveries(channel).await {
                tracing::warn!(
                    channel_id = %channel.id,
                    error = %e,
                    "delivery resume_deliveries failed at spawn",
                );
            }
            if let Err(e) = self.reconcile_message_delivery(channel).await {
                tracing::warn!(
                    channel_id = %channel.id,
                    error = %e,
                    "outbound: reconcile_message_delivery failed at spawn",
                );
            }
            Ok(())
        } else {
            state
                .channel_service
                .mark_status(&channel.id, ChannelStatus::Failed, connect_error)
                .await?;
            // Propagate so retry_loop's Err branch applies backoff
            // (otherwise the Failed broadcast triggers an immediate retry).
            Err(connect_result.unwrap_err())
        }
    }

    pub async fn stop(&self) {
        let mut tasks = self.tasks.lock().await;
        for (_, task) in tasks.drain() {
            task.cancel.cancel();
        }
    }

    pub async fn stop_channel(&self, channel_id: &str) {
        self.stop_running_task(channel_id).await;
        let retry = {
            let mut map = self.retry_cancels.lock().await;
            map.remove(channel_id)
        };
        if let Some(c) = retry {
            c.cancel();
        }
    }

    /// Leaves any active retry loop alive (otherwise it would cancel itself).
    async fn stop_running_task(&self, channel_id: &str) {
        let task = {
            let mut tasks = self.tasks.lock().await;
            tasks.remove(channel_id)
        };
        if let Some(task) = task {
            task.cancel.cancel();
        }
    }

    pub async fn dispatch_inbound_webhook(
        &self,
        channel_id: &str,
        request: Request<Bytes>,
    ) -> Result<Response, AppError> {
        let (adapter, ctx) = {
            let tasks = self.tasks.lock().await;
            let task = tasks.get(channel_id).ok_or_else(|| {
                AppError::NotFound(format!(
                    "channel {channel_id} is not running — start it before sending webhooks"
                ))
            })?;
            (task.adapter.clone(), task.ctx.clone())
        };
        adapter
            .on_webhook(&ctx, request)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))
    }

    pub async fn running_adapter(
        &self,
        channel_id: &str,
    ) -> Option<(std::sync::Arc<dyn ChannelAdapter>, ChannelCtx)> {
        let tasks = self.tasks.lock().await;
        let task = tasks.get(channel_id)?;
        Some((task.adapter.clone(), task.ctx.clone()))
    }

    fn spawn_broadcast_watcher(self: Arc<Self>, state: AppState) {
        let mut events = state.broadcast_service.subscribe_raw();
        let manager = self;
        tokio::spawn(async move {
            loop {
                let Some(event) = events.recv().await else { break };
                let BroadcastEventKind::EntityUpdated {
                    table,
                    record_id,
                    action,
                    ..
                } = &event.kind
                else {
                    continue;
                };
                if table != "channel" {
                    continue;
                }
                let manager = manager.clone();
                let state = state.clone();
                let record_id = record_id.clone();
                let action = *action;
                tokio::spawn(async move {
                    match action {
                        EntityAction::Created => {
                            manager.start_with_retry(state, record_id);
                        }
                        EntityAction::Updated => {
                            let new_channel =
                                match state.channel_service.find_by_id(&record_id).await {
                                    Ok(c) => c,
                                    Err(_) => {
                                        manager.stop_channel(&record_id).await;
                                        return;
                                    }
                                };
                            let prior = {
                                let tasks = manager.tasks.lock().await;
                                tasks.get(&record_id).map(|t| t.ctx.channel.clone())
                            };
                            if let Some(prior) = prior {
                                let needs_restart =
                                    channel_needs_restart(&prior, &new_channel);
                                let recovering =
                                    new_channel.status == ChannelStatus::Failed;
                                if !needs_restart && !recovering {
                                    return;
                                }
                                if needs_restart {
                                    // Clear the retry slot so the idempotent
                                    // start_with_retry below actually re-spawns.
                                    manager.stop_channel(&record_id).await;
                                }
                            }
                            manager.start_with_retry(state, record_id);
                        }
                        EntityAction::Deleted => {
                            manager.stop_channel(&record_id).await;
                        }
                    }
                });
            }
        });
    }

    pub async fn record_segment_progress(
        &self,
        message_id: &str,
    ) -> Result<(), AppError> {
        let mut message = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
        let Some(ref mut delivery) = message.delivery else {
            return Ok(());
        };
        let now = Utc::now();
        delivery.last_attempt_at = Some(now);
        delivery.tool_index = delivery.tool_index.saturating_add(1);
        delivery.last_error = None;
        delivery.failure_kind = None;
        delivery.next_attempt_at = Some(now);
        self.message_repo.update(&message).await?;
        Ok(())
    }

    /// Deliver the pending-HITL prompts on `message` to the channel adapter
    /// and persist a `HitlDelivery` per successfully-rendered call.
    ///
    /// Idempotent: tool_calls whose `hitl.delivery` is already populated are
    /// skipped, so safe to call again on partial failure, crash recovery,
    /// or a duplicate `Paused` broadcast.
    ///
    /// The returned report tells the caller how many HITLs were offered to
    /// the adapter (`attempted`) vs how many were confirmed rendered
    /// (`delivered`). A partial render means the adapter chose to stop
    /// mid-batch (e.g. sequential-cadence SMS); the undelivered ones stay
    /// `Pending` and get retried on the next call.
    pub async fn deliver_pending_hitls(
        &self,
        chat: &crate::chat::models::Chat,
        message_id: &str,
        adapter: &dyn ChannelAdapter,
        ctx: &ChannelCtx,
    ) -> Result<DeliverHitlReport, super::ChannelError> {
        let tool_calls = self
            .chat_service
            .get_tool_calls_by_message(message_id)
            .await?;
        let batch: Vec<crate::inference::tool_call::ToolCall> = tool_calls
            .into_iter()
            .filter(|tc| {
                tc.hitl.as_ref().is_some_and(|h| {
                    h.status == crate::inference::tool_call::ToolStatus::Pending
                        && h.delivery.is_none()
                })
            })
            .collect();
        if batch.is_empty() {
            return Ok(DeliverHitlReport { attempted: 0, delivered: 0 });
        }

        let msg = self
            .chat_service
            .find_message(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("message".into()))?;
        let deliveries = adapter.on_pending_hitl(&batch, &msg, chat, ctx).await?;
        let delivered = deliveries.len();

        for (tc, delivery) in batch.iter().zip(deliveries) {
            if let Err(e) = self
                .chat_service
                .set_hitl_delivery(&tc.id, delivery)
                .await
            {
                tracing::warn!(
                    tool_call_id = %tc.id,
                    error = %e,
                    "failed to persist HitlDelivery",
                );
            }
        }
        Ok(DeliverHitlReport { attempted: batch.len(), delivered })
    }

    pub async fn ensure_pending_delivery(
        &self,
        message_id: &str,
    ) -> Result<(), AppError> {
        let mut message = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
        if message.delivery.is_some() {
            return Ok(());
        }
        message.delivery = Some(crate::chat::message::models::MessageDelivery::pending(
            Utc::now(),
        ));
        self.message_repo.update(&message).await?;
        Ok(())
    }

    /// Channel-side entry point for resolving a HITL prompt. Wraps
    /// `Harness::resolve_and_resume` and spawns the agent resume (run_task
    /// or harness.resume) when the per-message barrier has cleared.
    pub async fn resolve_hitl(
        &self,
        tool_call_id: &str,
        response: crate::inference::hitl::HitlResponse,
    ) -> Result<crate::inference::hitl::ResolveOutcome, AppError> {
        let outcome = self
            .harness
            .resolve_and_resume(tool_call_id, response)
            .await?;
        if let crate::inference::hitl::ResolveOutcome::Resolved {
            should_resume: true, user_id, chat_id, message_id, task_id,
        } = &outcome
        {
            let h = self.harness.clone();
            let exec = self.task_executor.clone();
            let (u, c, m, tid) = (
                user_id.clone(),
                chat_id.clone(),
                message_id.clone(),
                task_id.clone(),
            );
            tokio::spawn(async move {
                if let Some(tid) = tid {
                    let _ = exec.run_task_by_id(&tid).await;
                } else if let Err(e) = h.resume(&u, &c, &m).await {
                    tracing::error!(error = %e, chat_id = %c, "Failed to resume chat after HITL resolve");
                }
            });
        }
        if let Ok(Some(te)) = self.chat_service.get_tool_call(tool_call_id).await {
            let _ = self.ensure_pending_delivery(&te.message_id).await;
            // Resume delivery for any remaining undelivered HITL.
            if let Ok(Some(chat)) = self.chat_service.find_chat(&te.chat_id).await
                && let Some(channel_id) = chat.channel_id.as_deref()
                && let Some((adapter, ctx)) = self.running_adapter(channel_id).await
            {
                let _ = self
                    .deliver_pending_hitls(&chat, &te.message_id, adapter.as_ref(), &ctx)
                    .await;
            }
        }
        Ok(outcome)
    }

    pub async fn record_segment_complete(
        &self,
        message_id: &str,
    ) -> Result<(), AppError> {
        let mut message = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
        let Some(ref mut delivery) = message.delivery else {
            return Ok(());
        };
        let now = Utc::now();
        delivery.state = DeliveryState::Sent;
        delivery.sent_at = Some(now);
        delivery.last_attempt_at = Some(now);
        delivery.next_attempt_at = None;
        delivery.last_error = None;
        delivery.failure_kind = None;
        self.message_repo.update(&message).await?;
        Ok(())
    }

    pub async fn record_segment_failure(
        &self,
        message_id: &str,
        err: super::ChannelError,
    ) -> Result<(), AppError> {
        let mut message = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
        let Some(ref mut delivery) = message.delivery else {
            return Ok(());
        };
        let now = Utc::now();
        delivery.last_attempt_at = Some(now);
        delivery.attempts = delivery.attempts.saturating_add(1);
        delivery.last_error = Some(err.message.clone());
        delivery.failure_kind = Some(err.kind);
        let terminal = err.kind.is_terminal() || delivery.attempts >= DELIVERY_MAX_ATTEMPTS;
        delivery.state = DeliveryState::Failed;
        delivery.next_attempt_at = if terminal {
            None
        } else {
            let backoff = err
                .retry_hint
                .unwrap_or_else(|| backoff_for(delivery.attempts));
            Some(now + chrono::Duration::from_std(backoff).unwrap())
        };
        tracing::warn!(
            msg_id = %message_id,
            attempts = delivery.attempts,
            kind = ?err.kind,
            terminal = terminal,
            retry_at = ?delivery.next_attempt_at,
            error = %err.message,
            "channel delivery segment failed",
        );
        self.message_repo.update(&message).await?;
        Ok(())
    }

    pub async fn record_carrier_status(
        &self,
        message_id: &str,
        status: CarrierStatus,
    ) -> Result<(), AppError> {
        let mut message = self
            .message_repo
            .find_by_id(message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
        let Some(ref mut delivery) = message.delivery else {
            return Ok(());
        };
        let now = Utc::now();
        match status {
            CarrierStatus::Delivered => {
                delivery.state = DeliveryState::Delivered;
                delivery.delivered_at = Some(now);
                delivery.last_error = None;
            }
            CarrierStatus::Failed { error } => {
                delivery.state = DeliveryState::Failed;
                delivery.last_error = Some(error);
                delivery.next_attempt_at = None;
            }
        }
        self.message_repo.update(&message).await?;
        Ok(())
    }

    pub async fn resume_deliveries(&self, channel: &Channel) -> Result<u64, AppError> {
        self.message_repo
            .resume_deliveries_for_channel(&channel.id, Utc::now())
            .await
    }

    /// Stamps `Pending` only; does NOT dispatch directly. Defers to the retry
    /// poller so dispatch stays single-sourced past adapter handshake.
    pub async fn reconcile_message_delivery(&self, channel: &Channel) -> Result<u64, AppError> {
        if channel.dispatch_mode != DispatchMode::Message {
            return Ok(0);
        }
        // Signal-mode rows are observability-only — never delivered.
        let orphans: Vec<_> = self
            .message_repo
            .find_undelivered_completed_for_channel(&channel.id)
            .await?
            .into_iter()
            .filter(|m| m.dispatch_mode != Some(DispatchMode::Signal))
            .collect();
        let count = orphans.len() as u64;
        if count == 0 {
            return Ok(0);
        }
        tracing::info!(
            channel_id = %channel.id,
            count = %count,
            "outbound: stamping orphan messages as Pending for retry pickup",
        );
        for msg in orphans {
            if let Err(e) = self.ensure_pending_delivery(&msg.id).await {
                tracing::warn!(
                    channel_id = %channel.id,
                    msg_id = %msg.id,
                    error = %e,
                    "outbound recovery: ensure_pending_delivery failed",
                );
            }
        }
        Ok(count)
    }

    pub async fn retry_due_deliveries(&self) -> Result<u64, AppError> {
        let due = self
            .message_repo
            .find_due_deliveries(Utc::now(), DELIVERY_RETRY_BATCH)
            .await?;
        let count = due.len() as u64;
        for msg in due {
            let chat = match self.chat_service.find_chat(&msg.chat_id).await? {
                Some(c) => c,
                None => continue,
            };
            let Some(channel_id) = chat.channel_id.as_deref() else {
                continue;
            };
            let Some((adapter, ctx)) = self.running_adapter(channel_id).await else {
                continue;
            };
            self.attempt_all_segments(msg, chat, adapter, ctx).await;
        }
        Ok(count)
    }

    pub async fn attempt_all_segments(
        &self,
        msg: Message,
        chat: crate::chat::models::Chat,
        adapter: Arc<dyn ChannelAdapter>,
        ctx: ChannelCtx,
    ) {
        let mut current = msg;
        for _ in 0..MAX_SEGMENTS_PER_DISPATCH {
            match self.attempt_send(&current, &chat, adapter.as_ref(), &ctx).await {
                Ok(SegmentOutcome::Continue) => {
                    match self.message_repo.find_by_id(&current.id).await {
                        Ok(Some(reloaded)) => current = reloaded,
                        Ok(None) | Err(_) => return,
                    }
                }
                Ok(SegmentOutcome::Done) | Ok(SegmentOutcome::Halted) => return,
                Err(e) => {
                    tracing::warn!(
                        msg_id = %current.id,
                        channel_id = %ctx.channel.id,
                        error = %e,
                        "attempt_send failed mid-loop",
                    );
                    return;
                }
            }
        }
        tracing::warn!(
            msg_id = %current.id,
            channel_id = %ctx.channel.id,
            "attempt_all_segments hit MAX_SEGMENTS_PER_DISPATCH; aborting (likely a buggy adapter or runaway tool list)",
        );
    }

    async fn attempt_send(
        &self,
        msg: &Message,
        chat: &crate::chat::models::Chat,
        adapter: &dyn ChannelAdapter,
        ctx: &ChannelCtx,
    ) -> Result<SegmentOutcome, AppError> {
        // Funnel for broadcast + retry-poller: catches Signal-fallback replies
        // that the broadcast-side gate misses after crash-recovery reconcile.
        let effective_mode = msg
            .dispatch_mode
            .unwrap_or(ctx.channel.dispatch_mode);
        if effective_mode != DispatchMode::Message {
            tracing::debug!(
                channel_id = %ctx.channel.id,
                msg_id = %msg.id,
                msg_mode = ?msg.dispatch_mode,
                channel_mode = ?ctx.channel.dispatch_mode,
                "attempt_send skip: effective mode is not Message",
            );
            return Ok(SegmentOutcome::Done);
        }
        // Mirrors `handle_outbound_event`'s status filter.
        if !matches!(msg.status, Some(MessageStatus::Completed) | None) {
            return Ok(SegmentOutcome::Done);
        }
        let Some(delivery) = msg.delivery.as_ref() else {
            return Ok(SegmentOutcome::Done);
        };
        if matches!(delivery.state, DeliveryState::Sent | DeliveryState::Delivered) {
            return Ok(SegmentOutcome::Done);
        }

        let tool_calls = self
            .chat_service
            .get_tool_calls_by_message(&msg.id)
            .await?;
        let final_index = tool_calls.len() as u32;

        // HITL prefix handling — delegated to `deliver_pending_hitls`.
        // Filter + adapter call + `HitlDelivery` persist live in one place;
        // attempt_send only owns the cursor advance + Halt-vs-Continue
        // decision based on the report.
        let cursor = delivery.tool_index as usize;
        let cursor_is_pending_hitl = tool_calls
            .get(cursor)
            .and_then(|tc| tc.hitl.as_ref())
            .is_some_and(|h| h.status == crate::inference::tool_call::ToolStatus::Pending);
        if cursor_is_pending_hitl {
            let report = match self
                .deliver_pending_hitls(chat, &msg.id, adapter, ctx)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    self.record_segment_failure(&msg.id, e).await?;
                    return Ok(SegmentOutcome::Halted);
                }
            };
            for _ in 0..report.delivered {
                self.record_segment_progress(&msg.id).await?;
            }
            if report.delivered < report.attempted {
                // Adapter rendered a partial batch — park until next trigger.
                return Ok(SegmentOutcome::Halted);
            }
            if report.attempted > 0 {
                return Ok(SegmentOutcome::Continue);
            }
        }

        // If the cursor is sitting on a HITL that's STILL pending but already
        // rendered (delivery is Some), park — we wait for resolution to
        // advance the cursor.
        if let Some(tc) = tool_calls.get(cursor)
            && let Some(h) = tc.hitl.as_ref()
            && h.status == crate::inference::tool_call::ToolStatus::Pending
            && h.delivery.is_some()
        {
            return Ok(SegmentOutcome::Halted);
        }

        // Resolved/Denied HITLs at the cursor: advance past them.
        if let Some(tc) = tool_calls.get(cursor)
            && let Some(h) = tc.hitl.as_ref()
            && matches!(
                h.status,
                crate::inference::tool_call::ToolStatus::Resolved
                    | crate::inference::tool_call::ToolStatus::Denied
            )
        {
            self.record_segment_progress(&msg.id).await?;
            return Ok(SegmentOutcome::Continue);
        }

        if delivery.tool_index < final_index {
            let tc = &tool_calls[delivery.tool_index as usize];
            let has_text = tc
                .turn_text
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_text {
                self.record_segment_progress(&msg.id).await?;
                return Ok(SegmentOutcome::Continue);
            }
            tracing::debug!(
                channel_id = %ctx.channel.id,
                msg_id = %msg.id,
                tool_index = delivery.tool_index,
                "outbound dispatch: invoking adapter.on_tool",
            );
            match adapter.on_tool(tc, msg, chat, ctx).await {
                Ok(()) => {
                    self.record_segment_progress(&msg.id).await?;
                    Ok(SegmentOutcome::Continue)
                }
                Err(e) => {
                    self.record_segment_failure(&msg.id, e).await?;
                    Ok(SegmentOutcome::Halted)
                }
            }
        } else {
            tracing::debug!(
                channel_id = %ctx.channel.id,
                msg_id = %msg.id,
                tool_index = delivery.tool_index,
                "outbound dispatch: invoking adapter.on_send",
            );
            match adapter.on_send(msg, &tool_calls, chat, ctx).await {
                Ok(()) => {
                    self.record_segment_complete(&msg.id).await?;
                    Ok(SegmentOutcome::Done)
                }
                Err(e) => {
                    self.record_segment_failure(&msg.id, e).await?;
                    Ok(SegmentOutcome::Halted)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SegmentOutcome {
    Continue,
    Done,
    Halted,
}

#[derive(Debug, Clone, Copy)]
pub struct DeliverHitlReport {
    /// HITLs handed to the adapter in the batch.
    pub attempted: usize,
    /// HITLs the adapter confirmed it rendered (≤ `attempted`).
    pub delivered: usize,
}

const MAX_SEGMENTS_PER_DISPATCH: usize = 256;

async fn run_outbound(
    adapter: Arc<dyn ChannelAdapter>,
    ctx: ChannelCtx,
    state: AppState,
    cancel: CancellationToken,
) -> Result<(), AppError> {
    let space_id = ctx.space.id.clone();
    let mut events = state.broadcast_service.subscribe_raw();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                if let Err(e) = adapter.on_disconnect(&ctx).await {
                    tracing::warn!(channel_id = %ctx.channel.id, error = %e, "channel.on_disconnect failed during cancel");
                }
                return Ok(());
            }
            event = events.recv() => {
                let Some(event) = event else {
                    tracing::info!(channel_id = %ctx.channel.id, "broadcast channel closed; exiting dispatcher");
                    return Ok(());
                };

                if let Err(e) = handle_outbound_event(adapter.clone(), &state, &space_id, &ctx, &event).await {
                    tracing::warn!(channel_id = %ctx.channel.id, error = %e, "channel event dispatch failed");
                }
            }
        }
    }
}

async fn handle_outbound_event(
    adapter: Arc<dyn ChannelAdapter>,
    state: &AppState,
    space_id: &str,
    ctx: &ChannelCtx,
    event: &BroadcastEvent,
) -> Result<(), AppError> {
    match &event.kind {
        BroadcastEventKind::EntityUpdated {
            table,
            record_id,
            action,
            space_id: ev_space_id,
            ..
        } if table == "message"
            && matches!(action, EntityAction::Created | EntityAction::Updated)
            && ev_space_id.as_deref() == Some(space_id) =>
        {
            let msg = state.chat_service.get_message(&event.user_id, record_id).await?;
            if !matches!(msg.role, MessageRole::Agent | MessageRole::TaskCompletion) {
                return Ok(());
            }
            if !matches!(msg.status, Some(MessageStatus::Completed) | None) {
                tracing::debug!(
                    channel_id = %ctx.channel.id,
                    msg_id = %msg.id,
                    status = ?msg.status,
                    "outbound skip: message status not deliverable",
                );
                return Ok(());
            }
            let delivery_state = msg.delivery.as_ref().map(|d| d.state);
            if matches!(delivery_state, Some(DeliveryState::Sent) | Some(DeliveryState::Failed)) {
                tracing::debug!(
                    channel_id = %ctx.channel.id,
                    msg_id = %msg.id,
                    delivery_state = ?delivery_state,
                    "outbound skip: delivery already terminal",
                );
                return Ok(());
            }
            // Per-message override: Message-mode channel may carry a Signal-mode reply.
            let effective_mode = msg
                .dispatch_mode
                .unwrap_or(ctx.channel.dispatch_mode);
            if effective_mode != DispatchMode::Message {
                tracing::debug!(
                    channel_id = %ctx.channel.id,
                    msg_id = %msg.id,
                    msg_mode = ?msg.dispatch_mode,
                    channel_mode = ?ctx.channel.dispatch_mode,
                    "outbound skip: effective mode is not Message",
                );
                return Ok(());
            }
            let chat = state.chat_service.get_chat(&event.user_id, &msg.chat_id).await?;
            if chat.channel_id.is_none() {
                tracing::debug!(
                    channel_id = %ctx.channel.id,
                    msg_id = %msg.id,
                    chat_id = %chat.id,
                    "outbound skip: chat is not channel-bound",
                );
                return Ok(());
            }
            ctx.channel_manager.ensure_pending_delivery(&msg.id).await?;
            let msg = state.chat_service.get_message(&event.user_id, &msg.id).await?;
            tracing::info!(
                channel_id = %ctx.channel.id,
                msg_id = %msg.id,
                chat_id = %chat.id,
                "outbound dispatch: starting segmented send loop",
            );
            state
                .channel_manager
                .attempt_all_segments(msg, chat, adapter.clone(), ctx.clone())
                .await;
            Ok(())
        }
        BroadcastEventKind::Inference(kind) => {
            if event.space_id.as_deref() != Some(space_id) {
                return Ok(());
            }
            let chat_id = match event.chat_id.as_deref() {
                Some(id) => id,
                None => return Ok(()),
            };
            let chat = match state.chat_service.get_chat(&event.user_id, chat_id).await {
                Ok(c) => c,
                Err(_) => return Ok(()),
            };
            if chat.channel_id.is_none() {
                return Ok(());
            }
            // Streaming hooks are best-effort. Log the classified failure but
            // don't bubble — one failed typing-indicator or token-edit
            // shouldn't kill the whole event loop.
            macro_rules! log_streaming_err {
                ($result:expr, $hook:literal) => {
                    if let Err(e) = $result {
                        tracing::warn!(
                            channel_id = %ctx.channel.id,
                            chat_id = %chat.id,
                            kind = ?e.kind,
                            error = %e.message,
                            "{} failed", $hook,
                        );
                    }
                };
            }
            match kind {
                InferenceEventKind::Start | InferenceEventKind::Resume { .. } => {
                    log_streaming_err!(adapter.on_inference_start(&chat, ctx).await, "on_inference_start");
                }
                InferenceEventKind::Text(text) => {
                    log_streaming_err!(adapter.on_text(&chat, text, ctx).await, "on_text");
                }
                InferenceEventKind::Reasoning(text) => {
                    log_streaming_err!(adapter.on_reasoning(&chat, text, ctx).await, "on_reasoning");
                }
                InferenceEventKind::ToolCall { name, arguments, .. } => {
                    log_streaming_err!(adapter.on_tool_call(&chat, name, arguments, ctx).await, "on_tool_call");
                }
                InferenceEventKind::ToolResult { name, success, result } => {
                    log_streaming_err!(adapter.on_tool_result(&chat, name, *success, result, ctx).await, "on_tool_result");
                }
                InferenceEventKind::Done { .. }
                | InferenceEventKind::Cancelled { .. }
                | InferenceEventKind::Failed { .. } => {
                    log_streaming_err!(adapter.on_inference_done(&chat, ctx).await, "on_inference_done");
                }
                InferenceEventKind::Paused { reason, message } => {
                    log_streaming_err!(adapter.on_inference_done(&chat, ctx).await, "on_inference_done");
                    match reason {
                        crate::inference::tool_loop::PauseReason::Hitl => {
                            if let Err(e) = state
                                .channel_manager
                                .deliver_pending_hitls(&chat, &message.id, adapter.as_ref(), ctx)
                                .await
                            {
                                tracing::warn!(
                                    channel_id = %ctx.channel.id,
                                    chat_id = %chat.id,
                                    kind = ?e.kind,
                                    error = %e.message,
                                    "deliver_pending_hitls during Paused event failed",
                                );
                            }
                        }
                    }
                }
                // No adapter hook for infra-level events.
                InferenceEventKind::EntityUpdated { .. } | InferenceEventKind::Retry { .. } => {}
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn run_inbound_pipeline(
    ctx: ChannelCtx,
    state: AppState,
    mut rx: mpsc::Receiver<ExternalMessage>,
    cancel: CancellationToken,
) -> Result<(), AppError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!(channel_id = %ctx.channel.id, "inbound pipeline cancelled");
                return Ok(());
            }
            event = rx.recv() => {
                let Some(event) = event else {
                    tracing::info!(channel_id = %ctx.channel.id, "inbound emit channel closed");
                    return Ok(());
                };
                if let Err(e) = process_inbound(&state, &ctx, event).await {
                    tracing::warn!(
                        channel_id = %ctx.channel.id,
                        error = %e,
                        "inbound external message processing failed",
                    );
                }
            }
        }
    }
}

async fn process_inbound(
    state: &AppState,
    ctx: &ChannelCtx,
    event: ExternalMessage,
) -> Result<Option<Message>, AppError> {
    // ctx.channel is a stale snapshot — pairing flips user_address.
    let live = state.channel_service.find_by_id(&ctx.channel.id).await?;
    let channel = &live;

    if channel.status == ChannelStatus::Pairing {
        let _ = state
            .channel_service
            .try_redeem_pairing(&channel.id, &event.sender_address, &event.content)
            .await?;
        return Ok(None);
    }

    if event.content.trim().is_empty() && event.attachments.is_empty() {
        tracing::debug!(
            channel_id = %channel.id,
            sender = %event.sender_address,
            "inbound dropped: empty content with no attachments",
        );
        return Ok(None);
    }

    let agent = state
        .agent_service
        .find_by_id(&channel.agent_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Agent {} not found", channel.agent_id)))?;

    let initial_title = event
        .sender_display_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(event.sender_address.as_str());
    let chat = state
        .chat_service
        .upsert_channel_chat(
            &channel.user_id,
            &channel.space_id,
            &channel.agent_id,
            &channel.id,
            &event.external_chat_id,
            Some(initial_title),
        )
        .await?;

    let user = state.user_service.find_by_id(&channel.user_id).await?;
    let address = event.sender_address.as_str();
    let is_self = channel
        .user_address
        .as_ref()
        .and_then(|ua| ua.address.as_deref())
        == Some(address);
    let real_contact: Option<Contact> = if is_self {
        None
    } else if let Some(ext_id) = event.sender_external_id.as_deref() {
        let display = event
            .sender_display_name
            .as_deref()
            .unwrap_or(address);
        Some(
            state
                .contact_service
                .upsert_by_channel_address(
                    &channel.user_id,
                    &channel.space_id,
                    &channel.provider,
                    ext_id,
                    Some(&channel.id),
                    display,
                )
                .await?,
        )
    } else {
        None
    };

    let sender_contact = match (&real_contact, is_self) {
        (Some(c), false) => PolicyContact::from_contact(c, address),
        (None, true) => synthesize_self_contact(user.as_ref(), address),
        (None, false) => PolicyContact::unresolved(&channel.user_id, address),
        (Some(_), true) => unreachable!("self-source never upserts a real Contact"),
    };

    let paired_addresses: Vec<String> = channel
        .user_address
        .as_ref()
        .and_then(|ua| ua.address.clone())
        .map(|a| vec![a])
        .unwrap_or_default();
    let effective = effective_dispatch_mode(
        state,
        channel,
        &agent,
        &sender_contact,
        &paired_addresses,
    )
    .await?;
    let Some(effective) = effective else {
        tracing::info!(
            user_id = %channel.user_id,
            agent_id = %channel.agent_id,
            provider = %channel.provider,
            space_id = %channel.space_id,
            sender = %event.sender_address,
            contact_id = ?real_contact.as_ref().map(|c| c.id.as_str()),
            is_self = %is_self,
            mode = ?channel.dispatch_mode,
            "Inbound discarded — Cedar denied",
        );
        return Ok(None);
    };
    if effective != channel.dispatch_mode {
        tracing::info!(
            channel_id = %channel.id,
            sender = %event.sender_address,
            channel_mode = ?channel.dispatch_mode,
            effective_mode = ?effective,
            "Inbound authorized as signal fallback on Message-mode channel",
        );
    }

    let builder = Message::builder(&chat.id, MessageRole::User, event.content.clone())
        .from_address(event.sender_address.clone())
        .dispatch_mode(effective);
    let mut msg = builder.build();
    if let Some(c) = &real_contact {
        msg.contact_id = Some(c.id.clone());
    }
    msg.attachments = event.attachments.clone();

    let saved = state.chat_service.persist_inbound_message(&msg).await?;
    Ok(Some(saved))
}

/// Message-mode channels fall back to `ReceiveSignal` when `ReceiveMessage`
/// denies (covers agents with an open watch). Signal-mode only checks `ReceiveSignal`.
async fn effective_dispatch_mode(
    state: &AppState,
    channel: &Channel,
    agent: &Agent,
    sender_contact: &PolicyContact,
    paired_addresses: &[String],
) -> Result<Option<DispatchMode>, AppError> {
    let receive_message = || PolicyAction::ReceiveMessage {
        connector_id: channel.space_id.clone(),
        channel_handle: channel.handle.clone(),
        sender: sender_contact.clone(),
        paired_addresses: paired_addresses.to_vec(),
    };
    let receive_signal = || PolicyAction::ReceiveSignal {
        connector_id: channel.space_id.clone(),
        channel_handle: channel.handle.clone(),
        sender: sender_contact.clone(),
        paired_addresses: paired_addresses.to_vec(),
    };

    if channel.dispatch_mode == DispatchMode::Message {
        let decision = state
            .policy_service
            .authorize(&channel.user_id, agent, receive_message())
            .await?;
        if decision.allowed {
            return Ok(Some(DispatchMode::Message));
        }
    }

    let decision = state
        .policy_service
        .authorize(&channel.user_id, agent, receive_signal())
        .await?;
    if decision.allowed {
        return Ok(Some(DispatchMode::Signal));
    }
    Ok(None)
}

fn synthesize_self_contact(
    user: Option<&crate::auth::User>,
    address: &str,
) -> PolicyContact {
    let (id, user_id, name) = match user {
        Some(u) => (u.id.clone(), u.id.clone(), u.name.clone()),
        None => (String::new(), String::new(), String::new()),
    };
    PolicyContact {
        id,
        user_id,
        name,
        address: address.to_string(),
        addresses: vec![address.to_string()],
    }
}

pub fn spawn_inference_dispatcher(state: AppState) {
    let mut events = state.broadcast_service.subscribe_raw();
    let shutdown = state.shutdown_token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = shutdown.cancelled() => {
                    tracing::info!("Channel inbound loop stopping for shutdown");
                    break;
                }
                event = events.recv() => {
                    let Some(event) = event else { break };
                    let BroadcastEventKind::EntityUpdated {
                        table, record_id, action, ..
                    } = &event.kind else { continue };
                    if table != "message" || *action != EntityAction::Created {
                        continue;
                    }
                    if let Err(e) = handle_inbound_message(&state, &event.user_id, record_id).await {
                        tracing::warn!(
                            user_id = %event.user_id,
                            message_id = %record_id,
                            error = %e,
                            "Channel inbound dispatch failed",
                        );
                    }
                }
            }
        }
    });
}

async fn handle_inbound_message(
    state: &AppState,
    user_id: &str,
    message_id: &str,
) -> Result<(), AppError> {
    let Some(msg) = state.chat_service.find_message(message_id).await? else {
        return Ok(());
    };
    if !matches!(msg.role, MessageRole::User) {
        return Ok(());
    }
    // `from_address` distinguishes channel-inbound from web-submitted; the
    // web route already triggers its own inference, fanning out here would
    // run two loops on the same turn.
    if msg.from_address.is_none() {
        return Ok(());
    }

    let Some(chat) = state.chat_service.find_chat(&msg.chat_id).await? else {
        return Ok(());
    };

    let space = if let Some(space_id) = chat.space_id.as_deref() {
        state.space_service.find_by_id(space_id).await?
    } else {
        None
    };

    let channel_row = match space.as_ref() {
        Some(s) => state.channel_service.find_by_space(&s.id).await?,
        None => None,
    };
    let Some(channel_row) = channel_row else {
        tracing::debug!(
            chat_id = %msg.chat_id,
            space_id = ?chat.space_id,
            "no channel bound to chat — inbound message persisted but inference will not fire",
        );
        return Ok(());
    };
    let channel = channel_row.provider.clone();
    // Legacy rows pre-dating `msg.dispatch_mode` fall back to the channel's mode.
    let mode = msg.dispatch_mode.unwrap_or(channel_row.dispatch_mode);

    let chat_type = ChatType::from_chat(&chat);
    let sender = msg.from_address.as_deref();

    let awaiting_categories = match state.signal_service() {
        Some(svc) => svc.pending_category_hints(user_id).await,
        None => Vec::new(),
    };

    if matches!(mode, DispatchMode::Signal) {
        // dispatch_mode=Signal causes `attempt_send` to refuse delivery.
        let Some(signal_service) = state.signal_service() else {
            tracing::warn!(
                channel_id = %channel_row.id,
                "Signal-mode dispatch but signal_service unavailable; skipping",
            );
            return Ok(());
        };
        signal_service
            .process_inbound_extract(
                &state.chat_service,
                state.chat_service.provider_registry(),
                &channel_row,
                &chat,
                &msg,
                &awaiting_categories,
            )
            .await?;
        return Ok(());
    }

    let inbound_prompt = compose_inbound_prompt(
        state,
        mode,
        &channel,
        &chat.id,
        chat_type,
        sender,
        &awaiting_categories,
    );

    let agent_msg = state
        .chat_service
        .create_executing_agent_message(&chat.id, &chat.agent_id)
        .await?;

    let cancel_token = CancellationToken::new();

    let builder = Box::new(ChannelConversationBuilder {
        user_service: state.user_service.clone(),
        storage_service: state.storage_service.clone(),
        agent_service: state.agent_service.clone(),
        channel,
        sender: sender.map(String::from),
        inbound_prompt,
    });

    state.harness.run_turn(
        user_id,
        &chat.id,
        &agent_msg.id,
        cancel_token,
        builder,
        &[],
        None,
    )
    .await;
    Ok(())
}

fn compose_inbound_prompt(
    state: &AppState,
    mode: DispatchMode,
    channel: &str,
    chat_id: &str,
    chat_type: ChatType,
    sender: Option<&str>,
    awaiting_categories: &[(String, String)],
) -> Option<String> {
    if matches!(mode, DispatchMode::Message) && awaiting_categories.is_empty() {
        return None;
    }
    let sender_block = sender
        .map(|s| format!(" from {s}"))
        .unwrap_or_default();
    let categories_block = if awaiting_categories.is_empty() {
        String::new()
    } else {
        let awaiting_list = awaiting_categories
            .iter()
            .map(|(cat, info)| format!("- {cat}: {info}"))
            .collect::<Vec<_>>()
            .join("\n");
        state
            .prompts
            .read_with_vars(
                "channel/categories.md",
                &[("awaiting_categories", &awaiting_list)],
            )
            .unwrap_or_default()
    };
    let vars: &[(&str, &str)] = &[
        ("channel", channel),
        ("sender_block", &sender_block),
        ("chat_id", chat_id),
        ("chat_type", chat_type.as_str()),
        ("categories_block", &categories_block),
    ];
    let path = match mode {
        DispatchMode::Message => "channel/message.md",
        DispatchMode::Signal => "channel/signal.md",
    };
    Some(state.prompts.read_with_vars(path, vars).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_cap_at_last() {
        assert_eq!(backoff_for(0), Duration::from_secs(0));
        assert_eq!(backoff_for(1), Duration::from_secs(5));
        assert_eq!(backoff_for(2), Duration::from_secs(25));
        assert_eq!(backoff_for(3), Duration::from_secs(120));
        assert_eq!(backoff_for(4), Duration::from_secs(600));
        assert_eq!(backoff_for(5), Duration::from_secs(600));
        assert_eq!(backoff_for(99), Duration::from_secs(600));
    }

    #[test]
    fn channel_error_kind_terminality() {
        use super::super::ChannelErrorKind;
        assert!(!ChannelErrorKind::Transient.is_terminal());
        assert!(ChannelErrorKind::Forbidden.is_terminal());
        assert!(ChannelErrorKind::NotFound.is_terminal());
        assert!(ChannelErrorKind::PayloadInvalid.is_terminal());
        assert!(ChannelErrorKind::PayloadTooLarge.is_terminal());
        assert!(ChannelErrorKind::Unauthorized.is_terminal());
        assert!(ChannelErrorKind::Other.is_terminal());
    }
}
