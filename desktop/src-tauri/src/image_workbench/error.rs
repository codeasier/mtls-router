use crate::error::CommandError;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeKind {
    Busy,
    NotReady,
    InputEmpty,
    InputTooLarge,
    ImageInvalid,
    ImageTooLarge,
    NoReference,
    StoreCorrupt,
    StoreIncompatible,
    Cancelled,
    Interrupted,
    ChatFailed,
    ImageFailed,
    Timeout,
    Identity,
    Redial,
    Unknown,
}

impl SafeKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Busy => "WORKBENCH_BUSY",
            Self::NotReady => "WORKBENCH_NOT_READY",
            Self::InputEmpty => "WORKBENCH_INPUT_EMPTY",
            Self::InputTooLarge => "WORKBENCH_INPUT_TOO_LARGE",
            Self::ImageInvalid => "WORKBENCH_IMAGE_INVALID",
            Self::ImageTooLarge => "WORKBENCH_IMAGE_TOO_LARGE",
            Self::NoReference => "WORKBENCH_NO_REFERENCE",
            Self::StoreCorrupt => "WORKBENCH_STORE_CORRUPT",
            Self::StoreIncompatible => "WORKBENCH_STORE_INCOMPATIBLE",
            Self::Cancelled => "WORKBENCH_CANCELLED",
            Self::Interrupted => "WORKBENCH_INTERRUPTED",
            Self::ChatFailed => "WORKBENCH_CHAT_FAILED",
            Self::ImageFailed => "WORKBENCH_IMAGE_FAILED",
            Self::Timeout => "WORKBENCH_TIMEOUT",
            Self::Identity => "WORKBENCH_IDENTITY",
            Self::Redial => "WORKBENCH_REDIAL",
            Self::Unknown => "WORKBENCH_FAILED",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Busy => "a workbench operation is already running",
            Self::NotReady => "the workbench is not ready",
            Self::InputEmpty => "message text is empty",
            Self::InputTooLarge => "message or prompt exceeds the size limit",
            Self::ImageInvalid => "the image is not a static PNG, JPEG, or WebP",
            Self::ImageTooLarge => "the image exceeds the size or pixel limit",
            Self::NoReference => "edit requires a reference image in this conversation",
            Self::StoreCorrupt => "workbench storage is unreadable",
            Self::StoreIncompatible => "workbench storage needs a newer app",
            Self::Cancelled => "the operation was cancelled",
            Self::Interrupted => "the operation was interrupted",
            Self::ChatFailed => "the chat request failed",
            Self::ImageFailed => "the image request failed",
            Self::Timeout => "the request timed out",
            Self::Identity => "the local router identity changed",
            Self::Redial => "the trusted connection had to redial",
            Self::Unknown => "the workbench operation failed",
        }
    }
}

#[derive(Debug)]
pub struct WorkbenchError {
    pub kind: SafeKind,
}

impl WorkbenchError {
    pub fn new(kind: SafeKind) -> Self {
        Self { kind }
    }
}

impl std::fmt::Display for WorkbenchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.kind.message())
    }
}

impl From<WorkbenchError> for CommandError {
    fn from(error: WorkbenchError) -> Self {
        CommandError::new(error.kind.code(), error.kind.message())
    }
}
