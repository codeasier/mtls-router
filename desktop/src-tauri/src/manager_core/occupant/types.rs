use serde::{Deserialize, Serialize};

use super::super::process::Identity as ProcessIdentity;

pub const MAX_SUPERVISOR_IDENTIFIERS: usize = 16;
pub const MAX_SUPERVISOR_IDENTIFIER_SIZE: usize = 256;
pub const MAX_SUPERVISOR_SIZE: usize = 4 * 1024;
pub const MAX_SYSTEMD_UNIT_SIZE: usize = 255;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OccupantError {
    NotFound,
    NotOwned,
    IdentityUnavailable,
    Changed,
    Protected,
    PermissionDenied,
    TerminationFailed,
    PortReleaseTimeout,
    ConfirmationExpired,
}

impl OccupantError {
    pub fn as_protocol_code(self) -> crate::protocol::ErrorCode {
        use crate::protocol::ErrorCode;
        match self {
            Self::NotFound => ErrorCode::OccupantNotFound,
            Self::NotOwned => ErrorCode::OccupantNotOwned,
            Self::IdentityUnavailable => ErrorCode::OccupantIdentityUnavailable,
            Self::Changed => ErrorCode::OccupantChanged,
            Self::Protected => ErrorCode::OccupantProtected,
            Self::PermissionDenied => ErrorCode::OccupantPermissionDenied,
            Self::TerminationFailed => ErrorCode::OccupantTerminationFailed,
            Self::PortReleaseTimeout => ErrorCode::PortReleaseTimeout,
            Self::ConfirmationExpired => ErrorCode::ConfirmationExpired,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Identity {
    pub listen_addr: String,
    pub network: String,
    pub socket_id: String,
    pub process: ProcessIdentity,
    pub user_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMode {
    VerifiedIdentity,
    WindowsPidOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    ForceTerminate,
    ManualStopRequired,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryReason {
    ServiceManaged,
    InsufficientPrivilege,
    DifferentUser,
    ProtectedProcess,
    IdentityUnavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorKind {
    WindowsService,
    SystemdUser,
    SystemdSystem,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorScope {
    User,
    System,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Recovery {
    pub action: RecoveryAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<RecoveryReason>,
}

impl Default for RecoveryAction {
    fn default() -> Self {
        Self::Unavailable
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Supervisor {
    pub kind: SupervisorKind,
    pub scope: SupervisorScope,
    pub identifiers: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Target {
    pub mode: Option<VerificationMode>,
    pub identity: Identity,
    pub pid: i32,
    pub listen_addr: String,
    pub supervisor: Option<Supervisor>,
    pub block_reason: Option<RecoveryReason>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Inspection {
    pub pid: i32,
    pub verification_mode: Option<VerificationMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    pub listen_addr: String,
    pub recovery: Recovery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisor: Option<Supervisor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmation_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TerminateResult {
    pub termination: String,
    pub port_state: String,
}

pub fn released_result() -> TerminateResult {
    TerminateResult {
        termination: "process_terminated".into(),
        port_state: "released".into(),
    }
}

pub fn valid_supervisor(supervisor: &Supervisor) -> bool {
    if supervisor.identifiers.is_empty()
        || supervisor.identifiers.len() > MAX_SUPERVISOR_IDENTIFIERS
    {
        return false;
    }
    match (supervisor.kind, supervisor.scope) {
        (SupervisorKind::WindowsService, SupervisorScope::System)
        | (SupervisorKind::SystemdUser, SupervisorScope::User)
        | (SupervisorKind::SystemdSystem, SupervisorScope::System) => {}
        _ => return false,
    }
    for (index, identifier) in supervisor.identifiers.iter().enumerate() {
        if identifier.is_empty()
            || identifier.len() > MAX_SUPERVISOR_IDENTIFIER_SIZE
            || (index > 0 && supervisor.identifiers[index - 1] >= *identifier)
        {
            return false;
        }
        let valid = match supervisor.kind {
            SupervisorKind::WindowsService => valid_windows_service_name(identifier),
            SupervisorKind::SystemdUser | SupervisorKind::SystemdSystem => {
                valid_systemd_unit(identifier, ".service")
            }
        };
        if !valid {
            return false;
        }
    }
    serde_json::to_vec(supervisor).is_ok_and(|encoded| encoded.len() <= MAX_SUPERVISOR_SIZE)
}

pub fn valid_windows_service_name(identifier: &str) -> bool {
    !identifier.trim().is_empty()
        && !identifier
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | ',' | '"'))
}

pub fn valid_systemd_unit(unit: &str, suffix: &str) -> bool {
    if unit.len() > MAX_SYSTEMD_UNIT_SIZE || !unit.ends_with(suffix) {
        return false;
    }
    let stem = &unit[..unit.len() - suffix.len()];
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

pub fn recovery_for_reason(reason: RecoveryReason) -> Option<Recovery> {
    Some(match reason {
        RecoveryReason::ServiceManaged
        | RecoveryReason::InsufficientPrivilege
        | RecoveryReason::DifferentUser => Recovery {
            action: RecoveryAction::ManualStopRequired,
            reason: Some(reason),
        },
        RecoveryReason::ProtectedProcess | RecoveryReason::IdentityUnavailable => Recovery {
            action: RecoveryAction::Unavailable,
            reason: Some(reason),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_supervisor_identifier_grammar() {
        let cases = [
            (
                SupervisorKind::WindowsService,
                SupervisorScope::System,
                "Router Service",
                true,
            ),
            (
                SupervisorKind::WindowsService,
                SupervisorScope::System,
                "Svc<>&",
                true,
            ),
            (
                SupervisorKind::WindowsService,
                SupervisorScope::System,
                " ",
                false,
            ),
            (
                SupervisorKind::WindowsService,
                SupervisorScope::System,
                "Svc/name",
                false,
            ),
            (
                SupervisorKind::WindowsService,
                SupervisorScope::System,
                r"Svc\name",
                false,
            ),
            (
                SupervisorKind::WindowsService,
                SupervisorScope::System,
                "Svc,name",
                false,
            ),
            (
                SupervisorKind::WindowsService,
                SupervisorScope::System,
                "Svc\"name",
                false,
            ),
            (
                SupervisorKind::WindowsService,
                SupervisorScope::System,
                "Svc\nname",
                false,
            ),
            (
                SupervisorKind::SystemdUser,
                SupervisorScope::User,
                r"demo\x2dworker@blue.service",
                true,
            ),
            (
                SupervisorKind::SystemdSystem,
                SupervisorScope::System,
                &format!("{}{}", "x".repeat(247), ".service"),
                true,
            ),
            (
                SupervisorKind::SystemdSystem,
                SupervisorScope::System,
                "worker@.service",
                true,
            ),
            (
                SupervisorKind::SystemdSystem,
                SupervisorScope::System,
                "@demo.service",
                false,
            ),
            (
                SupervisorKind::SystemdSystem,
                SupervisorScope::System,
                "a@b@c.service",
                false,
            ),
            (
                SupervisorKind::SystemdSystem,
                SupervisorScope::System,
                r"demo\q20.service",
                false,
            ),
            (
                SupervisorKind::SystemdSystem,
                SupervisorScope::System,
                r"demo\x2.service",
                false,
            ),
            (
                SupervisorKind::SystemdSystem,
                SupervisorScope::System,
                "demo scope.service",
                false,
            ),
            (
                SupervisorKind::SystemdSystem,
                SupervisorScope::System,
                "demo<>&.service",
                false,
            ),
            (
                SupervisorKind::SystemdSystem,
                SupervisorScope::System,
                &format!("{}{}", "x".repeat(248), ".service"),
                false,
            ),
        ];
        for (kind, scope, identifier, want) in cases {
            let supervisor = Supervisor {
                kind,
                scope,
                identifiers: vec![identifier.to_owned()],
            };
            assert_eq!(valid_supervisor(&supervisor), want, "{identifier:?}");
        }
    }

    #[test]
    fn valid_supervisor_size_does_not_html_escape_windows_identifiers() {
        let identifiers = (0..16)
            .map(|index| format!("{index:02}{}", "<>&".repeat(60)))
            .collect();
        assert!(valid_supervisor(&Supervisor {
            kind: SupervisorKind::WindowsService,
            scope: SupervisorScope::System,
            identifiers,
        }));
    }
}
