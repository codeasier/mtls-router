use crate::{
    error::{CommandError, Result},
    manager::ManagerClient,
    model_config::{self, ModelConfig},
    scheduler::PollScheduler,
    types::{
        AgentDetect, AgentFragment, AgentPreview, AgentWriteResult, ComponentVersions,
        DesktopPaths, Diagnostics, ManagerInfo, NativeLanguage, OccupantInspection, PollSnapshot,
        RouterHealth, RouterLogs, RouterStatus, RouterVersion,
    },
};
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
}

pub(crate) struct ModelFlow {
    api_key: Zeroizing<String>,
    agents: Vec<String>,
    models: Vec<String>,
    catalog_token: String,
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
    pub api_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfigRequest {
    pub agents: Vec<String>,
    pub flow_id: String,
    pub catalog_token: String,
    pub model_config: ModelConfig,
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentModelsExisting {
    pub model_config: Value,
    pub unavailable_models: HashMap<String, Vec<String>>,
    pub drifted_agents: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ManagerModelsResult {
    models: Vec<String>,
    catalog_token: String,
    router_base_url: String,
    api_base_url: String,
    existing: AgentModelsExisting,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentModelsResult {
    pub flow_id: String,
    pub models: Vec<String>,
    pub catalog_token: String,
    pub router_base_url: String,
    pub api_base_url: String,
    pub existing: AgentModelsExisting,
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
    state: tauri::State<'_, AppState>,
) -> Result<RouterStatus> {
    if owner != "desktop" {
        return Err(CommandError::invalid_params("owner must be desktop"));
    }
    crate::orchestration::start(&state.manager, &state.scheduler).await
}

#[tauri::command]
pub async fn router_stop(state: tauri::State<'_, AppState>) -> Result<RouterStatus> {
    crate::orchestration::stop(&state.manager, &state.scheduler).await
}

#[tauri::command]
pub async fn router_inspect_occupant(
    state: tauri::State<'_, AppState>,
) -> Result<OccupantInspection> {
    state
        .manager
        .call("router.inspect_occupant", json!({}))
        .await
}

#[tauri::command]
pub async fn router_force_terminate_occupant(
    request: ForceTerminateOccupantRequest,
    state: tauri::State<'_, AppState>,
) -> Result<RouterStatus> {
    force_terminate_occupant_command(request, &state.manager, &state.scheduler).await
}

async fn force_terminate_occupant_command(
    mut request: ForceTerminateOccupantRequest,
    manager: &ManagerClient,
    scheduler: &PollScheduler,
) -> Result<RouterStatus> {
    if request.confirmation_token.trim().is_empty() || request.confirmation_token.len() > 4096 {
        request.confirmation_token.zeroize();
        return Err(CommandError::invalid_params(
            "confirmation token is invalid",
        ));
    }
    let token = std::mem::take(&mut request.confirmation_token);
    crate::orchestration::force_terminate_occupant(token, manager, scheduler).await
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
    agent_models_command(request, &state.manager, state.model_flows.clone()).await
}

async fn agent_models_command(
    mut request: AgentModelsRequest,
    manager: &ManagerClient,
    model_flows: Arc<Mutex<HashMap<String, ModelFlow>>>,
) -> Result<AgentModelsResult> {
    validate_agents(&request.agents)?;
    validate_api_key(&request.api_key)?;
    let flow_id = Uuid::new_v4().to_string();
    let key = std::mem::take(&mut request.api_key);
    request.api_key.zeroize();
    model_flows.lock().await.insert(
        flow_id.clone(),
        ModelFlow {
            api_key: Zeroizing::new(key),
            agents: request.agents.clone(),
            models: Vec::new(),
            catalog_token: String::new(),
        },
    );
    let mut pending = PendingFlow {
        flows: model_flows.clone(),
        id: flow_id.clone(),
        keep: false,
    };
    let params = {
        let flows = model_flows.lock().await;
        let flow = flows.get(&flow_id).ok_or_else(flow_expired)?;
        json!({ "owner": "desktop", "agents": request.agents, "api_key": flow.api_key.as_str() })
    };
    let result: ManagerModelsResult = manager.call("agent.models", params).await?;
    validate_models_result(&result)?;
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
    })
}

#[tauri::command]
pub async fn agent_render(
    request: AgentConfigRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AgentRenderResult> {
    validate_config_request(&request, &state).await?;
    state.manager.call("agent.render", json!({ "agents": request.agents, "catalog_token": request.catalog_token, "model_config": request.model_config })).await
}

#[tauri::command]
pub async fn agent_preview(
    request: AgentConfigRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AgentPreview> {
    validate_config_request(&request, &state).await?;
    state.manager.call("agent.preview", json!({ "agents": request.agents, "catalog_token": request.catalog_token, "model_config": request.model_config })).await
}

async fn agent_detect_command(manager: &ManagerClient) -> Result<AgentDetect> {
    manager.call("agent.detect", json!({})).await
}

#[tauri::command]
pub async fn agent_write(
    request: AgentWriteRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AgentWriteResult> {
    agent_write_command(request, &state.manager, &state.model_flows).await
}

async fn agent_write_command(
    request: AgentWriteRequest,
    manager: &ManagerClient,
    model_flows: &Arc<Mutex<HashMap<String, ModelFlow>>>,
) -> Result<AgentWriteResult> {
    validate_agents(&request.agents)?;
    if request.revision_token.trim().is_empty() || request.revision_token.len() > 512 * 1024 {
        return Err(CommandError::invalid_params("revision token is invalid"));
    }
    let flow = {
        let mut flows = model_flows.lock().await;
        let flow = flows.get(&request.flow_id).ok_or_else(flow_expired)?;
        if flow.agents != request.agents || flow.catalog_token != request.catalog_token {
            return Err(flow_expired());
        }
        model_config::validate(&request.model_config, &request.agents, &flow.models)?;
        if contains_exact_string(
            &serde_json::to_value(&request.model_config)
                .map_err(|_| CommandError::invalid_params("model config is invalid"))?,
            flow.api_key.as_str(),
        ) {
            return Err(CommandError::invalid_params(
                "model config contains a credential value",
            ));
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
        "api_key": flow.api_key.as_str(),
    });
    match manager.call("agent.write", params).await {
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

#[tauri::command]
pub async fn window_visibility(visible: bool, state: tauri::State<'_, AppState>) -> Result<()> {
    state.scheduler.set_visible(visible);
    if !visible {
        state.model_flows.lock().await.clear();
    }
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

fn validate_api_key(api_key: &str) -> Result<()> {
    if api_key.is_empty() || api_key.len() > 16 * 1024 {
        return Err(CommandError::invalid_params("API key is invalid"));
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

fn validate_models_result(result: &ManagerModelsResult) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    if result.models.is_empty()
        || result.models.len() > 1000
        || result.catalog_token.is_empty()
        || result.catalog_token.len() > 512 * 1024
        || result.models.iter().any(|model| {
            model.is_empty()
                || model.len() > 256
                || model.chars().any(char::is_control)
                || !seen.insert(model)
        })
    {
        return Err(CommandError::new(
            "INVALID_RESPONSE",
            "manager returned an invalid model catalog",
        ));
    }
    Ok(())
}

async fn validate_config_request(request: &AgentConfigRequest, state: &AppState) -> Result<()> {
    validate_agents(&request.agents)?;
    validate_flow_id(&request.flow_id)?;
    if request.catalog_token.is_empty() || request.catalog_token.len() > 512 * 1024 {
        return Err(CommandError::invalid_params("catalog token is invalid"));
    }
    let flows = state.model_flows.lock().await;
    let flow = flows.get(&request.flow_id).ok_or_else(flow_expired)?;
    if flow.agents != request.agents || flow.catalog_token != request.catalog_token {
        return Err(flow_expired());
    }
    model_config::validate(&request.model_config, &request.agents, &flow.models)
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
    fn flow_ids_keys_and_catalog_results_are_strictly_bounded() {
        assert!(validate_api_key("").is_err());
        assert!(validate_api_key("fixture-key").is_ok());
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
        };
        assert!(validate_models_result(&result).is_ok());
    }

    #[test]
    fn defense_in_depth_detects_exact_secret_in_model_config() {
        let key = "command-boundary-secret-fixture";
        assert!(contains_exact_string(&json!({"nested": [key]}), key));
        assert!(!contains_exact_string(&json!({"nested": ["display"]}), key));
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
                let error = agent_models_command(
                    AgentModelsRequest {
                        agents: vec!["claude".into()],
                        api_key: "terminal-path-secret".into(),
                    },
                    &manager,
                    flows.clone(),
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
            let task_flows = flows.clone();
            let task = tauri::async_runtime::spawn(async move {
                agent_models_command(
                    AgentModelsRequest {
                        agents: vec!["claude".into()],
                        api_key: "cancelled-secret".into(),
                    },
                    &manager,
                    task_flows,
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
            let flow_id = Uuid::new_v4().to_string();
            let flows = Arc::new(Mutex::new(HashMap::from([(
                flow_id.clone(),
                ModelFlow {
                    api_key: Zeroizing::new("write-secret".into()),
                    agents: vec!["claude".into()],
                    models: vec!["m1".into()],
                    catalog_token: "catalog".into(),
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
                },
                &manager,
                &flows,
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

    #[test]
    fn force_termination_command_submits_only_the_token_and_reconciles() {
        tauri::async_runtime::block_on(async {
            let force =
                serde_json::to_vec(&json!({"id":"desktop-1","result":{"state":"absent"}})).unwrap();
            let status =
                serde_json::to_vec(&json!({"id":"desktop-2","result":{"state":"absent"}})).unwrap();
            let (manager, writes) = fake_client(vec![force, status]);
            let scheduler = PollScheduler::new(manager.clone());
            let result = force_terminate_occupant_command(
                ForceTerminateOccupantRequest {
                    confirmation_token: "opaque-token".to_owned(),
                },
                &manager,
                &scheduler,
            )
            .await
            .unwrap();
            assert_eq!(result.state, "absent");

            let writes = writes.lock().unwrap();
            let request = writes
                .iter()
                .find(|request| request["method"] == "router.force_terminate_occupant")
                .unwrap();
            assert_eq!(
                request["params"],
                json!({ "confirmation_token": "opaque-token" })
            );
            assert!(request["params"].get("pid").is_none());
            assert!(request["params"].get("executable").is_none());
            assert!(writes
                .iter()
                .all(|request| request["method"] != "router.start"));
        });
    }
}
