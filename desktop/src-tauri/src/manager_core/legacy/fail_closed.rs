//! Fail-closed migration matrix.
//!
//! Covers missing identity, mismatched executable, PID reuse, stop timeout,
//! signal refusal, and a reoccupied listen port. Each case must keep the
//! closed protocol error, never issue a PID-only signal, never start a
//! replacement router, and never add a termination signal when the same
//! request is repeated. This is not the default sidecar runtime path.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::manager_core::errors::StartupStage;
use crate::manager_core::legacy::identity::CurrentLineage;
use crate::manager_core::legacy::stop::{prepare_legacy_stop, LegacyRuntime, MigrationHost};
use crate::manager_core::process::{Identity as ProcessIdentity, ProcessError, SignalKind, Status};
use crate::manager_core::state::{self, write as write_state, RouterState, StateError};
use crate::manager_core::{FakeBackend, Manager, StartedRouter};
use crate::protocol::{ErrorCode, Method, Request, RouterOwner};

const RELEASES: [&str; 2] = ["desktop-state-v0.1.8.json", "desktop-state-v0.2.0.json"];

struct CaseHost {
    statuses: Mutex<HashMap<i32, Status>>,
    inspect: Mutex<HashMap<i32, Result<ProcessIdentity, ProcessError>>>,
    signals: Mutex<Vec<(i32, SignalKind)>>,
    port_open: Mutex<bool>,
    after_signal: Mutex<HashMap<SignalKind, Status>>,
    refuse: Mutex<Option<ProcessError>>,
    mismatch_binary: Mutex<bool>,
    force_genuine: Mutex<bool>,
}

impl CaseHost {
    fn new() -> Self {
        Self {
            statuses: Mutex::new(HashMap::new()),
            inspect: Mutex::new(HashMap::new()),
            signals: Mutex::new(Vec::new()),
            port_open: Mutex::new(false),
            after_signal: Mutex::new(HashMap::new()),
            refuse: Mutex::new(None),
            mismatch_binary: Mutex::new(false),
            force_genuine: Mutex::new(false),
        }
    }

    fn set_status(&self, pid: i32, status: Status) {
        self.statuses.lock().unwrap().insert(pid, status);
    }

    fn live_inspect(&self, pid: i32, started_at: &str, executable: &str) {
        self.inspect.lock().unwrap().insert(
            pid,
            Ok(ProcessIdentity {
                pid,
                started_at: started_at.into(),
                executable: executable.into(),
            }),
        );
    }

    fn signals(&self) -> Vec<(i32, SignalKind)> {
        self.signals.lock().unwrap().clone()
    }
}

impl MigrationHost for CaseHost {
    fn inspect(&self, pid: i32) -> Result<ProcessIdentity, ProcessError> {
        self.inspect
            .lock()
            .unwrap()
            .get(&pid)
            .cloned()
            .unwrap_or(Err(ProcessError::NotFound))
    }

    fn validate(&self, identity: &ProcessIdentity, binary: &str) -> Result<Status, ProcessError> {
        if *self.force_genuine.lock().unwrap() {
            return Ok(self
                .statuses
                .lock()
                .unwrap()
                .get(&identity.pid)
                .copied()
                .unwrap_or(Status::Genuine));
        }
        if identity.pid <= 0
            || identity.started_at.is_empty()
            || identity.executable.is_empty()
            || binary.is_empty()
            || (*self.mismatch_binary.lock().unwrap() && binary != identity.executable)
        {
            return Ok(Status::Stale);
        }
        Ok(self
            .statuses
            .lock()
            .unwrap()
            .get(&identity.pid)
            .copied()
            .unwrap_or(Status::Stale))
    }

    fn signal(
        &self,
        identity: &ProcessIdentity,
        binary: &str,
        kind: SignalKind,
    ) -> Result<(), ProcessError> {
        if let Some(error) = self.refuse.lock().unwrap().clone() {
            return Err(error);
        }
        match self.validate(identity, binary)? {
            Status::Genuine => {
                self.signals.lock().unwrap().push((identity.pid, kind));
                if let Some(next) = self.after_signal.lock().unwrap().get(&kind) {
                    self.statuses.lock().unwrap().insert(identity.pid, *next);
                }
                Ok(())
            }
            _ => Err(ProcessError::IdentityMismatch),
        }
    }

    fn port_open(&self, _authority: &str) -> bool {
        *self.port_open.lock().unwrap()
    }

    fn sleep(&self, _delay: Duration) -> bool {
        false
    }
}

fn current_v4() -> CurrentLineage {
    CurrentLineage {
        session_id: "current-session".into(),
        installation_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".into(),
        package_generation: 1,
        deployment_id: "current-production".into(),
        management_protocol_version: "4".into(),
        manager: ProcessIdentity {
            pid: 9,
            started_at: "2026-09-06T00:00:00.000000000Z".into(),
            executable: "/manager".into(),
        },
    }
}

fn release_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../internal/manager/testdata")
        .join(name)
}

fn load_release(name: &str) -> RouterState {
    state::read(release_path(name)).expect("release golden")
}

fn temp_state(name: &str, value: &RouterState) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mtls-legacy-fail-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("desktop-state.json");
    write_state(&path, value).unwrap();
    path
}

struct Expect {
    code: ErrorCode,
    signals: usize,
    state_kept: bool,
}

fn runtime(path: &Path, host: &Arc<CaseHost>) -> LegacyRuntime {
    LegacyRuntime::new(
        current_v4(),
        path.to_path_buf(),
        path.with_file_name("legacy-migration.json"),
    )
    .with_host(host.clone())
}

fn assert_prepare(host: &Arc<CaseHost>, path: &Path, expect: &Expect) {
    let runtime = runtime(path, host);
    let error = prepare_legacy_stop(&runtime).expect_err("fail-closed prepare");
    assert_eq!(error.code, expect.code);
    assert_eq!(error.stage, Some(StartupStage::StateReconcile));
    assert_eq!(host.signals().len(), expect.signals, "{:?}", host.signals());
    if expect.state_kept {
        assert!(state::read(path).is_ok());
    } else {
        assert_eq!(state::read(path), Err(StateError::NotFound));
    }
    // Repeating the same request must never add a termination signal.
    let _ = prepare_legacy_stop(&runtime);
    assert_eq!(host.signals().len(), expect.signals, "{:?}", host.signals());
}

async fn assert_manager(host: Arc<CaseHost>, path: PathBuf, expect: &Expect) {
    let starts = Arc::new(AtomicUsize::new(0));
    let start_calls = starts.clone();
    let backend = FakeBackend::default();
    backend.set_start(move |owner| {
        start_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(owner, RouterOwner::Desktop);
        Ok(StartedRouter {
            owner,
            listen_addr: "127.0.0.1:19099".into(),
            pid: None,
        })
    });
    let manager = Manager::new(crate::manager_core::metadata::info(), backend)
        .with_legacy(runtime(&path, &host));
    let response = manager
        .handle(Request {
            id: "fail-closed".into(),
            method: Method::RouterMigrateLegacy,
            params: Some(json!({})),
        })
        .await;
    let body = serde_json::to_value(response).unwrap();
    let error = body.get("error").expect("protocol error");
    assert_eq!(error["code"], error_code_name(expect.code));
    assert!(body.get("result").is_none(), "{body}");
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    assert_eq!(host.signals().len(), expect.signals);
    if expect.state_kept {
        assert!(state::read(&path).is_ok());
    } else {
        assert_eq!(state::read(&path), Err(StateError::NotFound));
    }
    let _ = manager
        .handle(Request {
            id: "fail-closed-retry".into(),
            method: Method::RouterMigrateLegacy,
            params: Some(json!({})),
        })
        .await;
    assert_eq!(
        host.signals().len(),
        expect.signals,
        "retry must not re-signal"
    );
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

fn error_code_name(code: ErrorCode) -> Value {
    serde_json::to_value(code).expect("error code")
}

fn missing_identity_host(value: &RouterState) -> Arc<CaseHost> {
    let host = Arc::new(CaseHost::new());
    *host.force_genuine.lock().unwrap() = true;
    host.set_status(value.pid, Status::Genuine);
    host.set_status(value.manager_pid, Status::Absent);
    host
}

#[test]
fn missing_identity_never_signals_on_both_goldens() {
    for fixture in RELEASES {
        let mut value = load_release(fixture);
        value.process_started_at.clear();
        let path = temp_state(&format!("missing-start-{fixture}"), &value);
        let host = missing_identity_host(&value);
        assert_prepare(
            &host,
            &path,
            &Expect {
                code: ErrorCode::RouterAlreadyRunning,
                signals: 0,
                state_kept: true,
            },
        );

        let mut pid_only = load_release(fixture);
        pid_only.binary_path.clear();
        pid_only.process_started_at.clear();
        pid_only.process_executable.clear();
        let path = temp_state(&format!("pid-only-{fixture}"), &pid_only);
        let host = missing_identity_host(&pid_only);
        assert_prepare(
            &host,
            &path,
            &Expect {
                code: ErrorCode::RouterAlreadyRunning,
                signals: 0,
                state_kept: true,
            },
        );

        let mut manager_incomplete = load_release(fixture);
        manager_incomplete.manager_process_started_at.clear();
        let path = temp_state(&format!("missing-manager-{fixture}"), &manager_incomplete);
        let host = missing_identity_host(&manager_incomplete);
        host.set_status(manager_incomplete.pid, Status::Genuine);
        assert_prepare(
            &host,
            &path,
            &Expect {
                code: ErrorCode::RouterAlreadyRunning,
                signals: 0,
                state_kept: true,
            },
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}

#[test]
fn mismatched_executable_never_signals() {
    for fixture in RELEASES {
        let mut value = load_release(fixture);
        value.binary_path = r"C:\other\mtls-router.exe".into();
        let path = temp_state(&format!("exe-{fixture}"), &value);
        let host = Arc::new(CaseHost::new());
        *host.mismatch_binary.lock().unwrap() = true;
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.live_inspect(
            value.pid,
            &value.process_started_at,
            &value.process_executable,
        );
        assert_prepare(
            &host,
            &path,
            &Expect {
                code: ErrorCode::RouterStateStale,
                signals: 0,
                state_kept: true,
            },
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}

#[test]
fn pid_reuse_never_signals() {
    for fixture in RELEASES {
        let value = load_release(fixture);
        let path = temp_state(&format!("reuse-{fixture}"), &value);
        let host = Arc::new(CaseHost::new());
        host.set_status(value.pid, Status::Stale);
        host.set_status(value.manager_pid, Status::Absent);
        host.live_inspect(value.pid, "reused-start", "/other-router");
        assert_prepare(
            &host,
            &path,
            &Expect {
                code: ErrorCode::RouterStateStale,
                signals: 0,
                state_kept: true,
            },
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}

#[test]
fn timeout_keeps_state_and_does_not_start() {
    let value = load_release(RELEASES[0]);
    let path = temp_state("timeout", &value);
    let host = Arc::new(CaseHost::new());
    host.set_status(value.pid, Status::Genuine);
    host.set_status(value.manager_pid, Status::Absent);
    assert_prepare(
        &host,
        &path,
        &Expect {
            code: ErrorCode::OperationTimeout,
            signals: 2,
            state_kept: true,
        },
    );
    assert_eq!(
        host.signals(),
        vec![
            (value.pid, SignalKind::Interrupt),
            (value.pid, SignalKind::Kill)
        ]
    );
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn refusal_never_issues_termination() {
    let value = load_release(RELEASES[1]);
    let path = temp_state("refuse", &value);
    let host = Arc::new(CaseHost::new());
    host.set_status(value.pid, Status::Genuine);
    host.set_status(value.manager_pid, Status::Absent);
    *host.refuse.lock().unwrap() = Some(ProcessError::PermissionDenied);
    assert_prepare(
        &host,
        &path,
        &Expect {
            code: ErrorCode::RouterStateStale,
            signals: 0,
            state_kept: true,
        },
    );
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn reoccupied_port_does_not_start() {
    let value = load_release(RELEASES[0]);
    let path = temp_state("reoccupied", &value);
    let host = Arc::new(CaseHost::new());
    host.set_status(value.pid, Status::Genuine);
    host.set_status(value.manager_pid, Status::Absent);
    host.after_signal
        .lock()
        .unwrap()
        .insert(SignalKind::Interrupt, Status::Absent);
    *host.port_open.lock().unwrap() = true;
    assert_prepare(
        &host,
        &path,
        &Expect {
            code: ErrorCode::PortOccupied,
            signals: 1,
            state_kept: false,
        },
    );
    assert_eq!(host.signals(), vec![(value.pid, SignalKind::Interrupt)]);
    let _ = fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test]
async fn manager_fail_closed_matrix_never_starts_replacement() {
    struct Row {
        name: &'static str,
        fixture: &'static str,
        setup: fn(&CaseHost, &mut RouterState),
        expect: Expect,
    }

    let rows = [
        Row {
            name: "missing-identity",
            fixture: RELEASES[0],
            setup: |host, value| {
                value.process_executable.clear();
                *host.force_genuine.lock().unwrap() = true;
                host.set_status(value.pid, Status::Genuine);
                host.set_status(value.manager_pid, Status::Absent);
            },
            expect: Expect {
                code: ErrorCode::RouterAlreadyRunning,
                signals: 0,
                state_kept: true,
            },
        },
        Row {
            name: "mismatched-executable",
            fixture: RELEASES[1],
            setup: |host, value| {
                value.binary_path = r"C:\other\mtls-router.exe".into();
                *host.mismatch_binary.lock().unwrap() = true;
                host.set_status(value.pid, Status::Genuine);
                host.set_status(value.manager_pid, Status::Absent);
                host.live_inspect(
                    value.pid,
                    &value.process_started_at,
                    &value.process_executable,
                );
            },
            expect: Expect {
                code: ErrorCode::RouterStateStale,
                signals: 0,
                state_kept: true,
            },
        },
        Row {
            name: "pid-reuse",
            fixture: RELEASES[0],
            setup: |host, value| {
                host.set_status(value.pid, Status::Stale);
                host.set_status(value.manager_pid, Status::Absent);
                host.live_inspect(value.pid, "reused-start", "/other-router");
            },
            expect: Expect {
                code: ErrorCode::RouterStateStale,
                signals: 0,
                state_kept: true,
            },
        },
        Row {
            name: "timeout",
            fixture: RELEASES[1],
            setup: |host, value| {
                host.set_status(value.pid, Status::Genuine);
                host.set_status(value.manager_pid, Status::Absent);
            },
            expect: Expect {
                code: ErrorCode::OperationTimeout,
                signals: 2,
                state_kept: true,
            },
        },
        Row {
            name: "refusal",
            fixture: RELEASES[0],
            setup: |host, value| {
                host.set_status(value.pid, Status::Genuine);
                host.set_status(value.manager_pid, Status::Absent);
                *host.refuse.lock().unwrap() = Some(ProcessError::PermissionDenied);
            },
            expect: Expect {
                code: ErrorCode::RouterStateStale,
                signals: 0,
                state_kept: true,
            },
        },
        Row {
            name: "reoccupied-port",
            fixture: RELEASES[1],
            setup: |host, value| {
                host.set_status(value.pid, Status::Genuine);
                host.set_status(value.manager_pid, Status::Absent);
                host.after_signal
                    .lock()
                    .unwrap()
                    .insert(SignalKind::Interrupt, Status::Absent);
                *host.port_open.lock().unwrap() = true;
            },
            expect: Expect {
                code: ErrorCode::PortOccupied,
                signals: 1,
                state_kept: false,
            },
        },
    ];

    for row in rows {
        let mut value = load_release(row.fixture);
        let host = Arc::new(CaseHost::new());
        (row.setup)(&host, &mut value);
        let path = temp_state(row.name, &value);
        assert_manager(host, path, &row.expect).await;
    }
}

#[test]
fn closed_codes_stay_in_the_stable_set() {
    for code in [
        ErrorCode::RouterAlreadyRunning,
        ErrorCode::RouterStateStale,
        ErrorCode::OperationTimeout,
        ErrorCode::PortOccupied,
    ] {
        assert!(!error_code_name(code)
            .as_str()
            .unwrap_or_default()
            .is_empty());
    }
}
