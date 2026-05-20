use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::http::Request;
use axum::response::Response;
use chrono::Utc;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::agent::execution;
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

/// Keep filesystem path segments to a safe alphabet so usernames / record-id
/// suffixes can't drill into parent dirs or land on `/`.
fn sanitize_path_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for c in segment.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() { "_".into() } else { out }
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

fn is_permanent_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "bot was blocked",
        "chat not found",
        "user not found",
        "forbidden",
        "recipient is not a valid",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
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
}

impl ChannelManager {
    pub fn new(
        message_repo: Arc<dyn MessageRepository>,
        chat_service: ChatService,
        channel_service: Arc<ChannelService>,
    ) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            retry_cancels: Arc::new(Mutex::new(HashMap::new())),
            message_repo,
            chat_service,
            channel_service,
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

    /// Adapter background tasks call this when provider setup finishes.
    /// The adapter must keep running across the transition - the manager
    /// does not restart the channel.
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

    pub fn start_with_retry(self: Arc<Self>, state: AppState, channel_id: String) {
        let cancel = state.shutdown_token.child_token();
        let manager = self.clone();
        let id_for_task = channel_id.clone();
        let cancel_for_task = cancel.clone();
        tokio::spawn(async move {
            let prev = {
                let mut map = manager.retry_cancels.lock().await;
                map.insert(id_for_task.clone(), cancel_for_task.clone())
            };
            if let Some(p) = prev {
                p.cancel();
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
            // Connected is intentionally included: after a process restart the DB
            // still says Connected but no in-memory task exists, so we re-start.
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

        // Don't mark Setup from here: the watcher would catch the broadcast
        // and call start_channel again, looping forever. `Setup` is owned by
        // service.create/update; we just refuse to start.
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

        let webhook_base = state
            .config
            .server
            .external_base_url()
            .unwrap_or_else(|| format!("http://localhost:{}", state.config.server.port));
        let bare_id = channel
            .id
            .strip_prefix("channel:")
            .unwrap_or(&channel.id);
        let webhook_url = format!(
            "{}{}/{}/{}",
            webhook_base.trim_end_matches('/'),
            super::WEBHOOK_PATH_PREFIX,
            channel.provider,
            bare_id,
        );

        let username = state
            .user_service
            .find_by_id(&channel.user_id)
            .await?
            .map(|u| u.username)
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "channel {:?} references missing user {:?}",
                    channel.id, channel.user_id,
                ))
            })?;
        let data_dir = std::path::PathBuf::from(&state.config.storage.channels_data_path)
            .join(&channel.provider)
            .join(sanitize_path_segment(&username))
            .join(sanitize_path_segment(&channel.space_id));
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
            data_dir,
            cancel: cancel.clone(),
        };

        // Register before setup/connect: those hooks may call back into the
        // manager (e.g. `report_setup_complete`) and need the entry present.
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

        // Pipelines spawn before setup/connect so adapters can emit inbound
        // events from inside those hooks - `ctx.emit.send(...)` would
        // otherwise fail with "channel closed".
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

        // `Err` from `on_setup_begin` means "no setup required, proceed".
        // The adapter is responsible for spawning the background watcher
        // that calls `report_setup_complete` when setup finishes.
        if let Ok(setup) = adapter.on_setup_begin(&ctx).await {
            state
                .channel_service
                .begin_setup(&channel.id, setup)
                .await?;
            return Ok(());
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
            // Propagate so retry_loop's Err branch applies backoff -
            // otherwise the Failed broadcast triggers a fresh retry with
            // no sleep.
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

    /// Stops the adapter task but leaves any active retry loop alive -
    /// otherwise a retry loop calling `start_channel` cancels itself.
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
        adapter.on_webhook(&ctx, request).await
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
            while let Ok(event) = events.recv().await {
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
                            if let Some(prior) = prior
                                && !channel_needs_restart(&prior, &new_channel)
                            {
                                if new_channel.status == ChannelStatus::Failed {
                                    let already = manager
                                        .retry_cancels
                                        .lock()
                                        .await
                                        .contains_key(&record_id);
                                    if !already {
                                        manager.start_with_retry(state, record_id);
                                    }
                                }
                                return;
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
        delivery.next_attempt_at = Some(now);
        self.message_repo.update(&message).await?;
        Ok(())
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
        self.message_repo.update(&message).await?;
        Ok(())
    }

    pub async fn record_segment_failure(
        &self,
        message_id: &str,
        err: String,
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
        delivery.last_error = Some(err.clone());
        let terminal = is_permanent_error(&err) || delivery.attempts >= DELIVERY_MAX_ATTEMPTS;
        delivery.state = DeliveryState::Failed;
        delivery.next_attempt_at = if terminal {
            None
        } else {
            Some(now + chrono::Duration::from_std(backoff_for(delivery.attempts)).unwrap())
        };
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

    /// Stamps `Pending` on orphan Completed messages - but does NOT
    /// dispatch directly. At channel start the adapter transport may not
    /// be handshake-complete; letting the retry poller pick them up avoids
    /// spurious "client not connected" failures and keeps dispatch
    /// single-sourced.
    pub async fn reconcile_message_delivery(&self, channel: &Channel) -> Result<u64, AppError> {
        if channel.dispatch_mode != DispatchMode::Message {
            return Ok(0);
        }
        // Signal-mode rows are observability-only — never delivered, so they
        // shouldn't enter the retry queue.
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
        // Funnel point for both broadcast and retry-poller flows; gating here
        // catches Signal-fallback replies that the broadcast-side gate misses
        // after a crash-recovery `reconcile_message_delivery`.
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
        // Mirrors the outer watcher's status filter. See `handle_outbound_event`.
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
                    self.record_segment_failure(&msg.id, e.to_string()).await?;
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
                    self.record_segment_failure(&msg.id, e.to_string()).await?;
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
                let event = match event {
                    Ok(e) => e,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(channel_id = %ctx.channel.id, dropped = n, "channel dispatcher lagged");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!(channel_id = %ctx.channel.id, "broadcast channel closed; exiting dispatcher");
                        return Ok(());
                    }
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
            // Per-message Signal-fallback: a Message-mode channel can carry an
            // individual Signal-mode reply (e.g. unpaired sender). The
            // outbound dispatcher must respect that override.
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
            match kind {
                InferenceEventKind::Text(_)
                | InferenceEventKind::Reasoning(_)
                | InferenceEventKind::ToolCall { .. }
                | InferenceEventKind::ToolResult { .. } => {
                    adapter.on_inference_active(&chat, ctx).await?;
                }
                InferenceEventKind::Done(_)
                | InferenceEventKind::Cancelled(_)
                | InferenceEventKind::Error(_) => {
                    adapter.on_inference_done(&chat, ctx).await?;
                }
                _ => {}
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

/// On a Message-mode channel, fall back to `ReceiveSignal` if `ReceiveMessage`
/// is denied - covers the case where an agent has an open watch even though
/// the sender wouldn't normally satisfy the message policy. Signal-mode
/// channels only consult `ReceiveSignal`. Returns the action that authorized,
/// or `None` if all denied.
async fn effective_dispatch_mode(
    state: &AppState,
    channel: &Channel,
    agent: &Agent,
    sender_contact: &PolicyContact,
    paired_addresses: &[String],
) -> Result<Option<DispatchMode>, AppError> {
    let receive_message = || PolicyAction::ReceiveMessage {
        connector_id: channel.space_id.clone(),
        channel_id: channel.provider.clone(),
        sender: sender_contact.clone(),
        paired_addresses: paired_addresses.to_vec(),
    };
    let receive_signal = || PolicyAction::ReceiveSignal {
        connector_id: channel.space_id.clone(),
        channel_id: channel.provider.clone(),
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
                    let Ok(event) = event else { break };
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
    // Fall back to the channel's mode for legacy rows persisted before
    // `msg.dispatch_mode` existed.
    let mode = msg.dispatch_mode.unwrap_or(channel_row.dispatch_mode);

    let chat_type = ChatType::from_chat(&chat);
    let sender = msg.from_address.as_deref();

    let awaiting_categories = match state.signal_service() {
        Some(svc) => svc.pending_category_hints(user_id).await,
        None => Vec::new(),
    };

    if matches!(mode, DispatchMode::Signal) {
        // The chat row gets stamped with `dispatch_mode = Signal`, which the
        // outbound dispatcher refuses to deliver. See `attempt_send`.
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
        channel,
        sender: sender.map(String::from),
        inbound_prompt,
    });

    execution::run_agent_turn(
        state,
        user_id,
        &chat.id,
        &agent_msg.id,
        cancel_token,
        builder,
        None,
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
    fn permanent_error_detection() {
        assert!(is_permanent_error("Forbidden: bot was blocked by the user"));
        assert!(is_permanent_error("chat not found"));
        assert!(is_permanent_error("FORBIDDEN: bot was kicked"));
        assert!(is_permanent_error("user not found"));
        assert!(is_permanent_error("recipient is not a valid telegram user"));
        assert!(!is_permanent_error("connection timeout"));
        assert!(!is_permanent_error("Telegram returned 503"));
        assert!(!is_permanent_error("rate limit exceeded"));
    }
}
