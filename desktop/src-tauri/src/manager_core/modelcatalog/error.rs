use std::fmt;

use crate::manager_core::errors::closed_error;
use crate::protocol::{ErrorCode, ProtocolError};

#[derive(Clone, Debug)]
pub struct CatalogError {
    pub code: ErrorCode,
    message: &'static str,
}

impl CatalogError {
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

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

pub fn code_of(error: &CatalogError) -> ErrorCode {
    error.code
}

pub fn auth_failed() -> CatalogError {
    CatalogError::new(
        ErrorCode::ModelAuthFailed,
        "model catalog authentication failed",
    )
}

pub fn discovery_failed() -> CatalogError {
    CatalogError::new(
        ErrorCode::ModelDiscoveryFailed,
        "model catalog request failed",
    )
}

pub fn response_invalid() -> CatalogError {
    CatalogError::new(
        ErrorCode::ModelResponseInvalid,
        "model catalog response is invalid",
    )
}

pub fn catalog_empty() -> CatalogError {
    CatalogError::new(ErrorCode::ModelCatalogEmpty, "model catalog is empty")
}
