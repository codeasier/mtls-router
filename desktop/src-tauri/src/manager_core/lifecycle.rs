use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::protocol::{
    deadline, APIKeyUsageModel, APIKeyUsageParams, APIKeyUsageQuota, APIKeyUsageResult,
    APIKeyUsageSummary, AgentCleanupParams, AgentCleanupWriteParams, AgentConfigParams,
    AgentModelsParams, AgentWriteParams, DiagnosticsResult, ErrorCode,
    ForceTerminateOccupantParams, ManagerInfoResult, Method, ProtocolError, Request, Response,
    RouterHealthResult, RouterLogsParams, RouterLogsResult, RouterOwner, RouterStartParams,
    RouterStatusResult, RouterVersionResult,
};
use crate::redaction::{bound_text, sanitize_text, MAX_DIAGNOSTICS_SIZE};

use super::agent::{
    isolated_service, validate_refreshed_models, AgentService, ConfigMode, PreviewRequest,
    TrustedCatalog, WriteRequest,
};
use super::errors::{
    closed_error, discovery_error, invalid_params, map_agent_error, map_lifecycle_error,
    map_occupant_code, method_unavailable, startup_diagnostic, timeout_error, LifecycleError,
    StartupStage,
};
use super::occupant::{
    CallContext, Inspection as OccupantInspection, OccupantService, TerminateResult,
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
    detect: Option<fn() -> Result<Vec<AgentLine>, ()>>,
    desktop_log_path: Option<PathBuf>,
    backend: B,
    occupant: OccupantService,
    agent: AgentService,
    trusted: Option<Arc<dyn TrustedCatalog>>,
    failure: Mutex<Option<LatchedFailure>>,
}

impl<B: Backend> Manager<B> {
    pub fn new(info: ManagerInfoResult, backend: B) -> Self {
        Self {
            info,
            now: Utc::now,
            detect: None,
            desktop_log_path: None,
            backend,
            occupant: OccupantService::new(Default::default(), Default::default()),
            agent: isolated_service(),
            trusted: None,
            failure: Mutex::new(None),
        }
    }

    pub fn with_occupant(mut self, occupant: OccupantService) -> Self {
        self.occupant = occupant;
        self
    }

    pub fn with_clock(mut self, now: fn() -> DateTime<Utc>) -> Self {
        self.now = now;
        self
    }

    pub fn with_detect(mut self, detect: fn() -> Result<Vec<AgentLine>, ()>) -> Self {
        self.detect = Some(detect);
        self
    }

    pub fn with_agent(mut self, agent: AgentService) -> Self {
        self.agent = agent;
        self
    }

    pub fn with_trusted(mut self, trusted: Arc<dyn TrustedCatalog>) -> Self {
        self.trusted = Some(trusted);
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
            Method::RouterInspectOccupant => to_value(self.router_inspect_occupant(&params).await?),
            Method::RouterForceTerminateOccupant => {
                to_value(self.router_force_terminate_occupant(&params).await?)
            }
            Method::AgentDetect => to_value(self.agent_detect(&params)?),
            Method::AgentModels => to_value(self.agent_models(&params)?),
            Method::AgentRender => to_value(self.agent_render(&params)?),
            Method::AgentPreview => to_value(self.agent_preview(&params)?),
            Method::AgentWrite => to_value(self.agent_write(&params)?),
            Method::AgentCleanupPreview => to_value(self.agent_cleanup_preview(&params)?),
            Method::AgentCleanupWrite => to_value(self.agent_cleanup_write(&params)?),
            Method::ApiKeyUsage => to_value(self.api_key_usage(&params)?),
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
        match self.detect_lines() {
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

    async fn router_inspect_occupant(
        &self,
        params: &Value,
    ) -> Result<OccupantInspection, ProtocolError> {
        decode_empty(params)?;
        let found = self.backend.discover_status().await;
        self.occupant
            .inspect_for(
                &occupant_ctx(Method::RouterInspectOccupant),
                found.classification,
            )
            .map_err(map_occupant_error)
    }

    async fn router_force_terminate_occupant(
        &self,
        params: &Value,
    ) -> Result<TerminateResult, ProtocolError> {
        let request: ForceTerminateOccupantParams = decode_params(params)?;
        if request.confirmation_token.trim().is_empty() {
            return Err(invalid_params("confirmation_token is required"));
        }
        let found = self.backend.discover_status().await;
        self.occupant
            .force_terminate_for(
                &occupant_ctx(Method::RouterForceTerminateOccupant),
                found.classification,
                &request.confirmation_token,
            )
            .map_err(map_occupant_error)
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

fn occupant_ctx(method: Method) -> CallContext {
    CallContext::with_deadline(Instant::now() + deadline(method))
}

fn map_occupant_error(error: super::occupant::OccupantError) -> ProtocolError {
    map_occupant_code(error.as_protocol_code())
}

const MAX_API_KEY_SIZE: usize = 16 * 1024;
const MAX_MODEL_CONFIG_SIZE: usize = 2 * 1024 * 1024;

impl<B: Backend> Manager<B> {
    fn detect_lines(&self) -> Result<Vec<AgentLine>, ()> {
        if let Some(detect) = self.detect {
            return detect();
        }
        self.agent
            .detect(None)
            .map(|states| {
                states
                    .into_iter()
                    .map(|state| AgentLine {
                        agent: state.agent.as_str().to_owned(),
                        detected: state.detected,
                        exists: state.exists,
                        writable: state.writable,
                        configured: state.configured,
                        invalid: state.invalid,
                    })
                    .collect()
            })
            .map_err(|_| ())
    }

    fn agent_detect(&self, params: &Value) -> Result<Value, ProtocolError> {
        decode_empty(params)?;
        let states = self
            .agent
            .detect(None)
            .map_err(|_| closed_error(ErrorCode::ConfigInvalid, "Agent detection failed"))?;
        serde_json::to_value(json!({ "agents": states }))
            .map_err(|_| closed_error(ErrorCode::ConfigInvalid, "Agent detection failed"))
    }

    fn agent_models(&self, params: &Value) -> Result<Value, ProtocolError> {
        let request: AgentModelsParams = decode_params(params)?;
        let selected = parse_agents(&request.agents)?;
        if request.api_key.is_empty() || request.api_key.len() > MAX_API_KEY_SIZE {
            return Err(invalid_params("bounded api_key is required"));
        }
        let Some(trusted) = &self.trusted else {
            return Err(closed_error(
                ErrorCode::ModelDiscoveryFailed,
                "model discovery is unavailable",
            ));
        };
        let catalog = trusted.fetch(request.owner.as_str(), &request.api_key)?;
        let result = self
            .agent
            .discover_models(
                &selected,
                &catalog.models,
                crate::manager_core::agent::modelconfig::CatalogClaims {
                    version: 0,
                    models: catalog.models.clone(),
                    agents: selected.clone(),
                    owner: request.owner.as_str().to_owned(),
                    router_base_url: catalog.binding.router_base_url.clone(),
                    deployment_id: catalog.binding.deployment_id.clone(),
                    protocol_version: catalog.binding.protocol_version.clone(),
                    simplify: false,
                    canonicalization: String::new(),
                    key_generation: String::new(),
                },
                None,
            )
            .map_err(map_agent_error)?;
        let mut unavailable_preset = serde_json::Map::new();
        for (kind, models) in result.preset.unavailable_agents {
            unavailable_preset.insert(
                kind,
                json!({"code": "MODEL_NOT_AVAILABLE", "models": models}),
            );
        }
        Ok(json!({
            "models": catalog.models,
            "catalog_token": result.catalog_token,
            "router_base_url": catalog.binding.router_base_url,
            "api_base_url": catalog.binding.api_base_url,
            "existing": {
                "model_config": serde_json::from_slice::<Value>(&result.existing.model_config).unwrap_or(json!({})),
                "unavailable_models": result.existing.unavailable_models,
                "drifted_agents": result.existing.drifted_agents,
            },
            "preset": {
                "model_config": serde_json::from_slice::<Value>(&result.preset.model_config).unwrap_or(json!({})),
                "unavailable_agents": unavailable_preset,
            },
        }))
    }

    fn agent_render(&self, params: &Value) -> Result<Value, ProtocolError> {
        let request: AgentConfigParams = decode_params(params)?;
        if !request.modes.is_empty() {
            return Err(invalid_params(
                "modes are supported only by Agent preview and write",
            ));
        }
        let selected = validate_agent_config(
            &request.agents,
            &request.catalog_token,
            &request.model_config,
        )?;
        let raw = serde_json::to_vec(&request.model_config)
            .map_err(|_| invalid_params("model_config must be a bounded JSON object"))?;
        let result = self
            .agent
            .render(&selected, &request.catalog_token, &raw, None)
            .map_err(map_agent_error)?;
        Ok(json!({
            "model_config": serde_json::from_slice::<Value>(&result.model_config).unwrap_or(json!({})),
            "fragments": result.fragments.iter().map(|fragment| json!({
                "agent": fragment.agent.as_str(),
                "role": fragment.role,
                "path": fragment.path,
                "format": fragment.format.as_str(),
                "content": fragment.content,
            })).collect::<Vec<_>>(),
        }))
    }

    fn agent_preview(&self, params: &Value) -> Result<Value, ProtocolError> {
        let request: AgentConfigParams = decode_params(params)?;
        let selected = validate_agent_config(
            &request.agents,
            &request.catalog_token,
            &request.model_config,
        )?;
        let modes = agent_modes(&request.modes)?;
        let raw = serde_json::to_vec(&request.model_config)
            .map_err(|_| invalid_params("model_config must be a bounded JSON object"))?;
        let preview = self
            .agent
            .preview(
                PreviewRequest {
                    agents: selected,
                    modes,
                    catalog_token: request.catalog_token,
                    model_config: raw,
                },
                None,
            )
            .map_err(map_agent_error)?;
        Ok(map_preview(&preview))
    }

    fn agent_write(&self, params: &Value) -> Result<Value, ProtocolError> {
        let request: AgentWriteParams = decode_params(params)?;
        if request.approve_managed_overwrite.is_none()
            || request.approve_codex_auth_change.is_none()
        {
            return Err(invalid_params("write approval fields are required"));
        }
        let selected = validate_agent_config(
            &request.agents,
            &request.catalog_token,
            &request.model_config,
        )?;
        if request.revision_token.trim().is_empty()
            || request.api_key.is_empty()
            || request.api_key.len() > MAX_API_KEY_SIZE
        {
            return Err(invalid_params(
                "revision_token and bounded api_key are required",
            ));
        }
        let Some(trusted) = &self.trusted else {
            return Err(closed_error(
                ErrorCode::ModelCatalogStale,
                "model catalog validation is unavailable",
            ));
        };
        let modes = agent_modes(&request.modes)?;
        let approved = approved_agents(&request.approve_rebuild)?;
        let raw = serde_json::to_vec(&request.model_config)
            .map_err(|_| invalid_params("model_config must be a bounded JSON object"))?;
        let write_request = WriteRequest {
            agents: selected.clone(),
            modes,
            approve_rebuild: approved,
            revision_token: request.revision_token,
            api_key: request.api_key.clone(),
            catalog_token: request.catalog_token.clone(),
            model_config: raw.clone(),
            approve_managed_overwrite: request.approve_managed_overwrite.unwrap_or(false),
            approve_codex_auth_change: request.approve_codex_auth_change.unwrap_or(false),
        };
        self.agent
            .validate_preview(&write_request, None)
            .map_err(map_agent_error)?;
        let binding = self
            .agent
            .catalog_binding(&selected, &request.catalog_token, &raw, None)
            .map_err(map_agent_error)?;
        let api = crate::manager_core::agent::api_url(&binding.router_base_url).map_err(|_| {
            closed_error(ErrorCode::ModelCatalogStale, "Agent model catalog is stale")
        })?;
        let refreshed = trusted.revalidate(
            &binding.owner,
            &request.api_key,
            &crate::manager_core::agent::TrustedBinding {
                router_base_url: binding.router_base_url,
                api_base_url: api,
                deployment_id: binding.deployment_id,
                protocol_version: binding.protocol_version,
            },
        )?;
        validate_refreshed_models(&selected, &raw, &refreshed).map_err(map_agent_error)?;
        let result = self
            .agent
            .write(write_request, None)
            .map_err(map_agent_error)?;
        Ok(map_write(&result))
    }

    fn agent_cleanup_preview(&self, params: &Value) -> Result<Value, ProtocolError> {
        let request: AgentCleanupParams = decode_params(params)?;
        let agent = parse_agent(&request.agent)?;
        let preview = self
            .agent
            .cleanup_preview(agent, None)
            .map_err(map_agent_error)?;
        Ok(map_cleanup_preview(&preview))
    }

    fn agent_cleanup_write(&self, params: &Value) -> Result<Value, ProtocolError> {
        let request: AgentCleanupWriteParams = decode_params(params)?;
        if request.approve_managed_overwrite.is_none() {
            return Err(invalid_params("approve_managed_overwrite is required"));
        }
        if request.revision_token.trim().is_empty() {
            return Err(invalid_params("revision_token is required"));
        }
        let agent = parse_agent(&request.agent)?;
        let result = self
            .agent
            .cleanup_write(
                agent,
                &request.revision_token,
                request.approve_managed_overwrite.unwrap_or(false),
                None,
            )
            .map_err(map_agent_error)?;
        Ok(map_write(&result))
    }

    fn api_key_usage(&self, params: &Value) -> Result<APIKeyUsageResult, ProtocolError> {
        let request: APIKeyUsageParams = decode_params(params)?;
        let period = crate::manager_core::apikeyusage::normalize_period(&request.period)
            .map_err(|_| invalid_params("usage period is invalid"))?;
        if request.api_key.is_empty() || request.api_key.len() > MAX_API_KEY_SIZE {
            return Err(invalid_params("bounded api_key is required"));
        }
        let Some(trusted) = &self.trusted else {
            return Err(closed_error(
                ErrorCode::UsageUnavailable,
                "usage is unavailable",
            ));
        };
        let snapshot =
            trusted.fetch_usage(request.owner.as_str(), period.as_str(), &request.api_key)?;
        Ok(map_usage(&snapshot))
    }
}

fn map_usage(snapshot: &crate::manager_core::apikeyusage::Snapshot) -> APIKeyUsageResult {
    APIKeyUsageResult {
        period: snapshot.period.as_str().to_owned(),
        as_of: snapshot.as_of.clone(),
        summary: APIKeyUsageSummary {
            requests: snapshot.summary.requests,
            prompt_tokens: snapshot.summary.prompt_tokens,
            completion_tokens: snapshot.summary.completion_tokens,
            cost: snapshot.summary.cost,
        },
        quota: snapshot.quota.as_ref().map(|quota| APIKeyUsageQuota {
            used: quota.used,
            limit: quota.limit,
            unit: quota.unit.as_str().to_owned(),
            resets_at: quota.resets_at.clone(),
        }),
        by_model: snapshot
            .by_model
            .iter()
            .map(|model| APIKeyUsageModel {
                model: model.model.clone(),
                requests: model.requests,
                prompt_tokens: model.prompt_tokens,
                completion_tokens: model.completion_tokens,
                cost: model.cost,
            })
            .collect(),
    }
}

fn parse_agents(
    values: &[String],
) -> Result<Vec<crate::manager_core::agent::modelconfig::Agent>, ProtocolError> {
    if values.is_empty() {
        return Err(invalid_params("at least one Agent must be selected"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut selected = Vec::new();
    for value in values {
        let agent = parse_agent(value)?;
        if !seen.insert(agent) {
            return Err(invalid_params("duplicate Agent selection"));
        }
        selected.push(agent);
    }
    Ok(selected)
}

fn parse_agent(
    value: &str,
) -> Result<crate::manager_core::agent::modelconfig::Agent, ProtocolError> {
    crate::manager_core::agent::modelconfig::Agent::parse(value)
        .ok_or_else(|| invalid_params("unsupported Agent selection"))
}

fn agent_modes(
    values: &std::collections::HashMap<String, String>,
) -> Result<
    std::collections::HashMap<crate::manager_core::agent::modelconfig::Agent, ConfigMode>,
    ProtocolError,
> {
    if values.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let mut result = std::collections::HashMap::new();
    for (key, value) in values {
        let agent = parse_agent(key).map_err(|_| invalid_params("unsupported Agent mode"))?;
        let mode =
            ConfigMode::parse(value).ok_or_else(|| invalid_params("unsupported Agent mode"))?;
        result.insert(agent, mode);
    }
    Ok(result)
}

fn approved_agents(
    values: &[String],
) -> Result<Vec<crate::manager_core::agent::modelconfig::Agent>, ProtocolError> {
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for value in values {
        let agent = parse_agent(value).map_err(|_| invalid_params("invalid rebuild approval"))?;
        if !seen.insert(agent) {
            return Err(invalid_params("invalid rebuild approval"));
        }
        result.push(agent);
    }
    Ok(result)
}

fn map_preview(preview: &crate::manager_core::agent::Preview) -> Value {
    let mut files = Vec::new();
    for agent in &preview.agents {
        for file in &agent.files {
            files.push(json!({
                "agent": agent.agent.as_str(),
                "mode": agent.mode.as_str(),
                "path": file.path,
                "role": file.role,
                "format": file.format.as_str(),
                "operation": file.operation.as_str(),
                "backup_required": file.backup.required,
                "backup_pattern": file.backup.pattern,
                "backup_sensitive": file.backup.sensitive,
                "preserves": file.preserves,
                "warning": file.warning,
            }));
        }
    }
    json!({
        "revision_token": preview.revision_token,
        "model_config": preview.model_config,
        "fragments": preview.fragments.iter().map(|fragment| json!({
            "agent": fragment.agent.as_str(),
            "role": fragment.role,
            "path": fragment.path,
            "format": fragment.format.as_str(),
            "content": fragment.content,
        })).collect::<Vec<_>>(),
        "files": files,
        "managed_config_drift": preview.managed_config_drift,
        "drifted_agents": preview.drifted_agents.iter().map(|agent| agent.as_str()).collect::<Vec<_>>(),
        "managed_collisions": preview.managed_collisions.iter().map(|item| json!({
            "agent": item.agent.as_str(),
            "path": item.path,
            "type": item.kind,
            "action": item.action,
        })).collect::<Vec<_>>(),
        "requires_codex_auth_approval": preview.requires_codex_auth_approval,
        "state_change": preview.state_change.as_ref().map(map_file_preview),
        "state_backup": preview.state_backup.as_ref().map(map_file_preview),
    })
}

fn map_cleanup_preview(preview: &crate::manager_core::agent::CleanupPreview) -> Value {
    json!({
        "revision_token": preview.revision_token,
        "agent": preview.agent.as_str(),
        "files": preview.files.iter().map(map_file_preview).collect::<Vec<_>>(),
        "removed_paths": preview.removed_paths,
        "managed_config_drift": preview.managed_config_drift,
        "state_change": preview.state_change.as_ref().map(map_file_preview),
        "state_backup": preview.state_backup.as_ref().map(map_file_preview),
    })
}

fn map_file_preview(file: &crate::manager_core::agent::FilePreview) -> Value {
    json!({
        "path": file.path,
        "role": file.role,
        "format": file.format.as_str(),
        "operation": file.operation.as_str(),
        "backup_required": file.backup.required,
        "backup_pattern": file.backup.pattern,
        "backup_sensitive": file.backup.sensitive,
        "warning": file.backup.warning,
    })
}

fn map_write(result: &crate::manager_core::agent::WriteResult) -> Value {
    json!({
        "transaction_id": result.transaction_id,
        "agents": result.agents.iter().map(|status| json!({
            "agent": status.agent.as_str(),
            "success": status.success,
            "changed": status.changed,
            "backups": status.backups,
            "error_code": status.error_code,
        })).collect::<Vec<_>>(),
        "state_change": result.state_change.as_ref().map(|file| json!({
            "path": file.path,
            "role": "state",
            "format": "json",
            "operation": file.operation.as_str(),
        })),
        "state_backup": result.state_backup.as_ref().map(|file| json!({
            "path": file.path,
            "role": "state",
            "format": "json",
            "operation": file.operation.as_str(),
        })),
    })
}

fn validate_agent_config(
    agents: &[String],
    catalog_token: &str,
    model_config: &Value,
) -> Result<Vec<crate::manager_core::agent::modelconfig::Agent>, ProtocolError> {
    let selected = parse_agents(agents)?;
    if catalog_token.trim().is_empty() {
        return Err(invalid_params("catalog_token is required"));
    }
    let encoded = serde_json::to_vec(model_config)
        .map_err(|_| invalid_params("model_config must be a bounded JSON object"))?;
    if encoded.is_empty() || encoded.len() > MAX_MODEL_CONFIG_SIZE || !model_config.is_object() {
        return Err(invalid_params("model_config must be a bounded JSON object"));
    }
    Ok(selected)
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
