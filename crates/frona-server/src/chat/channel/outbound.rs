//! Outbound message delivery engine.
//!
//! `OutboundDeliveryService` is a single, stateless engine (over `message_repo` +
//! `chat_service`) that delivers an agent message's segments to a channel adapter it is
//! *handed*. It owns the segment cursor walk, HITL-prefix delivery, delivery-state
//! bookkeeping, and carrier-status callbacks. It never looks up the live adapter itself
//! — the caller (outbound pipeline / retry poller / supervisor) resolves that.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;

use crate::chat::message::models::{DeliveryState, Message, MessageStatus};
use crate::chat::message::repository::MessageRepository;
use crate::chat::service::ChatService;
use crate::core::error::AppError;

use super::models::{Channel, ChannelAdapter, ChannelCtx, DispatchMode};

const DELIVERY_MAX_ATTEMPTS: u32 = 5;

const DELIVERY_BACKOFF: &[Duration] = &[
    Duration::from_secs(5),
    Duration::from_secs(25),
    Duration::from_secs(120),
    Duration::from_secs(600),
];

const MAX_SEGMENTS_PER_DISPATCH: usize = 256;

pub(super) fn backoff_for(attempts: u32) -> Duration {
    if attempts == 0 {
        return Duration::from_secs(0);
    }
    let idx = (attempts as usize - 1).min(DELIVERY_BACKOFF.len() - 1);
    DELIVERY_BACKOFF[idx]
}

pub enum CarrierStatus {
    Delivered,
    Failed { error: String },
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

pub struct OutboundDeliveryService {
    message_repo: Arc<dyn MessageRepository>,
    chat_service: ChatService,
}

impl OutboundDeliveryService {
    pub fn new(message_repo: Arc<dyn MessageRepository>, chat_service: ChatService) -> Self {
        Self { message_repo, chat_service }
    }

    pub async fn record_segment_progress(&self, message_id: &str) -> Result<(), AppError> {
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

    pub async fn ensure_pending_delivery(&self, message_id: &str) -> Result<(), AppError> {
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

    async fn record_segment_complete(&self, message_id: &str) -> Result<(), AppError> {
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

    async fn record_segment_failure(
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
}
