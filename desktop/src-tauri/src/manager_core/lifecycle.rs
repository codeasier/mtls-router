use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::protocol::{
    deadline, DiagnosticsResult, ErrorCode, ManagerInfoResult, Method, ProtocolError, Request,
    Response, RouterHealthResult, RouterLogsParams, RouterLogsResult, RouterOwner,
    RouterStartParams, RouterStatusResult, RouterVersionResult,
};
use crate::redaction::{bound_text, sanitize_text, MAX_DIAGNOSTICS_SIZE};

use super::errors::{
    discovery_error, invalid_params, map_lifecycle_error, method_unavailable, startup_diagnostic,
    timeout_error, LifecycleError, StartupStage,
};
use super::types::{
    AgentLine, Classification, DiscoverySnapshot, HealthInfo, StartedRouter, VersionInfo,
};

pub const DEFAULT_LOG_LINES: usize = 200;
pub const MAX_LOG_LINES: i64 = 1000;
pub const MAX_LOG_LINE_BYTES: usize = 4096;
pub const MAX_LOG_READ_BYTES: u64 = 256 * 1024;

pub trait Backend: Send + Sync {
    fn start(
        &self,
        owner: RouterOwner,
    ) -> impl Future<Output = Result<StartedRouter, LifecycleError>> + Send;
    fn reclaim(&self) -> impl Future<Output = Result<StartedRouter, LifecycleError>> + Send;
    fn stop(&self) -> impl Future<Output = Result<(), LifecycleError>> + Send;
    fn discover_status(&self) -> impl Future<Output = DiscoverySnapshot> + Send;
    fn discover_health(&self) -> impl Future<Output = DiscoverySnapshot> + Send;
    fn recent_output(&self) -> String;
    fn log_path(&self) -> Option<PathBuf>;
}

struct LatchedFailure {
    last_error: String,
    recent_logs: Vec<String>,
    absent_start_ok: bool,
}

pub struct Manager<B> {
    info: ManagerInfoResult,
    now: fn() -> DateTime<Utc>,
    detect: fn() -> Result<Vec<AgentLine>, ()>,
    desktop_log_path: Option<PathBuf>,
    backend: B,
    failure: Mutex<Option<LatchedFailure>>,
}

impl<B: Backend> Manager<B> {
    pub fn new(info: ManagerInfoResult, backend: B) -> Self {
        Self {
            info,
            now: Utc::now,
            detect: || Err(()),
            desktop_log_path: None,
            backend,
            failure: Mutex::new(None),
        }
    }

    pub fn with_clock(mut self, now: fn() -> DateTime<Utc>) -> Self {
        self.now = now;
        self
    }

    pub fn with_detect(mut self, detect: fn() -> Result<Vec<AgentLine>, ()>) -> Self {
        self.detect = detect;
        self
    }

    pub fn with_desktop_log_path(mut self, path: PathBuf) -> Self {
        self.desktop_log_path = Some(path);
        self
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn into_backend(self) -> B {
        self.backend
    }

    pub fn absent_start_ok(&self) -> bool {
        self.failure
            .lock()
            .expect("failure lock")
            .as_ref()
            .map(|failure| failure.absent_start_ok)
            .unwrap_or(true)
    }

    pub async fn handle(&self, request: Request) -> Response {
        let id = request.id.clone();
        let timed = tokio::time::timeout(deadline(request.method), self.dispatch(request)).await;
        match timed {
            Ok(Ok(result)) => Response {
                id: Some(id),
                result: Some(result),
                error: None,
            },
            Ok(Err(error)) => Response {
                id: Some(id),
                result: None,
                error: Some(error),
            },
            Err(_) => Response {
                id: Some(id),
                result: None,
                error: Some(timeout_error()),
            },
        }
    }

    async fn dispatch(&self, request: Request) -> Result<Value, ProtocolError> {
        let params = match request.params {
            None | Some(Value::Null) => json!({}),
            Some(value) => value,
        };
        match request.method {
            Method::ManagerInfo => to_value(self.manager_info(&params)?),
            Method::DiagnosticsCollect => to_value(self.diagnostics_collect(&params).await?),
            Method::RouterStatus => to_value(self.router_status(&params).await?),
            Method::RouterStart => to_value(self.router_start(&params).await?),
            Method::RouterStop => to_value(self.router_stop(&params).await?),
            Method::RouterHealth => to_value(self.router_health(&params).await?),
            Method::RouterVersion => to_value(self.router_version(&params).await?),
            Method::RouterLogs => to_value(self.router_logs(&params).await?),
            _ => Err(method_unavailable()),
        }
    }

    fn manager_info(&self, params: &Value) -> Result<ManagerInfoResult, ProtocolError> {
        decode_empty(params)?;
        Ok(self.info.clone())
    }

    async fn diagnostics_collect(
        &self,
        params: &Value,
    ) -> Result<DiagnosticsResult, ProtocolError> {
        decode_empty(params)?;
        let found = self.backend.discover_status().await;
        let mut summary = format!(
            "manager version={} commit={} build_date={} target={} deployment_id={} protocol={}\n",
            self.info.version,
            self.info.commit,
            self.info.build_date,
            self.info.target,
            self.info.deployment_id,
            self.info.management_protocol_version
        );
        summary.push_str(&format!(
            "router state={} owner={} listen={} pid={} version={} health={}\n",
            found.classification.as_str(),
            found.owner.as_deref().unwrap_or(""),
            found.listen_addr.as_deref().unwrap_or(""),
            found
                .classification
                .is_trusted()
                .then_some(found.pid.unwrap_or(0))
                .unwrap_or(0),
            trusted_version(&found),
            trusted_health(&found)
        ));
        match (self.detect)() {
            Ok(agents) => {
                for agent in agents {
                    summary.push_str(&format!(
                        "agent name={} detected={} exists={} writable={} configured={} invalid={}\n",
                        agent.agent,
                        agent.detected,
                        agent.exists,
                        agent.writable,
                        agent.configured,
                        agent.invalid
                    ));
                }
            }
            Err(()) => summary.push_str("agents unavailable\n"),
        }
        Ok(DiagnosticsResult {
            summary: bound_text(&sanitize_text(&summary), MAX_DIAGNOSTICS_SIZE),
            router_state: Some(found.classification.as_str().to_owned()),
            manager_failure: None,
        })
    }

    async fn router_status(&self, params: &Value) -> Result<RouterStatusResult, ProtocolError> {
        decode_empty(params)?;
        let found = self.backend.discover_status().await;
        if matches!(
            found.classification,
            Classification::Absent | Classification::Stale
        ) {
            if let Some(failed) = self.failed_status() {
                return Ok(failed);
            }
        }
        Ok(status_result(&found))
    }

    async fn router_start(&self, params: &Value) -> Result<RouterStatusResult, ProtocolError> {
        let request: RouterStartParams = decode_params(params)?;
        let started = self.backend.start(request.owner).await;
        match started {
            Ok(value) => {
                if request.owner == RouterOwner::Desktop {
                    self.clear_failure();
                }
                Ok(status_from_state(&value))
            }
            Err(start_err) => {
                if request.owner == RouterOwner::Desktop
                    && matches!(
                        start_err.code,
                        ErrorCode::RouterAlreadyRunning | ErrorCode::RouterStateStale
                    )
                {
                    match self.backend.reclaim().await {
                        Ok(value) => {
                            self.clear_failure();
                            return Ok(status_from_state(&value));
                        }
                        Err(reclaim_err) => {
                            if start_err.should_latch() {
                                self.latch_startup_failure(&start_err);
                            }
                            return Err(map_lifecycle_error(&reclaim_err));
                        }
                    }
                }
                if request.owner == RouterOwner::Desktop && start_err.should_latch() {
                    self.latch_startup_failure(&start_err);
                }
                Err(map_lifecycle_error(&start_err))
            }
        }
    }

    async fn router_stop(&self, params: &Value) -> Result<RouterStatusResult, ProtocolError> {
        decode_empty(params)?;
        self.backend
            .stop()
            .await
            .map_err(|error| map_lifecycle_error(&error))?;
        Ok(RouterStatusResult {
            state: Classification::Absent.as_str().to_owned(),
            ..RouterStatusResult::default()
        })
    }

    async fn router_health(&self, params: &Value) -> Result<RouterHealthResult, ProtocolError> {
        decode_empty(params)?;
        let found = self.backend.discover_health().await;
        if let Some(error) = discovery_error(
            found.classification,
            true,
            found.health.as_ref().map(|health| health.status.as_str()),
            found
                .resolved_version()
                .map(|version| version.version.as_str()),
        ) {
            return Err(error);
        }
        Ok(RouterHealthResult {
            status: found.health.map(|health| health.status).unwrap_or_default(),
            checked_at: (self.now)().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        })
    }

    async fn router_version(&self, params: &Value) -> Result<RouterVersionResult, ProtocolError> {
        decode_empty(params)?;
        let found = self.backend.discover_status().await;
        if let Some(error) = discovery_error(
            found.classification,
            false,
            found.health.as_ref().map(|health| health.status.as_str()),
            found
                .resolved_version()
                .map(|version| version.version.as_str()),
        ) {
            return Err(error);
        }
        let version = found.resolved_version().cloned().unwrap_or_default();
        Ok(RouterVersionResult {
            version: version.version,
            deployment_id: version.deployment_id,
            management_protocol_version: version.management_protocol_version,
        })
    }

    async fn router_logs(&self, params: &Value) -> Result<RouterLogsResult, ProtocolError> {
        let request: RouterLogsParams = decode_params(params)?;
        if request.limit < 0 || request.limit > MAX_LOG_LINES {
            return Err(invalid_params("limit must be between 1 and 1000"));
        }
        let limit = if request.limit == 0 {
            DEFAULT_LOG_LINES
        } else {
            request.limit as usize
        };
        let found = self.backend.discover_status().await;
        let trusted_external = found.classification.is_trusted()
            && found.owner.as_deref() != Some(RouterOwner::Desktop.as_str());
        let mut path = trusted_log_path(&found);
        if path.is_none() && !trusted_external {
            path = self
                .backend
                .log_path()
                .or_else(|| self.desktop_log_path.clone());
        }
        let mut recent = Vec::new();
        if !trusted_external {
            let mut use_lifecycle_recent = true;
            if matches!(
                found.classification,
                Classification::Absent | Classification::Stale
            ) {
                if let Some(failure_logs) = self.failure_log_lines(limit) {
                    recent = failure_logs;
                    use_lifecycle_recent = false;
                }
            }
            if use_lifecycle_recent {
                recent = last_lines(&sanitize_text(&self.backend.recent_output()), limit);
            }
        }
        match path.as_deref().map(read_log_lines).transpose() {
            Ok(Some(lines)) => Ok(RouterLogsResult {
                lines: merge_log_lines(&lines, &recent, limit),
            }),
            Ok(None) => Ok(RouterLogsResult { lines: recent }),
            Err(LogReadError::NotFound) if recent.is_empty() => {
                Ok(RouterLogsResult { lines: Vec::new() })
            }
            Err(LogReadError::NotFound) => Ok(RouterLogsResult { lines: recent }),
            Err(LogReadError::Unavailable) if !recent.is_empty() => {
                Ok(RouterLogsResult { lines: recent })
            }
            Err(LogReadError::Unavailable) => Err(super::errors::closed_error(
                ErrorCode::RouterStateStale,
                "router log is unavailable",
            )),
        }
    }

    fn failed_status(&self) -> Option<RouterStatusResult> {
        let failure = self.failure.lock().expect("failure lock");
        failure.as_ref().map(|failure| RouterStatusResult {
            state: "start_failed".to_owned(),
            owner: Some(RouterOwner::Desktop.as_str().to_owned()),
            last_error: Some(failure.last_error.clone()),
            recent_logs: Some(failure.recent_logs.clone()),
            ..RouterStatusResult::default()
        })
    }

    fn failure_log_lines(&self, limit: usize) -> Option<Vec<String>> {
        let failure = self.failure.lock().expect("failure lock");
        let failure = failure.as_ref()?;
        Some(bound_failure_logs(
            &failure.recent_logs,
            &failure.last_error,
            limit,
        ))
    }

    fn latch_startup_failure(&self, start_err: &LifecycleError) {
        let mut failure = self.failure.lock().expect("failure lock");
        *failure = Some(if start_err.stage.is_some() {
            let diagnostic = startup_diagnostic(start_err);
            let mut recent_logs = last_lines(
                &sanitize_text(&start_err.recent_output),
                DEFAULT_LOG_LINES - 1,
            );
            recent_logs.insert(0, diagnostic.clone());
            LatchedFailure {
                last_error: diagnostic,
                recent_logs,
                absent_start_ok: start_err.stage == Some(StartupStage::StateReconcile)
                    && !start_err.launched,
            }
        } else {
            LatchedFailure {
                last_error: "desktop-owned router failed during startup".to_owned(),
                recent_logs: last_lines(
                    &sanitize_text(&start_err.recent_output),
                    DEFAULT_LOG_LINES,
                ),
                absent_start_ok: false,
            }
        });
    }

    fn clear_failure(&self) {
        *self.failure.lock().expect("failure lock") = None;
    }
}

fn trusted_version(found: &DiscoverySnapshot) -> String {
    if !found.classification.is_trusted() {
        return String::new();
    }
    found
        .resolved_version()
        .map(|version| version.version.clone())
        .unwrap_or_default()
}

fn trusted_health(found: &DiscoverySnapshot) -> String {
    if !found.classification.is_trusted() {
        return String::new();
    }
    found
        .health
        .as_ref()
        .map(|health| health.status.clone())
        .unwrap_or_default()
}

fn trusted_log_path(found: &DiscoverySnapshot) -> Option<PathBuf> {
    found
        .classification
        .is_trusted()
        .then(|| found.log_path.clone())
        .flatten()
}

fn status_result(found: &DiscoverySnapshot) -> RouterStatusResult {
    let mut result = RouterStatusResult {
        state: found.classification.as_str().to_owned(),
        owner: found.owner.clone(),
        listen_addr: found.listen_addr.clone(),
        ..RouterStatusResult::default()
    };
    if found.classification.is_trusted() {
        result.pid = found.pid;
        if result.listen_addr.as_deref().unwrap_or("").is_empty() {
            result.listen_addr = found.listen_addr.clone();
        }
        if found.classification == Classification::Degraded {
            result.last_error = Some("router health is degraded".to_owned());
        }
    }
    result
}

fn status_from_state(value: &StartedRouter) -> RouterStatusResult {
    RouterStatusResult {
        state: if value.owner == RouterOwner::Desktop {
            Classification::DesktopOwned.as_str()
        } else {
            Classification::ExternalCompatible.as_str()
        }
        .to_owned(),
        owner: Some(value.owner.as_str().to_owned()),
        listen_addr: Some(value.listen_addr.clone()).filter(|value| !value.is_empty()),
        pid: value.pid,
        ..RouterStatusResult::default()
    }
}

fn decode_empty(params: &Value) -> Result<(), ProtocolError> {
    decode_params::<EmptyParams>(params).map(|_| ())
}

fn decode_params<T: DeserializeOwned>(params: &Value) -> Result<T, ProtocolError> {
    serde_json::from_value(params.clone()).map_err(|_| invalid_params("invalid method parameters"))
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, ProtocolError> {
    serde_json::to_value(value).map_err(|_| {
        super::errors::closed_error(
            ErrorCode::InvalidRequest,
            "operation returned an invalid result",
        )
    })
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

enum LogReadError {
    NotFound,
    Unavailable,
}

fn read_log_lines(path: &Path) -> Result<Vec<String>, LogReadError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(LogReadError::NotFound)
        }
        Err(_) => return Err(LogReadError::Unavailable),
    };
    let mut file = std::fs::File::open(path).map_err(|_| LogReadError::Unavailable)?;
    let start = metadata.len().saturating_sub(MAX_LOG_READ_BYTES);
    if start > 0 {
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(start))
            .map_err(|_| LogReadError::Unavailable)?;
    }
    let mut data = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut data).map_err(|_| LogReadError::Unavailable)?;
    if start > 0 {
        if let Some(index) = data.iter().position(|byte| *byte == b'\n') {
            data = data[index + 1..].to_vec();
        }
    }
    let text = String::from_utf8_lossy(&data);
    Ok(last_lines(&sanitize_text(&text), usize::MAX))
}

pub fn last_lines(value: &str, limit: usize) -> Vec<String> {
    let value = value.trim_end_matches(['\r', '\n']);
    if value.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = value
        .split('\n')
        .map(|line| bound_text(line.trim_end_matches('\r'), MAX_LOG_LINE_BYTES))
        .collect();
    if lines.len() > limit {
        lines = lines[lines.len() - limit..].to_vec();
    }
    lines
}

pub fn merge_log_lines(disk: &[String], recent: &[String], limit: usize) -> Vec<String> {
    let mut overlap = 0;
    for size in (1..=disk.len().min(recent.len())).rev() {
        if disk[disk.len() - size..] == recent[..size] {
            overlap = size;
            break;
        }
    }
    let mut merged = disk.to_vec();
    merged.extend_from_slice(&recent[overlap..]);
    if merged.len() > limit {
        merged = merged[merged.len() - limit..].to_vec();
    }
    merged
}

fn bound_failure_logs(lines: &[String], last_error: &str, limit: usize) -> Vec<String> {
    if lines.len() <= limit {
        return lines.to_vec();
    }
    if limit > 0 && lines.first().map(String::as_str) == Some(last_error) {
        let mut result = vec![lines[0].clone()];
        result.extend_from_slice(&lines[lines.len() - (limit - 1)..]);
        result
    } else {
        lines[lines.len() - limit..].to_vec()
    }
}

#[allow(dead_code)]
pub fn version_from_parts(
    version: impl Into<String>,
    deployment_id: impl Into<String>,
    protocol: impl Into<String>,
) -> VersionInfo {
    VersionInfo {
        version: version.into(),
        deployment_id: deployment_id.into(),
        management_protocol_version: protocol.into(),
    }
}

#[allow(dead_code)]
pub fn health(status: impl Into<String>) -> HealthInfo {
    HealthInfo {
        status: status.into(),
    }
}
