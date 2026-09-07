#![allow(dead_code)]

//! Occupant inspection, one-shot confirmation, and verified termination.
//!
//! Ports Go `internal/manager/occupant` without changing the sidecar runtime path.

mod inspect;
mod inspect_darwin;
mod inspect_linux;
#[cfg(windows)]
mod inspect_windows;
mod service;
mod types;
mod windows_target;

pub use service::{
    fill_random_from_reader, CallContext, OccupantConfig, OccupantDependencies, OccupantService,
    TOKEN_LIFETIME,
};
pub use types::{
    valid_supervisor, Identity, Inspection, OccupantError, Recovery, RecoveryAction,
    RecoveryReason, Supervisor, SupervisorKind, SupervisorScope, Target, TerminateResult,
    VerificationMode,
};
pub use windows_target::{inspect_windows_target, select_tcp4_listener_owner, windows_pid};

#[cfg(test)]
mod service_tests;
