use presage::libsignal_service::content::ContentBody;
use presage::libsignal_service::protocol::ServiceId;
use presage::libsignal_service::proto::{
    body_range, BodyRange, DataMessage, GroupContextV2, ReceiptMessage, TypingMessage,
    receipt_message, typing_message,
};
use presage::manager::Registered;
use presage::store::Store;
use presage::Manager;
use tokio::sync::oneshot;

use crate::chat::channel::adapter::markdown::{self, SignalStyle, SignalText};
use crate::core::error::AppError;

use super::external_id::SignalTarget;

#[derive(Debug, Clone, Copy)]
pub enum TypingAction {
    Started,
    Stopped,
}

impl From<TypingAction> for typing_message::Action {
    fn from(a: TypingAction) -> Self {
        match a {
            TypingAction::Started => typing_message::Action::Started,
            TypingAction::Stopped => typing_message::Action::Stopped,
        }
    }
}

pub enum SignalCommand {
    SendText {
        target: SignalTarget,
        body: String,
        msg_id: String,
        /// On success the reply carries the Signal message timestamp (ms),
        /// which is Signal's protocol-level message identifier — used as
        /// `HitlDelivery.external_message_id` for HITL prompts so quote-replies
        /// can be matched back to the originating tool call.
        reply: oneshot::Sender<Result<u64, AppError>>,
    },
    SendTyping {
        target: SignalTarget,
        action: TypingAction,
    },
    SendReadReceipt {
        sender: ServiceId,
        timestamps: Vec<u64>,
    },
}

pub async fn handle<S: Store>(
    mgr: &mut Manager<S, Registered>,
    cmd: SignalCommand,
    channel_id: &str,
) {
    let now = now_ms();
    match cmd {
        SignalCommand::SendText { target, body, msg_id, reply } => {
            let signal_chat = target_label(&target);
            let r = send_text(mgr, target, body, now).await.map(|()| now);
            match &r {
                Ok(ts) => tracing::info!(
                    channel_id = %channel_id,
                    msg_id = %msg_id,
                    to = %signal_chat,
                    signal_ts = ts,
                    "Signal message sent",
                ),
                Err(e) => tracing::warn!(
                    channel_id = %channel_id,
                    msg_id = %msg_id,
                    to = %signal_chat,
                    error = %e,
                    "Signal send_message failed",
                ),
            }
            let _ = reply.send(r);
        }
        SignalCommand::SendTyping { target, action } => {
            if let Err(e) = send_typing(mgr, target, action, now).await {
                tracing::debug!(
                    channel_id = %channel_id,
                    error = %e,
                    "Signal send_typing failed (best-effort)",
                );
            }
        }
        SignalCommand::SendReadReceipt { sender, timestamps } => {
            let body = ReceiptMessage {
                r#type: Some(receipt_message::Type::Read as i32),
                timestamp: timestamps,
            };
            if let Err(e) = mgr
                .send_message(sender, ContentBody::ReceiptMessage(body), now)
                .await
            {
                tracing::debug!(
                    channel_id = %channel_id,
                    error = %e,
                    "Signal read receipt failed (best-effort)",
                );
            }
        }
    }
}

fn target_label(target: &SignalTarget) -> String {
    match target {
        SignalTarget::Dm { aci } => super::external_id::dm(*aci),
        SignalTarget::Group { master_key } => super::external_id::group(master_key),
    }
}

async fn send_text<S: Store>(
    mgr: &mut Manager<S, Registered>,
    target: SignalTarget,
    body: String,
    ts: u64,
) -> Result<(), AppError> {
    let SignalText { body, ranges } = markdown::to_signal(&body);
    let body_ranges = ranges.into_iter().map(to_proto_body_range).collect();
    match target {
        SignalTarget::Dm { aci } => {
            let dm = DataMessage {
                body: Some(body),
                body_ranges,
                timestamp: Some(ts),
                ..Default::default()
            };
            mgr.send_message(ServiceId::Aci(aci.into()), ContentBody::DataMessage(dm), ts)
                .await
                .map_err(into_app_error)
        }
        SignalTarget::Group { master_key } => {
            let dm = DataMessage {
                body: Some(body),
                body_ranges,
                timestamp: Some(ts),
                group_v2: Some(GroupContextV2 {
                    master_key: Some(master_key.to_vec()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            mgr.send_message_to_group(&master_key, ContentBody::DataMessage(dm), ts)
                .await
                .map_err(into_app_error)
        }
    }
}

fn to_proto_body_range(r: markdown::SignalBodyRange) -> BodyRange {
    let style = match r.style {
        SignalStyle::Bold => body_range::Style::Bold,
        SignalStyle::Italic => body_range::Style::Italic,
        SignalStyle::Strikethrough => body_range::Style::Strikethrough,
        SignalStyle::Monospace => body_range::Style::Monospace,
    };
    BodyRange {
        start: Some(r.start),
        length: Some(r.length),
        associated_value: Some(body_range::AssociatedValue::Style(style as i32)),
    }
}

async fn send_typing<S: Store>(
    mgr: &mut Manager<S, Registered>,
    target: SignalTarget,
    action: TypingAction,
    ts: u64,
) -> Result<(), AppError> {
    let proto_action: typing_message::Action = action.into();
    match target {
        SignalTarget::Dm { aci } => {
            let typing = TypingMessage {
                timestamp: Some(ts),
                action: Some(proto_action as i32),
                group_id: None,
            };
            mgr.send_message(
                ServiceId::Aci(aci.into()),
                ContentBody::TypingMessage(typing),
                ts,
            )
            .await
            .map_err(into_app_error)
        }
        SignalTarget::Group { master_key } => {
            let typing = TypingMessage {
                timestamp: Some(ts),
                action: Some(proto_action as i32),
                group_id: Some(master_key.to_vec()),
            };
            mgr.send_message_to_group(
                &master_key,
                ContentBody::TypingMessage(typing),
                ts,
            )
            .await
            .map_err(into_app_error)
        }
    }
}

fn into_app_error<E: std::fmt::Display>(e: E) -> AppError {
    AppError::Internal(format!("Signal send: {e}"))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
