//! Local device identity; GUI and model API keys are outside this crate.
mod client;
mod crypto;
mod platform;
mod store;
pub mod wire;
pub mod private_files {
    pub use crate::platform::{atomic_write, directory, read, Lock};
}

pub use client::{ConnectionSummary, NativeClient};
pub use crypto::{DeviceKey, IdentityTrust};
pub use store::{IdentityStore, PendingIdentity, State};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    Storage,
    InvalidState,
    Busy,
    Conflict,
    InvalidCertificate,
    Unavailable,
    Network,
    ReauthRequired,
    MembershipRequired,
    Rejected,
}

impl IdentityError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Storage => "IDENTITY_STORAGE_UNAVAILABLE",
            Self::InvalidState => "IDENTITY_STATE_INVALID",
            Self::Busy => "IDENTITY_BUSY",
            Self::Conflict => "CERT_OPERATION_CONFLICT",
            Self::InvalidCertificate => "CLIENT_CERT_REJECTED",
            Self::Unavailable => "CERT_SERVICE_UNAVAILABLE",
            Self::Network => "IDENTITY_NETWORK_UNAVAILABLE",
            Self::ReauthRequired => "CERT_REAUTH_REQUIRED",
            Self::MembershipRequired => "ORG_MEMBERSHIP_REQUIRED",
            Self::Rejected => "CLIENT_CERT_REJECTED",
        }
    }
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for IdentityError {}
pub type Result<T> = std::result::Result<T, IdentityError>;

pub const ENROLL_ORIGIN: &str = "https://enroll.codeasier.top";
pub const RELAY_ORIGIN: &str = "https://relay.codeasier.top";
pub const AUTH_PROTOCOL_VERSION: &str = "1";

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
