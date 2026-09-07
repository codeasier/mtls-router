//! Production desktop backend: the in-process router-core supervisor behind
//! Go `lifecycle.startDesktop` state reconciliation and `discovery`
//! classification.
//!
//! While the supervisor is bound the Tauri process *is* the router, so status
//! and trust derive from our own process identity. While it is not bound the
//! port is classified exactly like the Go manager did (stale records, legacy
//! desktop routers, compatible CLI routers, unknown occupants) so every
//! protocol v4 error code and state string survives the cutover. The router is
//! never adopted from another process: reclaim always fails closed and stop
//! only removes records this process wrote.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, SecondsFormat, Utc};

use crate::protocol::{ErrorCode, RouterOwner};
use crate::router_core::{ProxyLog, ProxyLogger, RouterSupervisor, RuntimePhase, SupervisorConfig};

use super::backend::fetch_health_status;
use super::discovery::{self, DiscoveryConfig, DiscoveryHost, Found, SystemHost};
use super::errors::{
    closed_error, lifecycle_error_from_supervisor, map_lifecycle_error, LifecycleError,
    StartupStage,
};
use super::legacy::{
    complete_desktop_state, installation_matches, manager_identity, manager_matches,
    recorded_process_absent, router_identity, verified_migratable, CurrentLineage, MigrationHost,
};
use super::lifecycle::Backend;
use super::process::{Identity as ProcessIdentity, ProcessError, SignalKind, Status};
use super::state::{self, RouterState, StateError};
use super::trustedrouter::{
    Discovery, Listener, RemoteVersion, RouterIdentity, StartOutcome, StartedIdentity,
    TrustedLifecycle,
};
use super::types::{Classification, DiscoverySnapshot, HealthInfo, StartedRouter, VersionInfo};

const LATEST_SESSION_MARKER: &str = ".latest-session";

#[derive(Clone, Debug)]
pub struct EmbeddedConfig {
    /// Session, installation, generation, deployment, protocol, and this
    /// process's identity (which is both manager and router).
    pub current: CurrentLineage,
    pub listener: Listener,
    pub desktop_state_path: PathBuf,
    pub cli_state_path: PathBuf,
    /// Go `DesktopLogPath`: `<data>/mtls-router.log`. Each start writes its
    /// access log to `<data>/mtls-router-logs/<date>/<time>.log`.
    pub log_base: PathBuf,
    pub version: String,
}

/// Current session access-log file shared with the supervisor's logger so a
/// restart can rotate to a new file without rebuilding the supervisor.
#[derive(Clone, Default)]
pub struct AccessLogTarget(Arc<Mutex<Option<PathBuf>>>);

impl AccessLogTarget {
    pub fn get(&self) -> Option<PathBuf> {
        self.0.lock().expect("log target lock").clone()
    }

    fn set(&self, path: PathBuf) {
        *self.0.lock().expect("log target lock") = Some(path);
    }
}

pub struct EmbeddedBackend {
    supervisor: RouterSupervisor,
    config: EmbeddedConfig,
    discovery: DiscoveryConfig,
    host: Arc<dyn DiscoveryHost>,
    identity: VersionInfo,
    recent: Mutex<String>,
    log_target: AccessLogTarget,
}

impl EmbeddedBackend {
    pub fn new(config: EmbeddedConfig, supervisor: SupervisorConfig) -> Self {
        Self::with_host(config, supervisor, Arc::new(SystemHost))
    }

    pub fn with_host(
        config: EmbeddedConfig,
        mut supervisor: SupervisorConfig,
        host: Arc<dyn DiscoveryHost>,
    ) -> Self {
        let identity = VersionInfo {
            version: supervisor.identity.version().to_owned(),
            deployment_id: supervisor.identity.deployment_id().to_owned(),
            management_protocol_version: supervisor
                .identity
                .management_protocol_version()
                .to_owned(),
        };
        let discovery = DiscoveryConfig::new(
            config.listener.router_base_url.clone(),
            config.listener.authority.clone(),
            config.desktop_state_path.clone(),
            config.cli_state_path.clone(),
            config.current.clone(),
        );
        let log_target = AccessLogTarget::default();
        if supervisor.logger.is_none() {
            supervisor.logger = Some(desktop_access_logger(log_target.clone()));
        }
        Self {
            supervisor: RouterSupervisor::spawn(supervisor),
            config,
            discovery,
            host,
            identity,
            recent: Mutex::new(String::new()),
            log_target,
        }
    }

    pub fn config(&self) -> &EmbeddedConfig {
        &self.config
    }

    pub fn supervisor(&self) -> &RouterSupervisor {
        &self.supervisor
    }

    fn own(&self) -> &ProcessIdentity {
        &self.config.current.manager
    }

    fn base_url(&self) -> &str {
        &self.config.listener.router_base_url
    }

    /// `Some(phase)` while our supervisor holds the listen socket.
    fn bound_phase(&self) -> Option<RuntimePhase> {
        let status = self.supervisor.status();
        status.bound.then_some(status.phase)
    }

    fn started(&self) -> StartedRouter {
        StartedRouter {
            owner: RouterOwner::Desktop,
            listen_addr: self.base_url().to_owned(),
            pid: u32::try_from(self.own().pid).ok(),
        }
    }

    fn owned_snapshot(&self, phase: RuntimePhase, health: Option<HealthInfo>) -> DiscoverySnapshot {
        let mut classification = match phase {
            RuntimePhase::Degraded => Classification::Degraded,
            _ => Classification::DesktopOwned,
        };
        if health.as_ref().is_some_and(|health| health.status != "ok") {
            classification = Classification::Degraded;
        }
        DiscoverySnapshot {
            classification,
            owner: Some(RouterOwner::Desktop.as_str().to_owned()),
            listen_addr: Some(self.base_url().to_owned()),
            pid: u32::try_from(self.own().pid).ok(),
            version: Some(self.identity.clone()),
            state_version: Some(self.identity.clone()),
            health,
            log_path: self.log_target.get(),
        }
    }

    /// Trusted-channel view of the in-process listener: `/version` reports our
    /// own pid, and our own identity is the router identity to validate.
    pub fn trusted_discovery(&self) -> Discovery {
        let Some(phase) = self.bound_phase() else {
            return discovery::discover_status(&self.discovery, self.host.as_ref())
                .trusted_discovery();
        };
        let own = self.own();
        Discovery {
            classification: match phase {
                RuntimePhase::Degraded => Classification::Degraded,
                _ => Classification::DesktopOwned,
            },
            owner: RouterOwner::Desktop.as_str().to_owned(),
            listen_addr: self.base_url().to_owned(),
            version: RemoteVersion {
                pid: own.pid,
                deployment_id: self.identity.deployment_id.clone(),
                management_protocol_version: self.identity.management_protocol_version.clone(),
            },
            state: RouterIdentity {
                pid: own.pid,
                owner: RouterOwner::Desktop.as_str().to_owned(),
                listen_addr: self.base_url().to_owned(),
                binary_path: own.executable.clone(),
                process_started_at: own.started_at.clone(),
                process_executable: own.executable.clone(),
                deployment_id: self.identity.deployment_id.clone(),
                management_protocol_version: self.identity.management_protocol_version.clone(),
            },
        }
    }

    fn own_state(&self, log_path: &Path) -> RouterState {
        let own = self.own();
        RouterState {
            pid: own.pid,
            listen_addr: self.base_url().to_owned(),
            binary_path: own.executable.clone(),
            log_path: log_path.to_string_lossy().into_owned(),
            started_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            process_started_at: own.started_at.clone(),
            process_executable: own.executable.clone(),
            owner: RouterOwner::Desktop.as_str().to_owned(),
            desktop_session_id: self.config.current.session_id.clone(),
            installation_id: self.config.current.installation_id.clone(),
            package_generation: self.config.current.package_generation,
            manager_pid: own.pid,
            manager_process_started_at: own.started_at.clone(),
            manager_process_executable: own.executable.clone(),
            manager_version: self.config.version.clone(),
            router_version: self.config.version.clone(),
            deployment_id: self.config.current.deployment_id.clone(),
            management_protocol_version: self.config.current.management_protocol_version.clone(),
        }
    }

    fn written_by_self(&self, value: &RouterState) -> bool {
        let own = self.own();
        value.owner == RouterOwner::Desktop.as_str()
            && router_identity(value) == *own
            && manager_matches(value, own)
    }

    fn remove_own_state(&self) {
        if self
            .host
            .read_state(&self.config.desktop_state_path)
            .is_ok_and(|value| self.written_by_self(&value))
        {
            let _ = fs::remove_file(&self.config.desktop_state_path);
        }
    }

    /// Go `startDesktop` desktop-state branch: a genuine recorded router is
    /// either an explicit-migration candidate or already running; a provably
    /// absent one is cleaned up; anything else stays fail closed.
    fn reconcile_recorded_state(&self) -> Result<(), LifecycleError> {
        let value = match self.host.read_state(&self.config.desktop_state_path) {
            Ok(value) => value,
            Err(StateError::NotFound) => return Ok(()),
            Err(_) => return Err(staged(ErrorCode::RouterStateStale)),
        };
        if value.owner != RouterOwner::Desktop.as_str() {
            return Ok(());
        }
        let predicates = PredicateHost(self.host.as_ref());
        let status = self
            .host
            .validate(&router_identity(&value), &value.binary_path)
            .unwrap_or(Status::Stale);
        if status == Status::Genuine {
            if self.written_by_self(&value) {
                // Our own record without a bound supervisor is a leftover of
                // this process; the socket is free to rebind.
                let _ = fs::remove_file(&self.config.desktop_state_path);
                return Ok(());
            }
            if verified_migratable(&self.config.current, &value, &predicates) {
                return Err(staged(ErrorCode::RouterLegacyManaged));
            }
            return Err(staged(ErrorCode::RouterAlreadyRunning));
        }
        if recorded_process_absent(&value, status, &predicates) {
            return match fs::remove_file(&self.config.desktop_state_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(staged(ErrorCode::RouterStateStale)),
            };
        }
        Ok(())
    }

    /// Go `startDesktop` discovery branch. `Ok(Some)` reuses a compatible
    /// external router without starting ours.
    fn reconcile_port(&self) -> Result<Option<StartedRouter>, LifecycleError> {
        let found = discovery::discover_desktop_startup(&self.discovery, self.host.as_ref());
        match found.classification {
            Classification::Absent => Ok(None),
            Classification::ExternalCompatible => {
                let state = found.state.unwrap_or_default();
                Ok(Some(StartedRouter {
                    owner: RouterOwner::Cli,
                    listen_addr: if state.listen_addr.is_empty() {
                        found.listen_addr
                    } else {
                        state.listen_addr
                    },
                    pid: u32::try_from(state.pid).ok(),
                }))
            }
            Classification::Degraded if found.owner == "cli" => {
                Err(LifecycleError::new(ErrorCode::RouterDegraded))
            }
            Classification::Degraded | Classification::DesktopOwned => {
                Err(staged(ErrorCode::RouterAlreadyRunning))
            }
            Classification::UnknownOccupant => Err(LifecycleError::new(ErrorCode::PortOccupied)),
            Classification::Stale => Err(staged(ErrorCode::RouterStateStale)),
            Classification::LegacyManaged => {
                let migratable = found.state.as_ref().is_some_and(|value| {
                    verified_migratable(
                        &self.config.current,
                        value,
                        &PredicateHost(self.host.as_ref()),
                    )
                });
                Err(staged(if migratable {
                    ErrorCode::RouterLegacyManaged
                } else {
                    ErrorCode::RouterAlreadyRunning
                }))
            }
        }
    }

    fn classify(&self, include_health: bool) -> Found {
        if include_health {
            discovery::discover(&self.discovery, self.host.as_ref())
        } else {
            discovery::discover_status(&self.discovery, self.host.as_ref())
        }
    }
}

fn staged(code: ErrorCode) -> LifecycleError {
    LifecycleError::new(code).with_stage(StartupStage::StateReconcile)
}

/// Lets the legacy predicates run against the discovery host. Signaling is
/// never reachable from the backend; only `LegacyRuntime` may terminate.
struct PredicateHost<'a>(&'a dyn DiscoveryHost);

impl MigrationHost for PredicateHost<'_> {
    fn inspect(&self, pid: i32) -> Result<ProcessIdentity, ProcessError> {
        self.0.inspect(pid)
    }

    fn validate(&self, identity: &ProcessIdentity, binary: &str) -> Result<Status, ProcessError> {
        self.0.validate(identity, binary)
    }

    fn signal(
        &self,
        _identity: &ProcessIdentity,
        _binary: &str,
        _kind: SignalKind,
    ) -> Result<(), ProcessError> {
        Err(ProcessError::IdentityMismatch)
    }

    fn port_open(&self, _authority: &str) -> bool {
        true
    }

    fn sleep(&self, _delay: Duration) -> bool {
        false
    }
}

impl Backend for EmbeddedBackend {
    async fn start(&self, owner: RouterOwner) -> Result<StartedRouter, LifecycleError> {
        if owner != RouterOwner::Desktop {
            return Err(LifecycleError::new(ErrorCode::InvalidParams));
        }
        if self.bound_phase().is_some() {
            return Ok(self.started());
        }
        self.reconcile_recorded_state()?;
        if let Some(external) = self.reconcile_port()? {
            return Ok(external);
        }
        let log_path = prepare_session_log(&self.config.log_base, Local::now()).map_err(|_| {
            LifecycleError::new(ErrorCode::RouterStartFailed).with_stage(StartupStage::LogDirectory)
        })?;
        self.log_target.set(log_path.clone());
        if let Err(error) = self.supervisor.start().await {
            let mapped = lifecycle_error_from_supervisor(error);
            *self.recent.lock().expect("recent lock") = mapped.recent_output.clone();
            return Err(mapped);
        }
        *self.recent.lock().expect("recent lock") = String::new();
        let _ = record_latest_session(&self.config.log_base, &log_path);
        if state::write(&self.config.desktop_state_path, &self.own_state(&log_path)).is_err() {
            let _ = self.supervisor.stop().await;
            return Err(LifecycleError::new(ErrorCode::RouterStartFailed)
                .with_stage(StartupStage::StatePersist));
        }
        Ok(self.started())
    }

    /// Go `reclaimLocked` failure branch: an in-process router can never be
    /// adopted from another process, so this only reports why not.
    async fn reclaim(&self) -> Result<StartedRouter, LifecycleError> {
        let Ok(value) = self.host.read_state(&self.config.desktop_state_path) else {
            return Err(LifecycleError::new(ErrorCode::RouterStateStale));
        };
        if value.owner != RouterOwner::Desktop.as_str()
            || !complete_desktop_state(&value)
            || !installation_matches(&self.config.current, &value)
        {
            return Err(LifecycleError::new(ErrorCode::RouterStateStale));
        }
        let previous = manager_identity(&value);
        match self
            .host
            .validate(&previous, &value.manager_process_executable)
        {
            Ok(Status::Absent) => Err(LifecycleError::new(ErrorCode::RouterStateStale)),
            _ => Err(LifecycleError::new(ErrorCode::RouterAlreadyRunning)),
        }
    }

    async fn stop(&self) -> Result<(), LifecycleError> {
        if self.bound_phase().is_some() {
            self.supervisor
                .stop()
                .await
                .map_err(lifecycle_error_from_supervisor)?;
            self.remove_own_state();
            return Ok(());
        }
        match self.host.read_state(&self.config.desktop_state_path) {
            Err(StateError::NotFound) => Err(LifecycleError::new(ErrorCode::RouterNotFound)),
            Err(_) => Err(LifecycleError::new(ErrorCode::RouterStateStale)),
            Ok(value) if self.written_by_self(&value) => {
                let _ = fs::remove_file(&self.config.desktop_state_path);
                Ok(())
            }
            Ok(_) => Err(LifecycleError::new(ErrorCode::RouterNotOwned)),
        }
    }

    async fn discover_status(&self) -> DiscoverySnapshot {
        match self.bound_phase() {
            Some(phase) => self.owned_snapshot(phase, None),
            None => self.classify(false).snapshot(),
        }
    }

    async fn discover_health(&self) -> DiscoverySnapshot {
        match self.bound_phase() {
            Some(phase) => {
                let health = fetch_health_status(&self.config.listener.authority)
                    .await
                    .map(|status| HealthInfo { status });
                self.owned_snapshot(phase, health)
            }
            None => self.classify(true).snapshot(),
        }
    }

    fn recent_output(&self) -> String {
        self.recent.lock().expect("recent lock").clone()
    }

    fn log_path(&self) -> Option<PathBuf> {
        self.log_target.get()
    }
}

impl<B: Backend> Backend for Arc<B> {
    async fn start(&self, owner: RouterOwner) -> Result<StartedRouter, LifecycleError> {
        (**self).start(owner).await
    }

    async fn reclaim(&self) -> Result<StartedRouter, LifecycleError> {
        (**self).reclaim().await
    }

    async fn stop(&self) -> Result<(), LifecycleError> {
        (**self).stop().await
    }

    async fn discover_status(&self) -> DiscoverySnapshot {
        (**self).discover_status().await
    }

    async fn discover_health(&self) -> DiscoverySnapshot {
        (**self).discover_health().await
    }

    fn recent_output(&self) -> String {
        (**self).recent_output()
    }

    fn log_path(&self) -> Option<PathBuf> {
        (**self).log_path()
    }
}

/// Synchronous trusted-channel start (Go `lifecycle.Start` inside the
/// coordinator). The session thread is inside its own runtime, so the async
/// start runs on a scoped helper thread with a fresh runtime.
pub struct EmbeddedLifecycle(pub Arc<EmbeddedBackend>);

impl TrustedLifecycle for EmbeddedLifecycle {
    fn start(&self, owner: RouterOwner, deadline: Instant) -> StartOutcome {
        let backend = self.0.clone();
        let budget = deadline.saturating_duration_since(Instant::now());
        let outcome = std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .ok()?;
                    runtime
                        .block_on(tokio::time::timeout(budget, backend.start(owner)))
                        .ok()
                })
                .join()
        });
        let identity = |started: &StartedRouter| StartedIdentity {
            pid: started
                .pid
                .and_then(|pid| i32::try_from(pid).ok())
                .unwrap_or_default(),
            owner: started.owner.as_str().to_owned(),
            listen_addr: started.listen_addr.clone(),
            deployment_id: self.0.identity.deployment_id.clone(),
            management_protocol_version: self.0.identity.management_protocol_version.clone(),
        };
        match outcome {
            Ok(Some(Ok(started))) => StartOutcome {
                state: identity(&started),
                error: None,
            },
            Ok(Some(Err(error))) => StartOutcome {
                state: StartedIdentity::default(),
                error: Some(map_lifecycle_error(&error)),
            },
            _ => StartOutcome {
                state: StartedIdentity::default(),
                error: Some(closed_error(
                    ErrorCode::OperationTimeout,
                    "router start timed out",
                )),
            },
        }
    }
}

/// Go `background.sessionLogParts`: `<dir>/<name>-logs`, extension `.log`.
fn session_log_parts(base: &Path) -> (PathBuf, String) {
    let extension = base
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_else(|| ".log".to_owned());
    let name = base
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && *value != ".")
        .unwrap_or("mtls-router");
    let directory = base
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join(format!("{name}-logs"));
    (directory, extension)
}

/// Go `background.PrepareSessionLogPath`: one exclusive file per launch,
/// grouped by local date, restricted to the current user.
pub fn prepare_session_log(base: &Path, started_at: DateTime<Local>) -> io::Result<PathBuf> {
    let (directory, extension) = session_log_parts(base);
    let day_dir = directory.join(started_at.format("%Y-%m-%d").to_string());
    fs::create_dir_all(&day_dir)?;
    restrict_dir(&day_dir)?;
    let start_name = started_at.format("%H-%M-%S").to_string();
    for sequence in 1u32.. {
        let file_name = if sequence == 1 {
            format!("{start_name}{extension}")
        } else {
            format!("{start_name}-{sequence}{extension}")
        };
        let path = day_dir.join(file_name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    unreachable!("sequence space exhausted")
}

/// Go `background.RecordLatestSessionLogPath`: marker holds `<date>/<file>`.
pub fn record_latest_session(base: &Path, session: &Path) -> io::Result<()> {
    let (directory, _) = session_log_parts(base);
    let relative = session
        .strip_prefix(&directory)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "session log outside log dir"))?;
    if relative.components().count() != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "session log must be <date>/<file>",
        ));
    }
    let encoded = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let marker = directory.join(LATEST_SESSION_MARKER);
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(marker)?.write_all(encoded.as_bytes())
}

fn restrict_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

/// Appends whitelist-only access lines (Go `internal/log` slog text shape) to
/// the current session log. Write failures are ignored so logging can never
/// take the router down.
pub fn desktop_access_logger(target: AccessLogTarget) -> ProxyLogger {
    Arc::new(move |entry: ProxyLog| {
        let Some(path) = target.get() else {
            return;
        };
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let line = match entry {
            ProxyLog::Access(record) => format!(
                "time={now} level=INFO msg=access method={} path={} status={} bytes={} latency={}ms\n",
                record.method, record.path, record.status, record.bytes, record.latency
            ),
            ProxyLog::RequestFailed {
                method,
                path,
                status,
                reason,
            } => format!(
                "time={now} level=ERROR msg=\"proxy request failed\" method={method} path={path} status={status} reason={reason}\n"
            ),
            ProxyLog::StreamFailed => {
                format!("time={now} level=ERROR msg=\"proxy stream failed\"\n")
            }
        };
        let mut options = OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if let Ok(mut file) = options.open(&path) {
            let _ = file.write_all(line.as_bytes());
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager_core::httpget::{DirectTransport, HttpTransport};
    use crate::manager_core::trustedrouter::normalize_listener;
    use crate::redaction::AccessLogRecord;
    use crate::router_core::{
        test_support::{start_mtls_server, MtlsServerOptions, TestCerts, TestMtlsServer},
        BuildInfo, RouterDefaults, RouterEnv, RouterFlags, SupervisorLimits,
    };
    use serde_json::{json, Value};
    use std::collections::HashMap;

    struct FakeHost {
        states: Mutex<HashMap<PathBuf, Result<RouterState, StateError>>>,
        statuses: Mutex<HashMap<i32, Status>>,
        default_status: Status,
        port_open: Mutex<bool>,
        version: Mutex<Option<Value>>,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                states: Mutex::new(HashMap::new()),
                statuses: Mutex::new(HashMap::new()),
                default_status: Status::Absent,
                port_open: Mutex::new(false),
                version: Mutex::new(None),
            }
        }

        fn serve(&self, pid: i32, deployment: &str, protocol: &str) {
            *self.port_open.lock().unwrap() = true;
            *self.version.lock().unwrap() = Some(json!({
                "version": "v1",
                "pid": pid,
                "deployment_id": deployment,
                "management_protocol_version": protocol,
            }));
        }

        fn status(&self, pid: i32, status: Status) {
            self.statuses.lock().unwrap().insert(pid, status);
        }

        fn state(&self, path: &Path, value: RouterState) {
            self.states
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), Ok(value));
        }
    }

    impl DiscoveryHost for FakeHost {
        fn read_state(&self, path: &Path) -> Result<RouterState, StateError> {
            if let Some(value) = self.states.lock().unwrap().get(path) {
                return value.clone();
            }
            state::read(path)
        }

        fn inspect(&self, _pid: i32) -> Result<ProcessIdentity, ProcessError> {
            Err(ProcessError::NotFound)
        }

        fn validate(
            &self,
            identity: &ProcessIdentity,
            _binary: &str,
        ) -> Result<Status, ProcessError> {
            Ok(self
                .statuses
                .lock()
                .unwrap()
                .get(&identity.pid)
                .copied()
                .unwrap_or(self.default_status))
        }

        fn port_open(&self, _authority: &str, _timeout: Duration) -> bool {
            *self.port_open.lock().unwrap()
        }

        fn get_json(&self, _authority: &str, path: &str, _timeout: Duration) -> Option<Value> {
            match path {
                "/version" => self.version.lock().unwrap().clone(),
                "/health" => Some(json!({ "status": "ok" })),
                _ => None,
            }
        }
    }

    fn free_listen_addr() -> String {
        let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = reserved.local_addr().unwrap().to_string();
        drop(reserved);
        addr
    }

    fn own_identity() -> ProcessIdentity {
        super::super::session::current_identity().expect("own identity")
    }

    fn lineage(own: &ProcessIdentity) -> CurrentLineage {
        CurrentLineage {
            session_id: "embedded-session".into(),
            installation_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".into(),
            package_generation: 2,
            deployment_id: "deploy-test".into(),
            management_protocol_version: "4".into(),
            manager: own.clone(),
        }
    }

    struct Fixture {
        dir: PathBuf,
        listen_addr: String,
        config: EmbeddedConfig,
        upstream: Option<TestMtlsServer>,
    }

    async fn fixture(with_upstream: bool) -> Fixture {
        let dir =
            std::env::temp_dir().join(format!("mtls-embedded-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        let listen_addr = free_listen_addr();
        let listener = normalize_listener(&listen_addr).expect("listener");
        let own = own_identity();
        let config = EmbeddedConfig {
            current: lineage(&own),
            listener,
            desktop_state_path: dir.join("desktop-state.json"),
            cli_state_path: dir.join("setup-state.json"),
            log_base: dir.join("mtls-router.log"),
            version: "v9.9.9".into(),
        };
        let upstream = if with_upstream {
            Some(start_mtls_server(MtlsServerOptions::default()).await)
        } else {
            None
        };
        Fixture {
            dir,
            listen_addr,
            config,
            upstream,
        }
    }

    impl Fixture {
        fn backend(&self, host: Arc<dyn DiscoveryHost>) -> EmbeddedBackend {
            let mut defaults = RouterDefaults::default();
            defaults.listen_addr = self.listen_addr.clone();
            defaults.timeout = Duration::from_secs(1);
            let materials = match &self.upstream {
                Some(upstream) => {
                    defaults.upstream_url = upstream.url.clone();
                    upstream.certs.materials()
                }
                None => {
                    defaults.upstream_url = "https://127.0.0.1:1".into();
                    TestCerts::generate().materials()
                }
            };
            EmbeddedBackend::with_host(
                self.config.clone(),
                SupervisorConfig {
                    defaults,
                    env: RouterEnv::default(),
                    flags: RouterFlags::default(),
                    materials,
                    identity: BuildInfo::new("v9.9.9", "commit", "date", "deploy-test"),
                    limits: SupervisorLimits {
                        command_timeout: Duration::from_secs(5),
                        ..SupervisorLimits::default()
                    },
                    logger: None,
                },
                host,
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn release_state(name: &str) -> RouterState {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../internal/manager/testdata")
            .join(name);
        state::read(path).expect("release golden")
    }

    #[tokio::test]
    async fn start_binds_records_own_identity_and_stop_removes_record() {
        let fixture = fixture(true).await;
        let host = Arc::new(FakeHost::new());
        let backend = fixture.backend(host);
        let own = own_identity();

        assert_eq!(
            backend.discover_status().await.classification,
            Classification::Absent
        );
        assert_eq!(backend.log_path(), None);
        let started = backend.start(RouterOwner::Desktop).await.expect("start");
        assert_eq!(started.owner, RouterOwner::Desktop);
        assert_eq!(started.listen_addr, fixture.config.listener.router_base_url);
        assert_eq!(started.pid, u32::try_from(own.pid).ok());

        let recorded = state::read(&fixture.config.desktop_state_path).expect("state");
        assert_eq!(recorded.pid, own.pid);
        assert_eq!(recorded.manager_pid, own.pid);
        assert_eq!(recorded.process_started_at, own.started_at);
        assert_eq!(recorded.process_executable, own.executable);
        assert_eq!(recorded.binary_path, own.executable);
        assert_eq!(recorded.owner, "desktop");
        assert_eq!(recorded.desktop_session_id, "embedded-session");
        assert_eq!(recorded.package_generation, 2);
        assert_eq!(recorded.management_protocol_version, "4");
        assert_eq!(
            recorded.listen_addr,
            fixture.config.listener.router_base_url
        );
        let log_path = backend.log_path().expect("session log");
        assert_eq!(recorded.log_path, log_path.to_string_lossy());
        assert!(log_path.starts_with(fixture.dir.join("mtls-router-logs")));
        assert_eq!(
            fs::read_to_string(fixture.dir.join("mtls-router-logs").join(".latest-session"))
                .unwrap()
                .matches('/')
                .count(),
            1
        );

        let status = backend.discover_status().await;
        assert_eq!(status.classification, Classification::DesktopOwned);
        assert_eq!(status.pid, u32::try_from(own.pid).ok());
        assert_eq!(status.log_path, Some(log_path.clone()));
        let health = backend.discover_health().await;
        assert_eq!(health.classification, Classification::DesktopOwned);
        assert_eq!(health.health.unwrap().status, "ok");
        let proxied = {
            let authority = fixture.config.listener.authority.clone();
            tokio::task::spawn_blocking(move || {
                let mut transport = DirectTransport { authority };
                transport
                    .get(
                        "/v1/probe?api_key=sk-embedded-canary-12345678",
                        None,
                        Instant::now() + Duration::from_secs(5),
                        64 * 1024,
                    )
                    .expect("proxied request")
            })
            .await
            .unwrap()
        };
        assert_eq!(proxied.status, 204);
        let logged = fs::read_to_string(&log_path).unwrap();
        assert!(
            logged.contains("msg=access method=GET path=/v1/probe status=204"),
            "{logged}"
        );
        assert!(!logged.contains("sk-embedded"), "{logged}");

        let trusted = backend.trusted_discovery();
        assert_eq!(trusted.classification, Classification::DesktopOwned);
        assert_eq!(trusted.version.pid, own.pid);
        assert_eq!(trusted.state.pid, own.pid);
        assert_eq!(trusted.state.process_started_at, own.started_at);
        assert_eq!(
            trusted.state.listen_addr,
            fixture.config.listener.router_base_url
        );
        assert!(matches!(
            crate::manager_core::process::validate(
                &ProcessIdentity {
                    pid: trusted.state.pid,
                    started_at: trusted.state.process_started_at.clone(),
                    executable: trusted.state.process_executable.clone(),
                },
                &trusted.state.binary_path
            ),
            Ok(Status::Genuine)
        ));

        let again = backend
            .start(RouterOwner::Desktop)
            .await
            .expect("idempotent");
        assert_eq!(again, started);

        backend.stop().await.expect("stop");
        assert_eq!(
            state::read(&fixture.config.desktop_state_path),
            Err(StateError::NotFound)
        );
        assert_eq!(
            backend.discover_status().await.classification,
            Classification::Absent
        );
        assert_eq!(
            backend.stop().await.unwrap_err().code,
            ErrorCode::RouterNotFound
        );
    }

    #[tokio::test]
    async fn cli_owner_and_probe_failure_are_closed_errors() {
        let fixture = fixture(false).await;
        let backend = fixture.backend(Arc::new(FakeHost::new()));
        assert_eq!(
            backend.start(RouterOwner::Cli).await.unwrap_err().code,
            ErrorCode::InvalidParams
        );
        let error = backend.start(RouterOwner::Desktop).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterStartFailed);
        assert_eq!(error.stage, Some(StartupStage::Readiness));
        assert!(backend.recent_output().starts_with("reason="));
        assert!(!backend.recent_output().contains("127.0.0.1:1"));
        assert_eq!(
            state::read(&fixture.config.desktop_state_path),
            Err(StateError::NotFound)
        );
    }

    #[tokio::test]
    async fn port_classification_maps_to_go_start_codes_without_binding() {
        let fixture = fixture(false).await;

        let host = Arc::new(FakeHost::new());
        host.serve(4242, "other-deploy", "4");
        let backend = fixture.backend(host);
        let error = backend.start(RouterOwner::Desktop).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::PortOccupied);
        assert!(!error.should_latch());
        assert_eq!(
            backend.discover_status().await.classification,
            Classification::UnknownOccupant
        );

        let host = Arc::new(FakeHost::new());
        let cli = RouterState {
            pid: 515,
            owner: "cli".into(),
            listen_addr: fixture.config.listener.router_base_url.clone(),
            binary_path: "/usr/local/bin/mtls-router".into(),
            process_started_at: "cli-start".into(),
            process_executable: "/usr/local/bin/mtls-router".into(),
            router_version: "v8".into(),
            deployment_id: "deploy-test".into(),
            management_protocol_version: "4".into(),
            ..RouterState::default()
        };
        host.state(&fixture.config.cli_state_path, cli);
        host.status(515, Status::Genuine);
        host.serve(515, "deploy-test", "4");
        let backend = fixture.backend(host.clone());
        let reused = backend.start(RouterOwner::Desktop).await.expect("reuse");
        assert_eq!(reused.owner, RouterOwner::Cli);
        assert_eq!(reused.pid, Some(515));
        assert!(!backend.supervisor().status().bound);
        assert_eq!(
            backend.discover_status().await.classification,
            Classification::ExternalCompatible
        );

        *host.port_open.lock().unwrap() = false;
        let error = backend.start(RouterOwner::Desktop).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::RouterDegraded);
        assert!(!error.should_latch());
    }

    #[tokio::test]
    async fn legacy_desktop_router_requires_explicit_migration() {
        let fixture = fixture(false).await;
        for name in ["desktop-state-v0.1.8.json", "desktop-state-v0.2.0.json"] {
            let value = release_state(name);
            let host = Arc::new(FakeHost::new());
            host.state(&fixture.config.desktop_state_path, value.clone());
            host.status(value.pid, Status::Genuine);
            host.status(value.manager_pid, Status::Absent);
            host.serve(
                value.pid,
                &value.deployment_id,
                &value.management_protocol_version,
            );
            let backend = fixture.backend(host.clone());
            let error = backend.start(RouterOwner::Desktop).await.unwrap_err();
            assert_eq!(error.code, ErrorCode::RouterLegacyManaged, "{name}");
            assert_eq!(error.stage, Some(StartupStage::StateReconcile));
            assert!(!backend.supervisor().status().bound);
            let status = backend.discover_status().await;
            assert_eq!(status.classification, Classification::LegacyManaged);
            assert_eq!(status.owner.as_deref(), Some("desktop"));
            assert_eq!(status.pid, None);

            host.status(value.manager_pid, Status::Genuine);
            let error = backend.start(RouterOwner::Desktop).await.unwrap_err();
            assert_eq!(error.code, ErrorCode::RouterAlreadyRunning, "{name}");
            assert_eq!(
                backend.reclaim().await.unwrap_err().code,
                ErrorCode::RouterStateStale,
                "legacy record lacks installation lineage"
            );
            assert_eq!(
                backend.stop().await.unwrap_err().code,
                ErrorCode::RouterNotOwned
            );
        }
    }

    #[tokio::test]
    async fn absent_recorded_process_is_cleaned_before_binding() {
        let fixture = fixture(true).await;
        let own = own_identity();
        let mut stale = release_state("desktop-state-v0.2.0.json");
        stale.installation_id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".into();
        stale.package_generation = 2;
        stale.deployment_id = "deploy-test".into();
        stale.management_protocol_version = "4".into();
        state::write(&fixture.config.desktop_state_path, &stale).unwrap();
        let host = Arc::new(FakeHost::new());
        host.status(stale.pid, Status::Absent);
        let backend = fixture.backend(host);
        let started = backend.start(RouterOwner::Desktop).await.expect("start");
        assert_eq!(started.pid, u32::try_from(own.pid).ok());
        let recorded = state::read(&fixture.config.desktop_state_path).unwrap();
        assert_eq!(recorded.pid, own.pid);
        backend.stop().await.unwrap();
    }

    #[tokio::test]
    async fn reclaim_fails_closed_with_go_codes() {
        let fixture = fixture(false).await;
        let host = Arc::new(FakeHost::new());
        let backend = fixture.backend(host.clone());
        assert_eq!(
            backend.reclaim().await.unwrap_err().code,
            ErrorCode::RouterStateStale
        );
        let mut value = release_state("desktop-state-v0.2.0.json");
        value.installation_id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".into();
        value.package_generation = 2;
        host.state(&fixture.config.desktop_state_path, value.clone());
        host.status(value.manager_pid, Status::Genuine);
        assert_eq!(
            backend.reclaim().await.unwrap_err().code,
            ErrorCode::RouterAlreadyRunning
        );
        host.status(value.manager_pid, Status::Absent);
        assert_eq!(
            backend.reclaim().await.unwrap_err().code,
            ErrorCode::RouterStateStale
        );
    }

    #[test]
    fn session_log_layout_matches_go_background_package() {
        let dir = std::env::temp_dir().join(format!(
            "mtls-session-log-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let base = dir.join("mtls-router.log");
        let started = Local::now();
        let first = prepare_session_log(&base, started).unwrap();
        let second = prepare_session_log(&base, started).unwrap();
        let day = started.format("%Y-%m-%d").to_string();
        let time = started.format("%H-%M-%S").to_string();
        assert_eq!(
            first,
            dir.join("mtls-router-logs")
                .join(&day)
                .join(format!("{time}.log"))
        );
        assert_eq!(
            second,
            dir.join("mtls-router-logs")
                .join(&day)
                .join(format!("{time}-2.log"))
        );
        record_latest_session(&base, &second).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("mtls-router-logs").join(".latest-session")).unwrap(),
            format!("{day}/{time}-2.log")
        );
        assert!(record_latest_session(&base, &dir.join("elsewhere.log")).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(first.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&first).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn access_logger_writes_whitelisted_slog_lines_only() {
        let dir =
            std::env::temp_dir().join(format!("mtls-access-log-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        let target = AccessLogTarget::default();
        let logger = desktop_access_logger(target.clone());
        logger(ProxyLog::StreamFailed);
        assert!(
            fs::read_dir(&dir).unwrap().next().is_none(),
            "no target, no file"
        );
        let path = dir.join("session.log");
        target.set(path.clone());
        logger(ProxyLog::Access(AccessLogRecord::new(
            "POST",
            "/v1/messages?api_key=sk-access-canary-12345678",
            200,
            42,
            Duration::from_millis(7),
        )));
        logger(ProxyLog::RequestFailed {
            method: "GET".into(),
            path: "/v1/fail".into(),
            status: 502,
            reason: "upstream_failure",
        });
        logger(ProxyLog::StreamFailed);
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines[0].contains(
            "level=INFO msg=access method=POST path=/v1/messages status=200 bytes=42 latency=7ms"
        ));
        assert!(!text.contains("sk-access"), "{text}");
        assert!(lines[1].contains(
            "level=ERROR msg=\"proxy request failed\" method=GET path=/v1/fail status=502 reason=upstream_failure"
        ));
        assert!(lines[2].ends_with("level=ERROR msg=\"proxy stream failed\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(dir);
    }
}
