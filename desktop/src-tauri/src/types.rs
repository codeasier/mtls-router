use crate::port_recovery::ReleaseObservation;
use chrono::DateTime;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PollError {
    pub code: String,
}

impl PollError {
    pub fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PollSnapshot {
    pub revision: u64,
    pub status: Option<RouterStatus>,
    pub health: Option<RouterHealth>,
    pub health_stale: bool,
    pub status_error: Option<PollError>,
    pub health_error: Option<PollError>,
    pub release_observation: Option<ReleaseObservation>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeLanguage {
    #[default]
    ZhCn,
    En,
}

impl NativeLanguage {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "zh-CN" => Some(Self::ZhCn),
            "en" => Some(Self::En),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouterStatus {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listen_addr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_logs: Option<Vec<String>>,
}

impl RouterStatus {
    pub fn available(&self) -> bool {
        matches!(
            self.state.as_str(),
            "desktop_owned" | "external_compatible" | "degraded"
        )
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouterHealth {
    pub status: String,
    pub checked_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouterVersion {
    pub version: String,
    pub deployment_id: String,
    pub management_protocol_version: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouterLogs {
    pub lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OccupantVerificationMode {
    VerifiedIdentity,
    WindowsPidOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    ForceTerminate,
    ManualStopRequired,
    Unavailable,
}

impl fmt::Display for RecoveryAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ForceTerminate => "force_terminate",
            Self::ManualStopRequired => "manual_stop_required",
            Self::Unavailable => "unavailable",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryReason {
    ServiceManaged,
    InsufficientPrivilege,
    DifferentUser,
    ProtectedProcess,
    IdentityUnavailable,
}

impl fmt::Display for RecoveryReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ServiceManaged => "service_managed",
            Self::InsufficientPrivilege => "insufficient_privilege",
            Self::DifferentUser => "different_user",
            Self::ProtectedProcess => "protected_process",
            Self::IdentityUnavailable => "identity_unavailable",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorKind {
    WindowsService,
    SystemdUser,
    SystemdSystem,
}

impl fmt::Display for SupervisorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WindowsService => "windows_service",
            Self::SystemdUser => "systemd_user",
            Self::SystemdSystem => "systemd_system",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorScope {
    User,
    System,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OccupantRecovery {
    pub action: RecoveryAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<RecoveryReason>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OccupantSupervisor {
    pub kind: SupervisorKind,
    pub scope: SupervisorScope,
    pub identifiers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OccupantInspection {
    pub pid: u32,
    pub verification_mode: OccupantVerificationMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    pub listen_addr: String,
    pub recovery: OccupantRecovery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisor: Option<OccupantSupervisor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmation_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OccupantTermination {
    ProcessTerminated,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OccupantPortState {
    Released,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OccupantTerminationResult {
    pub termination: OccupantTermination,
    pub port_state: OccupantPortState,
}

fn is_manager_rfc3339(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }
    for index in [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        if !bytes[index].is_ascii_digit() {
            return false;
        }
    }
    if bytes[17] > b'5' {
        return false;
    }

    let mut timezone_start = 19;
    if bytes.get(timezone_start) == Some(&b'.') {
        timezone_start += 1;
        let fraction_start = timezone_start;
        while bytes.get(timezone_start).is_some_and(u8::is_ascii_digit) {
            timezone_start += 1;
        }
        if timezone_start == fraction_start {
            return false;
        }
    }

    match &bytes[timezone_start..] {
        [b'Z'] => true,
        [sign @ (b'+' | b'-'), hour_tens, hour_ones, b':', minute_tens, minute_ones] => {
            let _ = sign;
            hour_tens.is_ascii_digit()
                && hour_ones.is_ascii_digit()
                && minute_tens.is_ascii_digit()
                && minute_ones.is_ascii_digit()
        }
        _ => false,
    }
}

fn valid_windows_service_name(identifier: &str) -> bool {
    !identifier.trim().is_empty()
        && !identifier
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | ',' | '"'))
}

fn valid_systemd_service_unit(identifier: &str) -> bool {
    const SUFFIX: &str = ".service";
    if identifier.len() > 255 || !identifier.ends_with(SUFFIX) {
        return false;
    }
    let stem = &identifier[..identifier.len() - SUFFIX.len()];
    if stem.is_empty() {
        return false;
    }

    let bytes = stem.as_bytes();
    let mut index = 0;
    let mut at_count = 0;
    while index < bytes.len() {
        let character = bytes[index];
        if character.is_ascii_alphanumeric() || matches!(character, b':' | b'_' | b'.' | b'-') {
            index += 1;
            continue;
        }
        if character == b'@' {
            if index == 0 || at_count == 1 {
                return false;
            }
            at_count += 1;
            index += 1;
            continue;
        }
        if character != b'\\'
            || bytes.get(index + 1) != Some(&b'x')
            || !bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit)
            || !bytes.get(index + 3).is_some_and(u8::is_ascii_hexdigit)
        {
            return false;
        }
        index += 4;
    }
    true
}

fn valid_supervisor(supervisor: &OccupantSupervisor) -> bool {
    if supervisor.identifiers.is_empty() || supervisor.identifiers.len() > 16 {
        return false;
    }
    if !matches!(
        (supervisor.kind, supervisor.scope),
        (SupervisorKind::WindowsService, SupervisorScope::System)
            | (SupervisorKind::SystemdUser, SupervisorScope::User)
            | (SupervisorKind::SystemdSystem, SupervisorScope::System)
    ) {
        return false;
    }

    for (index, identifier) in supervisor.identifiers.iter().enumerate() {
        if identifier.is_empty()
            || identifier.len() > 256
            || index > 0 && supervisor.identifiers[index - 1] >= *identifier
        {
            return false;
        }
        let valid_identifier = match supervisor.kind {
            SupervisorKind::WindowsService => valid_windows_service_name(identifier),
            SupervisorKind::SystemdUser | SupervisorKind::SystemdSystem => {
                valid_systemd_service_unit(identifier)
            }
        };
        if !valid_identifier {
            return false;
        }
    }

    serde_json::to_vec(supervisor).is_ok_and(|encoded| encoded.len() <= 4 * 1024)
}

impl<'de> Deserialize<'de> for OccupantInspection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireInspection {
            pid: u32,
            verification_mode: OccupantVerificationMode,
            process_name: Option<String>,
            executable: Option<String>,
            listen_addr: String,
            recovery: OccupantRecovery,
            supervisor: Option<OccupantSupervisor>,
            confirmation_token: Option<String>,
            expires_at: Option<String>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("occupant inspection must be an object"))?;
        let has_process_name = object.contains_key("process_name");
        let has_executable = object.contains_key("executable");
        let has_supervisor = object.contains_key("supervisor");
        let has_confirmation_token = object.contains_key("confirmation_token");
        let has_expires_at = object.contains_key("expires_at");
        let recovery_has_reason = object
            .get("recovery")
            .and_then(Value::as_object)
            .is_some_and(|recovery| recovery.contains_key("reason"));
        let wire: WireInspection = serde_json::from_value(value).map_err(D::Error::custom)?;

        if wire.pid == 0 {
            return Err(D::Error::custom("occupant inspection PID must be positive"));
        }
        if wire.listen_addr != "127.0.0.1:19099" {
            return Err(D::Error::custom(
                "occupant inspection listen address must match the fixed endpoint",
            ));
        }
        let forceable = match (wire.recovery.action, wire.recovery.reason) {
            (RecoveryAction::ForceTerminate, None) if !recovery_has_reason => true,
            (
                RecoveryAction::ManualStopRequired,
                Some(
                    RecoveryReason::ServiceManaged
                    | RecoveryReason::InsufficientPrivilege
                    | RecoveryReason::DifferentUser,
                ),
            ) if recovery_has_reason => false,
            (
                RecoveryAction::Unavailable,
                Some(RecoveryReason::ProtectedProcess | RecoveryReason::IdentityUnavailable),
            ) if recovery_has_reason => false,
            _ => {
                return Err(D::Error::custom(
                    "occupant inspection recovery action and reason are inconsistent",
                ));
            }
        };
        match wire.verification_mode {
            OccupantVerificationMode::VerifiedIdentity => {
                let has_complete_metadata = has_process_name
                    && has_executable
                    && matches!(wire.process_name.as_deref(), Some(value) if !value.trim().is_empty())
                    && matches!(wire.executable.as_deref(), Some(value) if !value.trim().is_empty());
                let may_redact_metadata = !has_process_name
                    && !has_executable
                    && wire.recovery.action == RecoveryAction::ManualStopRequired
                    && wire.recovery.reason == Some(RecoveryReason::DifferentUser);
                if !has_complete_metadata && !may_redact_metadata {
                    return Err(D::Error::custom(
                        "verified identity requires both process fields unless redacted for a different user",
                    ));
                }
            }
            OccupantVerificationMode::WindowsPidOnly => {
                if has_process_name || has_executable {
                    return Err(D::Error::custom(
                        "Windows PID-only inspection cannot include process metadata",
                    ));
                }
            }
        }

        if forceable {
            let token = wire.confirmation_token.as_deref().ok_or_else(|| {
                D::Error::custom("forceable occupant inspection requires a confirmation token")
            })?;
            let expires_at = wire.expires_at.as_deref().ok_or_else(|| {
                D::Error::custom("forceable occupant inspection requires an expiry")
            })?;
            if !has_confirmation_token || token.trim().is_empty() {
                return Err(D::Error::custom(
                    "occupant inspection confirmation token must not be blank",
                ));
            }
            if !has_expires_at
                || !is_manager_rfc3339(expires_at)
                || DateTime::parse_from_rfc3339(expires_at).is_err()
            {
                return Err(D::Error::custom(
                    "occupant inspection expiry must be RFC3339",
                ));
            }
        } else if has_confirmation_token || has_expires_at {
            return Err(D::Error::custom(
                "blocked occupant inspection cannot include confirmation fields",
            ));
        }

        if has_supervisor && wire.supervisor.is_none() {
            return Err(D::Error::custom(
                "occupant inspection supervisor must be an object",
            ));
        }
        if let Some(supervisor) = wire.supervisor.as_ref() {
            if wire.recovery.reason != Some(RecoveryReason::ServiceManaged)
                || !valid_supervisor(supervisor)
            {
                return Err(D::Error::custom(
                    "occupant inspection supervisor metadata is invalid",
                ));
            }
        }

        Ok(Self {
            pid: wire.pid,
            verification_mode: wire.verification_mode,
            process_name: wire.process_name,
            executable: wire.executable,
            listen_addr: wire.listen_addr,
            recovery: wire.recovery,
            supervisor: wire.supervisor,
            confirmation_token: wire.confirmation_token,
            expires_at: wire.expires_at,
        })
    }
}

#[cfg(test)]
mod occupant_inspection_tests {
    use super::{
        OccupantInspection, OccupantTerminationResult, OccupantVerificationMode, RecoveryAction,
        RecoveryReason, SupervisorKind,
    };
    use serde_json::json;

    fn forceable() -> serde_json::Value {
        json!({
            "pid": 7,
            "verification_mode": "windows_pid_only",
            "listen_addr": "127.0.0.1:19099",
            "recovery": {"action": "force_terminate"},
            "confirmation_token": "token",
            "expires_at": "2026-07-25T00:00:30Z"
        })
    }

    fn blocked(action: &str, reason: &str) -> serde_json::Value {
        json!({
            "pid": 8,
            "verification_mode": "windows_pid_only",
            "listen_addr": "127.0.0.1:19099",
            "recovery": {"action": action, "reason": reason}
        })
    }

    #[test]
    fn deserializes_every_valid_recovery_pair() {
        let cases = [
            ("force_terminate", None),
            ("manual_stop_required", Some("service_managed")),
            ("manual_stop_required", Some("insufficient_privilege")),
            ("manual_stop_required", Some("different_user")),
            ("unavailable", Some("protected_process")),
            ("unavailable", Some("identity_unavailable")),
        ];
        for (action, reason) in cases {
            let shape = if let Some(reason) = reason {
                blocked(action, reason)
            } else {
                forceable()
            };
            let inspection: OccupantInspection = serde_json::from_value(shape).unwrap();
            assert_eq!(inspection.recovery.action.to_string(), action);
        }
    }

    #[test]
    fn deserializes_verified_identity_shape() {
        let shape = json!({
            "pid": 4242,
            "verification_mode": "verified_identity",
            "process_name": "example-server",
            "executable": "C:\\example-server.exe",
            "listen_addr": "127.0.0.1:19099",
            "recovery": {"action": "force_terminate"},
            "confirmation_token": "verified-token",
            "expires_at": "2026-07-22T12:00:30Z"
        });
        let inspection: OccupantInspection = serde_json::from_value(shape.clone()).unwrap();

        assert_eq!(
            inspection.verification_mode,
            OccupantVerificationMode::VerifiedIdentity
        );
        assert_eq!(inspection.process_name.as_deref(), Some("example-server"));
        assert_eq!(
            inspection.executable.as_deref(),
            Some("C:\\example-server.exe")
        );
        assert_eq!(serde_json::to_value(inspection).unwrap(), shape);
    }

    #[test]
    fn deserializes_service_supervisors_with_exact_kind_scope_pairs() {
        let cases = [
            ("windows_service", "system", "Router Service"),
            ("systemd_user", "user", r"demo\x2dworker@blue.service"),
            ("systemd_system", "system", "demo.service"),
        ];
        for (kind, scope, identifier) in cases {
            let mut shape = blocked("manual_stop_required", "service_managed");
            shape.as_object_mut().unwrap().insert(
                "supervisor".to_owned(),
                json!({"kind": kind, "scope": scope, "identifiers": [identifier]}),
            );
            let inspection: OccupantInspection = serde_json::from_value(shape).unwrap();
            assert_eq!(inspection.supervisor.unwrap().identifiers, [identifier]);
        }
    }

    #[test]
    fn deserializes_windows_pid_only_shape_without_metadata() {
        let shape = forceable();
        let inspection: OccupantInspection = serde_json::from_value(shape.clone()).unwrap();

        assert_eq!(
            inspection.verification_mode,
            OccupantVerificationMode::WindowsPidOnly
        );
        assert_eq!(inspection.process_name, None);
        assert_eq!(inspection.executable, None);
        assert_eq!(serde_json::to_value(inspection).unwrap(), shape);
    }

    #[test]
    fn rejects_unknown_verification_mode() {
        let mut shape = forceable();
        shape["verification_mode"] = json!("unknown");
        assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
    }

    #[test]
    fn rejects_unknown_fields_at_every_protocol_level() {
        let mut top = forceable();
        top["extra"] = json!(true);
        let mut recovery = forceable();
        recovery["recovery"]["extra"] = json!(true);
        let mut supervisor = blocked("manual_stop_required", "service_managed");
        supervisor["supervisor"] = json!({
            "kind": "windows_service", "scope": "system", "identifiers": ["Svc"], "extra": true
        });
        let termination = json!({
            "termination": "process_terminated", "port_state": "released", "extra": true
        });

        for shape in [top, recovery, supervisor] {
            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
        assert!(serde_json::from_value::<OccupantTerminationResult>(termination).is_err());
    }

    #[test]
    fn rejects_invalid_base_and_metadata_fields() {
        for invalid_field in [
            json!({"pid": 0}),
            json!({"pid": 4_294_967_296_u64}),
            json!({"listen_addr": "127.0.0.1:19100"}),
            json!({"listen_addr": " "}),
            json!({"confirmation_token": " "}),
            json!({"expires_at": " "}),
            json!({"expires_at": "not-a-timestamp"}),
            json!({"expires_at": "2026-07-22t12:00:30Z"}),
            json!({"expires_at": "2026-07-22T12:00:30z"}),
            json!({"expires_at": "2026-07-22T12:00:60Z"}),
        ] {
            let mut shape = json!({
                "pid": 4242,
                "verification_mode": "windows_pid_only",
                "listen_addr": "127.0.0.1:19099",
                "recovery": {"action": "force_terminate"},
                "confirmation_token": "token",
                "expires_at": "2026-07-22T12:00:30+08:00"
            });
            shape.as_object_mut().unwrap().extend(
                invalid_field
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );

            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
    }

    #[test]
    fn accepts_manager_timestamp_forms() {
        for expires_at in ["2026-07-22T12:00:30Z", "2026-07-22T20:00:30+08:00"] {
            let mut shape = forceable();
            shape["expires_at"] = json!(expires_at);
            let result = serde_json::from_value::<OccupantInspection>(shape);

            assert!(result.is_ok(), "expected {expires_at} to be accepted");
        }
    }

    #[test]
    fn rejects_invalid_verified_identity_metadata() {
        for metadata in [
            json!({"executable": "C:\\example-server.exe"}),
            json!({"process_name": "example-server"}),
            json!({"process_name": "", "executable": "C:\\example-server.exe"}),
            json!({"process_name": "example-server", "executable": "  "}),
        ] {
            let mut shape = json!({
                "pid": 4242,
                "verification_mode": "verified_identity",
                "listen_addr": "127.0.0.1:19099",
                "recovery": {"action": "force_terminate"},
                "confirmation_token": "token",
                "expires_at": "2026-07-22T12:00:30Z"
            });
            shape.as_object_mut().unwrap().extend(
                metadata
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );

            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }

        for mut shape in [
            forceable(),
            blocked("manual_stop_required", "service_managed"),
            blocked("manual_stop_required", "insufficient_privilege"),
            blocked("unavailable", "protected_process"),
            blocked("unavailable", "identity_unavailable"),
        ] {
            shape["verification_mode"] = json!("verified_identity");
            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
    }

    #[test]
    fn accepts_redacted_verified_identity_only_for_different_user() {
        let mut shape = blocked("manual_stop_required", "different_user");
        shape["verification_mode"] = json!("verified_identity");
        let inspection: OccupantInspection = serde_json::from_value(shape.clone()).unwrap();

        assert_eq!(inspection.process_name, None);
        assert_eq!(inspection.executable, None);
        assert_eq!(serde_json::to_value(inspection).unwrap(), shape);

        for metadata in [
            json!({"process_name": "example-server"}),
            json!({"executable": "C:\\example-server.exe"}),
        ] {
            let mut partial = shape.clone();
            partial.as_object_mut().unwrap().extend(
                metadata
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
            assert!(serde_json::from_value::<OccupantInspection>(partial).is_err());
        }
    }

    #[test]
    fn rejects_windows_pid_only_with_process_metadata() {
        for metadata in [
            json!({"process_name": "example-server"}),
            json!({"executable": "C:\\example-server.exe"}),
            json!({"process_name": "", "executable": ""}),
        ] {
            let mut shape = json!({
                "pid": 4242,
                "verification_mode": "windows_pid_only",
                "listen_addr": "127.0.0.1:19099",
                "recovery": {"action": "force_terminate"},
                "confirmation_token": "token",
                "expires_at": "2026-07-22T12:00:30Z"
            });
            shape.as_object_mut().unwrap().extend(
                metadata
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );

            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
    }

    #[test]
    fn rejects_invalid_recovery_matrix_and_token_presence() {
        let invalid = [
            json!({"action": "force_terminate", "reason": "different_user"}),
            json!({"action": "manual_stop_required"}),
            json!({"action": "manual_stop_required", "reason": "protected_process"}),
            json!({"action": "unavailable", "reason": "service_managed"}),
            json!({"action": "unknown"}),
            json!({"action": "unavailable", "reason": "unknown"}),
        ];
        for recovery in invalid {
            let mut shape = blocked("unavailable", "identity_unavailable");
            shape["recovery"] = recovery;
            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }

        for field in ["confirmation_token", "expires_at"] {
            let mut shape = forceable();
            shape.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
        for value in [json!(null), json!("token")] {
            let mut shape = blocked("manual_stop_required", "different_user");
            shape["confirmation_token"] = value;
            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
        let mut shape = blocked("unavailable", "identity_unavailable");
        shape["expires_at"] = json!("2026-07-25T00:00:30Z");
        assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
    }

    #[test]
    fn rejects_invalid_supervisor_shape_and_bounds() {
        let seventeen: Vec<String> = (0..17).map(|index| format!("Svc{index:02}")).collect();
        let over_limit: Vec<String> = (0..16)
            .map(|index| format!("{index:02}{}", "x".repeat(254)))
            .collect();
        let invalid = [
            json!({"kind": "windows_service", "scope": "user", "identifiers": ["Svc"]}),
            json!({"kind": "systemd_user", "scope": "system", "identifiers": ["a.service"]}),
            json!({"kind": "systemd_system", "scope": "unknown", "identifiers": ["a.service"]}),
            json!({"kind": "unknown", "scope": "system", "identifiers": ["Svc"]}),
            json!({"kind": "windows_service", "scope": "system", "identifiers": []}),
            json!({"kind": "windows_service", "scope": "system", "identifiers": seventeen}),
            json!({"kind": "windows_service", "scope": "system", "identifiers": ["Beta", "Alpha"]}),
            json!({"kind": "windows_service", "scope": "system", "identifiers": ["Alpha", "Alpha"]}),
            json!({"kind": "windows_service", "scope": "system", "identifiers": ["é".repeat(129)]}),
            json!({"kind": "windows_service", "scope": "system", "identifiers": over_limit}),
        ];
        for supervisor in invalid {
            let mut shape = blocked("manual_stop_required", "service_managed");
            shape["supervisor"] = supervisor;
            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }

        let mut shape = blocked("manual_stop_required", "different_user");
        shape["supervisor"] =
            json!({"kind": "windows_service", "scope": "system", "identifiers": ["Svc"]});
        assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
    }

    #[test]
    fn rejects_unsafe_windows_and_noncanonical_systemd_identifiers() {
        for identifier in [
            " ",
            "Svc/name",
            r"Svc\name",
            "Svc,name",
            "Svc\"name",
            "Svc\nname",
        ] {
            let mut shape = blocked("manual_stop_required", "service_managed");
            shape["supervisor"] =
                json!({"kind": "windows_service", "scope": "system", "identifiers": [identifier]});
            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
        for identifier in [
            "@demo.service",
            "a@b@c.service",
            r"demo\q20.service",
            r"demo\x2.service",
            "demo scope.service",
            &("x".repeat(248) + ".service"),
        ] {
            let mut shape = blocked("manual_stop_required", "service_managed");
            shape["supervisor"] =
                json!({"kind": "systemd_system", "scope": "system", "identifiers": [identifier]});
            assert!(serde_json::from_value::<OccupantInspection>(shape).is_err());
        }
    }

    #[test]
    fn termination_accepts_only_exact_success_values() {
        let valid = json!({"termination": "process_terminated", "port_state": "released"});
        let result: OccupantTerminationResult = serde_json::from_value(valid.clone()).unwrap();
        assert_eq!(serde_json::to_value(result).unwrap(), valid);
        for invalid in [
            json!({"termination": "terminated", "port_state": "released"}),
            json!({"termination": "process_terminated", "port_state": "occupied"}),
        ] {
            assert!(serde_json::from_value::<OccupantTerminationResult>(invalid).is_err());
        }
    }

    #[test]
    fn public_enums_are_closed() {
        assert_eq!(
            RecoveryAction::ForceTerminate.to_string(),
            "force_terminate"
        );
        assert_eq!(
            RecoveryReason::ServiceManaged.to_string(),
            "service_managed"
        );
        assert_eq!(
            SupervisorKind::WindowsService.to_string(),
            "windows_service"
        );
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Diagnostics {
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ManagerInfo {
    pub version: String,
    pub commit: String,
    pub build_date: String,
    pub target: String,
    pub deployment_id: String,
    pub management_protocol_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ComponentVersions {
    pub desktop: String,
    pub manager: String,
    pub router: String,
    pub management_protocol: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentRecoveryFileState {
    pub role: String,
    pub path: String,
    pub format: String,
    pub exists: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentRecoveryState {
    pub eligible: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
    pub files: Vec<AgentRecoveryFileState>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentState {
    pub agent: String,
    pub name: String,
    pub detected: bool,
    #[serde(default)]
    pub command: String,
    pub path: String,
    #[serde(default)]
    pub auth_path: String,
    pub format: String,
    pub exists: bool,
    pub writable: bool,
    pub configured: bool,
    pub invalid: bool,
    #[serde(default)]
    pub migratable: bool,
    pub recovery: AgentRecoveryState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentDetect {
    pub agents: Vec<AgentState>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentFragment {
    pub agent: String,
    pub role: String,
    pub path: String,
    pub format: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentFileEffect {
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub mode: String,
    pub path: String,
    pub role: String,
    pub format: String,
    pub operation: String,
    #[serde(default)]
    pub backup_path: String,
    #[serde(default)]
    pub backup_required: bool,
    #[serde(default)]
    pub backup_pattern: String,
    #[serde(default)]
    pub backup_sensitive: bool,
    #[serde(default)]
    pub preserves: Vec<String>,
    #[serde(default)]
    pub warning: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ManagedCollision {
    pub agent: String,
    pub path: String,
    #[serde(rename = "type")]
    pub collision_type: String,
    pub action: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentPreview {
    pub revision_token: String,
    pub model_config: Value,
    pub fragments: Vec<AgentFragment>,
    #[serde(default)]
    pub files: Vec<AgentFileEffect>,
    pub managed_config_drift: bool,
    #[serde(default)]
    pub drifted_agents: Vec<String>,
    #[serde(default)]
    pub managed_collisions: Vec<ManagedCollision>,
    pub requires_codex_auth_approval: bool,
    pub state_change: Option<AgentFileEffect>,
    pub state_backup: Option<AgentFileEffect>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentWriteResult {
    pub transaction_id: String,
    pub agents: Vec<AgentWriteStatus>,
    pub state_change: Option<AgentFileEffect>,
    pub state_backup: Option<AgentFileEffect>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentWriteStatus {
    pub agent: String,
    pub success: bool,
    #[serde(default)]
    pub changed: Vec<String>,
    #[serde(default)]
    pub backups: Vec<String>,
    #[serde(default)]
    pub error_code: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CredentialSummary {
    pub present: bool,
    pub fingerprint: String,
    pub saved_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DesktopPaths {
    pub data_dir: String,
    pub log_file: String,
    pub credentials_path: String,
    pub can_prepare_for_uninstall: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_language_accepts_only_supported_values() {
        assert_eq!(NativeLanguage::parse("zh-CN"), Some(NativeLanguage::ZhCn));
        assert_eq!(NativeLanguage::parse("en"), Some(NativeLanguage::En));
        assert_eq!(NativeLanguage::parse("fr"), None);
        assert_eq!(NativeLanguage::parse(""), None);
    }

    #[test]
    fn native_language_defaults_to_chinese() {
        assert_eq!(NativeLanguage::default(), NativeLanguage::ZhCn);
    }

    #[test]
    fn router_status_deserializes_bounded_failure_logs() {
        let status: RouterStatus = serde_json::from_value(serde_json::json!({
            "state": "start_failed",
            "owner": "desktop",
            "last_error": "desktop-owned router exited unexpectedly",
            "recent_logs": ["safe line", "api_key=[REDACTED]"]
        }))
        .unwrap();

        assert_eq!(status.state, "start_failed");
        assert_eq!(
            status.recent_logs,
            Some(vec![
                "safe line".to_owned(),
                "api_key=[REDACTED]".to_owned()
            ])
        );
    }

    #[test]
    fn agent_recovery_and_rebuild_preview_fields_survive_serde() {
        let detection_json = serde_json::json!({
            "agents": [{
                "agent": "claude",
                "name": "Claude Code",
                "detected": true,
                "path": "/home/example/.claude/settings.json",
                "format": "json",
                "exists": true,
                "writable": false,
                "configured": false,
                "invalid": true,
                "migratable": false,
                "recovery": {
                    "eligible": true,
                    "reasons": ["syntax_invalid"],
                    "files": [{
                        "role": "config",
                        "path": "/home/example/.claude/settings.json",
                        "format": "json",
                        "exists": true,
                        "reasons": ["syntax_invalid"]
                    }]
                }
            }]
        });
        let detection: AgentDetect = serde_json::from_value(detection_json).unwrap();
        let detection_value = serde_json::to_value(detection).unwrap();
        assert_eq!(detection_value["agents"][0]["migratable"], false);
        assert_eq!(detection_value["agents"][0]["recovery"]["eligible"], true);
        assert_eq!(
            detection_value["agents"][0]["recovery"]["reasons"],
            serde_json::json!(["syntax_invalid"])
        );
        assert_eq!(
            detection_value["agents"][0]["recovery"]["files"][0],
            serde_json::json!({
                "role": "config",
                "path": "/home/example/.claude/settings.json",
                "format": "json",
                "exists": true,
                "reasons": ["syntax_invalid"]
            })
        );

        let preview_json = serde_json::json!({
            "revision_token": "revision-1",
            "model_config": {"version": 1},
            "fragments": [],
            "files": [{
                "agent": "claude",
                "mode": "rebuild",
                "path": "/home/example/.claude/settings.json",
                "role": "config",
                "format": "json",
                "operation": "replace",
                "backup_required": true,
                "backup_pattern": "/home/example/.claude/settings.json.bak.*",
                "backup_sensitive": true,
                "preserves": ["unrelated files"],
                "warning": "Existing invalid configuration will be rebuilt"
            }],
            "managed_config_drift": false,
            "drifted_agents": [],
            "managed_collisions": [],
            "requires_codex_auth_approval": false,
            "state_change": null,
            "state_backup": null
        });
        let preview: AgentPreview = serde_json::from_value(preview_json).unwrap();
        let preview_value = serde_json::to_value(preview).unwrap();
        assert_eq!(
            preview_value["files"][0],
            serde_json::json!({
                "agent": "claude",
                "mode": "rebuild",
                "path": "/home/example/.claude/settings.json",
                "role": "config",
                "format": "json",
                "operation": "replace",
                "backup_path": "",
                "backup_required": true,
                "backup_pattern": "/home/example/.claude/settings.json.bak.*",
                "backup_sensitive": true,
                "preserves": ["unrelated files"],
                "warning": "Existing invalid configuration will be rebuilt"
            })
        );
    }
}
