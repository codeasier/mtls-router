use crate::protocol::{ErrorCode, ProtocolError};
use crate::redaction::sanitize_protocol_error;
use crate::router_core::{StartupReason, SupervisorError};

use super::types::Classification;

const CLOSED_STAGES: [&str; 9] = [
    "log_directory",
    "log_open",
    "process_launch",
    "process_inspect",
    "readiness",
    "identity_validate",
    "state_reconcile",
    "state_persist",
    "process_exit",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupStage {
    LogDirectory,
    LogOpen,
    ProcessLaunch,
    ProcessInspect,
    Readiness,
    IdentityValidate,
    StateReconcile,
    StatePersist,
    ProcessExit,
}

impl StartupStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LogDirectory => "log_directory",
            Self::LogOpen => "log_open",
            Self::ProcessLaunch => "process_launch",
            Self::ProcessInspect => "process_inspect",
            Self::Readiness => "readiness",
            Self::IdentityValidate => "identity_validate",
            Self::StateReconcile => "state_reconcile",
            Self::StatePersist => "state_persist",
            Self::ProcessExit => "process_exit",
        }
    }

    pub fn parse_closed(value: &str) -> Option<Self> {
        Some(match value {
            "log_directory" => Self::LogDirectory,
            "log_open" => Self::LogOpen,
            "process_launch" => Self::ProcessLaunch,
            "process_inspect" => Self::ProcessInspect,
            "readiness" => Self::Readiness,
            "identity_validate" => Self::IdentityValidate,
            "state_reconcile" => Self::StateReconcile,
            "state_persist" => Self::StatePersist,
            "process_exit" => Self::ProcessExit,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleError {
    pub code: ErrorCode,
    pub launched: bool,
    pub recent_output: String,
    pub stage: Option<StartupStage>,
    pub os_error_code: Option<u64>,
}

impl LifecycleError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            launched: false,
            recent_output: String::new(),
            stage: None,
            os_error_code: None,
        }
    }

    pub fn with_stage(mut self, stage: StartupStage) -> Self {
        self.stage = Some(stage);
        self
    }

    pub fn launched(mut self) -> Self {
        self.launched = true;
        self
    }

    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        self.recent_output = output.into();
        self
    }

    pub fn with_os_error(mut self, code: u64) -> Self {
        self.os_error_code = Some(code);
        self
    }

    pub fn should_latch(&self) -> bool {
        self.launched || self.stage.is_some()
    }
}

pub fn error_code_name(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "UNKNOWN".to_owned())
}

pub fn invalid_params(message: impl Into<String>) -> ProtocolError {
    closed_error(ErrorCode::InvalidParams, message)
}

pub fn timeout_error() -> ProtocolError {
    closed_error(ErrorCode::OperationTimeout, "operation timed out")
}

pub fn method_unavailable() -> ProtocolError {
    closed_error(ErrorCode::InvalidRequest, "method is unavailable")
}

pub fn unknown_method() -> ProtocolError {
    closed_error(ErrorCode::UnknownMethod, "unknown method")
}

pub fn sidecar_missing() -> ProtocolError {
    closed_error(ErrorCode::SidecarMissing, "router sidecar is missing")
}

pub fn sidecar_invalid() -> ProtocolError {
    closed_error(ErrorCode::SidecarInvalid, "router sidecar is invalid")
}

pub fn closed_error(code: ErrorCode, message: impl Into<String>) -> ProtocolError {
    sanitize_protocol_error(code, &message.into(), None)
}

pub fn map_lifecycle_error(error: &LifecycleError) -> ProtocolError {
    map_lifecycle_code(error.code)
}

pub fn map_lifecycle_code(code: ErrorCode) -> ProtocolError {
    let message = match code {
        ErrorCode::InvalidParams => "invalid router lifecycle parameters",
        ErrorCode::RouterNotFound => "router was not found",
        ErrorCode::RouterAlreadyRunning => "router is already running",
        ErrorCode::RouterStartFailed => "router could not be started",
        ErrorCode::RouterNotReady => "router did not become ready",
        ErrorCode::RouterDegraded => "router upstream is unavailable",
        ErrorCode::RouterNotOwned => "router is not owned by this desktop session",
        ErrorCode::RouterStateStale => "router state is stale",
        ErrorCode::RouterLegacyManaged => "a legacy desktop router requires verified migration",
        ErrorCode::PortOccupied => "router port is occupied",
        ErrorCode::OperationTimeout => "router operation timed out",
        _ => return closed_error(ErrorCode::RouterStartFailed, "router operation failed"),
    };
    closed_error(code, message)
}

pub fn map_agent_error(error: crate::manager_core::agent::OperationError) -> ProtocolError {
    let mut mapped = map_agent_code(error.code);
    if let (Some(path), Some(rule)) = (&error.path, &error.rule) {
        mapped.details = Some(crate::protocol::ErrorDetails {
            path: path.clone(),
            rule: rule.clone(),
        });
    }
    mapped
}

pub fn map_agent_code(code: ErrorCode) -> ProtocolError {
    let message = match code {
        ErrorCode::InvalidParams => "invalid Agent parameters",
        ErrorCode::AgentNotFound => "selected Agent was not found",
        ErrorCode::ConfigInvalid => "Agent configuration is invalid",
        ErrorCode::ConfigNotWritable => "Agent configuration is not writable",
        ErrorCode::PreviewStale => "Agent preview is stale",
        ErrorCode::BackupFailed => "Agent configuration backup failed",
        ErrorCode::WriteFailed => "Agent configuration write failed",
        ErrorCode::RollbackFailed => "Agent configuration rollback failed",
        ErrorCode::OperationTimeout => "Agent operation timed out",
        ErrorCode::AgentOperationBusy => "Another Agent operation is in progress",
        ErrorCode::ModelStateInvalid => "Agent model state is invalid",
        ErrorCode::ModelCatalogStale => "Agent model catalog is stale",
        ErrorCode::ModelConfigInvalid => "Agent model configuration is invalid",
        ErrorCode::ModelNotAvailable => "A selected model is no longer available",
        ErrorCode::ManagedConfigDrift => "Managed Agent configuration drift requires approval",
        ErrorCode::CodexAuthUnsupported => "Codex authentication policy is unsupported",
        ErrorCode::AgentNotManaged => "Agent is not managed by CodeasierRouter",
        _ => return closed_error(ErrorCode::WriteFailed, "Agent operation failed"),
    };
    closed_error(code, message)
}

pub fn map_occupant_code(code: ErrorCode) -> ProtocolError {
    let message = match code {
        ErrorCode::OccupantNotFound => "port occupant was not found",
        ErrorCode::OccupantNotOwned => "port occupant belongs to another user",
        ErrorCode::OccupantIdentityUnavailable => "port occupant identity is unavailable",
        ErrorCode::OccupantChanged => "port occupant changed",
        ErrorCode::OccupantProtected => "port occupant is protected",
        ErrorCode::OccupantPermissionDenied => "permission to terminate port occupant was denied",
        ErrorCode::OccupantTerminationFailed => "port occupant could not be terminated",
        ErrorCode::PortReleaseTimeout => "router port was not released",
        ErrorCode::ConfirmationExpired => "occupant confirmation expired",
        _ => {
            return closed_error(
                ErrorCode::OccupantIdentityUnavailable,
                "port occupant identity is unavailable",
            )
        }
    };
    closed_error(code, message)
}

pub fn map_usage_code(code: ErrorCode) -> ProtocolError {
    let message = match code {
        ErrorCode::UsageAuthFailed => "usage authentication failed",
        ErrorCode::UsageUnavailable => "usage is unavailable",
        ErrorCode::UsageRequestFailed => "usage request failed",
        ErrorCode::UsageResponseInvalid => "usage response is invalid",
        _ => return closed_error(ErrorCode::UsageUnavailable, "usage is unavailable"),
    };
    closed_error(code, message)
}

pub fn map_model_catalog_code(code: ErrorCode) -> ProtocolError {
    let message = match code {
        ErrorCode::ModelAuthFailed => "model catalog authentication failed",
        ErrorCode::ModelDiscoveryFailed => "model catalog request failed",
        ErrorCode::ModelResponseInvalid => "model catalog response is invalid",
        ErrorCode::ModelCatalogEmpty => "model catalog is empty",
        ErrorCode::ModelCatalogStale => "trusted router identity changed",
        _ => {
            return closed_error(
                ErrorCode::ModelDiscoveryFailed,
                "model discovery is unavailable",
            )
        }
    };
    closed_error(code, message)
}

pub fn discovery_error(
    classification: Classification,
    require_health: bool,
    health_status: Option<&str>,
    version: Option<&str>,
) -> Option<ProtocolError> {
    match classification {
        Classification::DesktopOwned | Classification::ExternalCompatible => {
            if require_health && health_status.unwrap_or("").is_empty() {
                Some(closed_error(
                    ErrorCode::RouterDegraded,
                    "router health is unavailable",
                ))
            } else {
                None
            }
        }
        Classification::Degraded => {
            let has_health = !health_status.unwrap_or("").is_empty();
            let has_version = !version.unwrap_or("").is_empty();
            if (!require_health && has_version) || (require_health && has_health) {
                None
            } else {
                Some(closed_error(
                    ErrorCode::RouterDegraded,
                    "router endpoint is unavailable",
                ))
            }
        }
        Classification::Absent => Some(closed_error(
            ErrorCode::RouterNotFound,
            "router was not found",
        )),
        Classification::Stale => Some(closed_error(
            ErrorCode::RouterStateStale,
            "router state is stale",
        )),
        Classification::LegacyManaged => Some(closed_error(
            ErrorCode::RouterLegacyManaged,
            "a legacy desktop router is still running",
        )),
        Classification::UnknownOccupant => Some(closed_error(
            ErrorCode::PortOccupied,
            "router port is occupied by an unknown process",
        )),
    }
}

pub fn startup_diagnostic(error: &LifecycleError) -> String {
    let stage = error
        .stage
        .map(StartupStage::as_str)
        .filter(|stage| CLOSED_STAGES.contains(stage))
        .unwrap_or("unknown");
    let mut diagnostic = format!("stage={stage} code={}", error_code_name(error.code));
    if let Some(os_error) = error.os_error_code.filter(|code| *code != 0) {
        diagnostic.push_str(&format!(" os_error={os_error}"));
    }
    diagnostic
}

pub fn lifecycle_error_from_supervisor(error: SupervisorError) -> LifecycleError {
    match error {
        SupervisorError::AlreadyRunning => LifecycleError::new(ErrorCode::RouterAlreadyRunning),
        SupervisorError::CommandTimeout => LifecycleError::new(ErrorCode::OperationTimeout),
        SupervisorError::Disconnected => LifecycleError::new(ErrorCode::RouterDegraded),
        SupervisorError::Failure(failure) => LifecycleError::new(failure.code),
        SupervisorError::Startup(startup) => {
            let reason = startup.reason();
            let mut output = format!("reason={}", reason.as_str());
            if let Some(refusal) = startup.listen_refusal() {
                output.push_str(" listen_refusal=");
                output.push_str(refusal.as_str());
            }
            let mut mapped =
                LifecycleError::new(supervisor_startup_code(reason)).with_output(output);
            mapped.stage = Some(supervisor_startup_stage(reason));
            if let Some(code) = startup.os_error() {
                mapped = mapped.with_os_error(code);
            }
            mapped
        }
    }
}

fn supervisor_startup_code(reason: StartupReason) -> ErrorCode {
    match reason {
        StartupReason::ArgumentsInvalid | StartupReason::ConfigInvalid => ErrorCode::InvalidParams,
        StartupReason::UpstreamProbeFailed | StartupReason::ProbeSetupFailed => {
            ErrorCode::RouterStartFailed
        }
        _ => ErrorCode::RouterStartFailed,
    }
}

fn supervisor_startup_stage(reason: StartupReason) -> StartupStage {
    match reason {
        StartupReason::LogOpenFailed => StartupStage::LogOpen,
        StartupReason::TlsMaterialInvalid => StartupStage::IdentityValidate,
        StartupReason::ProbeSetupFailed | StartupReason::UpstreamProbeFailed => {
            StartupStage::Readiness
        }
        StartupReason::ListenFailed
        | StartupReason::BackendStartFailed
        | StartupReason::ArgumentsInvalid
        | StartupReason::ConfigInvalid => StartupStage::ProcessLaunch,
        StartupReason::ShutdownFailed | StartupReason::RouterFailure => StartupStage::ProcessExit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router_core::StartupError;

    #[test]
    fn lifecycle_messages_are_closed_and_stable() {
        let error = map_lifecycle_code(ErrorCode::RouterStartFailed);
        assert_eq!(error.code, ErrorCode::RouterStartFailed);
        assert_eq!(error.message, "router could not be started");
        let unknown = map_lifecycle_code(ErrorCode::AgentNotFound);
        assert_eq!(unknown.code, ErrorCode::RouterStartFailed);
        assert_eq!(unknown.message, "router operation failed");
    }

    #[test]
    fn agent_occupant_usage_and_catalog_fallbacks_are_closed() {
        assert_eq!(
            map_agent_code(ErrorCode::PortOccupied).code,
            ErrorCode::WriteFailed
        );
        assert_eq!(
            map_occupant_code(ErrorCode::RouterNotFound).code,
            ErrorCode::OccupantIdentityUnavailable
        );
        assert_eq!(
            map_usage_code(ErrorCode::RouterNotFound).code,
            ErrorCode::UsageUnavailable
        );
        assert_eq!(
            map_model_catalog_code(ErrorCode::RouterNotFound).code,
            ErrorCode::ModelDiscoveryFailed
        );
        assert_eq!(
            map_occupant_code(ErrorCode::OccupantProtected).message,
            "port occupant is protected"
        );
        assert_eq!(
            map_usage_code(ErrorCode::UsageAuthFailed).message,
            "usage authentication failed"
        );
    }

    #[test]
    fn discovery_error_matches_go_classification_contract() {
        assert_eq!(
            discovery_error(Classification::Absent, false, None, None)
                .unwrap()
                .code,
            ErrorCode::RouterNotFound
        );
        assert_eq!(
            discovery_error(Classification::UnknownOccupant, false, None, None)
                .unwrap()
                .code,
            ErrorCode::PortOccupied
        );
        assert_eq!(
            discovery_error(Classification::LegacyManaged, false, None, None)
                .unwrap()
                .code,
            ErrorCode::RouterLegacyManaged
        );
        assert!(discovery_error(Classification::DesktopOwned, false, None, Some("v1")).is_none());
        assert_eq!(
            discovery_error(Classification::DesktopOwned, true, None, Some("v1"))
                .unwrap()
                .code,
            ErrorCode::RouterDegraded
        );
        assert!(discovery_error(Classification::Degraded, true, Some("degraded"), None).is_none());
    }

    #[test]
    fn startup_diagnostic_uses_closed_stage_and_omits_zero_os_error() {
        let known =
            LifecycleError::new(ErrorCode::RouterStartFailed).with_stage(StartupStage::LogOpen);
        assert_eq!(
            startup_diagnostic(&known),
            "stage=log_open code=ROUTER_START_FAILED"
        );
        let with_os = known.clone().with_os_error(5);
        assert_eq!(
            startup_diagnostic(&with_os),
            "stage=log_open code=ROUTER_START_FAILED os_error=5"
        );
        let unknown = LifecycleError::new(ErrorCode::RouterStartFailed);
        assert_eq!(
            startup_diagnostic(&unknown),
            "stage=unknown code=ROUTER_START_FAILED"
        );
    }

    #[test]
    fn listen_failure_preserves_os_error_and_closed_refusal() {
        let reserved = lifecycle_error_from_supervisor(SupervisorError::Startup(
            StartupError::new(StartupReason::ListenFailed).with_os_error(10013),
        ));
        assert_eq!(reserved.code, ErrorCode::RouterStartFailed);
        assert_eq!(reserved.stage, Some(StartupStage::ProcessLaunch));
        assert_eq!(reserved.os_error_code, Some(10013));
        assert_eq!(
            reserved.recent_output,
            "reason=listen_failed listen_refusal=access_denied"
        );
        assert_eq!(
            startup_diagnostic(&reserved),
            "stage=process_launch code=ROUTER_START_FAILED os_error=10013"
        );
        assert!(!reserved.recent_output.contains("127.0.0.1"));
        assert!(!reserved.recent_output.contains("WSA"));

        let in_use = lifecycle_error_from_supervisor(SupervisorError::Startup(
            StartupError::new(StartupReason::ListenFailed).with_os_error(10048),
        ));
        assert_eq!(
            in_use.recent_output,
            "reason=listen_failed listen_refusal=address_in_use"
        );
        assert_eq!(in_use.os_error_code, Some(10048));

        let unknown = lifecycle_error_from_supervisor(SupervisorError::Startup(StartupError::new(
            StartupReason::ListenFailed,
        )));
        assert_eq!(
            unknown.recent_output,
            "reason=listen_failed listen_refusal=failed"
        );
        assert_eq!(unknown.os_error_code, None);
    }
}
