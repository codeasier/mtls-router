use chrono::{TimeZone, Utc};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::protocol::{
    deadline, ErrorCode, ManagerInfoResult, Method, Request, RouterOwner, RouterStatusResult,
    MANAGEMENT_PROTOCOL_VERSION,
};
use crate::router_core::{
    test_support::{start_mtls_server, MtlsServerOptions},
    BuildInfo, RouterDefaults, RouterEnv, RouterFlags, SupervisorConfig, SupervisorLimits,
};

use super::types::{Classification, DiscoverySnapshot, HealthInfo, VersionInfo};
use super::{
    FakeBackend, LifecycleError, Manager, StartedRouter, StartupStage, SupervisorBackend,
    DEFAULT_LOG_LINES, MAX_LOG_LINE_BYTES,
};

const INTEGRATION_KEY: &str = "sk-manager-integration-canary-9f3c";

fn test_info() -> ManagerInfoResult {
    ManagerInfoResult {
        version: "manager-v1".into(),
        commit: "commit-1".into(),
        build_date: "date-1".into(),
        target: "darwin/arm64".into(),
        deployment_id: "deploy-test".into(),
        management_protocol_version: MANAGEMENT_PROTOCOL_VERSION.into(),
    }
}

fn manager_with(backend: FakeBackend) -> Manager<FakeBackend> {
    Manager::new(test_info(), backend)
}

async fn call(manager: &Manager<FakeBackend>, method: Method, params: Value) -> Value {
    let response = manager
        .handle(Request {
            id: "req-1".into(),
            method,
            params: Some(params),
        })
        .await;
    serde_json::to_value(response).expect("response json")
}

fn result_of(response: Value) -> Value {
    assert!(
        response.get("error").is_none(),
        "unexpected error {response}"
    );
    response.get("result").cloned().expect("result")
}

fn error_of(response: Value) -> Value {
    assert!(
        response.get("result").is_none(),
        "unexpected result {response}"
    );
    response.get("error").cloned().expect("error")
}

#[tokio::test]
async fn manager_info_returns_injected_metadata_and_rejects_unknown_fields() {
    let manager = manager_with(FakeBackend::default());
    let response = call(&manager, Method::ManagerInfo, json!({})).await;
    let result = result_of(response);
    assert_eq!(
        result["management_protocol_version"],
        MANAGEMENT_PROTOCOL_VERSION
    );
    assert_eq!(result["version"], "manager-v1");
    assert_eq!(result["target"], "darwin/arm64");

    let rejected = call(&manager, Method::ManagerInfo, json!({"extra": true})).await;
    assert_eq!(error_of(rejected)["code"], "INVALID_PARAMS");
}

#[tokio::test]
async fn diagnostics_use_status_discovery_and_sanitize_listen_query() {
    let backend = FakeBackend::default();
    backend.set_status(DiscoverySnapshot {
        classification: Classification::DesktopOwned,
        owner: Some("desktop".into()),
        listen_addr: Some(format!("http://127.0.0.1:19099?api_key={INTEGRATION_KEY}")),
        pid: Some(42),
        version: Some(VersionInfo {
            version: "router-v1".into(),
            deployment_id: "deploy-test".into(),
            management_protocol_version: MANAGEMENT_PROTOCOL_VERSION.into(),
        }),
        state_version: None,
        health: None,
        log_path: None,
    });
    backend.set_health(DiscoverySnapshot {
        classification: Classification::DesktopOwned,
        health: Some(HealthInfo {
            status: INTEGRATION_KEY.into(),
        }),
        ..DiscoverySnapshot::absent()
    });
    let manager = manager_with(backend);
    let response = call(&manager, Method::DiagnosticsCollect, json!({})).await;
    let result = result_of(response);
    let summary = result["summary"].as_str().expect("summary");
    assert!(!summary.contains(INTEGRATION_KEY));
    assert!(summary.contains("listen=http://127.0.0.1:19099?[REDACTED]"));
    assert!(summary.contains(" health=\n"));
    assert!(summary.contains("agents unavailable"));
    assert_eq!(result["router_state"], "desktop_owned");
}

#[tokio::test]
async fn status_health_and_version_use_separate_discovery_paths() {
    let backend = FakeBackend::default();
    backend.set_status(DiscoverySnapshot {
        classification: Classification::DesktopOwned,
        owner: Some("desktop".into()),
        listen_addr: Some("127.0.0.1:19099".into()),
        pid: Some(7),
        version: Some(VersionInfo {
            version: "router-v1".into(),
            deployment_id: "prod-a".into(),
            management_protocol_version: MANAGEMENT_PROTOCOL_VERSION.into(),
        }),
        state_version: None,
        health: None,
        log_path: None,
    });
    backend.set_health(DiscoverySnapshot {
        classification: Classification::DesktopOwned,
        health: Some(HealthInfo {
            status: "ok".into(),
        }),
        ..DiscoverySnapshot::absent()
    });
    let manager = Manager::new(test_info(), backend)
        .with_clock(|| Utc.with_ymd_and_hms(2026, 7, 20, 1, 2, 3).unwrap());

    let status = result_of(call(&manager, Method::RouterStatus, json!({})).await);
    assert_eq!(status["state"], "desktop_owned");
    assert_eq!(status["owner"], "desktop");
    assert_eq!(status["pid"], 7);

    let health = result_of(call(&manager, Method::RouterHealth, json!({})).await);
    assert_eq!(health["status"], "ok");
    assert_eq!(health["checked_at"], "2026-07-20T01:02:03Z");

    let version = result_of(call(&manager, Method::RouterVersion, json!({})).await);
    assert_eq!(version["version"], "router-v1");
    assert_eq!(version["deployment_id"], "prod-a");
}

#[tokio::test]
async fn desktop_start_latches_safe_pre_launch_diagnostic_and_logs() {
    let backend = FakeBackend::default();
    backend.set_start(|_| {
        Err(LifecycleError::new(ErrorCode::RouterStartFailed)
            .with_stage(StartupStage::ProcessLaunch)
            .with_os_error(5)
            .with_output("reason=upstream_probe_failed\nAuthorization: Bearer hidden-canary"))
    });
    let manager = manager_with(backend);

    let start = error_of(call(&manager, Method::RouterStart, json!({"owner": "desktop"})).await);
    assert_eq!(start["code"], "ROUTER_START_FAILED");
    assert_eq!(start["message"], "router could not be started");

    let status: RouterStatusResult = serde_json::from_value(
        result_of(call(&manager, Method::RouterStatus, json!({})).await).clone(),
    )
    .unwrap();
    let want = "stage=process_launch code=ROUTER_START_FAILED os_error=5";
    assert_eq!(status.state, "start_failed");
    assert_eq!(status.last_error.as_deref(), Some(want));
    assert_eq!(
        status.recent_logs.as_deref(),
        Some(
            [
                want.to_owned(),
                "reason=upstream_probe_failed".into(),
                "Authorization: [REDACTED]".into()
            ]
            .as_slice()
        )
    );
    let serialized = format!("{status:?}");
    for leaked in ["secret", "hidden-canary", "Bearer hidden"] {
        assert!(
            !serialized.contains(leaked),
            "leaked {leaked}: {serialized}"
        );
    }

    let logs = result_of(call(&manager, Method::RouterLogs, json!({"limit": 10})).await);
    assert_eq!(logs["lines"], json!(status.recent_logs));
    let one = result_of(call(&manager, Method::RouterLogs, json!({"limit": 1})).await);
    assert_eq!(one["lines"], json!([want]));
}

#[tokio::test]
async fn launched_failure_without_stage_uses_closed_startup_message() {
    let recent = (0..DEFAULT_LOG_LINES + 2)
        .map(|index| format!("startup line {index}"))
        .chain([
            "Authorization: Bearer startup-secret-canary".into(),
            "x".repeat(MAX_LOG_LINE_BYTES + 20),
            "safe startup ending".into(),
        ])
        .collect::<Vec<_>>()
        .join("\n");
    let backend = FakeBackend::default();
    backend.set_start(move |_| {
        Err(LifecycleError::new(ErrorCode::RouterStartFailed)
            .launched()
            .with_output(recent.clone()))
    });
    let manager = manager_with(backend);
    let start = call(&manager, Method::RouterStart, json!({"owner": "desktop"})).await;
    assert_eq!(error_of(start)["message"], "router could not be started");
    let status: RouterStatusResult = serde_json::from_value(
        result_of(call(&manager, Method::RouterStatus, json!({})).await).clone(),
    )
    .unwrap();
    assert_eq!(status.state, "start_failed");
    assert_eq!(
        status.last_error.as_deref(),
        Some("desktop-owned router failed during startup")
    );
    let logs = status.recent_logs.expect("recent logs");
    assert_eq!(logs.len(), DEFAULT_LOG_LINES);
    assert_eq!(logs.last().map(String::as_str), Some("safe startup ending"));
    for line in &logs {
        assert!(!line.contains("startup-secret-canary"));
        assert!(line.len() <= MAX_LOG_LINE_BYTES + "[truncated]".len());
    }
}

#[tokio::test]
async fn reclaim_failure_returns_reclaim_code_but_latches_original_output() {
    let backend = FakeBackend::default();
    backend.set_status(DiscoverySnapshot {
        classification: Classification::Stale,
        ..DiscoverySnapshot::absent()
    });
    backend.set_start(|_| {
        Err(LifecycleError::new(ErrorCode::RouterStateStale)
            .launched()
            .with_output("original startup output"))
    });
    backend.set_reclaim(|| Err(LifecycleError::new(ErrorCode::RouterAlreadyRunning)));
    let manager = manager_with(backend);

    let start = error_of(call(&manager, Method::RouterStart, json!({"owner": "desktop"})).await);
    assert_eq!(start["code"], "ROUTER_ALREADY_RUNNING");
    assert_eq!(start["message"], "router is already running");
    let status: RouterStatusResult = serde_json::from_value(
        result_of(call(&manager, Method::RouterStatus, json!({})).await).clone(),
    )
    .unwrap();
    assert_eq!(
        status.recent_logs.as_deref(),
        Some(["original startup output".to_owned()].as_slice())
    );
}

#[tokio::test]
async fn successful_start_clears_latched_failure() {
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = FakeBackend::default();
    let start_calls = calls.clone();
    backend.set_start(move |_| {
        let count = start_calls.fetch_add(1, Ordering::SeqCst);
        if count == 0 {
            Err(LifecycleError::new(ErrorCode::RouterStartFailed)
                .launched()
                .with_output("first failure"))
        } else {
            Ok(StartedRouter {
                owner: RouterOwner::Desktop,
                listen_addr: "127.0.0.1:19099".into(),
                pid: Some(92),
            })
        }
    });
    let manager = manager_with(backend);
    let failed = call(&manager, Method::RouterStart, json!({"owner": "desktop"})).await;
    assert_eq!(error_of(failed)["code"], "ROUTER_START_FAILED");
    assert_eq!(
        result_of(call(&manager, Method::RouterStatus, json!({})).await)["state"],
        "start_failed"
    );

    let started = call(&manager, Method::RouterStart, json!({"owner": "desktop"})).await;
    let result = result_of(started);
    assert_eq!(result["state"], "desktop_owned");
    assert_eq!(result["pid"], 92);
    assert_eq!(
        result_of(call(&manager, Method::RouterStatus, json!({})).await)["state"],
        "absent"
    );
}

#[tokio::test]
async fn state_reconcile_failure_is_observable_without_blocking_absent_autostart() {
    let backend = FakeBackend::default();
    backend.set_start(|_| {
        Err(LifecycleError::new(ErrorCode::RouterStateStale)
            .with_stage(StartupStage::StateReconcile))
    });
    backend.set_reclaim(|| Err(LifecycleError::new(ErrorCode::RouterStateStale)));
    let manager = manager_with(backend);
    let start = call(&manager, Method::RouterStart, json!({"owner": "desktop"})).await;
    assert_eq!(error_of(start)["code"], "ROUTER_STATE_STALE");
    let status = result_of(call(&manager, Method::RouterStatus, json!({})).await);
    assert_eq!(status["state"], "start_failed");
    assert!(status["last_error"]
        .as_str()
        .unwrap()
        .contains("stage=state_reconcile"));
    assert!(manager.absent_start_ok());
}

#[tokio::test]
async fn stop_reports_absent_and_unported_methods_are_unavailable() {
    let backend = FakeBackend::default();
    let stops = Arc::new(AtomicUsize::new(0));
    let stop_calls = stops.clone();
    backend.set_stop(move || {
        stop_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let manager = manager_with(backend);
    let stopped = call(&manager, Method::RouterStop, json!({})).await;
    assert_eq!(result_of(stopped)["state"], "absent");
    assert_eq!(stops.load(Ordering::SeqCst), 1);

    let unavailable = error_of(call(&manager, Method::RouterMigrateLegacy, json!({})).await);
    assert_eq!(unavailable["code"], "INVALID_REQUEST");
    assert_eq!(unavailable["message"], "method is unavailable");
    assert_eq!(deadline(Method::RouterMigrateLegacy).as_secs(), 27);
}

#[tokio::test]
async fn logs_merge_disk_suffix_and_memory_prefix_and_apply_limit() {
    let directory = std::env::temp_dir().join(format!(
        "mtls-router-manager-core-logs-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let log_path = directory.join("router.log");
    std::fs::write(
        &log_path,
        b"disk-only\nrepeat\nrepeat\nshared-1\nshared-2\n",
    )
    .unwrap();
    let backend = FakeBackend::default();
    backend.set_status(DiscoverySnapshot {
        classification: Classification::DesktopOwned,
        owner: Some("desktop".into()),
        log_path: Some(log_path),
        ..DiscoverySnapshot::absent()
    });
    backend.set_recent("shared-1\nshared-2\nrepeat\nrepeat\nmemory-only\n");
    let manager = manager_with(backend);
    let logs = result_of(call(&manager, Method::RouterLogs, json!({"limit": 6})).await);
    assert_eq!(
        logs["lines"],
        json!([
            "repeat",
            "shared-1",
            "shared-2",
            "repeat",
            "repeat",
            "memory-only"
        ])
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[tokio::test]
async fn supervisor_start_stop_and_probe_failure_keep_protocol_codes() {
    let upstream = start_mtls_server(MtlsServerOptions::default()).await;
    let identity = BuildInfo::new("router-v1", "commit-1", "date-1", "deploy-test");
    let mut defaults = RouterDefaults::default();
    defaults.listen_addr = "127.0.0.1:0".into();
    defaults.upstream_url = upstream.url.clone();
    defaults.timeout = std::time::Duration::from_secs(1);
    let backend = SupervisorBackend::new(SupervisorConfig {
        defaults: defaults.clone(),
        env: RouterEnv::default(),
        flags: RouterFlags::default(),
        materials: upstream.certs.materials(),
        identity: identity.clone(),
        limits: SupervisorLimits::default(),
        logger: None,
    });
    let manager = Manager::new(test_info(), backend);
    let started = call_any(&manager, Method::RouterStart, json!({"owner": "desktop"})).await;
    let result = result_of(started);
    assert_eq!(result["state"], "desktop_owned");
    assert_eq!(result["owner"], "desktop");
    assert!(result.get("pid").is_none());

    let status = result_of(call_any(&manager, Method::RouterStatus, json!({})).await);
    assert_eq!(status["state"], "desktop_owned");
    let health = result_of(call_any(&manager, Method::RouterHealth, json!({})).await);
    assert_eq!(health["status"], "ok");
    let version = result_of(call_any(&manager, Method::RouterVersion, json!({})).await);
    assert_eq!(version["version"], "router-v1");
    assert_eq!(
        version["management_protocol_version"],
        MANAGEMENT_PROTOCOL_VERSION
    );

    let already = call_any(&manager, Method::RouterStart, json!({"owner": "desktop"})).await;
    assert_eq!(error_of(already)["code"], "ROUTER_ALREADY_RUNNING");

    let stopped = call_any(&manager, Method::RouterStop, json!({})).await;
    assert_eq!(result_of(stopped)["state"], "absent");
    manager.into_backend().shutdown().await.ok();

    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let failed_listen = reserved.local_addr().unwrap().to_string();
    drop(reserved);
    let mut failed_upstream = start_mtls_server(MtlsServerOptions {
        status: 503,
        ..MtlsServerOptions::default()
    })
    .await;
    defaults.listen_addr = failed_listen.clone();
    defaults.upstream_url = failed_upstream.url.clone();
    let failed_backend = SupervisorBackend::new(SupervisorConfig {
        defaults,
        env: RouterEnv::default(),
        flags: RouterFlags::default(),
        materials: failed_upstream.certs.materials(),
        identity,
        limits: SupervisorLimits {
            command_timeout: std::time::Duration::from_secs(2),
            request_timeout: std::time::Duration::from_millis(200),
            shutdown_timeout: std::time::Duration::from_millis(200),
            read_header_timeout: std::time::Duration::from_millis(200),
            ..SupervisorLimits::default()
        },
        logger: None,
    });
    let failed_manager = Manager::new(test_info(), failed_backend);
    let failed = call_any(
        &failed_manager,
        Method::RouterStart,
        json!({"owner": "desktop"}),
    )
    .await;
    assert_eq!(error_of(failed)["code"], "ROUTER_START_FAILED");
    assert!(!failed_manager.backend().supervisor().status().bound);
    assert!(tokio::net::TcpStream::connect(&failed_listen)
        .await
        .is_err());
    let latched = result_of(call_any(&failed_manager, Method::RouterStatus, json!({})).await);
    assert_eq!(latched["state"], "start_failed");
    assert!(latched["last_error"]
        .as_str()
        .unwrap()
        .contains("stage=readiness"));
    failed_upstream.shutdown();
    failed_manager.into_backend().shutdown().await.ok();
}

#[tokio::test]
async fn inspect_occupant_absent_maps_to_not_found() {
    let manager = manager_with(FakeBackend::default());
    let response = call(&manager, Method::RouterInspectOccupant, json!({})).await;
    let error = error_of(response);
    assert_eq!(error["code"], "OCCUPANT_NOT_FOUND");
    assert_eq!(error["message"], "port occupant was not found");
}

#[tokio::test]
async fn inspect_and_force_terminate_occupant_follow_confirmation_contract() {
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::manager_core::occupant::{
        fill_random_from_reader, Identity, OccupantConfig, OccupantDependencies, OccupantService,
        RecoveryReason, Supervisor, SupervisorKind, SupervisorScope, Target, VerificationMode,
    };
    use crate::manager_core::process::{Identity as ProcessIdentity, Status};
    use crate::types::OccupantInspection;

    let identity = Identity {
        listen_addr: "127.0.0.1:19099".into(),
        network: "tcp4".into(),
        socket_id: "socket".into(),
        process: ProcessIdentity {
            pid: 42,
            started_at: "start".into(),
            executable: "/tmp/listener".into(),
        },
        user_id: "user".into(),
    };
    let status = Arc::new(Mutex::new(Status::Genuine));
    let signals = Arc::new(AtomicUsize::new(0));
    let mut deps = OccupantDependencies::default();
    let inspect_identity = identity.clone();
    deps.inspect = Some(Box::new(move |_, _| {
        Ok(Target {
            mode: Some(VerificationMode::VerifiedIdentity),
            pid: inspect_identity.process.pid,
            listen_addr: inspect_identity.listen_addr.clone(),
            identity: inspect_identity.clone(),
            supervisor: None,
            block_reason: None,
        })
    }));
    deps.current_user = Some(Box::new(|| Ok("user".into())));
    deps.same_process = Some(Box::new(|left, right| Ok(left == right)));
    let status_validate = status.clone();
    deps.validate = Some(Box::new(move |_, _| Ok(*status_validate.lock().unwrap())));
    let signals_signal = signals.clone();
    let status_signal = status.clone();
    deps.signal = Some(Box::new(move |_| {
        signals_signal.fetch_add(1, Ordering::SeqCst);
        *status_signal.lock().unwrap() = Status::Absent;
        Ok(())
    }));
    deps.dial = Some(Box::new(|_, _, _| {
        Err(crate::manager_core::OccupantError::NotFound)
    }));
    deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([7u8; 64]), buffer)
    }));
    deps.now = Some(Box::new(|| {
        Utc.with_ymd_and_hms(2026, 7, 18, 1, 2, 3).unwrap()
    }));
    let backend = FakeBackend::default();
    backend.set_status(DiscoverySnapshot {
        classification: Classification::UnknownOccupant,
        ..DiscoverySnapshot::absent()
    });
    let manager = Manager::new(test_info(), backend).with_occupant(OccupantService::new(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        deps,
    ));

    let inspection_json = result_of(call(&manager, Method::RouterInspectOccupant, json!({})).await);
    let inspection: OccupantInspection =
        serde_json::from_value(inspection_json.clone()).expect("desktop inspection shape");
    assert_eq!(inspection.pid, 42);
    assert_eq!(inspection.listen_addr, "127.0.0.1:19099");
    assert_eq!(inspection_json["verification_mode"], "verified_identity");
    assert_eq!(inspection_json["expires_at"], "2026-07-18T01:02:33Z");
    assert_eq!(inspection.confirmation_token.as_ref().unwrap().len(), 43);

    let blank = error_of(
        call(
            &manager,
            Method::RouterForceTerminateOccupant,
            json!({"confirmation_token": "   "}),
        )
        .await,
    );
    assert_eq!(blank["code"], "INVALID_PARAMS");
    assert_eq!(blank["message"], "confirmation_token is required");

    let extra = error_of(
        call(
            &manager,
            Method::RouterForceTerminateOccupant,
            json!({"confirmation_token": "token", "pid": 42}),
        )
        .await,
    );
    assert_eq!(extra["code"], "INVALID_PARAMS");

    let terminated = result_of(
        call(
            &manager,
            Method::RouterForceTerminateOccupant,
            json!({"confirmation_token": inspection.confirmation_token}),
        )
        .await,
    );
    assert_eq!(terminated["termination"], "process_terminated");
    assert_eq!(terminated["port_state"], "released");
    assert_eq!(signals.load(Ordering::SeqCst), 1);

    let replay = error_of(
        call(
            &manager,
            Method::RouterForceTerminateOccupant,
            json!({"confirmation_token": inspection.confirmation_token}),
        )
        .await,
    );
    assert_eq!(replay["code"], "CONFIRMATION_EXPIRED");
    assert_eq!(replay["message"], "occupant confirmation expired");
    assert_eq!(signals.load(Ordering::SeqCst), 1);

    let mut blocked_deps = OccupantDependencies::default();
    blocked_deps.inspect = Some(Box::new(move |_, _| {
        Ok(Target {
            mode: Some(VerificationMode::VerifiedIdentity),
            pid: identity.process.pid,
            listen_addr: identity.listen_addr.clone(),
            identity: identity.clone(),
            supervisor: Some(Supervisor {
                kind: SupervisorKind::WindowsService,
                scope: SupervisorScope::System,
                identifiers: vec!["RouterSvc".into()],
            }),
            block_reason: None,
        })
    }));
    blocked_deps.current_user = Some(Box::new(|| Ok("user".into())));
    blocked_deps.random = Some(Box::new(|buffer| {
        fill_random_from_reader(&mut Cursor::new([0u8; 32]), buffer)
    }));
    let blocked_backend = FakeBackend::default();
    blocked_backend.set_status(DiscoverySnapshot {
        classification: Classification::UnknownOccupant,
        ..DiscoverySnapshot::absent()
    });
    let blocked = Manager::new(test_info(), blocked_backend).with_occupant(OccupantService::new(
        OccupantConfig {
            listen_addr: "127.0.0.1:19099".into(),
            ..OccupantConfig::default()
        },
        blocked_deps,
    ));
    let blocked_json = result_of(call(&blocked, Method::RouterInspectOccupant, json!({})).await);
    let blocked_inspection: OccupantInspection =
        serde_json::from_value(blocked_json.clone()).expect("blocked inspection");
    assert_eq!(
        blocked_inspection.recovery.reason,
        Some(crate::types::RecoveryReason::ServiceManaged)
    );
    assert!(blocked_inspection.confirmation_token.is_none());
    assert_eq!(blocked_json["recovery"]["action"], "manual_stop_required");
}

#[tokio::test]
async fn occupant_errors_use_closed_messages() {
    let backend = FakeBackend::default();
    backend.set_status(DiscoverySnapshot {
        classification: Classification::DesktopOwned,
        listen_addr: Some(format!("127.0.0.1:19099?api_key={INTEGRATION_KEY}")),
        ..DiscoverySnapshot::absent()
    });
    let manager = manager_with(backend);
    let error = error_of(call(&manager, Method::RouterInspectOccupant, json!({})).await);
    assert_eq!(error["code"], "OCCUPANT_PROTECTED");
    assert_eq!(error["message"], "port occupant is protected");
    let serialized = error.to_string();
    assert!(!serialized.contains(INTEGRATION_KEY));
}

async fn call_any<B: super::Backend>(manager: &Manager<B>, method: Method, params: Value) -> Value {
    let response = manager
        .handle(Request {
            id: "req-1".into(),
            method,
            params: Some(params),
        })
        .await;
    serde_json::to_value(response).expect("response json")
}
