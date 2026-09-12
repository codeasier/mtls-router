use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{TimeZone, Utc};

use super::super::process::{Identity as ProcessIdentity, Status};
use super::super::types::Classification;
use super::service::{
    fill_random_from_reader, CallContext, OccupantConfig, OccupantDependencies, OccupantService,
};
use super::types::{
    Identity, OccupantError, RecoveryAction, RecoveryReason, Supervisor, SupervisorKind,
    SupervisorScope, Target, VerificationMode,
};

fn test_identity() -> Identity {
    Identity {
        listen_addr: "127.0.0.1:19099".into(),
        network: "tcp4".into(),
        socket_id: "socket".into(),
        process: ProcessIdentity {
            pid: 42,
            started_at: "start".into(),
            executable: "/tmp/listener".into(),
        },
        user_id: "user".into(),
    }
}

fn verified_target(identity: Identity) -> Target {
    Target {
        mode: Some(VerificationMode::VerifiedIdentity),
        pid: identity.process.pid,
        listen_addr: identity.listen_addr.clone(),
        identity,
        supervisor: None,
        block_reason: None,
    }
}

fn pid_only_target() -> Target {
    Target {
        mode: Some(VerificationMode::WindowsPidOnly),
        pid: 4242,
        listen_addr: "127.0.0.1:19099".into(),
        ..Target::default()
    }
}

fn unknown_discover() -> OccupantDependencies {
    let mut deps = OccupantDependencies::default();
    deps.discover = Some(Box::new(|_| Classification::UnknownOccupant));
    deps
}

fn service_with(config: OccupantConfig, mut deps: OccupantDependencies) -> OccupantService {
    if deps.discover.is_none() {
        deps.discover = Some(Box::new(|_| Classification::UnknownOccupant));
    }
    OccupantService::new(config, deps)
}

fn fixed_now() -> OccupantDependencies {
    let mut deps = unknown_discover();
    deps.now = Some(Box::new(|| {
        Utc.with_ymd_and_hms(2026, 7, 18, 1, 2, 3).unwrap()
    }));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([7u8; 64]), buffer)
    }));
    deps
}

#[test]
fn inspection_token_and_termination() {
    let identity = test_identity();
    let status = Arc::new(Mutex::new(Status::Genuine));
    let signals = Arc::new(AtomicUsize::new(0));
    let mut deps = fixed_now();
    let inspect_identity = identity.clone();
    deps.inspect = Some(Box::new(move |_, _| {
        Ok(verified_target(inspect_identity.clone()))
    }));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.same_process = Some(Box::new(|left, right| Ok(left == right)));
    let status_validate = status.clone();
    deps.validate = Some(Box::new(move |_, _| Ok(*status_validate.lock().unwrap())));
    let signals_signal = signals.clone();
    let status_signal = status.clone();
    deps.signal = Some(Box::new(move |got| {
        assert_eq!(got.pid, 42);
        signals_signal.fetch_add(1, Ordering::SeqCst);
        *status_signal.lock().unwrap() = Status::Absent;
        Ok(())
    }));
    deps.dial = Some(Box::new(|_, _, _| Err(OccupantError::NotFound)));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: identity.listen_addr.clone(),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    assert_eq!(inspection.confirmation_token.as_ref().unwrap().len(), 43);
    assert_eq!(
        inspection.expires_at.as_deref(),
        Some("2026-07-18T01:02:33Z")
    );
    assert_eq!(inspection.process_name.as_deref(), Some("listener"));
    assert_eq!(
        inspection.verification_mode,
        Some(VerificationMode::VerifiedIdentity)
    );
    let result = service
        .force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap(),
        )
        .unwrap();
    assert_eq!(result.termination, "process_terminated");
    assert_eq!(result.port_state, "released");
    assert_eq!(signals.load(Ordering::SeqCst), 1);
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::ConfirmationExpired)
    );
    assert_eq!(signals.load(Ordering::SeqCst), 1);
}

#[test]
fn inspect_recovery_actions() {
    let identity = test_identity();
    let verified = verified_target(identity.clone());
    let mut service_target = verified.clone();
    service_target.supervisor = Some(Supervisor {
        kind: SupervisorKind::WindowsService,
        scope: SupervisorScope::System,
        identifiers: vec!["RouterSvc".into()],
    });
    let pid_only = pid_only_target();
    let mut pid_only_service = pid_only.clone();
    pid_only_service.supervisor = Some(Supervisor {
        kind: SupervisorKind::WindowsService,
        scope: SupervisorScope::System,
        identifiers: vec!["RouterSvc".into()],
    });
    pid_only_service.block_reason = Some(RecoveryReason::ServiceManaged);
    let mut session_zero = pid_only.clone();
    session_zero.block_reason = Some(RecoveryReason::IdentityUnavailable);
    let mut permission = pid_only.clone();
    permission.block_reason = Some(RecoveryReason::InsufficientPrivilege);
    let mut different_user = verified.clone();
    different_user.block_reason = Some(RecoveryReason::DifferentUser);
    let mut protected = verified.clone();
    protected.block_reason = Some(RecoveryReason::ProtectedProcess);

    let cases = [
        (
            "verified current user",
            verified,
            RecoveryAction::ForceTerminate,
            None,
            true,
            true,
        ),
        (
            "service",
            service_target,
            RecoveryAction::ManualStopRequired,
            Some(RecoveryReason::ServiceManaged),
            false,
            true,
        ),
        (
            "PID-only SCM service",
            pid_only_service,
            RecoveryAction::ManualStopRequired,
            Some(RecoveryReason::ServiceManaged),
            false,
            false,
        ),
        (
            "unnamed Session 0 target",
            session_zero,
            RecoveryAction::Unavailable,
            Some(RecoveryReason::IdentityUnavailable),
            false,
            false,
        ),
        (
            "permission denied",
            permission,
            RecoveryAction::ManualStopRequired,
            Some(RecoveryReason::InsufficientPrivilege),
            false,
            false,
        ),
        (
            "different user",
            different_user,
            RecoveryAction::ManualStopRequired,
            Some(RecoveryReason::DifferentUser),
            false,
            false,
        ),
        (
            "protected",
            protected,
            RecoveryAction::Unavailable,
            Some(RecoveryReason::ProtectedProcess),
            false,
            true,
        ),
    ];
    for (name, target, action, reason, want_token, want_metadata) in cases {
        let mut deps = unknown_discover();
        let inspect_target = target.clone();
        deps.inspect = Some(Box::new(move |_, _| Ok(inspect_target.clone())));
        deps.supports_pid_only = Some(Box::new(|| true));
        deps.current_user = Some(Box::new(|| Ok("user".into())));
        deps.random = Some(Box::new(|buffer| {
            fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
        }));
        let service = OccupantService::new(
            OccupantConfig {
                listen_addr: identity.listen_addr.clone(),
                ..OccupantConfig::default()
            },
            deps,
        );
        let inspection = service.inspect(&CallContext::unbounded()).unwrap();
        assert_eq!(inspection.recovery.action, action, "{name}");
        assert_eq!(inspection.recovery.reason, reason, "{name}");
        assert_eq!(
            inspection.confirmation_token.is_some(),
            want_token,
            "{name}"
        );
        assert_eq!(inspection.expires_at.is_some(), want_token, "{name}");
        let has_metadata = inspection.process_name.is_some() || inspection.executable.is_some();
        assert_eq!(has_metadata, want_metadata, "{name}");
    }
}

#[test]
fn inspect_classifies_systemd_supervisor_after_ownership() {
    let mut identity = test_identity();
    identity.user_id = "0".into();
    let mut target = verified_target(identity.clone());
    target.supervisor = Some(Supervisor {
        kind: SupervisorKind::SystemdSystem,
        scope: SupervisorScope::System,
        identifiers: vec!["router.service".into()],
    });
    for (name, current_user, reason, want_supervisor, want_metadata) in [
        (
            "same user remains service managed",
            "0",
            RecoveryReason::ServiceManaged,
            true,
            true,
        ),
        (
            "root service under different user redacts metadata",
            "1001",
            RecoveryReason::DifferentUser,
            false,
            false,
        ),
    ] {
        let mut deps = unknown_discover();
        let inspect_target = target.clone();
        deps.inspect = Some(Box::new(move |_, _| Ok(inspect_target.clone())));
        deps.current_user = Some(Box::new(move || Ok(current_user.to_owned())));
        deps.random = Some(Box::new(|buffer| {
            fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
        }));
        let service = OccupantService::new(
            OccupantConfig {
                listen_addr: identity.listen_addr.clone(),
                ..OccupantConfig::default()
            },
            deps,
        );
        let inspection = service.inspect(&CallContext::unbounded()).unwrap();
        assert_eq!(
            inspection.recovery.action,
            RecoveryAction::ManualStopRequired,
            "{name}"
        );
        assert_eq!(inspection.recovery.reason, Some(reason), "{name}");
        assert_eq!(inspection.supervisor.is_some(), want_supervisor, "{name}");
        let has_metadata = inspection.process_name.is_some() || inspection.executable.is_some();
        assert_eq!(has_metadata, want_metadata, "{name}");
    }
}

#[test]
fn inspect_rejects_malformed_supervisor_metadata() {
    let identity = test_identity();
    let seventeen: Vec<String> = (0..17).map(|index| format!("Service{index:02}")).collect();
    let large: Vec<String> = (0..16)
        .map(|index| format!("{index:02}{}", "x".repeat(254)))
        .collect();
    let cases = [
        Supervisor {
            kind: SupervisorKind::WindowsService,
            scope: SupervisorScope::User,
            identifiers: vec!["Alpha".into()],
        },
        Supervisor {
            kind: SupervisorKind::WindowsService,
            scope: SupervisorScope::System,
            identifiers: vec!["Beta".into(), "Alpha".into()],
        },
        Supervisor {
            kind: SupervisorKind::WindowsService,
            scope: SupervisorScope::System,
            identifiers: vec!["Alpha".into(), "Alpha".into()],
        },
        Supervisor {
            kind: SupervisorKind::WindowsService,
            scope: SupervisorScope::System,
            identifiers: seventeen,
        },
        Supervisor {
            kind: SupervisorKind::WindowsService,
            scope: SupervisorScope::System,
            identifiers: vec!["x".repeat(257)],
        },
        Supervisor {
            kind: SupervisorKind::WindowsService,
            scope: SupervisorScope::System,
            identifiers: large,
        },
    ];
    for supervisor in cases {
        let mut target = verified_target(identity.clone());
        target.supervisor = Some(supervisor);
        let mut deps = unknown_discover();
        deps.inspect = Some(Box::new(move |_, _| Ok(target.clone())));
        deps.current_user = Some(Box::new(|| Ok("user".into())));
        deps.random = Some(Box::new(|buffer| {
            fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
        }));
        let service = OccupantService::new(
            OccupantConfig {
                listen_addr: identity.listen_addr.clone(),
                ..OccupantConfig::default()
            },
            deps,
        );
        let inspection = service.inspect(&CallContext::unbounded()).unwrap();
        let encoded = serde_json::to_string(&inspection).unwrap();
        assert!(inspection.supervisor.is_none(), "{encoded}");
        assert_eq!(
            inspection.recovery.reason,
            Some(RecoveryReason::IdentityUnavailable)
        );
        assert!(inspection.confirmation_token.is_none());
        assert!(!encoded.contains("\"supervisor\""));
    }
}

#[test]
fn inspect_protected_target_omits_supervisor() {
    let identity = test_identity();
    let mut target = verified_target(identity.clone());
    target.supervisor = Some(Supervisor {
        kind: SupervisorKind::WindowsService,
        scope: SupervisorScope::System,
        identifiers: vec!["RouterSvc".into()],
    });
    let mut deps = unknown_discover();
    deps.inspect = Some(Box::new(move |_, _| Ok(target.clone())));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: identity.listen_addr.clone(),
            desktop_pid: identity.process.pid,
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    let encoded = serde_json::to_string(&inspection).unwrap();
    assert_eq!(
        encoded,
        r#"{"pid":42,"verification_mode":"verified_identity","process_name":"listener","executable":"/tmp/listener","listen_addr":"127.0.0.1:19099","recovery":{"action":"unavailable","reason":"protected_process"}}"#
    );
}

#[test]
fn inspect_different_user_omits_process_metadata() {
    let mut identity = test_identity();
    identity.user_id = "other-user".into();
    let mut deps = unknown_discover();
    let target = verified_target(identity.clone());
    deps.inspect = Some(Box::new(move |_, _| Ok(target.clone())));
    deps.current_user = Some(Box::new(|| Ok("current-user".into())));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: identity.listen_addr.clone(),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    let encoded = serde_json::to_string(&inspection).unwrap();
    assert_eq!(
        encoded,
        r#"{"pid":42,"verification_mode":"verified_identity","listen_addr":"127.0.0.1:19099","recovery":{"action":"manual_stop_required","reason":"different_user"}}"#
    );
}

#[test]
fn force_terminate_maps_permission_denied() {
    let identity = test_identity();
    let mut deps = unknown_discover();
    deps.inspect = Some(Box::new(move |_, _| Ok(verified_target(identity.clone()))));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.same_process = Some(Box::new(|left, right| Ok(left == right)));
    deps.validate = Some(Box::new(|_, _| Ok(Status::Genuine)));
    deps.signal = Some(Box::new(|_| Err(OccupantError::PermissionDenied)));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = service_with(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::PermissionDenied)
    );
}

#[test]
fn expiry_supersession_and_restart() {
    let now = Arc::new(Mutex::new(Utc::now()));
    let new_service = || {
        let mut random = vec![9u8; 32];
        random.extend([10u8; 96]);
        let random = Arc::new(Mutex::new(Cursor::new(random)));
        let mut deps = unknown_discover();
        deps.inspect = Some(Box::new(move |_, _| Ok(verified_target(test_identity()))));
        deps.current_user = Some(Box::new(|| Ok("user".into())));
        deps.same_process = Some(Box::new(|left, right| Ok(left == right)));
        deps.validate = Some(Box::new(|_, _| Ok(Status::Genuine)));
        let now = now.clone();
        deps.now = Some(Box::new(move || *now.lock().unwrap()));
        deps.random = Some(Box::new(move |buffer| {
            fill_random_from_reader(&mut *random.lock().unwrap(), buffer)
        }));
        OccupantService::new(
            OccupantConfig {
                listen_addr: "127.0.0.1:19099".into(),
                ..OccupantConfig::default()
            },
            deps,
        )
    };
    let service = new_service();
    let first = service.inspect(&CallContext::unbounded()).unwrap();
    let second = service.inspect(&CallContext::unbounded()).unwrap();
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            first.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::ConfirmationExpired)
    );
    assert_eq!(
        new_service().force_terminate(
            &CallContext::unbounded(),
            second.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::ConfirmationExpired)
    );
    let third = service.inspect(&CallContext::unbounded()).unwrap();
    *now.lock().unwrap() += chrono::Duration::seconds(30);
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            third.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::ConfirmationExpired)
    );
}

#[test]
fn changed_identity_consumes_token_without_signal() {
    let identity = test_identity();
    let live = Arc::new(Mutex::new(identity.clone()));
    let signals = Arc::new(AtomicUsize::new(0));
    let mut deps = unknown_discover();
    let live_inspect = live.clone();
    deps.inspect = Some(Box::new(move |_, _| {
        Ok(verified_target(live_inspect.lock().unwrap().clone()))
    }));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.same_process = Some(Box::new(|left, right| Ok(left == right)));
    let signals_signal = signals.clone();
    deps.signal = Some(Box::new(move |_| {
        signals_signal.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = service_with(
        OccupantConfig {
            listen_addr: identity.listen_addr,
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    live.lock().unwrap().socket_id = "replacement-socket".into();
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::Changed)
    );
    assert_eq!(signals.load(Ordering::SeqCst), 0);
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::ConfirmationExpired)
    );
}

#[test]
fn pid_only_admission_uses_pid_hooks_not_inspect_injection() {
    let target = pid_only_target();
    let mut deps = unknown_discover();
    deps.inspect = Some(Box::new(move |_, _| Ok(target.clone())));
    deps.supports_pid_only = Some(Box::new(|| true));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    if cfg!(windows) {
        assert_eq!(
            inspection.verification_mode,
            Some(VerificationMode::WindowsPidOnly)
        );
        assert_eq!(inspection.recovery.action, RecoveryAction::ForceTerminate);
        assert!(inspection.confirmation_token.is_some());
    } else {
        assert_eq!(
            inspection.recovery.reason,
            Some(RecoveryReason::IdentityUnavailable)
        );
        assert!(inspection.confirmation_token.is_none());
    }
}

#[test]
fn pid_only_force_terminate_and_replay() {
    let target = pid_only_target();
    let owners = Arc::new(Mutex::new(vec![Ok(4242), Err(OccupantError::NotFound)]));
    let signals = Arc::new(Mutex::new(Vec::new()));
    let mut deps = unknown_discover();
    deps.inspect = Some(Box::new(move |_, _| Ok(target.clone())));
    deps.supports_pid_only = Some(Box::new(|| true));
    let owners_lookup = owners.clone();
    deps.inspect_pid_owner = Some(Box::new(move |_, _| {
        let mut owners = owners_lookup.lock().unwrap();
        if owners.is_empty() {
            Err(OccupantError::IdentityUnavailable)
        } else {
            owners.remove(0)
        }
    }));
    let signals_signal = signals.clone();
    deps.signal_pid = Some(Box::new(move |pid| {
        signals_signal.lock().unwrap().push(pid);
        Ok(())
    }));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    deps.now = Some(Box::new(|| {
        Utc.with_ymd_and_hms(2026, 7, 22, 1, 2, 3).unwrap()
    }));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    assert_eq!(
        inspection.verification_mode,
        Some(VerificationMode::WindowsPidOnly)
    );
    assert!(inspection.process_name.is_none());
    let result = service
        .force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap(),
        )
        .unwrap();
    assert_eq!(result.port_state, "released");
    assert_eq!(*signals.lock().unwrap(), vec![4242]);
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::ConfirmationExpired)
    );
}

#[test]
fn pid_only_absent_after_confirmation_returns_changed() {
    let target = pid_only_target();
    let classification = Arc::new(Mutex::new(Classification::UnknownOccupant));
    let mut deps = OccupantDependencies::default();
    let classification_discover = classification.clone();
    deps.discover = Some(Box::new(move |_| *classification_discover.lock().unwrap()));
    deps.inspect = Some(Box::new(move |_, _| Ok(target.clone())));
    deps.supports_pid_only = Some(Box::new(|| true));
    deps.inspect_pid_owner = Some(Box::new(|_, _| Ok(4242)));
    deps.signal_pid = Some(Box::new(|_| Ok(())));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    *classification.lock().unwrap() = Classification::Absent;
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::Changed)
    );
}

#[test]
fn verified_absent_after_confirmation_remains_not_found() {
    let identity = test_identity();
    let classification = Arc::new(Mutex::new(Classification::UnknownOccupant));
    let mut deps = OccupantDependencies::default();
    let classification_discover = classification.clone();
    deps.discover = Some(Box::new(move |_| *classification_discover.lock().unwrap()));
    deps.inspect = Some(Box::new(move |_, _| Ok(verified_target(identity.clone()))));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    *classification.lock().unwrap() = Classification::Absent;
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::NotFound)
    );
}

#[test]
fn rejects_pid_only_without_explicit_support() {
    let target = pid_only_target();
    let mut deps = unknown_discover();
    deps.inspect = Some(Box::new(move |_, _| Ok(target.clone())));
    deps.supports_pid_only = Some(Box::new(|| false));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    assert_eq!(inspection.recovery.action, RecoveryAction::Unavailable);
    assert_eq!(
        inspection.recovery.reason,
        Some(RecoveryReason::IdentityUnavailable)
    );
    assert!(inspection.confirmation_token.is_none());
}

#[test]
fn release_wait_revalidates_process_on_timeout() {
    for (final_status, want) in [
        (Status::Absent, OccupantError::PortReleaseTimeout),
        (Status::Genuine, OccupantError::TerminationFailed),
        (Status::Stale, OccupantError::Changed),
    ] {
        let identity = test_identity();
        let validations = Arc::new(AtomicUsize::new(0));
        let mut deps = unknown_discover();
        deps.inspect = Some(Box::new(move |_, _| Ok(verified_target(identity.clone()))));
        deps.current_user = Some(Box::new(|| Ok("user".into())));
        deps.same_process = Some(Box::new(|left, right| Ok(left == right)));
        let validations_validate = validations.clone();
        deps.validate = Some(Box::new(move |_, _| {
            let count = validations_validate.fetch_add(1, Ordering::SeqCst);
            if count >= 2 {
                Ok(final_status)
            } else {
                Ok(Status::Genuine)
            }
        }));
        deps.signal = Some(Box::new(|_| Ok(())));
        deps.sleep = Some(Box::new(|_, _| Err(OccupantError::PortReleaseTimeout)));
        deps.random = Some(Box::new(|buffer| {
            fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
        }));
        let service = service_with(
            OccupantConfig {
                listen_addr: "127.0.0.1:19099".into(),
                ..OccupantConfig::default()
            },
            deps,
        );
        let inspection = service.inspect(&CallContext::unbounded()).unwrap();
        assert_eq!(
            service.force_terminate(
                &CallContext::unbounded(),
                inspection.confirmation_token.as_deref().unwrap()
            ),
            Err(want)
        );
    }
}

#[test]
fn diagnoses_changed_owner_and_protected_without_signal() {
    let identity = test_identity();
    let cases = [
        (
            "other user",
            {
                let mut changed = identity.clone();
                changed.user_id = "other".into();
                (
                    changed,
                    OccupantConfig {
                        listen_addr: identity.listen_addr.clone(),
                        ..OccupantConfig::default()
                    },
                )
            },
            RecoveryAction::ManualStopRequired,
            Some(RecoveryReason::DifferentUser),
        ),
        (
            "desktop",
            (
                identity.clone(),
                OccupantConfig {
                    listen_addr: identity.listen_addr.clone(),
                    desktop_pid: identity.process.pid,
                    ..OccupantConfig::default()
                },
            ),
            RecoveryAction::Unavailable,
            Some(RecoveryReason::ProtectedProcess),
        ),
        (
            "manager",
            (
                identity.clone(),
                OccupantConfig {
                    listen_addr: identity.listen_addr.clone(),
                    manager_identity: identity.process.clone(),
                    ..OccupantConfig::default()
                },
            ),
            RecoveryAction::Unavailable,
            Some(RecoveryReason::ProtectedProcess),
        ),
        (
            "managed router",
            (
                identity.clone(),
                OccupantConfig {
                    listen_addr: identity.listen_addr.clone(),
                    is_protected: Some(|_| true),
                    ..OccupantConfig::default()
                },
            ),
            RecoveryAction::Unavailable,
            Some(RecoveryReason::ProtectedProcess),
        ),
    ];
    for (name, (live, config), action, reason) in cases {
        let mut deps = unknown_discover();
        deps.inspect = Some(Box::new(move |_, _| Ok(verified_target(live.clone()))));
        deps.current_user = Some(Box::new(|| Ok("user".into())));
        deps.signal = Some(Box::new(|_| panic!("unexpected signal")));
        deps.random = Some(Box::new(|buffer| {
            fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
        }));
        let service = OccupantService::new(config, deps);
        let inspection = service.inspect(&CallContext::unbounded()).unwrap();
        assert_eq!(inspection.recovery.action, action, "{name}");
        assert_eq!(inspection.recovery.reason, reason, "{name}");
        assert!(inspection.confirmation_token.is_none(), "{name}");
    }
}

#[test]
fn concurrent_token_consumption_signals_once() {
    let identity = test_identity();
    let signals = Arc::new(AtomicUsize::new(0));
    let mut deps = unknown_discover();
    deps.inspect = Some(Box::new(move |_, _| Ok(verified_target(identity.clone()))));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.same_process = Some(Box::new(|left, right| Ok(left == right)));
    let status = Arc::new(Mutex::new(Status::Genuine));
    let status_validate = status.clone();
    deps.validate = Some(Box::new(move |_, _| Ok(*status_validate.lock().unwrap())));
    let signals_signal = signals.clone();
    let status_signal = status.clone();
    deps.signal = Some(Box::new(move |_| {
        signals_signal.fetch_add(1, Ordering::SeqCst);
        *status_signal.lock().unwrap() = Status::Absent;
        Ok(())
    }));
    deps.dial = Some(Box::new(|_, _, _| Err(OccupantError::NotFound)));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = Arc::new(service_with(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        deps,
    ));
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    let token = inspection.confirmation_token.unwrap();
    let first = {
        let service = service.clone();
        let token = token.clone();
        std::thread::spawn(move || service.force_terminate(&CallContext::unbounded(), &token))
    };
    let second = {
        let service = service.clone();
        std::thread::spawn(move || service.force_terminate(&CallContext::unbounded(), &token))
    };
    let _ = first.join().unwrap();
    let _ = second.join().unwrap();
    assert_eq!(signals.load(Ordering::SeqCst), 1);
}

#[test]
fn confirmation_permission_loss_consumes_token_without_signal() {
    let identity = test_identity();
    let denied = Arc::new(Mutex::new(false));
    let signals = Arc::new(AtomicUsize::new(0));
    let mut deps = unknown_discover();
    let denied_inspect = denied.clone();
    deps.inspect = Some(Box::new(move |_, _| {
        let mut target = verified_target(identity.clone());
        if *denied_inspect.lock().unwrap() {
            target.block_reason = Some(RecoveryReason::InsufficientPrivilege);
        }
        Ok(target)
    }));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.same_process = Some(Box::new(|left, right| Ok(left == right)));
    deps.validate = Some(Box::new(|_, _| Ok(Status::Genuine)));
    let signals_signal = signals.clone();
    deps.signal = Some(Box::new(move |_| {
        signals_signal.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = service_with(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    *denied.lock().unwrap() = true;
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::PermissionDenied)
    );
    assert_eq!(signals.load(Ordering::SeqCst), 0);
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::ConfirmationExpired)
    );
}

#[test]
fn never_signals_replacement_during_release_wait() {
    let identity = test_identity();
    let live = Arc::new(Mutex::new(identity.clone()));
    let signals = Arc::new(AtomicUsize::new(0));
    let mut deps = unknown_discover();
    let live_inspect = live.clone();
    deps.inspect = Some(Box::new(move |_, _| {
        Ok(verified_target(live_inspect.lock().unwrap().clone()))
    }));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.same_process = Some(Box::new(|left, right| Ok(left == right)));
    let status = Arc::new(Mutex::new(Status::Genuine));
    let status_validate = status.clone();
    deps.validate = Some(Box::new(move |_, _| Ok(*status_validate.lock().unwrap())));
    let signals_signal = signals.clone();
    let live_signal = live.clone();
    let status_signal = status.clone();
    deps.signal = Some(Box::new(move |_| {
        signals_signal.fetch_add(1, Ordering::SeqCst);
        *status_signal.lock().unwrap() = Status::Absent;
        let mut live = live_signal.lock().unwrap();
        live.socket_id = "replacement".into();
        live.process.pid += 1;
        Ok(())
    }));
    deps.dial = Some(Box::new(|_, _, _| Ok(())));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = service_with(
        OccupantConfig {
            listen_addr: identity.listen_addr,
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::Changed)
    );
    assert_eq!(signals.load(Ordering::SeqCst), 1);
}

#[test]
fn reserves_release_window_before_signaling() {
    let identity = test_identity();
    let mut deps = OccupantDependencies::default();
    deps.discover = Some(Box::new(|ctx| {
        if let Some(deadline) = ctx.deadline {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(
                remaining >= Duration::from_millis(900) && remaining <= Duration::from_secs(1),
                "pre-signal budget = {remaining:?}"
            );
        }
        Classification::UnknownOccupant
    }));
    deps.inspect = Some(Box::new(move |_, _| Ok(verified_target(identity.clone()))));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            release_timeout: Duration::from_secs(2),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    let ctx = CallContext::with_deadline(std::time::Instant::now() + Duration::from_secs(3));
    let _ = service.force_terminate(&ctx, inspection.confirmation_token.as_deref().unwrap());
}

#[test]
fn pid_only_cancellation_before_signal_returns_changed() {
    let target = pid_only_target();
    let signals = Arc::new(AtomicUsize::new(0));
    let mut deps = unknown_discover();
    deps.inspect = Some(Box::new(move |_, _| Ok(target.clone())));
    deps.supports_pid_only = Some(Box::new(|| true));
    deps.inspect_pid_owner = Some(Box::new(|ctx, _| {
        ctx.cancel();
        Ok(4242)
    }));
    let signals_signal = signals.clone();
    deps.signal_pid = Some(Box::new(move |_| {
        signals_signal.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    assert_eq!(
        service.force_terminate(
            &CallContext::unbounded(),
            inspection.confirmation_token.as_deref().unwrap()
        ),
        Err(OccupantError::Changed)
    );
    assert_eq!(signals.load(Ordering::SeqCst), 0);
}

#[test]
fn rejects_invalid_and_protected_pid_only_targets() {
    let target = pid_only_target();
    let cases = [
        (
            "missing PID",
            {
                let mut changed = target.clone();
                changed.pid = 0;
                (
                    changed,
                    OccupantConfig {
                        listen_addr: target.listen_addr.clone(),
                        ..OccupantConfig::default()
                    },
                )
            },
            RecoveryAction::Unavailable,
            Some(RecoveryReason::IdentityUnavailable),
        ),
        (
            "changed address",
            {
                let mut changed = target.clone();
                changed.listen_addr = "127.0.0.1:19100".into();
                (
                    changed,
                    OccupantConfig {
                        listen_addr: target.listen_addr.clone(),
                        ..OccupantConfig::default()
                    },
                )
            },
            RecoveryAction::Unavailable,
            Some(RecoveryReason::IdentityUnavailable),
        ),
        (
            "desktop",
            (
                target.clone(),
                OccupantConfig {
                    listen_addr: target.listen_addr.clone(),
                    desktop_pid: target.pid,
                    ..OccupantConfig::default()
                },
            ),
            RecoveryAction::Unavailable,
            Some(RecoveryReason::ProtectedProcess),
        ),
        (
            "protected PID",
            (
                target.clone(),
                OccupantConfig {
                    listen_addr: target.listen_addr.clone(),
                    is_protected_pid: Some(|pid| pid == 4242),
                    ..OccupantConfig::default()
                },
            ),
            RecoveryAction::Unavailable,
            Some(RecoveryReason::ProtectedProcess),
        ),
    ];
    for (name, (live, config), action, reason) in cases {
        let mut deps = unknown_discover();
        deps.inspect = Some(Box::new(move |_, _| Ok(live.clone())));
        deps.supports_pid_only = Some(Box::new(|| true));
        deps.inspect_pid_owner = Some(Box::new(|_, _| Ok(4242)));
        deps.signal_pid = Some(Box::new(|_| Ok(())));
        deps.random = Some(Box::new(|buffer| {
            fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
        }));
        let service = OccupantService::new(config, deps);
        let inspection = service.inspect(&CallContext::unbounded()).unwrap();
        assert_eq!(inspection.recovery.action, action, "{name}");
        assert_eq!(inspection.recovery.reason, reason, "{name}");
        assert!(inspection.confirmation_token.is_none(), "{name}");
    }
}

#[test]
fn state_files_protect_recorded_pid_and_full_identity() {
    let dir = std::env::temp_dir().join(format!(
        "mtls-occupant-state-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let desktop = dir.join("desktop-state.json");
    let identity = test_identity();
    crate::manager_core::state::write(
        &desktop,
        &crate::manager_core::state::RouterState {
            pid: identity.process.pid,
            process_started_at: identity.process.started_at.clone(),
            process_executable: identity.process.executable.clone(),
            ..crate::manager_core::state::RouterState::default()
        },
    )
    .unwrap();

    let mut deps = unknown_discover();
    let live = identity.clone();
    deps.inspect = Some(Box::new(move |_, _| Ok(verified_target(live.clone()))));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.signal = Some(Box::new(|_| panic!("unexpected signal")));
    let service = OccupantService::new(
        OccupantConfig {
            listen_addr: identity.listen_addr.clone(),
            state_paths: vec![desktop],
            ..OccupantConfig::default()
        },
        deps,
    );
    let inspection = service.inspect(&CallContext::unbounded()).unwrap();
    assert_eq!(inspection.recovery.action, RecoveryAction::Unavailable);
    assert_eq!(
        inspection.recovery.reason,
        Some(RecoveryReason::ProtectedProcess)
    );
    assert!(inspection.confirmation_token.is_none());
    let _ = std::fs::remove_dir_all(dir);
}
