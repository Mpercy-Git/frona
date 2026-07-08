//! In-memory capture of the server's own tracing output for the live log
//! viewer in Settings.
//!
//! A [`LogStreamLayer`] is installed alongside the `fmt` layer in `main.rs`.
//! Every event it sees is formatted into a [`LogLine`], pushed onto a bounded
//! ring buffer (so a newly-connected viewer can backfill recent history), and
//! broadcast to any live subscribers. Both the buffer and the channel live in
//! a process-global `OnceLock` so the layer (built before `AppState` exists)
//! and the SSE route (which only has `AppState`) share the same instance
//! without threading a handle through the whole constructor.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock};

use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// Most-recent lines kept for backfilling a freshly-opened viewer.
const RING_CAPACITY: usize = 1000;
/// Broadcast backlog before a slow subscriber starts dropping lines.
const CHANNEL_CAPACITY: usize = 1024;

/// One formatted log record, as delivered to the live viewer.
#[derive(Clone, serde::Serialize)]
pub struct LogLine {
    /// RFC 3339 timestamp (millisecond precision, UTC).
    pub timestamp: String,
    /// `ERROR` / `WARN` / `INFO` / `DEBUG` / `TRACE`.
    pub level: String,
    /// Event target, typically the emitting module path.
    pub target: String,
    /// The event message plus any structured fields.
    pub message: String,
}

struct Inner {
    tx: broadcast::Sender<LogLine>,
    ring: Mutex<VecDeque<LogLine>>,
}

static STREAM: OnceLock<Inner> = OnceLock::new();

fn inner() -> &'static Inner {
    STREAM.get_or_init(|| {
        let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        Inner {
            tx,
            ring: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
        }
    })
}

/// Subscribe to log lines emitted from now on.
pub fn subscribe() -> broadcast::Receiver<LogLine> {
    inner().tx.subscribe()
}

/// Snapshot of the most recent buffered lines, oldest first.
pub fn recent() -> Vec<LogLine> {
    inner().ring.lock().unwrap().iter().cloned().collect()
}

fn publish(line: LogLine) {
    let inner = inner();
    {
        let mut ring = inner.ring.lock().unwrap();
        if ring.len() >= RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(line.clone());
    }
    // Ignore the error when there are no live subscribers.
    let _ = inner.tx.send(line);
}

/// `tracing` layer that mirrors every event into the shared log stream.
pub struct LogStreamLayer;

impl<S> Layer<S> for LogStreamLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let meta = event.metadata();
        publish(LogLine {
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            level: meta.level().to_string(),
            target: meta.target().to_string(),
            message: visitor.finish(),
        });
    }
}

/// Collects the `message` field and appends any remaining structured fields as
/// `key=value` pairs, mirroring the console formatter closely enough to be
/// useful in the viewer.
#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl MessageVisitor {
    fn finish(self) -> String {
        if self.fields.is_empty() {
            self.message
        } else if self.message.is_empty() {
            self.fields.trim_start().to_string()
        } else {
            format!("{}{}", self.message, self.fields)
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            let _ = write!(self.fields, " {}={:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            let _ = write!(self.fields, " {}={:?}", field.name(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Level;

    #[test]
    fn message_visitor_joins_message_and_fields() {
        let mut v = MessageVisitor::default();
        v.message = "hello".into();
        v.fields = " count=3".into();
        assert_eq!(v.finish(), "hello count=3");
    }

    #[test]
    fn ring_backfill_and_broadcast() {
        let mut rx = subscribe();
        publish(LogLine {
            timestamp: "t".into(),
            level: Level::INFO.to_string(),
            target: "test".into(),
            message: "line-one".into(),
        });
        // Broadcast delivers to the live subscriber.
        let got = rx.try_recv().expect("should receive published line");
        assert_eq!(got.message, "line-one");
        // And the ring keeps it for backfill.
        assert!(recent().iter().any(|l| l.message == "line-one"));
    }
}
