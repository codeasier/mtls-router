//! Legacy installation identity, verified one-shot stop, and the durable
//! per-generation attempt record that forbids duplicate termination.
//!
//! This module is not wired into the current sidecar runtime path.

mod identity;
mod record;
mod stop;

#[cfg(test)]
mod fail_closed;

pub use identity::{
    complete_desktop_state, complete_process, correlated_installation, current_owned,
    generation_compatible, installation_matches, manager_identity, manager_matches,
    parse_current_installation, parse_current_installation_bytes, parse_record,
    record_allows_migration, record_allows_reclaim, router_identity, supported_legacy_source,
    CurrentLineage, RecordedIdentity,
};
pub use record::{IssuedSignal, MigrationOutcome, MigrationRecord, CORRUPT_RECORD_LINE};
pub use stop::{
    prepare_legacy_stop, previous_manager_absent, resample_port, router_genuine, stop_verified,
    verified_migratable, LegacyRuntime, MigrationHost, ProcessHost, StopBudget,
};
