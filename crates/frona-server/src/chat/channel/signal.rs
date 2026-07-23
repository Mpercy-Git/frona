//! Adapter → supervisor lifecycle signals.
//!
//! One uniform, per-attempt, buffered stream carrying every runtime transition
//! an adapter can report — connect, disconnect, and device-linking. The supervisor
//! owns the receiver and is the sole consumer; adapters hold a [`ChannelSignalSink`]
//! via `ctx.signals`.

use tokio::sync::mpsc;

use super::error::FailureKind;
use super::models::SetupConfig;

/// Buffer depth for the per-attempt signal channel. Headroom so an adapter that
/// emits from *within* `on_connect` (before the supervisor starts draining)
/// never blocks; on overflow the signal is dropped with a warning.
pub const SIGNAL_BUF: usize = 8;

/// A lifecycle transition reported by an adapter to its supervisor.
#[derive(Debug)]
pub enum ChannelSignal {
    /// Link mode: a device-link QR/code is available → sets the `setup` overlay.
    SetupReady { config: SetupConfig },
    /// Device linking completed → clears the `setup` overlay.
    Linked,
    /// Transport is live and drained → status becomes `Connected`, backoff resets.
    Connected,
    /// Transport dropped or failed to establish → `Reconnecting` (transient) or
    /// `Failed` (terminal), per `kind`.
    Disconnected { kind: FailureKind, reason: String },
}

/// Non-blocking sender adapters use to report lifecycle transitions. Uses
/// `try_send` (never `.await`) so an emit from inside `on_connect` cannot
/// deadlock against a supervisor that has not begun draining yet.
#[derive(Clone)]
pub struct ChannelSignalSink {
    tx: mpsc::Sender<ChannelSignal>,
    channel_id: String,
}

impl ChannelSignalSink {
    pub fn new(tx: mpsc::Sender<ChannelSignal>, channel_id: String) -> Self {
        Self { tx, channel_id }
    }

    fn emit(&self, sig: ChannelSignal) {
        if let Err(e) = self.tx.try_send(sig) {
            tracing::warn!(
                channel_id = %self.channel_id,
                error = %e,
                "channel signal dropped (supervisor gone or buffer full)",
            );
        }
    }

    pub fn setup_ready(&self, config: SetupConfig) {
        self.emit(ChannelSignal::SetupReady { config });
    }

    pub fn linked(&self) {
        self.emit(ChannelSignal::Linked);
    }

    pub fn connected(&self) {
        self.emit(ChannelSignal::Connected);
    }

    pub fn disconnected_transient(&self, reason: impl Into<String>) {
        self.emit(ChannelSignal::Disconnected {
            kind: FailureKind::Transient,
            reason: reason.into(),
        });
    }

    pub fn disconnected_terminal(&self, reason: impl Into<String>) {
        self.emit(ChannelSignal::Disconnected {
            kind: FailureKind::Terminal,
            reason: reason.into(),
        });
    }
}
