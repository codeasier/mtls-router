#![allow(dead_code)]

//! Channel-bound router trust for catalog and usage fetches.
//!
//! Frozen against Go `internal/manager/trustedrouter`. This module is not the
//! default sidecar runtime path.

mod channel;
mod coordinator;
mod listener;

pub use channel::{Channel, VERSION_BUDGET};
pub use coordinator::{
    Coordinator, StartOutcome, TrustedLifecycle, UsageResult, USAGE_ESTABLISH_BUDGET,
};
pub use listener::{normalize_listener, Listener};

use crate::manager_core::agent::TrustedBinding;
use crate::manager_core::types::Classification;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Binding {
    pub router_base_url: String,
    pub api_base_url: String,
    pub deployment_id: String,
    pub protocol_version: String,
}

impl From<Binding> for TrustedBinding {
    fn from(value: Binding) -> Self {
        Self {
            router_base_url: value.router_base_url,
            api_base_url: value.api_base_url,
            deployment_id: value.deployment_id,
            protocol_version: value.protocol_version,
        }
    }
}

impl From<&TrustedBinding> for Binding {
    fn from(value: &TrustedBinding) -> Self {
        Self {
            router_base_url: value.router_base_url.clone(),
            api_base_url: value.api_base_url.clone(),
            deployment_id: value.deployment_id.clone(),
            protocol_version: value.protocol_version.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RemoteVersion {
    pub pid: i32,
    pub deployment_id: String,
    pub management_protocol_version: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StartedIdentity {
    pub pid: i32,
    pub owner: String,
    pub listen_addr: String,
    pub deployment_id: String,
    pub management_protocol_version: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RouterIdentity {
    pub pid: i32,
    pub owner: String,
    pub listen_addr: String,
    pub binary_path: String,
    pub process_started_at: String,
    pub process_executable: String,
    pub deployment_id: String,
    pub management_protocol_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Discovery {
    pub classification: Classification,
    pub owner: String,
    pub listen_addr: String,
    pub version: RemoteVersion,
    pub state: RouterIdentity,
}

impl Discovery {
    pub fn absent() -> Self {
        Self {
            classification: Classification::Absent,
            owner: String::new(),
            listen_addr: String::new(),
            version: RemoteVersion::default(),
            state: RouterIdentity::default(),
        }
    }
}

#[cfg(test)]
mod tests;
