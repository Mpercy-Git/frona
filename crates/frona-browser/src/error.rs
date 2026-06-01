use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("connection closed")]
    Disconnected,

    #[error("no active page")]
    NoActivePage,

    #[error("snapshot index {0} not found; call snapshot() first")]
    UnknownSnapshotIndex(usize),

    #[error("invalid target: must supply selector or snapshot index, not both")]
    InvalidTarget,

    #[error("operation timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("tool {tool}: {message}")]
    ToolFailed { tool: &'static str, message: String },

    #[error(transparent)]
    Cdp(#[from] chromiumoxide::error::CdpError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    pub fn is_disconnect(&self) -> bool {
        if matches!(self, Error::Disconnected | Error::NoActivePage) {
            return true;
        }
        if let Error::Cdp(e) = self {
            let s = e.to_string();
            return s.contains("connection closed")
                || s.contains("channel closed")
                || s.contains("WebSocket")
                || s.contains("ConnectionClosed");
        }
        false
    }
}
