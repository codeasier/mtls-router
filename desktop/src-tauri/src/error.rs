use serde::Serialize;
use std::{error::Error, fmt};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    #[serde(skip)]
    pub(crate) recoverable: bool,
}

impl CommandError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            recoverable: false,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new("INVALID_PARAMS", message)
    }

    pub fn manager_failed() -> Self {
        Self::recoverable(
            "MANAGER_FAILED",
            "the manager is unavailable; restart the desktop application",
        )
    }

    pub(crate) fn recoverable(code: impl Into<String>, message: impl Into<String>) -> Self {
        let mut error = Self::new(code, message);
        error.recoverable = true;
        error
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for CommandError {}

pub type Result<T> = std::result::Result<T, CommandError>;
