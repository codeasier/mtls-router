//! One-shot verified stop of a recorded legacy desktop router.
//!
//! Frozen against Go `lifecycle.stopVerified` / `migratable` / `previousManagerAbsent`,
//! plus the spec requirements to resample the listen port after exit, to
//! terminate at most once per installation generation, and to keep a redacted
//! migration result. Failures carry the closed `state_reconcile` stage so the
//! manager latches them into `router.status` / `router.logs` exactly like the
//! Go reconcile branch. This module does not load secrets and is not the
//! default sidecar runtime path.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::protocol::ErrorCode;

use super::super::errors::{LifecycleError, StartupStage};
use super::super::process::{self, Identity as ProcessIdentity, ProcessError, SignalKind, Status};
use super::super::state::{self, RouterState, StateError};
use super::super::trustedrouter::normalize_listener;
use super::identity::{
    complete_process, manager_identity, record_allows_migration, router_identity, CurrentLineage,
};
use super::record::{self, MigrationOutcome, MigrationRecord, CORRUPT_RECORD_LINE};

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
    pub desktop_state_path: PathBuf,
    /// Durable attempt record (`Paths::legacy_migration_file`).
    pub record_path: PathBuf,
    pub host: Arc<dyn MigrationHost>,
    pub budget: StopBudget,
    pub now: fn() -> DateTime<Utc>,
}

impl LegacyRuntime {
    pub fn new(current: CurrentLineage, desktop_state_path: PathBuf, record_path: PathBuf) -> Self {
        Self {
            current,
            desktop_state_path,
            record_path,
            host: Arc::new(ProcessHost),
            budget: StopBudget::default(),
            now: Utc::now,
        }
    }

    pub fn with_host(mut self, host: Arc<dyn MigrationHost>) -> Self {
        self.host = host;
        self
    }

    pub fn with_clock(mut self, now: fn() -> DateTime<Utc>) -> Self {
        self.now = now;
        self
    }

    /// Redacted one-line summary of the last attempt for diagnostics. `None`
    /// when no attempt was ever recorded.
    pub fn diagnostic_line(&self) -> Option<String> {
        match record::read(&self.record_path) {
            Ok(Some(record)) => Some(record.diagnostic_line()),
            Ok(None) => None,
            Err(_) => Some(CORRUPT_RECORD_LINE.to_owned()),
        }
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
/// the caller may start. Never uses a PID-only signal, and never signals a
/// second time within one installation generation: once the attempt record
/// shows an accepted signal (or a pending marker left by a crash), a live
/// target is reported as `ROUTER_ALREADY_RUNNING` without being touched.
pub fn prepare_legacy_stop(runtime: &LegacyRuntime) -> Result<(), LifecycleError> {
    let host = runtime.host.as_ref();
    let state_path = runtime.desktop_state_path.as_path();
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
        if !verified_migratable(&runtime.current, &value, host) {
            return Err(already_running());
        }
        let prior = match record::read(&runtime.record_path) {
            Ok(prior) => prior,
            Err(_) => return Err(stale().with_output(CORRUPT_RECORD_LINE)),
        };
        if let Some(prior) = prior
            .filter(|prior| prior.for_generation(&runtime.current) && prior.blocks_termination())
        {
            return Err(
                already_running().with_output(format!("{} retry=refused", prior.diagnostic_line()))
            );
        }
        let mut attempt = MigrationRecord::pending(&runtime.current, &value, (runtime.now)());
        if record::write(&runtime.record_path, &attempt).is_err() {
            return Err(stale());
        }
        let result = stop_verified(&value, state_path, host, &runtime.budget, &mut attempt)
            .and_then(|()| {
                resample_port(&value.listen_addr, host).map_err(|error| {
                    attempt.outcome = MigrationOutcome::PortReoccupied;
                    error
                })
            });
        // A failed final write leaves the pending marker, which still blocks
        // any further termination for this generation.
        let _ = record::write(&runtime.record_path, &attempt);
        return result.map_err(|error| error.with_output(attempt.diagnostic_line()));
    }
    if recorded_process_absent(&value, status, host) {
        remove_state(state_path)?;
        settle_prior_attempt(runtime, &value);
        return Ok(());
    }
    Err(stale())
}

/// A target that exited after a timed-out attempt is now `stopped`; the
/// signals list is left as issued.
fn settle_prior_attempt(runtime: &LegacyRuntime, value: &RouterState) {
    let Ok(Some(mut prior)) = record::read(&runtime.record_path) else {
        return;
    };
    if prior.for_generation(&runtime.current)
        && prior.target_pid == value.pid
        && prior.target_started_at == value.process_started_at
        && !prior.signals.is_empty()
        && prior.outcome != MigrationOutcome::Stopped
    {
        prior.outcome = MigrationOutcome::Stopped;
        let _ = record::write(&runtime.record_path, &prior);
    }
}

/// Journals every accepted signal and the final outcome into `attempt`; the
/// caller persists it. Refused signals only set the outcome.
pub fn stop_verified(
    value: &RouterState,
    state_path: &Path,
    host: &dyn MigrationHost,
    budget: &StopBudget,
    attempt: &mut MigrationRecord,
) -> Result<(), LifecycleError> {
    let identity = router_identity(value);
    if !complete_process(&identity) || value.binary_path.is_empty() {
        attempt.outcome = MigrationOutcome::IdentityChanged;
        return Err(stale());
    }
    if !matches!(
        host.validate(&identity, &value.binary_path),
        Ok(Status::Genuine)
    ) {
        attempt.outcome = MigrationOutcome::IdentityChanged;
        return Err(stale());
    }
    match host.signal(&identity, &value.binary_path, SignalKind::Interrupt) {
        Ok(()) => attempt.issued(SignalKind::Interrupt),
        Err(ProcessError::NotFound) => {}
        Err(_) => {
            attempt.outcome = MigrationOutcome::SignalRefused;
            return Err(stale());
        }
    }
    let exited = wait_absent(
        &identity,
        &value.binary_path,
        host,
        budget.stop_timeout,
        budget.poll_interval,
    )
    .map_err(|error| {
        attempt.outcome = MigrationOutcome::IdentityChanged;
        error
    })?;
    if exited {
        attempt.outcome = MigrationOutcome::Stopped;
        return remove_state(state_path);
    }
    if !matches!(
        host.validate(&identity, &value.binary_path),
        Ok(Status::Genuine)
    ) {
        attempt.outcome = MigrationOutcome::IdentityChanged;
        return Err(stale());
    }
    match host.signal(&identity, &value.binary_path, SignalKind::Kill) {
        Ok(()) => attempt.issued(SignalKind::Kill),
        Err(ProcessError::NotFound) => {}
        Err(_) => {
            attempt.outcome = MigrationOutcome::SignalRefused;
            return Err(stale());
        }
    }
    let exited = wait_absent(
        &identity,
        &value.binary_path,
        host,
        budget.force_timeout,
        budget.poll_interval,
    )
    .map_err(|error| {
        attempt.outcome = MigrationOutcome::IdentityChanged;
        error
    })?;
    if exited {
        attempt.outcome = MigrationOutcome::Stopped;
        return remove_state(state_path);
    }
    attempt.outcome = MigrationOutcome::StopTimeout;
    Err(staged(ErrorCode::OperationTimeout))
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

/// Migration happens while reconciling recorded state before any launch, so
/// every failure uses the closed `state_reconcile` stage (Go `stagedError`).
fn staged(code: ErrorCode) -> LifecycleError {
    LifecycleError::new(code).with_stage(StartupStage::StateReconcile)
}

fn stale() -> LifecycleError {
    staged(ErrorCode::RouterStateStale)
}

fn already_running() -> LifecycleError {
    staged(ErrorCode::RouterAlreadyRunning)
}

fn port_occupied() -> LifecycleError {
    staged(ErrorCode::PortOccupied)
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
        refuse: Mutex<Option<ProcessError>>,
    }

    impl TestHost {
        fn new() -> Self {
            Self {
                statuses: Mutex::new(HashMap::new()),
                inspect: Mutex::new(HashMap::new()),
                signals: Mutex::new(Vec::new()),
                port_open: Mutex::new(false),
                after_signal: Mutex::new(HashMap::new()),
                refuse: Mutex::new(None),
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
            if let Some(error) = self.refuse.lock().unwrap().clone() {
                return Err(error);
            }
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

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-07T01:30:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn record_path(state_path: &Path) -> PathBuf {
        state_path.with_file_name("legacy-migration.json")
    }

    fn runtime(current: &CurrentLineage, state_path: &Path, host: &Arc<TestHost>) -> LegacyRuntime {
        LegacyRuntime::new(
            current.clone(),
            state_path.to_path_buf(),
            record_path(state_path),
        )
        .with_host(host.clone())
        .with_clock(fixed_now)
    }

    fn prepare(
        current: &CurrentLineage,
        state_path: &Path,
        host: &Arc<TestHost>,
    ) -> Result<(), LifecycleError> {
        prepare_legacy_stop(&runtime(current, state_path, host))
    }

    fn recorded(state_path: &Path) -> MigrationRecord {
        record::read(&record_path(state_path))
            .expect("readable record")
            .expect("record present")
    }

    #[test]
    fn verified_golden_stops_waits_and_resamples() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.1.8.json");
        assert!(record_allows_migration(&current, &value));
        let path = temp_state("golden", &value);
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.set_after_signal(value.pid, SignalKind::Interrupt, Status::Absent);

        prepare(&current, &path, &host).unwrap();
        assert_eq!(host.signals(), vec![(value.pid, SignalKind::Interrupt)]);
        assert_eq!(state::read(&path), Err(StateError::NotFound));
        assert_eq!(
            parse_record(release_path("desktop-state-v0.1.8.json"))
                .unwrap()
                .router
                .pid,
            41018
        );
        let attempt = recorded(&path);
        assert_eq!(attempt.outcome, MigrationOutcome::Stopped);
        assert_eq!(attempt.target_pid, value.pid);
        assert_eq!(
            attempt.diagnostic_line(),
            "legacy_migration generation=1 source_generation=0 source_protocol=1 pid=41018 signals=interrupt outcome=stopped attempted_at=2026-09-07T01:30:00Z"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn force_stop_after_graceful_wait_then_resample() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.2.0.json");
        let path = temp_state("force", &value);
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.set_after_signal(value.pid, SignalKind::Kill, Status::Absent);

        prepare(&current, &path, &host).unwrap();
        assert_eq!(
            host.signals(),
            vec![
                (value.pid, SignalKind::Interrupt),
                (value.pid, SignalKind::Kill)
            ]
        );
        assert_eq!(state::read(&path), Err(StateError::NotFound));
        let attempt = recorded(&path);
        assert_eq!(attempt.outcome, MigrationOutcome::Stopped);
        assert_eq!(attempt.signals.len(), 2);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn incomplete_or_unproven_identity_never_signals() {
        let current = current_v4();
        let mut incomplete = load_release("desktop-state-v0.1.8.json");
        incomplete.process_started_at.clear();
        let path = temp_state("incomplete", &incomplete);
        let host = Arc::new(TestHost::new());
        host.inspect.lock().unwrap().insert(
            incomplete.pid,
            Ok(ProcessIdentity {
                pid: incomplete.pid,
                started_at: "live-but-unproven".into(),
                executable: "/router".into(),
            }),
        );
        let error = prepare(&current, &path, &host).unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterStateStale);
        assert_eq!(error.stage, Some(StartupStage::StateReconcile));
        assert!(host.signals().is_empty());
        assert!(state::read(&path).is_ok());
        assert_eq!(record::read(&record_path(&path)), Ok(None));

        let live_manager = load_release("desktop-state-v0.2.0.json");
        let path = temp_state("manager-live", &live_manager);
        let host = Arc::new(TestHost::new());
        host.set_status(live_manager.pid, Status::Genuine);
        host.set_status(live_manager.manager_pid, Status::Genuine);
        let error = prepare(&current, &path, &host).unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterAlreadyRunning);
        assert_eq!(error.stage, Some(StartupStage::StateReconcile));
        assert!(host.signals().is_empty());
        assert_eq!(record::read(&record_path(&path)), Ok(None));

        let reused = load_release("desktop-state-v0.1.8.json");
        let path = temp_state("reused", &reused);
        let host = Arc::new(TestHost::new());
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
        let error = prepare(&current, &path, &host).unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterStateStale);
        assert!(host.signals().is_empty());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn timeout_and_reoccupied_port_fail_closed() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.1.8.json");
        let path = temp_state("timeout", &value);
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        let error = prepare(&current, &path, &host).unwrap_err();
        assert_eq!(error.code, ErrorCode::OperationTimeout);
        assert_eq!(error.stage, Some(StartupStage::StateReconcile));
        assert!(
            error
                .recent_output
                .contains("signals=interrupt,kill outcome=stop_timeout"),
            "{}",
            error.recent_output
        );
        assert_eq!(
            host.signals(),
            vec![
                (value.pid, SignalKind::Interrupt),
                (value.pid, SignalKind::Kill)
            ]
        );
        assert!(state::read(&path).is_ok());
        assert_eq!(recorded(&path).outcome, MigrationOutcome::StopTimeout);

        let path = temp_state("reoccupied", &value);
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.set_after_signal(value.pid, SignalKind::Interrupt, Status::Absent);
        *host.port_open.lock().unwrap() = true;
        let error = prepare(&current, &path, &host).unwrap_err();
        assert_eq!(error.code, ErrorCode::PortOccupied);
        assert!(error.recent_output.contains("outcome=port_reoccupied"));
        assert_eq!(host.signals(), vec![(value.pid, SignalKind::Interrupt)]);
        assert_eq!(state::read(&path), Err(StateError::NotFound));
        assert_eq!(recorded(&path).outcome, MigrationOutcome::PortReoccupied);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_state_is_ready_to_start_without_signal() {
        let missing = std::env::temp_dir().join(format!(
            "mtls-legacy-missing-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let host = Arc::new(TestHost::new());
        prepare(&current_v4(), &missing, &host).unwrap();
        assert!(host.signals().is_empty());
        assert_eq!(record::read(&record_path(&missing)), Ok(None));
    }

    #[test]
    fn retry_after_timeout_never_signals_again() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.1.8.json");
        let path = temp_state("retry", &value);
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        assert_eq!(
            prepare(&current, &path, &host).unwrap_err().code,
            ErrorCode::OperationTimeout
        );
        assert_eq!(host.signals().len(), 2);

        for _ in 0..3 {
            let error = prepare(&current, &path, &host).unwrap_err();
            assert_eq!(error.code, ErrorCode::RouterAlreadyRunning);
            assert_eq!(error.stage, Some(StartupStage::StateReconcile));
            assert!(
                error.recent_output.ends_with(
                    "outcome=stop_timeout attempted_at=2026-09-07T01:30:00Z retry=refused"
                ),
                "{}",
                error.recent_output
            );
            assert_eq!(host.signals().len(), 2, "retry must not re-signal");
            assert!(state::read(&path).is_ok());
        }
        assert_eq!(recorded(&path).outcome, MigrationOutcome::StopTimeout);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn retry_after_target_exits_settles_without_signal() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.2.0.json");
        let path = temp_state("settle", &value);
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        assert_eq!(
            prepare(&current, &path, &host).unwrap_err().code,
            ErrorCode::OperationTimeout
        );

        host.set_status(value.pid, Status::Absent);
        prepare(&current, &path, &host).unwrap();
        assert_eq!(host.signals().len(), 2);
        assert_eq!(state::read(&path), Err(StateError::NotFound));
        let attempt = recorded(&path);
        assert_eq!(attempt.outcome, MigrationOutcome::Stopped);
        assert_eq!(attempt.signals.len(), 2);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn pending_marker_from_a_crash_blocks_termination() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.1.8.json");
        let path = temp_state("pending", &value);
        let pending = MigrationRecord::pending(&current, &value, fixed_now());
        record::write(&record_path(&path), &pending).unwrap();
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.set_after_signal(value.pid, SignalKind::Interrupt, Status::Absent);

        let error = prepare(&current, &path, &host).unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterAlreadyRunning);
        assert!(error.recent_output.contains("outcome=pending"));
        assert!(host.signals().is_empty());
        assert!(state::read(&path).is_ok());
        assert_eq!(recorded(&path), pending);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn other_generation_record_does_not_block_a_fresh_attempt() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.1.8.json");
        let path = temp_state("other-gen", &value);
        let mut previous = current.clone();
        previous.package_generation = 0;
        let mut old = MigrationRecord::pending(&previous, &value, fixed_now());
        old.issued(SignalKind::Interrupt);
        old.issued(SignalKind::Kill);
        old.outcome = MigrationOutcome::StopTimeout;
        record::write(&record_path(&path), &old).unwrap();
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.set_after_signal(value.pid, SignalKind::Interrupt, Status::Absent);

        prepare(&current, &path, &host).unwrap();
        assert_eq!(host.signals(), vec![(value.pid, SignalKind::Interrupt)]);
        let attempt = recorded(&path);
        assert_eq!(attempt.package_generation, 1);
        assert_eq!(attempt.outcome, MigrationOutcome::Stopped);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn unwritable_or_corrupt_record_fails_closed_before_signaling() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.1.8.json");
        let path = temp_state("corrupt", &value);
        fs::write(&record_path(&path), b"{not json").unwrap();
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.set_after_signal(value.pid, SignalKind::Interrupt, Status::Absent);
        let error = prepare(&current, &path, &host).unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterStateStale);
        assert_eq!(error.recent_output, CORRUPT_RECORD_LINE);
        assert!(host.signals().is_empty());
        assert!(state::read(&path).is_ok());
        assert_eq!(
            runtime(&current, &path, &host).diagnostic_line(),
            Some(CORRUPT_RECORD_LINE.to_owned())
        );

        let path = temp_state("unwritable", &value);
        fs::create_dir_all(record_path(&path)).unwrap();
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        host.set_after_signal(value.pid, SignalKind::Interrupt, Status::Absent);
        let error = prepare(&current, &path, &host).unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterStateStale);
        assert!(host.signals().is_empty());
        assert!(state::read(&path).is_ok());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn refused_signal_leaves_budget_for_a_later_verified_attempt() {
        let current = current_v4();
        let value = load_release("desktop-state-v0.2.0.json");
        let path = temp_state("refused", &value);
        let host = Arc::new(TestHost::new());
        host.set_status(value.pid, Status::Genuine);
        host.set_status(value.manager_pid, Status::Absent);
        *host.refuse.lock().unwrap() = Some(ProcessError::PermissionDenied);
        let error = prepare(&current, &path, &host).unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterStateStale);
        assert!(host.signals().is_empty());
        let attempt = recorded(&path);
        assert_eq!(attempt.outcome, MigrationOutcome::SignalRefused);
        assert!(attempt.signals.is_empty());
        assert!(!attempt.blocks_termination());

        *host.refuse.lock().unwrap() = None;
        host.set_after_signal(value.pid, SignalKind::Interrupt, Status::Absent);
        prepare(&current, &path, &host).unwrap();
        assert_eq!(host.signals(), vec![(value.pid, SignalKind::Interrupt)]);
        assert_eq!(recorded(&path).outcome, MigrationOutcome::Stopped);
        let _ = fs::remove_dir_all(path.parent().unwrap());
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

    fn counting_backend() -> (
        crate::manager_core::FakeBackend,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use crate::manager_core::{FakeBackend, StartedRouter};
        use crate::protocol::RouterOwner;

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
        (backend, starts)
    }

    async fn call(
        manager: &crate::manager_core::Manager<crate::manager_core::FakeBackend>,
        method: crate::protocol::Method,
    ) -> serde_json::Value {
        use crate::protocol::Request;

        let response = manager
            .handle(Request {
                id: "m".into(),
                method,
                params: Some(serde_json::json!({})),
            })
            .await;
        serde_json::to_value(response).unwrap()
    }

    #[tokio::test]
    async fn manager_migrate_legacy_starts_after_verified_stop() {
        use crate::manager_core::Manager;
        use crate::protocol::Method;

        let current = current_v4();
        let recorded = load_release("desktop-state-v0.1.8.json");
        let path = temp_state("manager", &recorded);
        let host = Arc::new(TestHost::new());
        host.set_status(recorded.pid, Status::Genuine);
        host.set_status(recorded.manager_pid, Status::Absent);
        host.set_after_signal(recorded.pid, SignalKind::Interrupt, Status::Absent);

        let (backend, starts) = counting_backend();
        let manager = Manager::new(crate::manager_core::metadata::info(), backend)
            .with_legacy(runtime(&current, &path, &host));
        let body = call(&manager, Method::RouterMigrateLegacy).await;
        assert!(body.get("error").is_none(), "{body}");
        assert_eq!(body["result"]["state"], "desktop_owned");
        assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(host.signals(), vec![(recorded.pid, SignalKind::Interrupt)]);

        let diagnostics = call(&manager, Method::DiagnosticsCollect).await;
        let summary = diagnostics["result"]["summary"].as_str().unwrap();
        assert!(
            summary.contains(
                "\nlegacy_migration generation=1 source_generation=0 source_protocol=1 pid=41018 signals=interrupt outcome=stopped attempted_at=2026-09-07T01:30:00Z\n"
            ),
            "{summary}"
        );
        assert!(!summary.contains("Program Files"), "{summary}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn manager_latches_migration_diagnostics_and_refuses_duplicate_termination() {
        use crate::manager_core::Manager;
        use crate::protocol::Method;

        let current = current_v4();
        let recorded = load_release("desktop-state-v0.2.0.json");
        let path = temp_state("manager-timeout", &recorded);
        let host = Arc::new(TestHost::new());
        host.set_status(recorded.pid, Status::Genuine);
        host.set_status(recorded.manager_pid, Status::Absent);

        let (backend, starts) = counting_backend();
        let manager = Manager::new(crate::manager_core::metadata::info(), backend)
            .with_legacy(runtime(&current, &path, &host));

        let first = call(&manager, Method::RouterMigrateLegacy).await;
        assert_eq!(first["error"]["code"], "OPERATION_TIMEOUT", "{first}");
        assert_eq!(first["error"]["message"], "router operation timed out");
        assert!(first["error"].get("details").is_none());
        assert_eq!(host.signals().len(), 2);

        let status = call(&manager, Method::RouterStatus).await;
        assert_eq!(status["result"]["state"], "start_failed", "{status}");
        assert_eq!(
            status["result"]["last_error"],
            "stage=state_reconcile code=OPERATION_TIMEOUT"
        );
        let logs = status["result"]["recent_logs"].as_array().unwrap();
        assert_eq!(logs.len(), 2, "{logs:?}");
        assert_eq!(logs[0], "stage=state_reconcile code=OPERATION_TIMEOUT");
        assert_eq!(
            logs[1],
            "legacy_migration generation=1 source_generation=0 source_protocol=3 pid=42018 signals=interrupt,kill outcome=stop_timeout attempted_at=2026-09-07T01:30:00Z"
        );

        let router_logs = call(&manager, Method::RouterLogs).await;
        let lines = router_logs["result"]["lines"].as_array().unwrap();
        assert!(
            lines
                .iter()
                .any(|line| line.as_str().unwrap().starts_with("legacy_migration ")),
            "{lines:?}"
        );

        let second = call(&manager, Method::RouterMigrateLegacy).await;
        assert_eq!(
            second["error"]["code"], "ROUTER_ALREADY_RUNNING",
            "{second}"
        );
        assert_eq!(host.signals().len(), 2, "retry must not re-signal");
        assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(state::read(&path).is_ok());

        let status = call(&manager, Method::RouterStatus).await;
        let logs = status["result"]["recent_logs"].as_array().unwrap();
        assert_eq!(logs[0], "stage=state_reconcile code=ROUTER_ALREADY_RUNNING");
        assert!(
            logs[1].as_str().unwrap().ends_with(" retry=refused"),
            "{logs:?}"
        );

        let diagnostics = call(&manager, Method::DiagnosticsCollect).await;
        let summary = diagnostics["result"]["summary"].as_str().unwrap();
        assert!(
            summary.contains("legacy_migration generation=1 source_generation=0 source_protocol=3 pid=42018 signals=interrupt,kill outcome=stop_timeout"),
            "{summary}"
        );
        assert!(!summary.contains("Program Files"), "{summary}");
        assert!(diagnostics["result"].get("manager_failure").is_none());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
