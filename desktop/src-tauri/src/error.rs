pub use router_contract::error::*;

use serde::Serialize;

/// Command error as it crosses the Tauri boundary. Conversion records the stable
/// code and stage. The serialized payload is unchanged, and the message stays off disk.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct IpcError(pub CommandError);

impl From<crate::credential::CredentialError> for IpcError {
    fn from(error: crate::credential::CredentialError) -> Self {
        CommandError::from(error).into()
    }
}

impl From<CommandError> for IpcError {
    fn from(error: CommandError) -> Self {
        crate::app_log::record_command_failure(&error);
        Self(error)
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0.code)
    }
}

pub type CommandResult<T> = std::result::Result<T, IpcError>;

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
