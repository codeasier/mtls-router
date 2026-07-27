use crate::{
    credential::{CredentialError, CredentialStore, MAX_KEY_BYTES},
    error::{CommandError, Result},
    lifecycle::{LifecycleState, OperationOutput, QuitAction},
    manager::ManagerClient,
    model_config::{self, ModelConfig},
    scheduler::PollScheduler,
    types::{
        AgentDetect, AgentFragment, AgentPreview, AgentWriteResult, ComponentVersions,
        CredentialSummary, DesktopPaths, Diagnostics, ManagerInfo, NativeLanguage,
        OccupantInspection, OccupantTerminationResult, PollSnapshot, RecoveryAction, RouterHealth,
        RouterLogs, RouterStatus, RouterVersion,
    },
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, env, path::PathBuf, sync::Arc};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

pub struct AppState {
    pub manager: ManagerClient,
    pub scheduler: PollScheduler,
    pub paths: DesktopPaths,
    pub model_flows: Arc<Mutex<HashMap<String, ModelFlow>>>,
    pub pending_occupant: Arc<Mutex<Option<PendingOccupant>>>,
    pub credentials: Arc<CredentialStore>,
    pub lifecycle: Arc<LifecycleState>,
}

impl AppState {
    fn set_window_visibility(&self, visible: bool) {
        self.scheduler.set_visible(visible);
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ModelFlow {
    agents: Vec<String>,
    models: Vec<String>,
    catalog_token: String,
    modes: Option<HashMap<String, String>>,
}

pub(crate) struct PendingOccupant {
    confirmation_token: Zeroizing<String>,
    manager_session_epoch: u64,
    expires_at: DateTime<Utc>,
}

struct PendingFlow {
    flows: Arc<Mutex<HashMap<String, ModelFlow>>>,
    id: String,
    keep: bool,
}

impl Drop for PendingFlow {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        if let Ok(mut flows) = self.flows.try_lock() {
            flows.remove(&self.id);
            return;
        }
        let flows = self.flows.clone();
        let id = self.id.clone();
        tauri::async_runtime::spawn(async move {
            flows.lock().await.remove(&id);
        });
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModelsRequest {
    pub agents: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfigRequest {
    pub agents: Vec<String>,
    pub flow_id: String,
    pub catalog_token: String,
    pub model_config: ModelConfig,
    pub modes: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWriteRequest {
    pub agents: Vec<String>,
    pub flow_id: String,
    pub catalog_token: String,
    pub model_config: ModelConfig,
    pub revision_token: String,
    pub approve_managed_overwrite: bool,
    pub approve_codex_auth_change: bool,
    pub approve_rebuild: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModelsExisting {
    pub model_config: Value,
    pub unavailable_models: HashMap<String, Vec<String>>,
    pub drifted_agents: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManagerModelsResult {
    models: Vec<String>,
    catalog_token: String,
    router_base_url: String,
    api_base_url: String,
    existing: AgentModelsExisting,
    preset: AgentModelsPreset,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetUnavailable {
    code: ModelUnavailableCode,
    models: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum ModelUnavailableCode {
    #[serde(rename = "MODEL_NOT_AVAILABLE")]
    ModelNotAvailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentModelsPreset {
    pub model_config: Value,
    pub unavailable_agents: HashMap<String, AgentPresetUnavailable>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentModelsResult {
    pub flow_id: String,
    pub models: Vec<String>,
    pub catalog_token: String,
    pub router_base_url: String,
    pub api_base_url: String,
    pub existing: AgentModelsExisting,
    pub preset: AgentModelsPreset,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentRenderResult {
    pub model_config: Value,
    pub fragments: Vec<AgentFragment>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForceTerminateOccupantRequest {
    pub confirmation_token: String,
}

#[tauri::command]
pub async fn router_status(state: tauri::State<'_, AppState>) -> Result<RouterStatus> {
    let value: RouterStatus = state.manager.call("router.status", json!({})).await?;
    state.scheduler.set_status(value.clone()).await;
    Ok(value)
}

#[tauri::command]
pub async fn router_start(
    owner: String,
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<RouterStatus> {
    if owner != "desktop" {
        return Err(CommandError::invalid_params("owner must be desktop"));
    }
    let output = run_lifecycle_command(
        &state.lifecycle,
        crate::orchestration::start(&state.manager, &state.scheduler),
    )
    .await?;
    resume_quit_after_operation(&app, output.quit_action);
    output.value
}

#[tauri::command]
pub async fn router_stop(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<RouterStatus> {
    let output = run_lifecycle_command(
        &state.lifecycle,
        crate::orchestration::stop(&state.manager, &state.scheduler),
    )
    .await?;
    resume_quit_after_operation(&app, output.quit_action);
    output.value
}

#[tauri::command]
pub async fn router_inspect_occupant(
    state: tauri::State<'_, AppState>,
) -> Result<OccupantInspection> {
    inspect_occupant_command(&state.manager, &state.pending_occupant).await
}

async fn inspect_occupant_command(
    manager: &ManagerClient,
    pending_occupant: &Arc<Mutex<Option<PendingOccupant>>>,
) -> Result<OccupantInspection> {
    let mut pending = pending_occupant.lock().await;
    pending.take();
    let (inspection, manager_session_epoch): (OccupantInspection, u64) = manager
        .call_with_session_epoch("router.inspect_occupant", json!({}))
        .await?;
    if inspection.recovery.action == RecoveryAction::ForceTerminate {
        let confirmation_token = inspection
            .confirmation_token
            .as_ref()
            .expect("strict forceable inspection has a token")
            .clone();
        *pending = Some(PendingOccupant {
            confirmation_token: Zeroizing::new(confirmation_token),
            manager_session_epoch,
            expires_at: DateTime::parse_from_rfc3339(
                inspection
                    .expires_at
                    .as_deref()
                    .expect("strict forceable inspection has an expiry"),
            )
            .expect("strict forceable inspection expiry is RFC3339")
            .with_timezone(&Utc),
        });
    }
    Ok(inspection)
}

#[tauri::command]
pub async fn router_force_terminate_occupant(
    request: ForceTerminateOccupantRequest,
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<OccupantTerminationResult> {
    let output = run_lifecycle_command(
        &state.lifecycle,
        force_terminate_occupant_command(
            request,
            &state.manager,
            &state.scheduler,
            &state.pending_occupant,
        ),
    )
    .await?;
    resume_quit_after_operation(&app, output.quit_action);
    output.value
}

async fn run_lifecycle_command<F, T>(
    lifecycle: &LifecycleState,
    operation: F,
) -> Result<OperationOutput<Result<T>>>
where
    F: std::future::Future<Output = Result<T>>,
{
    lifecycle.run_operation(operation).await.ok_or_else(|| {
        CommandError::new(
            "OPERATION_IN_PROGRESS",
            "another Router lifecycle operation is in progress",
        )
    })
}

fn resume_quit_after_operation(app: &AppHandle, action: QuitAction) {
    if action == QuitAction::ExecuteQuit {
        crate::tray::execute_quit(app.clone());
    }
}

async fn force_terminate_occupant_command(
    request: ForceTerminateOccupantRequest,
    manager: &ManagerClient,
    scheduler: &PollScheduler,
    pending_occupant: &Arc<Mutex<Option<PendingOccupant>>>,
) -> Result<OccupantTerminationResult> {
    force_terminate_occupant_command_at(request, manager, scheduler, pending_occupant, Utc::now())
        .await
}

async fn force_terminate_occupant_command_at(
    mut request: ForceTerminateOccupantRequest,
    manager: &ManagerClient,
    scheduler: &PollScheduler,
    pending_occupant: &Arc<Mutex<Option<PendingOccupant>>>,
    now: DateTime<Utc>,
) -> Result<OccupantTerminationResult> {
    if request.confirmation_token.trim().is_empty() || request.confirmation_token.len() > 4096 {
        request.confirmation_token.zeroize();
        return Err(CommandError::invalid_params(
            "confirmation token is invalid",
        ));
    }
    let mut token = std::mem::take(&mut request.confirmation_token);
    let mut cached = pending_occupant.lock().await;
    if cached.as_ref().is_some_and(|pending| {
        pending.manager_session_epoch != manager.session_epoch() || now >= pending.expires_at
    }) {
        cached.take();
        token.zeroize();
        return Err(CommandError::invalid_params(
            "inspected occupant confirmation has expired",
        ));
    }
    if !cached
        .as_ref()
        .is_some_and(|pending| pending.confirmation_token.as_str() == token)
    {
        token.zeroize();
        return Err(CommandError::invalid_params(
            "confirmation token does not match the inspected occupant",
        ));
    }
    let pending = cached.take().expect("matching pending occupant exists");
    crate::orchestration::force_terminate_occupant(
        token,
        pending.manager_session_epoch,
        manager,
        scheduler,
    )
    .await
}

#[tauri::command]
pub async fn router_cancel_release_observation(state: tauri::State<'_, AppState>) -> Result<()> {
    state.scheduler.cancel_release_observation().await;
    Ok(())
}

#[tauri::command]
pub async fn router_health(state: tauri::State<'_, AppState>) -> Result<RouterHealth> {
    let value: RouterHealth = state.manager.call("router.health", json!({})).await?;
    state.scheduler.set_health(value.clone()).await;
    Ok(value)
}

#[tauri::command]
pub async fn poll_snapshot(state: tauri::State<'_, AppState>) -> Result<PollSnapshot> {
    Ok(state.scheduler.snapshot().await)
}

#[tauri::command]
pub async fn router_logs(limit: u16, state: tauri::State<'_, AppState>) -> Result<RouterLogs> {
    validate_log_limit(limit)?;
    state
        .manager
        .call("router.logs", json!({ "limit": limit }))
        .await
}

#[tauri::command]
pub async fn component_versions(state: tauri::State<'_, AppState>) -> Result<ComponentVersions> {
    let manager: ManagerInfo = state.manager.call("manager.info", json!({})).await?;
    let router = state
        .manager
        .call::<RouterVersion>("router.version", json!({}))
        .await
        .ok();
    Ok(ComponentVersions {
        desktop: env!("CARGO_PKG_VERSION").to_owned(),
        manager: manager.version,
        router: router
            .as_ref()
            .map(|value| value.version.clone())
            .unwrap_or_default(),
        management_protocol: manager.management_protocol_version,
    })
}

#[tauri::command]
pub async fn diagnostics_collect(state: tauri::State<'_, AppState>) -> Result<Diagnostics> {
    state.manager.call("diagnostics.collect", json!({})).await
}

#[tauri::command]
pub fn open_log_location(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<()> {
    let directory = PathBuf::from(&state.paths.log_file)
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| CommandError::new("INVALID_PATH", "log location is unavailable"))?;
    app.opener()
        .open_path(directory.to_string_lossy(), None::<&str>)
        .map_err(|_| CommandError::new("OPEN_FAILED", "cannot open the log location"))
}

#[tauri::command]
pub async fn agent_detect(state: tauri::State<'_, AppState>) -> Result<AgentDetect> {
    agent_detect_command(&state.manager).await
}

#[tauri::command]
pub async fn agent_models(
    request: AgentModelsRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AgentModelsResult> {
    agent_models_command(
        request,
        &state.manager,
        state.model_flows.clone(),
        &state.credentials,
    )
    .await
}

async fn agent_models_command(
    request: AgentModelsRequest,
    manager: &ManagerClient,
    model_flows: Arc<Mutex<HashMap<String, ModelFlow>>>,
    credentials: &CredentialStore,
) -> Result<AgentModelsResult> {
    validate_agents(&request.agents)?;
    let flow_id = Uuid::new_v4().to_string();
    model_flows.lock().await.insert(
        flow_id.clone(),
        ModelFlow {
            agents: request.agents.clone(),
            models: Vec::new(),
            catalog_token: String::new(),
            modes: None,
        },
    );
    let mut pending = PendingFlow {
        flows: model_flows.clone(),
        id: flow_id.clone(),
        keep: false,
    };
    let params = json!({ "owner": "desktop", "agents": request.agents });
    let key = credentials.use_().await.map_err(CommandError::from)?;
    let result: ManagerModelsResult = manager.call_with_key("agent.models", params, key).await?;
    validate_models_result(&result, &request.agents)?;
    {
        let mut flows = model_flows.lock().await;
        let flow = flows.get_mut(&flow_id).ok_or_else(flow_expired)?;
        flow.models = result.models.clone();
        flow.catalog_token = result.catalog_token.clone();
    }
    pending.keep = true;
    Ok(AgentModelsResult {
        flow_id,
        models: result.models,
        catalog_token: result.catalog_token,
        router_base_url: result.router_base_url,
        api_base_url: result.api_base_url,
        existing: result.existing,
        preset: result.preset,
    })
}

#[tauri::command]
pub async fn agent_render(
    request: AgentConfigRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AgentRenderResult> {
    agent_render_command(request, &state.manager, &state.model_flows).await
}

async fn agent_render_command(
    request: AgentConfigRequest,
    manager: &ManagerClient,
    model_flows: &Arc<Mutex<HashMap<String, ModelFlow>>>,
) -> Result<AgentRenderResult> {
    if request
        .modes
        .as_ref()
        .is_some_and(|modes| !modes.is_empty())
    {
        return Err(CommandError::invalid_params(
            "modes are supported only by Agent preview and write",
        ));
    }
    validate_config_request(&request, model_flows).await?;
    manager.call("agent.render", json!({ "agents": request.agents, "catalog_token": request.catalog_token, "model_config": request.model_config })).await
}

#[tauri::command]
pub async fn agent_preview(
    request: AgentConfigRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AgentPreview> {
    agent_preview_command(request, &state.manager, &state.model_flows).await
}

async fn agent_preview_command(
    request: AgentConfigRequest,
    manager: &ManagerClient,
    model_flows: &Arc<Mutex<HashMap<String, ModelFlow>>>,
) -> Result<AgentPreview> {
    let modes = validate_agent_modes(&request.agents, request.modes.as_ref())?;
    validate_config_request(&request, model_flows).await?;
    let result = manager
        .call(
            "agent.preview",
            json!({
                "agents": request.agents,
                "modes": modes,
                "catalog_token": request.catalog_token,
                "model_config": request.model_config,
            }),
        )
        .await?;
    let mut flows = model_flows.lock().await;
    let flow = flows.get_mut(&request.flow_id).ok_or_else(flow_expired)?;
    if flow.agents != request.agents || flow.catalog_token != request.catalog_token {
        return Err(flow_expired());
    }
    flow.modes = Some(modes);
    Ok(result)
}

async fn agent_detect_command(manager: &ManagerClient) -> Result<AgentDetect> {
    manager.call("agent.detect", json!({})).await
}

#[tauri::command]
pub async fn agent_write(
    request: AgentWriteRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AgentWriteResult> {
    agent_write_command(
        request,
        &state.manager,
        &state.model_flows,
        &state.credentials,
    )
    .await
}

async fn agent_write_command(
    request: AgentWriteRequest,
    manager: &ManagerClient,
    model_flows: &Arc<Mutex<HashMap<String, ModelFlow>>>,
    credentials: &CredentialStore,
) -> Result<AgentWriteResult> {
    validate_agents(&request.agents)?;
    if request.revision_token.trim().is_empty() || request.revision_token.len() > 512 * 1024 {
        return Err(CommandError::invalid_params("revision token is invalid"));
    }
    let model_config_value = serde_json::to_value(&request.model_config)
        .map_err(|_| CommandError::invalid_params("model config is invalid"))?;
    let expected_flow = {
        let flows = model_flows.lock().await;
        let flow = flows.get(&request.flow_id).ok_or_else(flow_expired)?;
        if flow.agents != request.agents || flow.catalog_token != request.catalog_token {
            return Err(flow_expired());
        }
        let modes = flow.modes.as_ref().ok_or_else(|| {
            CommandError::invalid_params("a successful Agent preview is required before write")
        })?;
        validate_rebuild_approval(&request.agents, modes, &request.approve_rebuild)?;
        model_config::validate(&request.model_config, &request.agents, &flow.models)?;
        flow.clone()
    };
    let current_key = credentials.use_().await.map_err(CommandError::from)?;
    if contains_exact_string(&model_config_value, &current_key) {
        return Err(CommandError::invalid_params(
            "model config contains a credential value",
        ));
    }
    let flow = {
        let mut flows = model_flows.lock().await;
        if flows.get(&request.flow_id) != Some(&expected_flow) {
            return Err(flow_expired());
        }
        flows.remove(&request.flow_id).ok_or_else(flow_expired)?
    };
    let params = json!({
        "agents": request.agents,
        "catalog_token": request.catalog_token,
        "model_config": request.model_config,
        "revision_token": request.revision_token,
        "approve_managed_overwrite": request.approve_managed_overwrite,
        "approve_codex_auth_change": request.approve_codex_auth_change,
        "modes": flow.modes.as_ref().expect("validated bound preview modes"),
        "approve_rebuild": request.approve_rebuild,
    });
    match manager
        .call_with_key("agent.write", params, current_key)
        .await
    {
        Err(error) if error.code == "PREVIEW_STALE" => {
            model_flows.lock().await.insert(request.flow_id, flow);
            Err(error)
        }
        result => result,
    }
}

#[tauri::command]
pub async fn agent_model_flow_destroy(
    flow_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<()> {
    validate_flow_id(&flow_id)?;
    state.model_flows.lock().await.remove(&flow_id);
    Ok(())
}

#[tauri::command]
pub async fn agent_model_config_import(
    content: String,
    agents: Vec<String>,
    flow_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ModelConfig> {
    validate_agents(&agents)?;
    let flows = state.model_flows.lock().await;
    let flow = flows.get(&flow_id).ok_or_else(flow_expired)?;
    if flow.agents != agents {
        return Err(flow_expired());
    }
    model_config::import_json(&content, &agents, &flow.models)
}

#[tauri::command]
pub async fn agent_model_config_export(
    model_config: ModelConfig,
    agents: Vec<String>,
    flow_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<String> {
    validate_agents(&agents)?;
    let flows = state.model_flows.lock().await;
    let flow = flows.get(&flow_id).ok_or_else(flow_expired)?;
    if flow.agents != agents {
        return Err(flow_expired());
    }
    model_config::export_json(&model_config, &agents, &flow.models)
}

#[tauri::command]
pub fn desktop_paths(state: tauri::State<'_, AppState>) -> DesktopPaths {
    state.paths.clone()
}

async fn get_credential_command(credentials: &CredentialStore) -> Result<CredentialSummary> {
    match credentials.read_summary().await {
        Ok(summary) => Ok(summary),
        Err(CredentialError::NotFound) => Ok(CredentialSummary::default()),
        Err(error) => Err(error.into()),
    }
}

#[tauri::command]
pub async fn get_credential(state: tauri::State<'_, AppState>) -> Result<CredentialSummary> {
    get_credential_command(&state.credentials).await
}

async fn save_credential_command(
    mut api_key: String,
    credentials: &CredentialStore,
) -> Result<CredentialSummary> {
    let trimmed = api_key.trim().to_owned();
    api_key.zeroize();
    if trimmed.is_empty() || trimmed.len() > MAX_KEY_BYTES {
        return Err(CommandError::credential_invalid());
    }
    credentials
        .write(Zeroizing::new(trimmed))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn save_credential(
    api_key: String,
    state: tauri::State<'_, AppState>,
) -> Result<CredentialSummary> {
    save_credential_command(api_key, &state.credentials).await
}

async fn delete_credential_command(credentials: &CredentialStore) -> Result<CredentialSummary> {
    credentials.delete().await.map_err(CommandError::from)?;
    Ok(CredentialSummary::default())
}

#[tauri::command]
pub async fn delete_credential(state: tauri::State<'_, AppState>) -> Result<CredentialSummary> {
    delete_credential_command(&state.credentials).await
}

#[tauri::command]
pub async fn window_visibility(visible: bool, state: tauri::State<'_, AppState>) -> Result<()> {
    state.set_window_visibility(visible);
    Ok(())
}

#[tauri::command]
pub fn set_native_language(language: String, app: AppHandle) -> Result<()> {
    let language = NativeLanguage::parse(&language)
        .ok_or_else(|| CommandError::invalid_params("language must be zh-CN or en"))?;
    crate::tray::set_language(&app, language).map_err(|_| {
        CommandError::new(
            "NATIVE_UI_UPDATE_FAILED",
            "could not update native interface language",
        )
    })
}

pub fn validate_log_limit(limit: u16) -> Result<()> {
    if !(1..=200).contains(&limit) {
        return Err(CommandError::invalid_params(
            "log limit must be between 1 and 200",
        ));
    }
    Ok(())
}

pub fn validate_agents(agents: &[String]) -> Result<()> {
    if agents.is_empty() || agents.len() > 3 {
        return Err(CommandError::invalid_params(
            "one to three Agents must be selected",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    if agents.iter().any(|agent| {
        !matches!(agent.as_str(), "claude" | "opencode" | "codex") || !seen.insert(agent)
    }) {
        return Err(CommandError::invalid_params("Agent selection is invalid"));
    }
    Ok(())
}

fn validate_flow_id(flow_id: &str) -> Result<()> {
    match Uuid::parse_str(flow_id) {
        Ok(id) if id.get_version_num() == 4 => Ok(()),
        _ => Err(CommandError::invalid_params("model flow is invalid")),
    }
}

fn flow_expired() -> CommandError {
    CommandError::new(
        "MODEL_FLOW_EXPIRED",
        "enter the API key and discover models again",
    )
}

fn validate_models_result(result: &ManagerModelsResult, requested_agents: &[String]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    if result.models.is_empty()
        || result.models.len() > model_config::MAX_REFERENCED_MODELS_PER_AGENT
        || result.catalog_token.is_empty()
        || result.catalog_token.len() > 512 * 1024
        || result
            .models
            .iter()
            .any(|model| !model_config::valid_model_id(model) || !seen.insert(model))
    {
        return Err(CommandError::new(
            "INVALID_RESPONSE",
            "manager returned an invalid model catalog",
        ));
    }
    validate_preset_result(&result.preset, &result.models, requested_agents)?;
    Ok(())
}

fn validate_preset_result(
    preset: &AgentModelsPreset,
    catalog: &[String],
    requested_agents: &[String],
) -> Result<()> {
    let invalid_response = || {
        CommandError::new(
            "INVALID_RESPONSE",
            "manager returned invalid model preset metadata",
        )
    };
    let object = preset
        .model_config
        .as_object()
        .ok_or_else(invalid_response)?;
    let allowed = ["version", "claude", "opencode", "codex"];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid_response());
    }
    let agents: Vec<String> = ["claude", "opencode", "codex"]
        .into_iter()
        .filter(|agent| object.contains_key(*agent))
        .map(str::to_owned)
        .collect();
    if agents.iter().any(|agent| !requested_agents.contains(agent)) {
        return Err(invalid_response());
    }
    if agents.is_empty() {
        if !object.is_empty() {
            return Err(invalid_response());
        }
    } else {
        let config: ModelConfig =
            serde_json::from_value(preset.model_config.clone()).map_err(|_| invalid_response())?;
        model_config::validate(&config, &agents, catalog).map_err(|_| invalid_response())?;
    }
    for (agent, unavailable) in &preset.unavailable_agents {
        if !requested_agents.contains(agent)
            || unavailable.models.is_empty()
            || unavailable.models.len() > model_config::MAX_REFERENCED_MODELS_PER_AGENT
            || agents.contains(agent)
        {
            return Err(invalid_response());
        }
        if unavailable
            .models
            .iter()
            .any(|model| !model_config::valid_model_id(model) || catalog.contains(model))
            || unavailable.models.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(invalid_response());
        }
    }
    Ok(())
}

async fn validate_config_request(
    request: &AgentConfigRequest,
    model_flows: &Arc<Mutex<HashMap<String, ModelFlow>>>,
) -> Result<()> {
    validate_agents(&request.agents)?;
    validate_flow_id(&request.flow_id)?;
    if request.catalog_token.is_empty() || request.catalog_token.len() > 512 * 1024 {
        return Err(CommandError::invalid_params("catalog token is invalid"));
    }
    let flows = model_flows.lock().await;
    let flow = flows.get(&request.flow_id).ok_or_else(flow_expired)?;
    if flow.agents != request.agents || flow.catalog_token != request.catalog_token {
        return Err(flow_expired());
    }
    model_config::validate(&request.model_config, &request.agents, &flow.models)
}

fn validate_agent_modes(
    agents: &[String],
    modes: Option<&HashMap<String, String>>,
) -> Result<HashMap<String, String>> {
    validate_agents(agents)?;
    let modes = modes.ok_or_else(|| CommandError::invalid_params("Agent modes are required"))?;
    if modes.len() != agents.len() {
        return Err(CommandError::invalid_params(
            "Agent modes must exactly match selected Agents",
        ));
    }
    let mut normalized = HashMap::with_capacity(agents.len());
    for agent in agents {
        let mode = modes.get(agent).ok_or_else(|| {
            CommandError::invalid_params("Agent modes must exactly match selected Agents")
        })?;
        if !matches!(mode.as_str(), "merge" | "rebuild") {
            return Err(CommandError::invalid_params(
                "Agent mode must be merge or rebuild",
            ));
        }
        normalized.insert(agent.clone(), mode.clone());
    }
    Ok(normalized)
}

fn validate_rebuild_approval(
    agents: &[String],
    modes: &HashMap<String, String>,
    approval: &[String],
) -> Result<()> {
    let mut approved = std::collections::HashSet::with_capacity(approval.len());
    if approval.iter().any(|agent| !approved.insert(agent)) {
        return Err(CommandError::invalid_params(
            "rebuild approval contains duplicate Agents",
        ));
    }
    let rebuild = agents
        .iter()
        .filter(|agent| modes.get(*agent).is_some_and(|mode| mode == "rebuild"))
        .collect::<std::collections::HashSet<_>>();
    if approved != rebuild {
        return Err(CommandError::invalid_params(
            "rebuild approval must exactly match previewed rebuild Agents",
        ));
    }
    Ok(())
}

fn contains_exact_string(value: &Value, secret: &str) -> bool {
    match value {
        Value::String(value) => value == secret,
        Value::Array(values) => values
            .iter()
            .any(|value| contains_exact_string(value, secret)),
        Value::Object(values) => values
            .values()
            .any(|value| contains_exact_string(value, secret)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{TransportChild, TransportEvent, TransportFactory, TransportSession};
    use serde_json::Value;
    use std::{collections::VecDeque, sync::Mutex as StdMutex, time::Duration};
    use tokio::sync::mpsc;

    struct FakeChild {
        events: mpsc::Sender<TransportEvent>,
        requests: Arc<StdMutex<Vec<Value>>>,
        responses: Arc<StdMutex<VecDeque<Vec<u8>>>>,
    }

    impl TransportChild for FakeChild {
        fn write(&mut self, bytes: &[u8]) -> Result<()> {
            let request: Value = serde_json::from_slice(bytes).unwrap();
            self.requests.lock().unwrap().push(request.clone());
            let id = request["id"].clone();
            let method = request["method"].as_str().unwrap();
            let response = if method == "manager.info" {
                serde_json::to_vec(&json!({
                    "id": id,
                    "result": {
                        "version": env!("MTLS_MANAGER_VERSION"),
                        "commit": "test",
                        "build_date": "test",
                        "target": env!("MTLS_MANAGER_TARGET"),
                        "deployment_id": env!("MTLS_DEPLOYMENT_ID"),
                        "management_protocol_version": env!("MTLS_MANAGEMENT_PROTOCOL_VERSION")
                    }
                }))
                .unwrap()
            } else {
                self.responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| b"not-json".to_vec())
            };
            let events = self.events.clone();
            tauri::async_runtime::spawn(async move {
                let response = if response == b"delay" {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    b"not-json".to_vec()
                } else {
                    response
                };
                let _ = events.send(TransportEvent::Stdout(response)).await;
            });
            Ok(())
        }

        fn kill(self: Box<Self>) {}
    }

    struct FakeFactory {
        requests: Arc<StdMutex<Vec<Value>>>,
        responses: Arc<StdMutex<VecDeque<Vec<u8>>>>,
    }

    impl TransportFactory for FakeFactory {
        fn spawn(&self) -> Result<TransportSession> {
            let (events, receiver) = mpsc::channel(8);
            Ok(TransportSession {
                child: Box::new(FakeChild {
                    events,
                    requests: self.requests.clone(),
                    responses: self.responses.clone(),
                }),
                events: receiver,
            })
        }
    }

    fn fake_client(responses: Vec<Vec<u8>>) -> (ManagerClient, Arc<StdMutex<Vec<Value>>>) {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let factory = FakeFactory {
            requests: requests.clone(),
            responses: Arc::new(StdMutex::new(responses.into())),
        };
        (ManagerClient::new(Arc::new(factory)), requests)
    }

    struct CredentialTempDir(PathBuf);

    impl Drop for CredentialTempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn test_credentials(name: &str) -> (CredentialTempDir, Arc<CredentialStore>) {
        let directory = std::env::temp_dir().join(format!(
            "mtls-router-command-credential-{name}-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let store = Arc::new(CredentialStore::new(directory.join("credentials.json")));
        (CredentialTempDir(directory), store)
    }

    async fn configured_credentials(
        name: &str,
        key: &str,
    ) -> (CredentialTempDir, Arc<CredentialStore>) {
        let (directory, store) = test_credentials(name);
        store.write(Zeroizing::new(key.to_owned())).await.unwrap();
        (directory, store)
    }

    fn mixed_model_config() -> ModelConfig {
        serde_json::from_value(json!({
            "version": 1,
            "claude": {
                "primary": {"model": "m1"},
                "haiku": {"inherit_primary": true},
                "sonnet": {"inherit_primary": true},
                "opus": {"inherit_primary": true}
            },
            "opencode": {"default_model": "m1", "models": {"m1": {}}}
        }))
        .unwrap()
    }

    fn mixed_flow(modes: Option<HashMap<String, String>>) -> ModelFlow {
        ModelFlow {
            agents: vec!["claude".into(), "opencode".into()],
            models: vec!["m1".into()],
            catalog_token: "catalog".into(),
            modes,
        }
    }

    fn preview_response() -> Vec<u8> {
        serde_json::to_vec(&json!({"id":"desktop-1","result":{
            "revision_token":"revision",
            "model_config": serde_json::to_value(mixed_model_config()).unwrap(),
            "fragments":[],
            "files":[],
            "managed_config_drift":false,
            "drifted_agents":[],
            "managed_collisions":[],
            "requires_codex_auth_approval":false,
            "state_change":null,
            "state_backup":null
        }}))
        .unwrap()
    }

    fn write_request(flow_id: String, approval: Vec<String>) -> AgentWriteRequest {
        AgentWriteRequest {
            agents: vec!["claude".into(), "opencode".into()],
            flow_id,
            catalog_token: "catalog".into(),
            model_config: mixed_model_config(),
            revision_token: "revision".into(),
            approve_managed_overwrite: false,
            approve_codex_auth_change: false,
            approve_rebuild: approval,
        }
    }

    #[test]
    fn command_arguments_are_restricted() {
        assert!(validate_log_limit(0).is_err());
        assert!(validate_log_limit(200).is_ok());
        assert!(validate_log_limit(201).is_err());
        assert!(validate_agents(&["claude".to_owned(), "codex".to_owned()]).is_ok());
        assert!(validate_agents(&["shell".to_owned()]).is_err());
        assert!(validate_agents(&["claude".to_owned(), "claude".to_owned()]).is_err());
    }

    #[test]
    fn credential_commands_round_trip_without_exposing_the_key() {
        tauri::async_runtime::block_on(async {
            let (_directory, credentials) = test_credentials("commands-round-trip");
            assert_eq!(
                get_credential_command(&credentials).await.unwrap(),
                CredentialSummary::default()
            );

            let saved = save_credential_command("  fixture-secret  ".into(), &credentials)
                .await
                .unwrap();
            assert!(saved.present);
            assert_eq!(saved.fingerprint.len(), 4);
            assert_eq!(get_credential_command(&credentials).await.unwrap(), saved);
            assert_eq!(credentials.use_().await.unwrap().as_str(), "fixture-secret");

            let deleted = delete_credential_command(&credentials).await.unwrap();
            assert_eq!(deleted, CredentialSummary::default());
            assert!(matches!(
                credentials.use_().await,
                Err(CredentialError::NotFound)
            ));
            assert_eq!(
                save_credential_command(String::new(), &credentials)
                    .await
                    .unwrap_err()
                    .code,
                "CREDENTIAL_INVALID"
            );
        });
    }

    #[test]
    fn credential_save_models_delete_models_round_trip() {
        tauri::async_runtime::block_on(async {
            let models_response = |id: &str| {
                serde_json::to_vec(&json!({"id": id, "result": {
                    "models": ["m1"],
                    "catalog_token": "catalog",
                    "router_base_url": "http://127.0.0.1:19099",
                    "api_base_url": "http://127.0.0.1:19099/v1",
                    "existing": {
                        "model_config": {},
                        "unavailable_models": {},
                        "drifted_agents": []
                    },
                    "preset": {"model_config": {}, "unavailable_agents": {}}
                }}))
                .unwrap()
            };
            let (manager, requests) = fake_client(vec![models_response("desktop-1")]);
            let (_directory, credentials) = test_credentials("full-round-trip");
            let flows = Arc::new(Mutex::new(HashMap::new()));

            save_credential_command("fixture-secret".into(), &credentials)
                .await
                .unwrap();
            let first = agent_models_command(
                AgentModelsRequest {
                    agents: vec!["claude".into()],
                },
                &manager,
                flows.clone(),
                &credentials,
            )
            .await
            .unwrap();
            flows.lock().await.remove(&first.flow_id);
            delete_credential_command(&credentials).await.unwrap();
            let error = agent_models_command(
                AgentModelsRequest {
                    agents: vec!["claude".into()],
                },
                &manager,
                flows.clone(),
                &credentials,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, "CREDENTIAL_NOT_FOUND");
            tokio::task::yield_now().await;
            assert!(flows.lock().await.is_empty());

            let requests = requests.lock().unwrap();
            let model_requests = requests
                .iter()
                .filter(|request| request["method"] == "agent.models")
                .collect::<Vec<_>>();
            assert_eq!(model_requests.len(), 1);
            assert_eq!(model_requests[0]["params"]["api_key"], "fixture-secret");
        });
    }

    #[test]
    fn flow_ids_and_catalog_results_are_strictly_bounded() {
        assert!(validate_flow_id(&Uuid::new_v4().to_string()).is_ok());
        assert!(validate_flow_id("guessable").is_err());
        let result = ManagerModelsResult {
            models: vec!["model-a".into()],
            catalog_token: "token".into(),
            router_base_url: "http://127.0.0.1:19099".into(),
            api_base_url: "http://127.0.0.1:19099/v1".into(),
            existing: AgentModelsExisting {
                model_config: json!({}),
                unavailable_models: HashMap::new(),
                drifted_agents: vec![],
            },
            preset: AgentModelsPreset {
                model_config: json!({}),
                unavailable_agents: HashMap::new(),
            },
        };
        assert!(validate_models_result(&result, &["claude".into()]).is_ok());
    }

    #[test]
    fn concurrent_router_lifecycle_command_returns_stable_busy_error() {
        tauri::async_runtime::block_on(async {
            let lifecycle = LifecycleState::default();
            let output = lifecycle
                .run_operation(async {
                    run_lifecycle_command(&lifecycle, async { Ok::<_, CommandError>(()) })
                        .await
                        .unwrap_err()
                })
                .await
                .unwrap();

            assert_eq!(output.value.code, "OPERATION_IN_PROGRESS");
            let failure = run_lifecycle_command(&lifecycle, async {
                Err::<(), _>(CommandError::new("START_FAILED", "start failed"))
            })
            .await
            .unwrap();
            assert_eq!(failure.value.unwrap_err().code, "START_FAILED");
            assert!(run_lifecycle_command(&lifecycle, async { Ok(()) })
                .await
                .is_ok());
        });
    }

    #[test]
    fn window_visibility_is_not_a_model_flow_lifecycle_boundary() {
        tauri::async_runtime::block_on(async {
            let (manager, _) = fake_client(vec![]);
            let scheduler = PollScheduler::new(manager.clone());
            let (_credential_dir, credentials) = test_credentials("visibility");
            let flow_id = Uuid::new_v4().to_string();
            let flows = Arc::new(Mutex::new(HashMap::from([(
                flow_id.clone(),
                ModelFlow {
                    agents: vec!["claude".into()],
                    models: vec!["m1".into()],
                    catalog_token: "catalog".into(),
                    modes: None,
                },
            )])));
            let state = AppState {
                manager,
                scheduler,
                paths: DesktopPaths {
                    data_dir: String::new(),
                    log_file: String::new(),
                    credentials_path: String::new(),
                    can_prepare_for_uninstall: false,
                },
                model_flows: flows.clone(),
                pending_occupant: Default::default(),
                credentials,
                lifecycle: Default::default(),
            };

            state.set_window_visibility(false);

            let flows = flows.lock().await;
            assert!(flows.contains_key(&flow_id));
        });
    }

    #[test]
    fn preset_result_shape_models_and_agent_keys_are_strict() {
        let valid: ManagerModelsResult = serde_json::from_value(json!({
            "models": ["m1"],
            "catalog_token": "token",
            "router_base_url": "http://127.0.0.1:19099",
            "api_base_url": "http://127.0.0.1:19099/v1",
            "existing": {"model_config": {}, "unavailable_models": {}, "drifted_agents": []},
            "preset": {
                "model_config": {"version":1,"claude":{"primary":{"model":"m1","context":"1m"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}},
                "unavailable_agents": {"codex":{"code":"MODEL_NOT_AVAILABLE","models":["missing"]}}
            }
        })).unwrap();
        assert!(validate_models_result(&valid, &["claude".into(), "codex".into()]).is_ok());

        for invalid in [
            json!({"model_config": {}, "unavailable_agents": {"shell":{"code":"MODEL_NOT_AVAILABLE","models":["missing"]}}}),
            json!({"model_config": {}, "unavailable_agents": {"codex":{"code":"OTHER","models":["missing"]}}}),
            json!({"model_config": {"version":1,"codex":{"model":"missing"}}, "unavailable_agents": {}}),
            json!({"model_config": {}, "unavailable_agents": {}, "secret":"canary"}),
        ] {
            let mut response = serde_json::to_value(&valid).unwrap();
            response["preset"] = invalid;
            match serde_json::from_value::<ManagerModelsResult>(response) {
                Ok(result) => assert!(validate_models_result(
                    &result,
                    &["claude".into(), "codex".into()]
                )
                .is_err()),
                Err(_) => {}
            }
        }
    }

    #[test]
    fn preset_unavailable_models_accept_boundary_and_reject_malformed_metadata() {
        let unavailable = (0..model_config::MAX_REFERENCED_MODELS_PER_AGENT)
            .map(|index| format!("missing-{index:04}"))
            .collect::<Vec<_>>();
        let response = |models: Vec<String>| {
            serde_json::from_value::<ManagerModelsResult>(json!({
                "models": ["available"],
                "catalog_token": "token",
                "router_base_url": "http://127.0.0.1:19099",
                "api_base_url": "http://127.0.0.1:19099/v1",
                "existing": {"model_config": {}, "unavailable_models": {}, "drifted_agents": []},
                "preset": {"model_config": {}, "unavailable_agents": {
                    "opencode": {"code": "MODEL_NOT_AVAILABLE", "models": models}
                }}
            }))
            .unwrap()
        };

        assert!(
            validate_models_result(&response(unavailable.clone()), &["opencode".into()]).is_ok()
        );
        for malformed in [
            {
                let mut values = unavailable.clone();
                values.push("missing-overflow".into());
                values
            },
            vec![" leading-space".into()],
            vec!["trailing-space\u{00a0}".into()],
            vec!["available".into()],
            vec!["duplicate".into(), "duplicate".into()],
            vec!["z".into(), "a".into()],
        ] {
            assert!(validate_models_result(&response(malformed), &["opencode".into()]).is_err());
        }

        let mut contradictory = response(vec!["missing".into()]);
        contradictory.preset.model_config = json!({
            "version": 1,
            "opencode": {"default_model": "available", "models": {"available": {}}}
        });
        assert!(validate_models_result(&contradictory, &["opencode".into()]).is_err());
    }

    #[test]
    fn preset_is_forwarded_without_entering_secret_flow() {
        tauri::async_runtime::block_on(async {
            let response = serde_json::to_vec(&json!({"id":"desktop-1","result":{
                "models":["m1"],"catalog_token":"catalog","router_base_url":"http://127.0.0.1:19099","api_base_url":"http://127.0.0.1:19099/v1",
                "existing":{"model_config":{},"unavailable_models":{},"drifted_agents":[]},
                "preset":{"model_config":{"version":1,"codex":{"model":"m1"}},"unavailable_agents":{}}
            }})).unwrap();
            let (manager, requests) = fake_client(vec![response]);
            let flows = Arc::new(Mutex::new(HashMap::new()));
            let (_credential_dir, credentials) =
                configured_credentials("preset", "flow-secret").await;
            let result = agent_models_command(
                AgentModelsRequest {
                    agents: vec!["codex".into()],
                },
                &manager,
                flows.clone(),
                &credentials,
            )
            .await
            .unwrap();
            assert_eq!(result.preset.model_config["codex"]["model"], "m1");
            let requests = requests.lock().unwrap();
            let request = requests
                .iter()
                .find(|request| request["method"] == "agent.models")
                .unwrap();
            assert_eq!(request["params"]["api_key"], "flow-secret");
            drop(requests);
            let flows = flows.lock().await;
            let flow = flows.get(&result.flow_id).unwrap();
            assert!(!format!("{:?}", flow.models).contains("preset"));
        });
    }

    #[test]
    fn defense_in_depth_detects_exact_secret_in_model_config() {
        let key = "command-boundary-secret-fixture";
        assert!(contains_exact_string(&json!({"nested": [key]}), key));
        assert!(!contains_exact_string(&json!({"nested": ["display"]}), key));
    }

    #[test]
    fn agent_preview_forwards_complete_normalized_modes_and_binds_after_success() {
        tauri::async_runtime::block_on(async {
            let (manager, requests) = fake_client(vec![preview_response()]);
            let flow_id = Uuid::new_v4().to_string();
            let flows = Arc::new(Mutex::new(HashMap::from([(
                flow_id.clone(),
                mixed_flow(None),
            )])));
            let mut caller_modes = HashMap::from([
                ("claude".into(), "rebuild".into()),
                ("opencode".into(), "merge".into()),
            ]);

            agent_preview_command(
                AgentConfigRequest {
                    agents: vec!["claude".into(), "opencode".into()],
                    flow_id: flow_id.clone(),
                    catalog_token: "catalog".into(),
                    model_config: mixed_model_config(),
                    modes: Some(caller_modes.clone()),
                },
                &manager,
                &flows,
            )
            .await
            .unwrap();
            caller_modes.insert("claude".into(), "merge".into());

            let requests = requests.lock().unwrap();
            let request = requests
                .iter()
                .find(|request| request["method"] == "agent.preview")
                .unwrap();
            assert_eq!(request["params"]["modes"]["claude"], "rebuild");
            assert_eq!(request["params"]["modes"]["opencode"], "merge");
            assert!(request["params"].get("api_key").is_none());
            drop(requests);
            assert_eq!(
                flows.lock().await[&flow_id].modes,
                Some(HashMap::from([
                    ("claude".into(), "rebuild".into()),
                    ("opencode".into(), "merge".into())
                ]))
            );
        });
    }

    #[test]
    fn agent_preview_rejects_malformed_modes_without_mutating_bound_flow() {
        tauri::async_runtime::block_on(async {
            let (manager, requests) = fake_client(vec![]);
            let flow_id = Uuid::new_v4().to_string();
            let bound = HashMap::from([
                ("claude".into(), "merge".into()),
                ("opencode".into(), "merge".into()),
            ]);
            let flows = Arc::new(Mutex::new(HashMap::from([(
                flow_id.clone(),
                mixed_flow(Some(bound.clone())),
            )])));
            let malformed = [
                None,
                Some(HashMap::from([("claude".into(), "merge".into())])),
                Some(HashMap::from([
                    ("claude".into(), "merge".into()),
                    ("opencode".into(), "merge".into()),
                    ("codex".into(), "merge".into()),
                ])),
                Some(HashMap::from([
                    ("claude".into(), "replace".into()),
                    ("opencode".into(), "merge".into()),
                ])),
            ];

            for modes in malformed {
                let error = agent_preview_command(
                    AgentConfigRequest {
                        agents: vec!["claude".into(), "opencode".into()],
                        flow_id: flow_id.clone(),
                        catalog_token: "catalog".into(),
                        model_config: mixed_model_config(),
                        modes,
                    },
                    &manager,
                    &flows,
                )
                .await
                .unwrap_err();
                assert_eq!(error.code, "INVALID_PARAMS");
            }
            assert!(requests
                .lock()
                .unwrap()
                .iter()
                .all(|request| request["method"] != "agent.preview"));
            assert_eq!(flows.lock().await[&flow_id].modes, Some(bound));
        });
    }

    #[test]
    fn agent_preview_manager_failure_does_not_replace_bound_modes() {
        tauri::async_runtime::block_on(async {
            let failed = serde_json::to_vec(&json!({"id":"desktop-1","error":{
                "code":"PREVIEW_FAILED", "message":"preview failed"
            }}))
            .unwrap();
            let (manager, _) = fake_client(vec![failed]);
            let flow_id = Uuid::new_v4().to_string();
            let bound = HashMap::from([
                ("claude".into(), "merge".into()),
                ("opencode".into(), "merge".into()),
            ]);
            let flows = Arc::new(Mutex::new(HashMap::from([(
                flow_id.clone(),
                mixed_flow(Some(bound.clone())),
            )])));

            let error = agent_preview_command(
                AgentConfigRequest {
                    agents: vec!["claude".into(), "opencode".into()],
                    flow_id: flow_id.clone(),
                    catalog_token: "catalog".into(),
                    model_config: mixed_model_config(),
                    modes: Some(HashMap::from([
                        ("claude".into(), "rebuild".into()),
                        ("opencode".into(), "merge".into()),
                    ])),
                },
                &manager,
                &flows,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, "PREVIEW_FAILED");
            assert_eq!(flows.lock().await[&flow_id].modes, Some(bound));
        });
    }

    #[test]
    fn agent_render_rejects_modes_and_write_request_rejects_caller_modes() {
        tauri::async_runtime::block_on(async {
            let (manager, requests) = fake_client(vec![]);
            let flow_id = Uuid::new_v4().to_string();
            let flows = Arc::new(Mutex::new(HashMap::from([(
                flow_id.clone(),
                mixed_flow(None),
            )])));
            let error = agent_render_command(
                AgentConfigRequest {
                    agents: vec!["claude".into(), "opencode".into()],
                    flow_id,
                    catalog_token: "catalog".into(),
                    model_config: mixed_model_config(),
                    modes: Some(HashMap::from([
                        ("claude".into(), "merge".into()),
                        ("opencode".into(), "merge".into()),
                    ])),
                },
                &manager,
                &flows,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, "INVALID_PARAMS");
            assert!(requests
                .lock()
                .unwrap()
                .iter()
                .all(|request| request["method"] != "agent.render"));
        });

        let request = json!({
            "agents":["claude", "opencode"],
            "flow_id":Uuid::new_v4().to_string(),
            "catalog_token":"catalog",
            "model_config":serde_json::to_value(mixed_model_config()).unwrap(),
            "revision_token":"revision",
            "approve_managed_overwrite":false,
            "approve_codex_auth_change":false,
            "approve_rebuild":["claude"],
            "modes":{"claude":"merge", "opencode":"merge"}
        });
        assert!(serde_json::from_value::<AgentWriteRequest>(request).is_err());
    }

    #[test]
    fn agent_write_forwards_only_bound_modes_and_exact_rebuild_approval() {
        tauri::async_runtime::block_on(async {
            let response = serde_json::to_vec(&json!({"id":"desktop-1","result":{
                "transaction_id":"transaction",
                "agents":[],
                "state_change":null,
                "state_backup":null
            }}))
            .unwrap();
            let (manager, requests) = fake_client(vec![response]);
            let (_credential_dir, credentials) =
                configured_credentials("write-forward", "flow-secret").await;
            let flow_id = Uuid::new_v4().to_string();
            let bound = HashMap::from([
                ("claude".into(), "rebuild".into()),
                ("opencode".into(), "merge".into()),
            ]);
            let flows = Arc::new(Mutex::new(HashMap::from([(
                flow_id.clone(),
                mixed_flow(Some(bound.clone())),
            )])));

            agent_write_command(
                write_request(flow_id.clone(), vec!["claude".into()]),
                &manager,
                &flows,
                &credentials,
            )
            .await
            .unwrap();

            let requests = requests.lock().unwrap();
            let request = requests
                .iter()
                .find(|request| request["method"] == "agent.write")
                .unwrap();
            assert_eq!(request["params"]["modes"], json!(bound));
            assert_eq!(request["params"]["approve_rebuild"], json!(["claude"]));
            assert_eq!(request["params"]["api_key"], "flow-secret");
            assert!(!flows.lock().await.contains_key(&flow_id));
        });
    }

    #[test]
    fn agent_write_rejects_malformed_or_non_exact_rebuild_approvals() {
        tauri::async_runtime::block_on(async {
            let (manager, requests) = fake_client(vec![]);
            let (_credential_dir, credentials) = test_credentials("write-invalid-approval");
            let flow_id = Uuid::new_v4().to_string();
            let bound = HashMap::from([
                ("claude".into(), "rebuild".into()),
                ("opencode".into(), "merge".into()),
            ]);
            let flows = Arc::new(Mutex::new(HashMap::from([(
                flow_id.clone(),
                mixed_flow(Some(bound.clone())),
            )])));

            for approval in [
                vec![],
                vec!["claude".into(), "claude".into()],
                vec!["opencode".into()],
                vec!["claude".into(), "opencode".into()],
            ] {
                let error = agent_write_command(
                    write_request(flow_id.clone(), approval),
                    &manager,
                    &flows,
                    &credentials,
                )
                .await
                .unwrap_err();
                assert_eq!(error.code, "INVALID_PARAMS");
            }
            assert!(requests
                .lock()
                .unwrap()
                .iter()
                .all(|request| request["method"] != "agent.write"));
            assert_eq!(flows.lock().await[&flow_id].modes, Some(bound));
        });
    }

    #[test]
    fn agent_write_preview_stale_retains_complete_bound_flow() {
        tauri::async_runtime::block_on(async {
            let stale = serde_json::to_vec(&json!({"id":"desktop-1","error":{
                "code":"PREVIEW_STALE", "message":"preview is stale"
            }}))
            .unwrap();
            let (manager, _) = fake_client(vec![stale]);
            let (_credential_dir, credentials) =
                configured_credentials("write-stale", "flow-secret").await;
            let flow_id = Uuid::new_v4().to_string();
            let bound = HashMap::from([
                ("claude".into(), "rebuild".into()),
                ("opencode".into(), "merge".into()),
            ]);
            let flows = Arc::new(Mutex::new(HashMap::from([(
                flow_id.clone(),
                mixed_flow(Some(bound.clone())),
            )])));

            let error = agent_write_command(
                write_request(flow_id.clone(), vec!["claude".into()]),
                &manager,
                &flows,
                &credentials,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, "PREVIEW_STALE");
            let flows = flows.lock().await;
            let flow = flows.get(&flow_id).unwrap();
            assert_eq!(flow.modes, Some(bound));
        });
    }

    #[test]
    fn discovery_destroys_secret_flow_on_malformed_and_schema_invalid_responses() {
        tauri::async_runtime::block_on(async {
            for response in [
                b"not-json".to_vec(),
                serde_json::to_vec(&json!({"id":"desktop-1","result":{"lines":[]}})).unwrap(),
            ] {
                let recovery_status =
                    serde_json::to_vec(&json!({"id":"desktop-2","result":{"state":"absent"}}))
                        .unwrap();
                let (manager, _) = fake_client(vec![response, recovery_status]);
                let flows = Arc::new(Mutex::new(HashMap::new()));
                let (_credential_dir, credentials) =
                    configured_credentials("discovery-malformed", "terminal-path-secret").await;
                let error = agent_models_command(
                    AgentModelsRequest {
                        agents: vec!["claude".into()],
                    },
                    &manager,
                    flows.clone(),
                    &credentials,
                )
                .await
                .unwrap_err();
                assert_eq!(error.code, "INVALID_RESPONSE");
                tokio::task::yield_now().await;
                assert!(flows.lock().await.is_empty());
            }
        });
    }

    #[test]
    fn cancelled_discovery_destroys_secret_flow() {
        tauri::async_runtime::block_on(async {
            let (manager, requests) = fake_client(vec![b"delay".to_vec()]);
            let flows = Arc::new(Mutex::new(HashMap::new()));
            let (_credential_dir, credentials) =
                configured_credentials("discovery-cancel", "cancelled-secret").await;
            let task_flows = flows.clone();
            let task_credentials = credentials.clone();
            let task = tauri::async_runtime::spawn(async move {
                agent_models_command(
                    AgentModelsRequest {
                        agents: vec!["claude".into()],
                    },
                    &manager,
                    task_flows,
                    &task_credentials,
                )
                .await
            });
            for _ in 0..100 {
                if requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|request| request["method"] == "agent.models")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            task.abort();
            let _ = task.await;
            for _ in 0..100 {
                if flows.lock().await.is_empty() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("cancelled discovery retained flow state");
        });
    }

    #[test]
    fn malformed_write_is_terminal_and_destroys_secret_flow() {
        tauri::async_runtime::block_on(async {
            let recovery_status =
                serde_json::to_vec(&json!({"id":"desktop-2","result":{"state":"absent"}})).unwrap();
            let (manager, _) = fake_client(vec![b"not-json".to_vec(), recovery_status]);
            let (_credential_dir, credentials) =
                configured_credentials("write-malformed", "write-secret").await;
            let flow_id = Uuid::new_v4().to_string();
            let flows = Arc::new(Mutex::new(HashMap::from([(
                flow_id.clone(),
                ModelFlow {
                    agents: vec!["claude".into()],
                    models: vec!["m1".into()],
                    catalog_token: "catalog".into(),
                    modes: Some(HashMap::from([("claude".into(), "merge".into())])),
                },
            )])));
            let error = agent_write_command(
                AgentWriteRequest {
                    agents: vec!["claude".into()],
                    flow_id,
                    catalog_token: "catalog".into(),
                    model_config: minimal_model_config(),
                    revision_token: "revision".into(),
                    approve_managed_overwrite: false,
                    approve_codex_auth_change: false,
                    approve_rebuild: vec![],
                },
                &manager,
                &flows,
                &credentials,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, "INVALID_RESPONSE");
            assert!(flows.lock().await.is_empty());
        });
    }

    fn minimal_model_config() -> ModelConfig {
        serde_json::from_value(json!({"version":1,"claude":{"primary":{"model":"m1"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}})).unwrap()
    }

    fn forceable_inspection_response(id: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"id":id,"result":{
            "pid":4242,
            "verification_mode":"windows_pid_only",
            "listen_addr":"127.0.0.1:19099",
            "recovery":{"action":"force_terminate"},
            "confirmation_token":"opaque-token",
            "expires_at":"2099-07-25T00:00:30Z"
        }}))
        .unwrap()
    }

    fn termination_response(id: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"id":id,"result":{
            "termination":"process_terminated", "port_state":"released"
        }}))
        .unwrap()
    }

    #[test]
    fn force_termination_requires_and_consumes_the_inspected_token() {
        tauri::async_runtime::block_on(async {
            let (manager, writes) = fake_client(vec![
                forceable_inspection_response("desktop-1"),
                termination_response("desktop-2"),
            ]);
            let scheduler = PollScheduler::new(manager.clone());
            let pending = Arc::new(Mutex::new(None));
            let inspection = inspect_occupant_command(&manager, &pending).await.unwrap();
            let generation_before_force = scheduler.status_generation();

            let mismatch = force_terminate_occupant_command(
                ForceTerminateOccupantRequest {
                    confirmation_token: "wrong-token".to_owned(),
                },
                &manager,
                &scheduler,
                &pending,
            )
            .await
            .unwrap_err();
            assert_eq!(mismatch.code, "INVALID_PARAMS");

            let result = force_terminate_occupant_command(
                ForceTerminateOccupantRequest {
                    confirmation_token: inspection.confirmation_token.unwrap(),
                },
                &manager,
                &scheduler,
                &pending,
            )
            .await
            .unwrap();
            assert!(scheduler.status_generation() > generation_before_force);
            assert_eq!(
                result.termination,
                crate::types::OccupantTermination::ProcessTerminated
            );
            assert!(pending.lock().await.is_none());
            assert_eq!(scheduler.snapshot().await.release_observation, None);

            let replay = force_terminate_occupant_command(
                ForceTerminateOccupantRequest {
                    confirmation_token: "opaque-token".to_owned(),
                },
                &manager,
                &scheduler,
                &pending,
            )
            .await
            .unwrap_err();
            assert_eq!(replay.code, "INVALID_PARAMS");

            let writes = writes.lock().unwrap();
            let requests = writes
                .iter()
                .filter(|request| request["method"] == "router.force_terminate_occupant")
                .collect::<Vec<_>>();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0]["params"],
                json!({ "confirmation_token": "opaque-token" })
            );
            assert!(requests[0]["params"].get("pid").is_none());
            assert!(writes
                .iter()
                .all(|request| request["method"] != "router.start"));
        });
    }

    #[test]
    fn blocked_or_failed_inspection_clears_pending_force_target() {
        tauri::async_runtime::block_on(async {
            let blocked = serde_json::to_vec(&json!({"id":"desktop-2","result":{
                "pid":9,
                "verification_mode":"windows_pid_only",
                "listen_addr":"127.0.0.1:19099",
                "recovery":{"action":"unavailable","reason":"identity_unavailable"}
            }}))
            .unwrap();
            let (manager, _) = fake_client(vec![
                forceable_inspection_response("desktop-1"),
                blocked,
                b"not-json".to_vec(),
            ]);
            let pending = Arc::new(Mutex::new(None));

            inspect_occupant_command(&manager, &pending).await.unwrap();
            assert!(pending.lock().await.is_some());
            inspect_occupant_command(&manager, &pending).await.unwrap();
            assert!(pending.lock().await.is_none());

            *pending.lock().await = Some(PendingOccupant {
                confirmation_token: Zeroizing::new("stale".into()),
                manager_session_epoch: manager.session_epoch(),
                expires_at: "2099-07-25T00:00:30Z".parse::<DateTime<Utc>>().unwrap(),
            });
            assert!(inspect_occupant_command(&manager, &pending).await.is_err());
            assert!(pending.lock().await.is_none());
        });
    }

    #[test]
    fn invalid_termination_result_consumes_target_without_beginning_observation() {
        tauri::async_runtime::block_on(async {
            let invalid = serde_json::to_vec(&json!({"id":"desktop-2","result":{
                "termination":"process_terminated", "port_state":"occupied"
            }}))
            .unwrap();
            let (manager, _) =
                fake_client(vec![forceable_inspection_response("desktop-1"), invalid]);
            let scheduler = PollScheduler::new(manager.clone());
            let pending = Arc::new(Mutex::new(None));
            inspect_occupant_command(&manager, &pending).await.unwrap();
            let generation_before_force = scheduler.status_generation();

            let error = force_terminate_occupant_command(
                ForceTerminateOccupantRequest {
                    confirmation_token: "opaque-token".into(),
                },
                &manager,
                &scheduler,
                &pending,
            )
            .await
            .unwrap_err();

            assert_eq!(error.code, "INVALID_RESPONSE");
            assert!(scheduler.status_generation() > generation_before_force);
            assert!(pending.lock().await.is_none());
            assert_eq!(scheduler.snapshot().await.release_observation, None);
        });
    }

    #[test]
    fn expired_or_stale_session_pending_target_is_consumed_locally() {
        tauri::async_runtime::block_on(async {
            for stale_epoch in [false, true] {
                let (manager, writes) =
                    fake_client(vec![forceable_inspection_response("desktop-1")]);
                let scheduler = PollScheduler::new(manager.clone());
                let pending = Arc::new(Mutex::new(None));
                inspect_occupant_command(&manager, &pending).await.unwrap();
                if stale_epoch {
                    manager.invalidate_session_for_test();
                }
                let now = if stale_epoch {
                    "2099-07-24T00:00:00Z"
                } else {
                    "2099-07-25T00:00:30Z"
                }
                .parse::<DateTime<Utc>>()
                .unwrap();

                let error = force_terminate_occupant_command_at(
                    ForceTerminateOccupantRequest {
                        confirmation_token: "opaque-token".into(),
                    },
                    &manager,
                    &scheduler,
                    &pending,
                    now,
                )
                .await
                .unwrap_err();

                assert_eq!(error.code, "INVALID_PARAMS");
                assert!(pending.lock().await.is_none());
                assert!(writes
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|request| request["method"] != "router.force_terminate_occupant"));
            }
        });
    }
}
