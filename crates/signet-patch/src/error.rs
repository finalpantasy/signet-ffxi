//! One error type for the whole crate, split the way a caller needs to react:
//! the user stopped it, a safety rule refused it, or something failed.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    /// The cancel token fired. Nothing in the install was changed.
    Cancelled,
    /// A safety rule said no (game running, not a retail install, bad path).
    Refused(String),
    /// I/O, protocol or verification failure.
    Failed(String),
}

impl PatchError {
    pub fn failed(msg: impl Into<String>) -> Self {
        PatchError::Failed(msg.into())
    }
    pub fn refused(msg: impl Into<String>) -> Self {
        PatchError::Refused(msg.into())
    }
    /// Prefix a failure with what was being worked on.
    pub fn context(self, what: impl fmt::Display) -> Self {
        match self {
            PatchError::Failed(m) => PatchError::Failed(format!("{what}: {m}")),
            PatchError::Refused(m) => PatchError::Refused(format!("{what}: {m}")),
            PatchError::Cancelled => PatchError::Cancelled,
        }
    }
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchError::Cancelled => write!(f, "cancelled"),
            PatchError::Refused(m) => write!(f, "refused: {m}"),
            PatchError::Failed(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for PatchError {}

impl From<std::io::Error> for PatchError {
    fn from(e: std::io::Error) -> Self {
        PatchError::Failed(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, PatchError>;

/// `io::Result` with the path that failed in the message.
pub(crate) fn io_at<T>(r: std::io::Result<T>, path: &std::path::Path) -> Result<T> {
    r.map_err(|e| PatchError::failed(format!("{}: {e}", path.display())))
}
