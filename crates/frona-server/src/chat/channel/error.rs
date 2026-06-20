use std::time::Duration;

use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::core::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", rename_all = "snake_case")]
pub enum ChannelErrorKind {
    Transient,
    Forbidden,
    NotFound,
    PayloadInvalid,
    PayloadTooLarge,
    Unauthorized,
    Other,
}

impl ChannelErrorKind {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Transient)
    }
}

#[derive(Debug)]
pub struct ChannelError {
    pub message: String,
    pub kind: ChannelErrorKind,
    pub retry_hint: Option<Duration>,
}

impl ChannelError {
    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ChannelErrorKind::Transient,
            retry_hint: None,
        }
    }

    pub fn terminal(message: impl Into<String>, kind: ChannelErrorKind) -> Self {
        debug_assert!(kind.is_terminal(), "ChannelError::terminal called with Transient");
        Self {
            message: message.into(),
            kind,
            retry_hint: None,
        }
    }

    pub fn with_retry_hint(mut self, after: Duration) -> Self {
        self.retry_hint = Some(after);
        self
    }
}

impl std::fmt::Display for ChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ChannelError {}

impl From<AppError> for ChannelError {
    fn from(e: AppError) -> Self {
        Self::transient(e.to_string())
    }
}
