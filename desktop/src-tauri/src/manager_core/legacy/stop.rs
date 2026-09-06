//! One-shot verified stop of a recorded legacy desktop router.
//!
//! Frozen against Go `lifecycle.stopVerified` / `migratable` / `previousManagerAbsent`,
//! plus the spec requirement to resample the listen port after exit. This
//! module does not load secrets and is not the default sidecar runtime path.

use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::protocol::ErrorCode;

use super::super::errors::LifecycleError;
use super::super::process::{self, Identity as ProcessIdentity, ProcessError, SignalKind, Status};
use super::super::state::{self, RouterState, StateError};
use super::super::trustedrouter::normalize_listener;
use super::identity::{
    complete_process, manager_identity, record_allows_migration, router_identity, CurrentLineage,
};

pub const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_FORCE_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub trait MigrationHost: Send + Sync {
    fn inspect(&self, pid: i32) -> Result<ProcessIdentity, ProcessError>;
    fn validate(&self, identity: &ProcessIdentity, binary: &str) -> Result<Status, ProcessError>;
    fn signal(
        &self,
        identity: &ProcessIdentity,
        binary: &str,
        kind: SignalKind,
    ) -> Result<(), ProcessError>;
    fn port_open(&self, authority: &str) -> bool;
    fn sleep(&self, delay: Duration) -> bool;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessHost;

impl MigrationHost for ProcessHost {
    fn inspect(&self, pid: i32) -> Result<ProcessIdentity, ProcessError> {
        process::inspect(pid)
    }

    fn validate(&self, identity: &ProcessIdentity, binary: &str) -> Result<Status, ProcessError> {
        process::validate(identity, binary)
    }

    fn signal(
        &self,
        identity: &ProcessIdentity,
        binary: &str,
        kind: SignalKind,
    ) -> Result<(), ProcessError> {
        process::signal(identity, binary, kind)
    }

    fn port_open(&self, authority: &str) -> bool {
        default_port_open(authority)
    }

    fn sleep(&self, delay: Duration) -> bool {
        std::thread::sleep(delay);
        true
    }
}

#[derive(Clone, Debug)]
pub struct StopBudget {
    pub stop_timeout: Duration,
    pub force_timeout: Duration,
    pub poll_interval: Duration,
}

impl Default for StopBudget {
    fn default() -> Self {
        Self {
            stop_timeout: DEFAULT_STOP_TIMEOUT,
            force_timeout: DEFAULT_FORCE_TIMEOUT,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

#[derive(Clone)]
pub struct LegacyRuntime {
    pub current: CurrentLineage,
    pub desktop_state_path: std::path::PathBuf,
    pub host: Arc<dyn MigrationHost>,
    pub budget: StopBudget,
}

impl LegacyRuntime {
    pub fn new(current: CurrentLineage, desktop_state_path: std::path::PathBuf) -> Self {
        Self {
            current,
            desktop_state_path,
            host: Arc::new(ProcessHost),
            budget: StopBudget::default(),
        }
    }

    pub fn with_host(mut self, host: Arc<dyn MigrationHost>) -> Self {
        self.host = host;
        self
    }
}

pub fn previous_manager_absent(value: &RouterState, host: &dyn MigrationHost) -> bool {
    let identity = manager_identity(value);
    if !complete_process(&identity) {
        return false;
    }
    matches!(
        host.validate(&identity, &value.manager_process_executable),
        Ok(Status::Absent)
    )
}

pub fn router_genuine(value: &RouterState, host: &dyn MigrationHost) -> bool {
    matches!(
        host.validate(&router_identity(value), &value.binary_path),
        Ok(Status::Genuine)
    )
}

pub fn verified_migratable(
    current: &CurrentLineage,
    value: &RouterState,
    host: &dyn MigrationHost,
) -> bool {
    record_allows_migration(current, value)
        && previous_manager_absent(value, host)
        && router_genuine(value, host)
}

pub fn recorded_process_absent(
    value: &RouterState,
    status: Status,
    host: &dyn MigrationHost,
) -> bool {
    if status == Status::Absent {
        return true;
    }
    if status != Status::Stale || value.pid <= 0 {
        return false;
    }
    matches!(host.inspect(value.pid), Err(ProcessError::NotFound))
}

/// Prepare a recorded legacy router for replacement: verify identity, stop it,
/// wait for exit, and resample the listen port. Missing state is a no-op so
/// the caller may start. Never uses a PID-only signal.
pub fn prepare_legacy_stop(
    current: &CurrentLineage,
    state_path: &Path,
    host: &dyn MigrationHost,
    budget: &StopBudget,
) -> Result<(), LifecycleError> {
    let value = match state::read(state_path) {
        Ok(value) => value,
        Err(StateError::NotFound) => return Ok(()),
        Err(_) => return Err(stale()),
    };
    if value.owner != "desktop" {
        return Ok(());
    }
    let status = host
        .validate(&router_identity(&value), &value.binary_path)
        .unwrap_or(Status::Stale);
    if status == Status::Genuine {
        if !verified_migratable(current, &value, host) {
            return Err(already_running());
        }
        stop_verified(&value, state_path, host, budget)?;
        resample_port(&value.listen_addr, host)?;
        return Ok(());
    }
    if recorded_process_absent(&value, status, host) {
        remove_state(state_path)?;
        return Ok(());
    }
    Err(stale())
}

pub fn stop_verified(
    value: &RouterState,
    state_path: &Path,
    host: &dyn MigrationHost,
    budget: &StopBudget,
) -> Result<(), LifecycleError> {
    let identity = router_identity(value);
    if !complete_process(&identity) || value.binary_path.is_empty() {
        return Err(stale());
    }
    if !matches!(
        host.validate(&identity, &value.binary_path),
        Ok(Status::Genuine)
    ) {
        return Err(stale());
    }
    if let Err(error) = host.signal(&identity, &value.binary_path, SignalKind::Interrupt) {
        if error != ProcessError::NotFound {
            return Err(stale());
        }
    }
    if wait_absent(
        &identity,
        &value.binary_path,
        host,
        budget.stop_timeout,
        budget.poll_interval,
    )? {
        return remove_state(state_path);
    }
    if !matches!(
        host.validate(&identity, &value.binary_path),
        Ok(Status::Genuine)
    ) {
        return Err(stale());
    }
    if let Err(error) = host.signal(&identity, &value.binary_path, SignalKind::Kill) {
        if error != ProcessError::NotFound {
            return Err(stale());
        }
    }
    if wait_absent(
        &identity,
        &value.binary_path,
        host,
        budget.force_timeout,
        budget.poll_interval,
    )? {
        return remove_state(state_path);
    }
    Err(LifecycleError::new(ErrorCode::OperationTimeout))
}

pub fn resample_port(listen_addr: &str, host: &dyn MigrationHost) -> Result<(), LifecycleError> {
    let Some(authority) = listen_authority(listen_addr) else {
        return Err(port_occupied());
    };
    if host.port_open(&authority) {
        return Err(port_occupied());
    }
    Ok(())
}

fn wait_absent(
    identity: &ProcessIdentity,
    binary: &str,
    host: &dyn MigrationHost,
    timeout: Duration,
    poll: Duration,
) -> Result<bool, LifecycleError> {
    let deadline = Instant::now() + timeout;
    loop {
        match host.validate(identity, binary) {
            Ok(Status::Absent) => return Ok(true),
            Ok(Status::Stale) => return Err(stale()),
            Ok(Status::Genuine) => {}
            Err(_) => return Err(stale()),
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || !host.sleep(poll.min(remaining)) {
            return match host.validate(identity, binary) {
                Ok(Status::Absent) => Ok(true),
                Ok(Status::Genuine) => Ok(false),
                _ => Err(stale()),
            };
        }
    }
}

fn listen_authority(listen_addr: &str) -> Option<String> {
    let value = listen_addr
        .strip_prefix("http://")
        .or_else(|| listen_addr.strip_prefix("https://"))
        .unwrap_or(listen_addr);
    normalize_listener(value)
        .ok()
        .map(|listener| listener.authority)
}

fn default_port_open(authority: &str) -> bool {
    let Ok(addr) = authority.parse() else {
        return true;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_ok()
}

fn remove_state(path: &Path) -> Result<(), LifecycleError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(stale()),
    }
}

fn stale() -> LifecycleError {
    LifecycleError::new(ErrorCode::RouterStateStale)
}

fn already_running() -> LifecycleError {
    LifecycleError::new(ErrorCode::RouterAlreadyRunning)
}

fn port_occupied() -> LifecycleError {
    LifecycleError::new(ErrorCode::PortOccupied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager_core::legacy::identity::{parse_record, record_allows_migration};
    use crate::manager_core::state::write as write_state;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;

    struct TestHost {
        statuses: Mutex<HashMap<i32, Status>>,
        inspect: Mutex<HashMap<i32, Result<ProcessIdentity, ProcessError>>>,
        signals: Mutex<Vec<(i32, SignalKind)>>,
        port_open: Mutex<bool>,
        after_signal: Mutex<HashMap<(i32, SignalKind), Status>>,
    }

    impl TestHost {
        fn new() -> Self {
            Self {
                statuses: Mutex::new(HashMap::new()),
                inspect: Mutex::new(HashMap::new()),
                signals: Mutex::new(Vec::new()),
                port_open: Mutex::new(false),
                after_signal: Mutex::new(HashMap::new()),
            }
        }

        fn set_status(&self, pid: i32, status: Status) {
            self.statuses.lock().unwrap().insert(pid, status);
        }

        fn set_after_signal(&self, pid: i32, kind: SignalKind, status: Status) {
            self.after_signal
                .lock()
                .unwrap()
                .insert((pid, kind), status);
        }

        fn signals(&self) -> Vec<(i32, SignalKind)> {
            self.signals.lock().unwrap().clone()
        }
    }

    impl MigrationHost for TestHost {
        fn inspect(&self, pid: i32) -> Result<ProcessIdentity, ProcessError> {
            self.inspect
                .lock()
                .unwrap()
                .get(&pid)
                .cloned()
                .unwrap_or(Err(ProcessError::NotFound))
        }

        fn validate(
            &self,
            identity: &ProcessIdentity,
            binary: &str,
        ) -> Result<Status, ProcessError> {
            if identity.pid <= 0
                || identity.started_at.is_empty()
                || identity.executable.is_empty()
                || binary.is_empty()
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
            match self.validate(identity, binary)? {
                Status::Genuine => {
                    self.signals.lock().unwrap().push((identity.pid, kind));
                    if let Some(next) = self.after_signal.lock().unwrap().get(&(identity.pid, kind))
                    {
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

    fn release_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../internal/manager/testdata")
            .join(name)
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

    fn temp_state(name: &str, value: &RouterState) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mtls-legacy-stop-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("desktop-state.json");
        write_state(&path, value).unwrap();
        path
    }

    fn load_release(name: &str) -> RouterState {
        state::read(release_path(name)).expect("release golden")
    }

    #[test]
    fn verified_golden_stops_waits_and_resamples() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.1.8.json");
        assert!(record_allows_migration(&current, &value));
        let path = temp_state("golden", &value);
        let host = TestHost::new();
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.set_after_signal(value.pid, SignalKind::Interrupt, Status::Absent);

        prepare_legacy_stop(&current, &path, &host, &StopBudget::default()).unwrap();
        assert_eq!(host.signals(), vec![(value.pid, SignalKind::Interrupt)]);
        assert_eq!(state::read(&path), Err(StateError::NotFound));
        assert_eq!(
            parse_record(release_path("desktop-state-v0.1.8.json"))
                .unwrap()
                .router
                .pid,
            41018
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn force_stop_after_graceful_wait_then_resample() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.2.0.json");
        let path = temp_state("force", &value);
        let host = TestHost::new();
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.set_after_signal(value.pid, SignalKind::Kill, Status::Absent);

        prepare_legacy_stop(&current, &path, &host, &StopBudget::default()).unwrap();
        assert_eq!(
            host.signals(),
            vec![
                (value.pid, SignalKind::Interrupt),
                (value.pid, SignalKind::Kill)
            ]
        );
        assert_eq!(state::read(&path), Err(StateError::NotFound));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn incomplete_or_unproven_identity_never_signals() {
        let current = current_v4();
        let mut incomplete = load_release("desktop-state-v0.1.8.json");
        incomplete.process_started_at.clear();
        let path = temp_state("incomplete", &incomplete);
        let host = TestHost::new();
        host.inspect.lock().unwrap().insert(
            incomplete.pid,
            Ok(ProcessIdentity {
                pid: incomplete.pid,
                started_at: "live-but-unproven".into(),
                executable: "/router".into(),
            }),
        );
        let error =
            prepare_legacy_stop(&current, &path, &host, &StopBudget::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterStateStale);
        assert!(host.signals().is_empty());
        assert!(state::read(&path).is_ok());

        let live_manager = load_release("desktop-state-v0.2.0.json");
        let path = temp_state("manager-live", &live_manager);
        let host = TestHost::new();
        host.set_status(live_manager.pid, Status::Genuine);
        host.set_status(live_manager.manager_pid, Status::Genuine);
        let error =
            prepare_legacy_stop(&current, &path, &host, &StopBudget::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterAlreadyRunning);
        assert!(host.signals().is_empty());

        let reused = load_release("desktop-state-v0.1.8.json");
        let path = temp_state("reused", &reused);
        let host = TestHost::new();
        host.set_status(reused.pid, Status::Stale);
        host.set_status(reused.manager_pid, Status::Absent);
        host.inspect.lock().unwrap().insert(
            reused.pid,
            Ok(ProcessIdentity {
                pid: reused.pid,
                started_at: "other-start".into(),
                executable: "/other".into(),
            }),
        );
        let error =
            prepare_legacy_stop(&current, &path, &host, &StopBudget::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterStateStale);
        assert!(host.signals().is_empty());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn timeout_and_reoccupied_port_fail_closed() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.1.8.json");
        let path = temp_state("timeout", &value);
        let host = TestHost::new();
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        let error =
            prepare_legacy_stop(&current, &path, &host, &StopBudget::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::OperationTimeout);
        assert_eq!(
            host.signals(),
            vec![
                (value.pid, SignalKind::Interrupt),
                (value.pid, SignalKind::Kill)
            ]
        );
        assert!(state::read(&path).is_ok());

        let path = temp_state("reoccupied", &value);
        let host = TestHost::new();
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.set_after_signal(value.pid, SignalKind::Interrupt, Status::Absent);
        *host.port_open.lock().unwrap() = true;
        let error =
            prepare_legacy_stop(&current, &path, &host, &StopBudget::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::PortOccupied);
        assert_eq!(host.signals(), vec![(value.pid, SignalKind::Interrupt)]);
        assert_eq!(state::read(&path), Err(StateError::NotFound));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_state_is_ready_to_start_without_signal() {
        let missing = std::env::temp_dir().join(format!(
            "mtls-legacy-missing-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let host = TestHost::new();
        prepare_legacy_stop(&current_v4(), &missing, &host, &StopBudget::default()).unwrap();
        assert!(host.signals().is_empty());
    }

    #[test]
    fn native_signal_still_rejects_pid_only() {
        let identity = process::inspect(std::process::id() as i32).expect("inspect");
        let mut forged = identity.clone();
        forged.started_at.push_str("-reused");
        assert_eq!(
            process::signal(&forged, &identity.executable, SignalKind::Interrupt),
            Err(ProcessError::IdentityMismatch)
        );
    }

    #[tokio::test]
    async fn manager_migrate_legacy_starts_after_verified_stop() {
        use crate::manager_core::{FakeBackend, Manager, StartedRouter};
        use crate::protocol::{Method, Request, RouterOwner};
        use serde_json::json;

        let current = current_v4();
        let recorded = load_release("desktop-state-v0.1.8.json");
        let path = temp_state("manager", &recorded);
        let host = Arc::new(TestHost::new());
        host.set_status(recorded.pid, Status::Genuine);
        host.set_status(recorded.manager_pid, Status::Absent);
        host.set_after_signal(recorded.pid, SignalKind::Interrupt, Status::Absent);

        let backend = FakeBackend::default();
        let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let start_calls = starts.clone();
        backend.set_start(move |owner| {
            start_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(owner, RouterOwner::Desktop);
            Ok(StartedRouter {
                owner,
                listen_addr: "127.0.0.1:19099".into(),
                pid: None,
            })
        });
        let manager = Manager::new(crate::manager_core::metadata::info(), backend)
            .with_legacy(LegacyRuntime::new(current, path.clone()).with_host(host.clone()));
        let response = manager
            .handle(Request {
                id: "m1".into(),
                method: Method::RouterMigrateLegacy,
                params: Some(json!({})),
            })
            .await;
        let body = serde_json::to_value(response).unwrap();
        assert!(body.get("error").is_none(), "{body}");
        assert_eq!(body["result"]["state"], "desktop_owned");
        assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(host.signals(), vec![(recorded.pid, SignalKind::Interrupt)]);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
