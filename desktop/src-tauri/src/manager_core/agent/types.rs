use serde::{Deserialize, Serialize};

use super::modelconfig::Agent;
use crate::protocol::ErrorCode;

pub const MAX_CONFIG_SIZE: usize = 16 << 20;
pub const MAX_RENDER_SIZE: usize = 2 << 20;
pub const REDACTED_API_KEY: &str = "<redacted-api-key>";
pub const PROVIDER_DISPLAY_NAME: &str = "CodeasierRouter";
pub const LEGACY_PROVIDER_DISPLAY_NAME: &str = "mtls-router";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Json,
    Jsonc,
    Toml,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Jsonc => "jsonc",
            Self::Toml => "toml",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryReason {
    SyntaxInvalid,
    UnsupportedStructure,
    Unreadable,
    Oversized,
    NonRegular,
    Linked,
    NotWritable,
    ParentUnavailable,
    TransactionRecoveryPending,
    WritesDisabled,
}

impl RecoveryReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SyntaxInvalid => "syntax_invalid",
            Self::UnsupportedStructure => "unsupported_structure",
            Self::Unreadable => "unreadable",
            Self::Oversized => "oversized",
            Self::NonRegular => "non_regular",
            Self::Linked => "linked",
            Self::NotWritable => "not_writable",
            Self::ParentUnavailable => "parent_unavailable",
            Self::TransactionRecoveryPending => "transaction_recovery_pending",
            Self::WritesDisabled => "writes_disabled",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupUnavailableReason {
    NotManaged,
    ModelStateInvalid,
    WritesDisabled,
}

impl CleanupUnavailableReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotManaged => "not_managed",
            Self::ModelStateInvalid => "model_state_invalid",
            Self::WritesDisabled => "writes_disabled",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CleanupState {
    pub managed: bool,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CleanupUnavailableReason>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryFileState {
    pub role: String,
    pub path: String,
    pub format: Format,
    pub exists: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<RecoveryReason>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryState {
    pub eligible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<RecoveryReason>,
    pub files: Vec<RecoveryFileState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub agent: Agent,
    pub name: String,
    pub detected: bool,
    pub command: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub auth_path: String,
    pub format: Format,
    pub exists: bool,
    pub writable: bool,
    pub configured: bool,
    pub invalid: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub migratable: bool,
    pub recovery: RecoveryState,
    pub cleanup: CleanupState,
    #[serde(skip)]
    pub path_overridden: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationError {
    pub code: ErrorCode,
    pub message: String,
    pub path: Option<String>,
    pub rule: Option<String>,
}

impl OperationError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
            rule: None,
        }
    }

    pub fn with_details(mut self, path: impl Into<String>, rule: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self.rule = Some(rule.into());
        self
    }
}

impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OperationError {}

pub fn is_managed_provider_display_name(name: &str) -> bool {
    name == PROVIDER_DISPLAY_NAME || name == LEGACY_PROVIDER_DISPLAY_NAME
}

pub fn append_reason(reasons: &mut Vec<RecoveryReason>, reason: RecoveryReason) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}
