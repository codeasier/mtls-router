use std::fmt;

use crate::manager_core::errors::closed_error;
use crate::protocol::{ErrorCode, ProtocolError};

#[derive(Clone, Debug)]
pub struct UsageError {
    pub code: ErrorCode,
    message: &'static str,
}

impl UsageError {
    pub fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }

    pub fn message(&self) -> &'static str {
        self.message
    }

    pub fn into_protocol(self) -> ProtocolError {
        closed_error(self.code, self.message)
    }
}

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

pub fn code_of(error: &UsageError) -> ErrorCode {
    error.code
}

pub fn auth_failed() -> UsageError {
    UsageError::new(ErrorCode::UsageAuthFailed, "usage authentication failed")
}

pub fn unavailable() -> UsageError {
    UsageError::new(ErrorCode::UsageUnavailable, "usage is unavailable")
}

pub fn request_failed() -> UsageError {
    UsageError::new(ErrorCode::UsageRequestFailed, "usage request failed")
}

pub fn response_invalid() -> UsageError {
    UsageError::new(ErrorCode::UsageResponseInvalid, "usage response is invalid")
}

pub fn invalid_period() -> UsageError {
    UsageError::new(ErrorCode::InvalidParams, "usage period is invalid")
}
