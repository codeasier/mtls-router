#![allow(dead_code, unused_imports)]

//! Embedded manager control plane for protocol v4 metadata, diagnostics,
//! lifecycle state, and closed error mapping.
//!
//! This module is not wired into the current sidecar runtime path.

mod agent;
mod apikeyusage;
mod backend;
mod errors;
mod httpget;
mod lifecycle;
mod metadata;
mod modelcatalog;
mod occupant;
mod process;
mod trustedrouter;
mod types;

pub use agent::{
    AgentService, Detector, Options as AgentOptions, StaticCatalog, TrustedCatalog, TrustedModels,
};
pub use apikeyusage::{Period as UsagePeriod, Snapshot as UsageSnapshot};
pub use backend::{FakeBackend, SupervisorBackend};
pub use errors::{
    closed_error, discovery_error, error_code_name, invalid_params,
    lifecycle_error_from_supervisor, map_agent_code, map_agent_error, map_lifecycle_code,
    map_lifecycle_error, map_model_catalog_code, map_occupant_code, map_usage_code,
    method_unavailable, sidecar_invalid, sidecar_missing, startup_diagnostic, timeout_error,
    unknown_method, LifecycleError, StartupStage,
};
pub use lifecycle::{
    last_lines, merge_log_lines, Backend, Manager, DEFAULT_LOG_LINES, MAX_LOG_LINE_BYTES,
};
pub use metadata::{info, validate_production, ArtifactIdentity};
pub use occupant::{
    CallContext, OccupantConfig, OccupantDependencies, OccupantError, OccupantService,
};
pub use types::{
    AgentLine, Classification, DiscoverySnapshot, HealthInfo, StartedRouter, VersionInfo,
};

#[cfg(test)]
mod contract;
