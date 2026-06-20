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
        if let Error::Cdp(cdp) = self {
            return is_cdp_disconnect(cdp);
        }
        false
    }
}

/// Shared by the handler loop and the public `is_disconnect` so they
/// classify identically.
pub(crate) fn is_cdp_disconnect(e: &chromiumoxide::error::CdpError) -> bool {
    use chromiumoxide::error::CdpError;
    matches!(
        e,
        CdpError::Ws(_) | CdpError::ChannelSendError(_) | CdpError::Timeout | CdpError::NoResponse
    )
}
