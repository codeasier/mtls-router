#![allow(dead_code, unused_imports)]

//! Embedded manager control plane for protocol v4 metadata, diagnostics,
//! lifecycle state, and closed error mapping.
//!
//! `session::production_factory` is the desktop runtime host: the Tauri
//! process serves protocol v4 in-process and drives the embedded router-core
//! supervisor, with no Go sidecar on the normal path.

mod agent;
mod apikeyusage;
mod backend;
mod discovery;
mod embedded;
mod errors;
mod httpget;
mod legacy;
mod lifecycle;
mod metadata;
mod modelcatalog;
mod occupant;
mod paths;
mod process;
mod session;
mod state;
mod trustedrouter;
mod types;

pub use agent::{
    AgentService, Detector, Options as AgentOptions, StaticCatalog, TrustedCatalog, TrustedModels,
};
pub use apikeyusage::{Period as UsagePeriod, Snapshot as UsageSnapshot};
pub use backend::{FakeBackend, SupervisorBackend};
pub use discovery::{
    discover, discover_desktop_startup, discover_status, is_development_id, DiscoveryConfig,
    DiscoveryHost, Found, SystemHost,
};
pub use embedded::{desktop_access_logger, EmbeddedBackend, EmbeddedConfig, EmbeddedLifecycle};
pub use errors::{
    closed_error, discovery_error, error_code_name, invalid_params,
    lifecycle_error_from_supervisor, map_agent_code, map_agent_error, map_lifecycle_code,
    map_lifecycle_error, map_model_catalog_code, map_occupant_code, map_usage_code,
    method_unavailable, sidecar_invalid, sidecar_missing, startup_diagnostic, timeout_error,
    unknown_method, LifecycleError, StartupStage,
};
pub use legacy::{
    parse_current_installation, parse_record, record_allows_migration, CurrentLineage,
    LegacyRuntime, RecordedIdentity,
};
pub use lifecycle::{
    last_lines, merge_log_lines, Backend, Manager, DEFAULT_LOG_LINES, MAX_LOG_LINE_BYTES,
};
pub use metadata::{info, validate_production, ArtifactIdentity};
pub use occupant::{
    CallContext, OccupantConfig, OccupantDependencies, OccupantError, OccupantService,
};
pub use paths::Paths;
pub use session::{
    current_identity, desktop_eligible, fixture_config, fixture_manager, load_embedded_preset,
    production_factory, production_manager, InProcessFactory, ProductionRuntime, SessionConfig,
    DEFAULT_LISTEN,
};
pub use state::{read as read_router_state, write as write_router_state, RouterState};
pub use trustedrouter::{normalize_listener, Listener};
pub use types::{
    AgentLine, Classification, DiscoverySnapshot, HealthInfo, StartedRouter, VersionInfo,
};

#[cfg(test)]
mod contract;
