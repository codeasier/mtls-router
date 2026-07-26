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

    pub fn credential_not_found() -> Self {
        Self::new("CREDENTIAL_NOT_FOUND", "credential is not configured")
    }

    pub fn credential_invalid() -> Self {
        Self::new(
            "CREDENTIAL_INVALID",
            "credential file is malformed; save the credential again",
        )
    }

    pub fn credential_io() -> Self {
        Self::new("CREDENTIAL_IO_ERROR", "credential file operation failed")
    }

    pub fn credential_lock_timeout() -> Self {
        Self::new(
            "CREDENTIAL_LOCK_TIMEOUT",
            "another credential operation is in progress",
        )
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

impl From<crate::credential::CredentialError> for CommandError {
    fn from(error: crate::credential::CredentialError) -> Self {
        match error {
            crate::credential::CredentialError::NotFound => Self::credential_not_found(),
            crate::credential::CredentialError::InvalidFormat(_) => Self::credential_invalid(),
            crate::credential::CredentialError::Io(_) => Self::credential_io(),
            crate::credential::CredentialError::LockTimeout => Self::credential_lock_timeout(),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for CommandError {}

pub type Result<T> = std::result::Result<T, CommandError>;
