pub use router_contract::error::*;

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
