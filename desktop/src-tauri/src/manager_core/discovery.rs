//! Classifies whoever is listening on the router endpoint.
//!
//! Frozen against Go `internal/manager/discovery`: HTTP `/version` (and
//! optionally `/health`) metadata is correlated with the desktop and CLI
//! state files and with OS process identity. Any missing source degrades the
//! classification instead of trusting the endpoint alone. The embedded
//! backend consults this only while its own supervisor is not bound.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use super::httpget::{DirectTransport, HttpTransport};
use super::legacy::{
    complete_process, generation_compatible, installation_matches, manager_identity,
    router_identity, supported_legacy_source, CurrentLineage,
};
use super::process::{self, Identity as ProcessIdentity, ProcessError, Status};
use super::state::{self, RouterState, StateError};
use super::trustedrouter::{self, RemoteVersion as TrustedVersion, RouterIdentity};
use super::types::{Classification, DiscoverySnapshot, HealthInfo, VersionInfo};

pub const DEFAULT_PORT_TIMEOUT: Duration = Duration::from_millis(250);
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(1);
pub const DEFAULT_HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(11);
const MAX_ENDPOINT_BODY: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct DiscoveryConfig {
    /// `http://127.0.0.1:19099` — reported as `listen_addr`, compared with
    /// recorded state addresses.
    pub base_url: String,
    /// `127.0.0.1:19099` — dialed for the port probe and endpoint requests.
    pub authority: String,
    pub desktop_state_path: PathBuf,
    pub cli_state_path: PathBuf,
    pub current: CurrentLineage,
    pub port_timeout: Duration,
    pub request_timeout: Duration,
    pub health_request_timeout: Duration,
}

impl DiscoveryConfig {
    pub fn new(
        base_url: impl Into<String>,
        authority: impl Into<String>,
        desktop_state_path: PathBuf,
        cli_state_path: PathBuf,
        current: CurrentLineage,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            authority: authority.into(),
            desktop_state_path,
            cli_state_path,
            current,
            port_timeout: DEFAULT_PORT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            health_request_timeout: DEFAULT_HEALTH_REQUEST_TIMEOUT,
        }
    }
}

/// Injectable OS/network edge so classification is testable without live
/// processes or listeners.
pub trait DiscoveryHost: Send + Sync {
    fn read_state(&self, path: &Path) -> Result<RouterState, StateError>;
    fn inspect(&self, pid: i32) -> Result<ProcessIdentity, ProcessError>;
    fn validate(&self, identity: &ProcessIdentity, binary: &str) -> Result<Status, ProcessError>;
    fn port_open(&self, authority: &str, timeout: Duration) -> bool;
    /// Bounded loopback GET; `None` for any transport, status, or decode
    /// failure. Errors never carry endpoint details.
    fn get_json(&self, authority: &str, path: &str, timeout: Duration) -> Option<Value>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemHost;

impl DiscoveryHost for SystemHost {
    fn read_state(&self, path: &Path) -> Result<RouterState, StateError> {
        state::read(path)
    }

    fn inspect(&self, pid: i32) -> Result<ProcessIdentity, ProcessError> {
        process::inspect(pid)
    }

    fn validate(&self, identity: &ProcessIdentity, binary: &str) -> Result<Status, ProcessError> {
        process::validate(identity, binary)
    }

    fn port_open(&self, authority: &str, timeout: Duration) -> bool {
        let Ok(mut addrs) = authority.to_socket_addrs() else {
            return false;
        };
        let Some(addr) = addrs.next() else {
            return false;
        };
        TcpStream::connect_timeout(&addr, timeout).is_ok()
    }

    fn get_json(&self, authority: &str, path: &str, timeout: Duration) -> Option<Value> {
        let mut transport = DirectTransport {
            authority: authority.to_owned(),
        };
        let response = transport
            .get(path, None, Instant::now() + timeout, MAX_ENDPOINT_BODY)
            .ok()?;
        if response.status != 200 {
            return None;
        }
        serde_json::from_slice(&response.body).ok()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct RemoteVersion {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub pid: i32,
    #[serde(default)]
    pub deployment_id: String,
    #[serde(default)]
    pub management_protocol_version: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct RemoteHealth {
    #[serde(default)]
    status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub classification: Classification,
    pub owner: String,
    pub listen_addr: String,
    pub version: RemoteVersion,
    pub health: String,
    pub state: Option<RouterState>,
}

impl Found {
    fn unknown(base_url: &str) -> Self {
        Self {
            classification: Classification::UnknownOccupant,
            owner: String::new(),
            listen_addr: base_url.to_owned(),
            version: RemoteVersion::default(),
            health: String::new(),
            state: None,
        }
    }

    pub fn trusted(&self) -> bool {
        self.classification.is_trusted()
    }

    /// Go `statusResult` / `trusted*` helpers: process and log details are
    /// exposed only for trusted classifications.
    pub fn snapshot(&self) -> DiscoverySnapshot {
        let state = self.state.as_ref().filter(|_| self.trusted());
        DiscoverySnapshot {
            classification: self.classification,
            owner: nonempty(&self.owner),
            listen_addr: Some(self.listen_addr.clone()),
            pid: state.and_then(|value| u32::try_from(value.pid).ok()),
            version: (!self.version.version.is_empty()).then(|| VersionInfo {
                version: self.version.version.clone(),
                deployment_id: self.version.deployment_id.clone(),
                management_protocol_version: self.version.management_protocol_version.clone(),
            }),
            state_version: state.map(|value| VersionInfo {
                version: value.router_version.clone(),
                deployment_id: value.deployment_id.clone(),
                management_protocol_version: value.management_protocol_version.clone(),
            }),
            health: nonempty(&self.health).map(|status| HealthInfo { status }),
            log_path: state
                .map(|value| value.log_path.as_str())
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        }
    }

    /// Shape consumed by the trusted-router channel before it sends an API key.
    pub fn trusted_discovery(&self) -> trustedrouter::Discovery {
        let state = self.state.clone().unwrap_or_default();
        trustedrouter::Discovery {
            classification: self.classification,
            owner: self.owner.clone(),
            listen_addr: self.listen_addr.clone(),
            version: TrustedVersion {
                pid: self.version.pid,
                deployment_id: self.version.deployment_id.clone(),
                management_protocol_version: self.version.management_protocol_version.clone(),
            },
            state: RouterIdentity {
                pid: state.pid,
                owner: state.owner,
                listen_addr: state.listen_addr,
                binary_path: state.binary_path,
                process_started_at: state.process_started_at,
                process_executable: state.process_executable,
                deployment_id: state.deployment_id,
                management_protocol_version: state.management_protocol_version,
            },
        }
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

/// Go `Discover`: includes `/health`.
pub fn discover(config: &DiscoveryConfig, host: &dyn DiscoveryHost) -> Found {
    classify(config, host, true, false)
}

/// Go `DiscoverStatus`: `/version` and state files only.
pub fn discover_status(config: &DiscoveryConfig, host: &dyn DiscoveryHost) -> Found {
    classify(config, host, false, false)
}

/// Go `DiscoverStartupStatus` for a desktop owner: a stale CLI record must
/// not block desktop startup while the port is free.
pub fn discover_desktop_startup(config: &DiscoveryConfig, host: &dyn DiscoveryHost) -> Found {
    classify(config, host, false, true)
}

fn classify(
    config: &DiscoveryConfig,
    host: &dyn DiscoveryHost,
    include_health: bool,
    ignore_stale_cli: bool,
) -> Found {
    let mut found = Found::unknown(&config.base_url);
    let desktop = read(host, &config.desktop_state_path);
    let cli = read(host, &config.cli_state_path);
    if config.authority.is_empty() {
        return found;
    }
    if !host.port_open(&config.authority, config.port_timeout) {
        let desktop_status = validate(host, &desktop);
        let desktop_manager_status = validate_manager(host, &desktop);
        let cli_status = validate(host, &cli);
        let desktop_stale = desktop_status == Status::Stale
            || (desktop_status == Status::Genuine && desktop_manager_status != Status::Genuine);
        let cli_stale = cli_status == Status::Stale;
        if desktop_stale || (cli_stale && !ignore_stale_cli) {
            found.classification = Classification::Stale;
        } else if let Some(value) = desktop.as_ref().ok().filter(|value| {
            desktop_status == Status::Genuine
                && desktop_manager_status == Status::Genuine
                && complete_desktop(value)
                && state_metadata_matches(value, config)
        }) {
            found.classification = Classification::Degraded;
            found.owner = "desktop".to_owned();
            found.state = Some(value.clone());
        } else if let Some(value) = cli.as_ref().ok().filter(|value| {
            cli_status == Status::Genuine
                && complete_cli(value)
                && state_metadata_matches(value, config)
                && !is_development_id(&config.current.deployment_id)
        }) {
            found.classification = Classification::Degraded;
            found.owner = "cli".to_owned();
            found.state = Some(value.clone());
        } else {
            found.classification = Classification::Absent;
        }
        return found;
    }
    // A live listener must always pass endpoint and process correlation; only
    // an unoccupied port lets desktop startup ignore another owner's stale state.
    let Some(version) = host
        .get_json(&config.authority, "/version", config.request_timeout)
        .and_then(|value| serde_json::from_value::<RemoteVersion>(value).ok())
    else {
        return found;
    };
    found.version = version;
    let desktop_status = validate(host, &desktop);
    let desktop_manager_status = validate_manager(host, &desktop);
    let cli_status = validate(host, &cli);
    let matched = match_state(
        config,
        &found.version,
        desktop.as_ref().ok(),
        desktop_status,
        desktop_manager_status,
        cli.as_ref().ok(),
        cli_status,
    );
    let Some((matched, owner)) = matched else {
        if let Some(legacy) = desktop.as_ref().ok().and_then(|value| {
            legacy_managed(
                config,
                &found.version,
                value,
                desktop_status,
                desktop_manager_status,
            )
        }) {
            found.classification = Classification::LegacyManaged;
            found.owner = "desktop".to_owned();
            found.state = Some(legacy.clone());
            return found;
        }
        let desktop_correlates = desktop.as_ref().ok().is_some_and(|value| {
            state_correlates(config, &found.version, value, desktop_status)
                || (desktop_status == Status::Genuine
                    && desktop_manager_status != Status::Genuine
                    && endpoint_correlates(config, &found.version, value))
        });
        let cli_correlates = cli
            .as_ref()
            .ok()
            .is_some_and(|value| state_correlates(config, &found.version, value, cli_status));
        if desktop_correlates || cli_correlates {
            found.classification = Classification::Stale;
        }
        return found;
    };
    found.state = Some(matched.clone());
    found.owner = owner.to_owned();
    if include_health {
        let healthy = host
            .get_json(&config.authority, "/health", config.health_request_timeout)
            .and_then(|value| serde_json::from_value::<RemoteHealth>(value).ok());
        match healthy {
            Some(health) if health.status == "ok" => found.health = health.status,
            _ => {
                found.classification = Classification::Degraded;
                return found;
            }
        }
    }
    found.classification = if owner == "desktop" {
        Classification::DesktopOwned
    } else {
        Classification::ExternalCompatible
    };
    found
}

fn read(host: &dyn DiscoveryHost, path: &Path) -> Result<RouterState, StateError> {
    if path.as_os_str().is_empty() {
        return Err(StateError::NotFound);
    }
    host.read_state(path)
}

fn validate(host: &dyn DiscoveryHost, value: &Result<RouterState, StateError>) -> Status {
    match value {
        Err(StateError::NotFound) => Status::Absent,
        Err(_) => Status::Stale,
        Ok(value) => host
            .validate(&router_identity(value), &value.binary_path)
            .unwrap_or(Status::Stale),
    }
}

fn validate_manager(host: &dyn DiscoveryHost, value: &Result<RouterState, StateError>) -> Status {
    let Ok(value) = value else {
        return Status::Absent;
    };
    if value.owner != "desktop" {
        return Status::Absent;
    }
    let identity = manager_identity(value);
    host.validate(&identity, &value.manager_process_executable)
        .unwrap_or(Status::Stale)
}

fn match_state<'a>(
    config: &DiscoveryConfig,
    remote: &RemoteVersion,
    desktop: Option<&'a RouterState>,
    desktop_status: Status,
    desktop_manager_status: Status,
    cli: Option<&'a RouterState>,
    cli_status: Status,
) -> Option<(&'a RouterState, &'static str)> {
    if !remote_metadata_matches(config, remote) {
        return None;
    }
    if let Some(value) = desktop {
        if desktop_status == Status::Genuine
            && desktop_manager_status == Status::Genuine
            && complete_desktop(value)
            && remote.pid == value.pid
            && installation_matches(&config.current, value)
            && generation_compatible(&config.current, value)
            && state_metadata_matches(value, config)
            && state_address_matches(value, &config.base_url)
        {
            return Some((value, "desktop"));
        }
    }
    // Development identities deliberately cannot authorize reuse of a router
    // owned by another manager or setup process.
    if is_development_id(&config.current.deployment_id) {
        return None;
    }
    if let Some(value) = cli {
        if cli_status == Status::Genuine
            && complete_cli(value)
            && remote.pid == value.pid
            && state_metadata_matches(value, config)
            && state_address_matches(value, &config.base_url)
        {
            return Some((value, "cli"));
        }
    }
    None
}

fn legacy_managed<'a>(
    config: &DiscoveryConfig,
    remote: &RemoteVersion,
    desktop: &'a RouterState,
    desktop_status: Status,
    desktop_manager_status: Status,
) -> Option<&'a RouterState> {
    if desktop.owner != "desktop"
        || desktop_status != Status::Genuine
        || remote.pid <= 0
        || remote.pid != desktop.pid
    {
        return None;
    }
    if !installation_allows(config, desktop) || !complete_identity(desktop) {
        return None;
    }
    let same_session = config.current.session_id.is_empty()
        || desktop.desktop_session_id == config.current.session_id;
    if same_session
        && generation_compatible(&config.current, desktop)
        && complete_desktop(desktop)
        && desktop_manager_status != Status::Genuine
    {
        return None;
    }
    Some(desktop)
}

fn installation_allows(config: &DiscoveryConfig, value: &RouterState) -> bool {
    if value.installation_id.is_empty() {
        return supported_legacy_source(value) && !value.binary_path.is_empty();
    }
    value.package_generation > 0 && installation_matches(&config.current, value)
}

fn remote_metadata_matches(config: &DiscoveryConfig, remote: &RemoteVersion) -> bool {
    remote.pid > 0
        && remote.deployment_id == config.current.deployment_id
        && remote.management_protocol_version == config.current.management_protocol_version
}

fn state_correlates(
    config: &DiscoveryConfig,
    remote: &RemoteVersion,
    value: &RouterState,
    status: Status,
) -> bool {
    status == Status::Stale && endpoint_correlates(config, remote, value)
}

fn endpoint_correlates(
    config: &DiscoveryConfig,
    remote: &RemoteVersion,
    value: &RouterState,
) -> bool {
    value.pid > 0 && remote.pid == value.pid && remote_metadata_matches(config, remote)
}

fn complete_identity(value: &RouterState) -> bool {
    complete_process(&router_identity(value)) && !value.binary_path.is_empty()
}

fn complete_cli(value: &RouterState) -> bool {
    value.owner == "cli"
        && value.pid > 0
        && !value.listen_addr.is_empty()
        && !value.binary_path.is_empty()
        && !value.process_started_at.is_empty()
        && !value.process_executable.is_empty()
        && !value.router_version.is_empty()
        && !value.deployment_id.is_empty()
        && !value.management_protocol_version.is_empty()
}

fn complete_desktop(value: &RouterState) -> bool {
    value.owner == "desktop"
        && value.pid > 0
        && !value.listen_addr.is_empty()
        && !value.binary_path.is_empty()
        && !value.log_path.is_empty()
        && !value.process_started_at.is_empty()
        && !value.process_executable.is_empty()
        && !value.desktop_session_id.is_empty()
        && value.manager_pid > 0
        && !value.manager_process_started_at.is_empty()
        && !value.manager_process_executable.is_empty()
        && !value.manager_version.is_empty()
        && !value.router_version.is_empty()
        && !value.deployment_id.is_empty()
        && !value.management_protocol_version.is_empty()
}

fn state_metadata_matches(value: &RouterState, config: &DiscoveryConfig) -> bool {
    value.deployment_id == config.current.deployment_id
        && value.management_protocol_version == config.current.management_protocol_version
}

fn state_address_matches(value: &RouterState, base_url: &str) -> bool {
    value.listen_addr.trim_end_matches('/') == base_url.trim_end_matches('/')
}

pub fn is_development_id(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "dev" | "unknown"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    const INSTALLATION: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    const BASE_URL: &str = "http://127.0.0.1:19099";
    const AUTHORITY: &str = "127.0.0.1:19099";

    struct FakeHost {
        states: Mutex<HashMap<PathBuf, Result<RouterState, StateError>>>,
        statuses: Mutex<HashMap<i32, Status>>,
        default_status: Mutex<Option<Status>>,
        port_open: Mutex<bool>,
        version: Mutex<Option<Value>>,
        health: Mutex<Option<Value>>,
        health_calls: AtomicUsize,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                states: Mutex::new(HashMap::new()),
                statuses: Mutex::new(HashMap::new()),
                default_status: Mutex::new(None),
                port_open: Mutex::new(false),
                version: Mutex::new(None),
                health: Mutex::new(None),
                health_calls: AtomicUsize::new(0),
            }
        }

        fn state(&self, name: &str, value: RouterState) {
            self.states
                .lock()
                .unwrap()
                .insert(PathBuf::from(name), Ok(value));
        }

        fn state_error(&self, name: &str, error: StateError) {
            self.states
                .lock()
                .unwrap()
                .insert(PathBuf::from(name), Err(error));
        }

        fn status(&self, pid: i32, status: Status) {
            self.statuses.lock().unwrap().insert(pid, status);
        }

        fn all_status(&self, status: Status) {
            *self.default_status.lock().unwrap() = Some(status);
        }

        fn serve(&self, pid: i32, deployment: &str, protocol: &str, health: &str) {
            *self.port_open.lock().unwrap() = true;
            *self.version.lock().unwrap() = Some(json!({
                "version": "v1",
                "pid": pid,
                "deployment_id": deployment,
                "management_protocol_version": protocol,
            }));
            *self.health.lock().unwrap() = Some(json!({ "status": health }));
        }
    }

    impl DiscoveryHost for FakeHost {
        fn read_state(&self, path: &Path) -> Result<RouterState, StateError> {
            self.states
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .unwrap_or(Err(StateError::NotFound))
        }

        fn inspect(&self, _pid: i32) -> Result<ProcessIdentity, ProcessError> {
            Err(ProcessError::NotFound)
        }

        fn validate(
            &self,
            identity: &ProcessIdentity,
            _binary: &str,
        ) -> Result<Status, ProcessError> {
            if let Some(status) = self.statuses.lock().unwrap().get(&identity.pid) {
                return Ok(*status);
            }
            match *self.default_status.lock().unwrap() {
                Some(status) => Ok(status),
                None => Err(ProcessError::NotFound),
            }
        }

        fn port_open(&self, _authority: &str, _timeout: Duration) -> bool {
            *self.port_open.lock().unwrap()
        }

        fn get_json(&self, _authority: &str, path: &str, _timeout: Duration) -> Option<Value> {
            match path {
                "/version" => self.version.lock().unwrap().clone(),
                "/health" => {
                    self.health_calls.fetch_add(1, Ordering::SeqCst);
                    self.health.lock().unwrap().clone()
                }
                _ => None,
            }
        }
    }

    fn lineage(deployment: &str, protocol: &str) -> CurrentLineage {
        CurrentLineage {
            session_id: String::new(),
            installation_id: INSTALLATION.into(),
            package_generation: 1,
            deployment_id: deployment.into(),
            management_protocol_version: protocol.into(),
            manager: ProcessIdentity::default(),
        }
    }

    fn config(deployment: &str, protocol: &str) -> DiscoveryConfig {
        DiscoveryConfig::new(
            BASE_URL,
            AUTHORITY,
            PathBuf::from("desktop"),
            PathBuf::from("cli"),
            lineage(deployment, protocol),
        )
    }

    fn complete_state(owner: &str, pid: i32, deployment: &str) -> RouterState {
        let mut value = RouterState {
            pid,
            owner: owner.into(),
            listen_addr: BASE_URL.into(),
            binary_path: "/router".into(),
            log_path: "/router.log".into(),
            process_started_at: "router-start".into(),
            process_executable: "/router".into(),
            router_version: "v1".into(),
            deployment_id: deployment.into(),
            management_protocol_version: "1".into(),
            ..RouterState::default()
        };
        if owner == "desktop" {
            value.desktop_session_id = "session".into();
            value.installation_id = INSTALLATION.into();
            value.package_generation = 1;
            value.manager_pid = 74;
            value.manager_process_started_at = "manager-start".into();
            value.manager_process_executable = "/manager".into();
            value.manager_version = "v1".into();
        }
        value
    }

    fn release_state(name: &str) -> RouterState {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../internal/manager/testdata")
            .join(name);
        let value = state::read(path).expect("release golden");
        assert!(value.installation_id.is_empty() && value.package_generation == 0);
        value
    }

    #[test]
    fn classifies_correlated_routers() {
        for (owner, health, deployment, want) in [
            ("desktop", "ok", "prod-a", Classification::DesktopOwned),
            ("cli", "ok", "prod-a", Classification::ExternalCompatible),
            ("cli", "degraded", "prod-a", Classification::Degraded),
            ("cli", "ok", "dev", Classification::UnknownOccupant),
        ] {
            let host = FakeHost::new();
            host.serve(73, deployment, "1", health);
            host.state(owner, complete_state(owner, 73, deployment));
            host.all_status(Status::Genuine);
            let found = discover(&config(deployment, "1"), &host);
            assert_eq!(found.classification, want, "{owner} {health} {deployment}");
            if want.is_trusted() {
                assert_eq!(found.owner, owner);
                assert_eq!(found.snapshot().pid, Some(73));
                assert_eq!(found.snapshot().listen_addr.as_deref(), Some(BASE_URL));
                assert_eq!(
                    found.snapshot().log_path,
                    Some(PathBuf::from("/router.log"))
                );
                assert_eq!(found.trusted_discovery().state.pid, 73);
            } else {
                assert_eq!(found.snapshot().pid, None);
            }
        }
    }

    #[test]
    fn status_mode_never_probes_health() {
        let host = FakeHost::new();
        host.serve(73, "prod-a", "1", "ok");
        host.state("desktop", complete_state("desktop", 73, "prod-a"));
        host.all_status(Status::Genuine);
        let found = discover_status(&config("prod-a", "1"), &host);
        assert_eq!(found.classification, Classification::DesktopOwned);
        assert_eq!(host.health_calls.load(Ordering::SeqCst), 0);
        assert!(found.snapshot().health.is_none());
        let found = discover(&config("prod-a", "1"), &host);
        assert_eq!(host.health_calls.load(Ordering::SeqCst), 1);
        assert_eq!(found.snapshot().health.unwrap().status, "ok");
    }

    #[test]
    fn endpoint_only_and_metadata_mismatch_are_unknown() {
        for (deployment, protocol) in [("prod-a", "1"), ("prod-b", "1"), ("prod-a", "2")] {
            let host = FakeHost::new();
            host.serve(73, deployment, protocol, "ok");
            let found = discover(&config("prod-a", "1"), &host);
            assert_eq!(found.classification, Classification::UnknownOccupant);
            assert_eq!(host.health_calls.load(Ordering::SeqCst), 0);
        }
        let host = FakeHost::new();
        *host.port_open.lock().unwrap() = true;
        *host.version.lock().unwrap() = Some(json!("not an object"));
        assert_eq!(
            discover(&config("prod-a", "1"), &host).classification,
            Classification::UnknownOccupant
        );
    }

    #[test]
    fn absent_and_stale_without_listener() {
        let host = FakeHost::new();
        assert_eq!(
            discover(&config("prod-a", "1"), &host).classification,
            Classification::Absent
        );
        host.state("desktop", complete_state("desktop", 73, "prod-a"));
        host.all_status(Status::Stale);
        assert_eq!(
            discover(&config("prod-a", "1"), &host).classification,
            Classification::Stale
        );
        let host = FakeHost::new();
        host.state_error("desktop", StateError::Corrupt);
        assert_eq!(
            discover(&config("prod-a", "1"), &host).classification,
            Classification::Stale
        );
    }

    #[test]
    fn desktop_startup_ignores_only_stale_cli_state_without_listener() {
        let host = FakeHost::new();
        host.state("cli", complete_state("cli", 73, "prod-a"));
        host.all_status(Status::Stale);
        let config = config("prod-a", "1");
        assert_eq!(
            discover_status(&config, &host).classification,
            Classification::Stale
        );
        assert_eq!(
            discover_desktop_startup(&config, &host).classification,
            Classification::Absent
        );
    }

    #[test]
    fn desktop_startup_remains_blocked_by_current_state() {
        let host = FakeHost::new();
        host.state("desktop", complete_state("desktop", 73, "prod-a"));
        host.all_status(Status::Stale);
        assert_eq!(
            discover_desktop_startup(&config("prod-a", "1"), &host).classification,
            Classification::Stale
        );

        let host = FakeHost::new();
        host.state("cli", complete_state("cli", 73, "prod-a"));
        host.all_status(Status::Genuine);
        let found = discover_desktop_startup(&config("prod-a", "1"), &host);
        assert_eq!(found.classification, Classification::Degraded);
        assert_eq!(found.owner, "cli");
    }

    #[test]
    fn desktop_startup_does_not_ignore_correlated_stale_cli_with_listener() {
        let host = FakeHost::new();
        host.serve(73, "prod-a", "1", "ok");
        host.state("cli", complete_state("cli", 73, "prod-a"));
        host.all_status(Status::Stale);
        assert_eq!(
            discover_desktop_startup(&config("prod-a", "1"), &host).classification,
            Classification::Stale
        );
    }

    #[test]
    fn generic_status_prefers_running_desktop_over_stale_cli() {
        let host = FakeHost::new();
        host.serve(73, "prod-a", "1", "ok");
        host.state("desktop", complete_state("desktop", 73, "prod-a"));
        host.state("cli", complete_state("cli", 99, "prod-a"));
        host.all_status(Status::Genuine);
        host.status(99, Status::Stale);
        let found = discover_status(&config("prod-a", "1"), &host);
        assert_eq!(found.classification, Classification::DesktopOwned);
        assert_eq!(found.owner, "desktop");
    }

    #[test]
    fn genuine_state_with_unavailable_endpoint_is_degraded() {
        let host = FakeHost::new();
        host.state("desktop", complete_state("desktop", 73, "prod-a"));
        host.all_status(Status::Genuine);
        let found = discover(&config("prod-a", "1"), &host);
        assert_eq!(found.classification, Classification::Degraded);
        assert_eq!(found.owner, "desktop");
        assert_eq!(found.state.as_ref().unwrap().pid, 73);
        assert_eq!(found.snapshot().pid, Some(73));
    }

    #[test]
    fn release_legacy_fixtures_classify_correlated_router() {
        for name in ["desktop-state-v0.1.8.json", "desktop-state-v0.2.0.json"] {
            let value = release_state(name);
            let host = FakeHost::new();
            host.serve(
                value.pid,
                &value.deployment_id,
                &value.management_protocol_version,
                "ok",
            );
            host.state("desktop", value.clone());
            host.all_status(Status::Genuine);
            host.status(value.manager_pid, Status::Absent);
            let mut config = config("current-production", "4");
            config.current.session_id = "current-session".into();
            let found = discover(&config, &host);
            assert_eq!(
                found.classification,
                Classification::LegacyManaged,
                "{name}"
            );
            assert_eq!(found.owner, "desktop");
            assert_eq!(found.state, Some(value));
            assert_eq!(found.snapshot().pid, None, "legacy is not trusted");
        }
    }

    #[test]
    fn release_legacy_fixtures_fail_closed_when_lineage_or_identity_incomplete() {
        let mutations: [(&str, fn(&mut RouterState)); 6] = [
            ("router start", |value| value.process_started_at.clear()),
            ("router executable", |value| {
                value.process_executable.clear()
            }),
            ("manager start", |value| {
                value.manager_process_started_at.clear()
            }),
            ("manager executable", |value| {
                value.manager_process_executable.clear()
            }),
            ("unknown ancestor protocol", |value| {
                value.management_protocol_version = "2".into()
            }),
            ("different installation", |value| {
                value.installation_id = "ffffffff-bbbb-4ccc-8ddd-eeeeeeeeeeee".into()
            }),
        ];
        for name in ["desktop-state-v0.1.8.json", "desktop-state-v0.2.0.json"] {
            for (label, mutate) in mutations {
                let mut value = release_state(name);
                mutate(&mut value);
                let host = FakeHost::new();
                host.serve(
                    value.pid,
                    &value.deployment_id,
                    &value.management_protocol_version,
                    "ok",
                );
                host.state("desktop", value);
                host.all_status(Status::Genuine);
                let mut config = config("current-production", "4");
                config.current.session_id = "current-session".into();
                assert_eq!(
                    discover(&config, &host).classification,
                    Classification::UnknownOccupant,
                    "{name} {label}"
                );
            }
        }
    }

    #[test]
    fn correlated_legacy_desktop_is_legacy_managed() {
        let host = FakeHost::new();
        host.serve(73, "old-prod", "3", "ok");
        let mut value = complete_state("desktop", 73, "old-prod");
        value.management_protocol_version = "3".into();
        value.desktop_session_id = "previous-session".into();
        host.state("desktop", value);
        host.all_status(Status::Genuine);
        host.status(74, Status::Absent);
        let mut config = config("prod-a", "4");
        config.current.session_id = "current-session".into();
        let found = discover(&config, &host);
        assert_eq!(found.classification, Classification::LegacyManaged);
        assert_eq!(found.state.as_ref().unwrap().pid, 73);
    }

    #[test]
    fn cross_session_compatible_desktop_is_legacy_managed() {
        let host = FakeHost::new();
        host.serve(73, "prod-a", "1", "ok");
        let mut value = complete_state("desktop", 73, "prod-a");
        value.desktop_session_id = "previous-session".into();
        host.state("desktop", value);
        host.all_status(Status::Genuine);
        host.status(74, Status::Absent);
        let mut config = config("prod-a", "1");
        config.current.session_id = "current-session".into();
        assert_eq!(
            discover(&config, &host).classification,
            Classification::LegacyManaged
        );
    }

    #[test]
    fn unknown_pre_installation_protocol_and_foreign_installation_are_not_legacy() {
        let host = FakeHost::new();
        host.serve(73, "old-prod", "2", "ok");
        let mut value = complete_state("desktop", 73, "old-prod");
        value.management_protocol_version = "2".into();
        value.installation_id.clear();
        value.package_generation = 0;
        host.state("desktop", value);
        host.all_status(Status::Genuine);
        let mut config = config("prod-a", "4");
        config.current.session_id = "current-session".into();
        assert_eq!(
            discover(&config, &host).classification,
            Classification::UnknownOccupant
        );

        let host = FakeHost::new();
        host.serve(73, "old-prod", "3", "ok");
        let mut value = complete_state("desktop", 73, "old-prod");
        value.management_protocol_version = "3".into();
        host.state("desktop", value);
        host.all_status(Status::Genuine);
        let mut foreign = super::tests::config("prod-a", "4");
        foreign.current.installation_id = "ffffffff-bbbb-4ccc-8ddd-eeeeeeeeeeee".into();
        assert_eq!(
            discover(&foreign, &host).classification,
            Classification::UnknownOccupant
        );
    }

    #[test]
    fn current_protocol_desktop_fails_closed_for_invalid_installation_lineage() {
        let mutations: [fn(&mut RouterState); 2] = [
            |value| value.package_generation = 0,
            |value| value.installation_id = "ffffffff-bbbb-4ccc-8ddd-eeeeeeeeeeee".into(),
        ];
        for mutate in mutations {
            let host = FakeHost::new();
            host.serve(73, "prod-a", "1", "ok");
            let mut value = complete_state("desktop", 73, "prod-a");
            mutate(&mut value);
            host.state("desktop", value);
            host.all_status(Status::Genuine);
            let mut config = config("prod-a", "1");
            config.current.session_id = "session".into();
            assert_eq!(
                discover(&config, &host).classification,
                Classification::UnknownOccupant
            );
        }
    }

    #[test]
    fn healthy_desktop_router_with_absent_manager_is_stale() {
        let host = FakeHost::new();
        host.serve(73, "prod-a", "1", "ok");
        host.state("desktop", complete_state("desktop", 73, "prod-a"));
        host.all_status(Status::Genuine);
        host.status(74, Status::Absent);
        assert_eq!(
            discover(&config("prod-a", "1"), &host).classification,
            Classification::Stale
        );
    }

    #[test]
    fn system_host_probes_loopback_endpoint_with_bounded_json() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let authority = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 1024];
                let _ = stream.read(&mut buffer);
                let body = br#"{"version":"v9","pid":73,"deployment_id":"prod-a","management_protocol_version":"4"}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body);
            }
        });
        let host = SystemHost;
        assert!(host.port_open(&authority, Duration::from_millis(500)));
        let value = host
            .get_json(&authority, "/version", Duration::from_secs(2))
            .expect("json");
        assert_eq!(value["pid"], 73);
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let free = closed.local_addr().unwrap().to_string();
        drop(closed);
        assert!(!host.port_open(&free, Duration::from_millis(200)));
        let _ = host.get_json(&authority, "/version", Duration::from_secs(2));
        let _ = server.join();
    }
}
