use serde::Serialize;
use std::{error::Error, fmt};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ErrorDetails {
    pub path: String,
    pub rule: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ErrorDetails>,
    #[serde(skip)]
    pub(crate) recoverable: bool,
}

impl CommandError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
            recoverable: false,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new("INVALID_PARAMS", message)
    }

    pub fn model_config_invalid(path: impl Into<String>, rule: impl Into<String>) -> Self {
        Self::new(
            "MODEL_CONFIG_INVALID",
            "Agent model configuration is invalid",
        )
        .with_validation_details(path, rule)
    }

    pub(crate) fn with_validation_details(
        mut self,
        path: impl Into<String>,
        rule: impl Into<String>,
    ) -> Self {
        let path = path.into();
        let rule = rule.into();
        if path.starts_with('/')
            && path.len() <= 1024
            && !path.chars().any(char::is_control)
            && !rule.is_empty()
            && rule.len() <= 64
            && rule
                .bytes()
                .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'_')
        {
            self.details = Some(ErrorDetails { path, rule });
        }
        self
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
