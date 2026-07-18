use crate::{
    error::{CommandError, Result},
    manager::ManagerClient,
    scheduler::PollScheduler,
    types::{
        AgentDetect, AgentPreview, AgentWriteResult, ComponentVersions, DesktopPaths, Diagnostics,
        ManagerInfo, NativeLanguage, OccupantInspection, PollSnapshot, RouterHealth, RouterLogs,
        RouterStatus, RouterVersion,
    },
};
use serde::Deserialize;
use serde_json::json;
use std::{env, path::PathBuf};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use zeroize::Zeroize;

pub struct AppState {
    pub manager: ManagerClient,
    pub scheduler: PollScheduler,
    pub paths: DesktopPaths,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSelection {
    pub agents: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWriteRequest {
    pub agents: Vec<String>,
    pub revision_token: String,
    pub api_key: String,
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
pub async fn agent_preview(
    request: AgentSelection,
    state: tauri::State<'_, AppState>,
) -> Result<AgentPreview> {
    agent_preview_command(request, &state.manager).await
}

async fn agent_detect_command(manager: &ManagerClient) -> Result<AgentDetect> {
    manager.call("agent.detect", json!({})).await
}

async fn agent_preview_command(
    request: AgentSelection,
    manager: &ManagerClient,
) -> Result<AgentPreview> {
    validate_agents(&request.agents)?;
    manager
        .call("agent.preview", json!({ "agents": request.agents }))
        .await
}

#[tauri::command]
pub async fn agent_write(
    request: AgentWriteRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AgentWriteResult> {
    agent_write_command(request, &state.manager).await
}

async fn agent_write_command(
    mut request: AgentWriteRequest,
    manager: &ManagerClient,
) -> Result<AgentWriteResult> {
    validate_agents(&request.agents)?;
    if request.revision_token.trim().is_empty() || request.revision_token.len() > 1024 {
        request.api_key.zeroize();
        return Err(CommandError::invalid_params("revision token is invalid"));
    }
    if request.api_key.is_empty() || request.api_key.len() > 16 * 1024 {
        request.api_key.zeroize();
        return Err(CommandError::invalid_params("API key is invalid"));
    }
    let params = json!({
        "agents": request.agents,
        "revision_token": request.revision_token,
        "api_key": request.api_key,
    });
    request.api_key.zeroize();
    let result = manager.call("agent.write", params).await;
    result
}

#[tauri::command]
pub fn desktop_paths(state: tauri::State<'_, AppState>) -> DesktopPaths {
    state.paths.clone()
}

#[tauri::command]
pub fn window_visibility(visible: bool, state: tauri::State<'_, AppState>) {
    state.scheduler.set_visible(visible);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{TransportChild, TransportEvent, TransportFactory, TransportSession};
    use serde_json::Value;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex, OnceLock},
    };
    use tokio::sync::mpsc;

    struct FixtureChild {
        writes: Arc<Mutex<Vec<Value>>>,
        responder: mpsc::Sender<TransportEvent>,
        stale_write: bool,
    }

    impl TransportChild for FixtureChild {
        fn write(&mut self, bytes: &[u8]) -> Result<()> {
            let request: Value = serde_json::from_slice(bytes).expect("valid manager request");
            self.writes.lock().unwrap().push(request.clone());
            let response = fixture_response(&request, self.stale_write);
            self.responder
                .try_send(TransportEvent::Stdout(
                    serde_json::to_vec(&response).unwrap(),
                ))
                .unwrap();
            Ok(())
        }

        fn kill(self: Box<Self>) {}
    }

    struct FixtureFactory {
        writes: Arc<Mutex<Vec<Value>>>,
        stale_writes: Mutex<VecDeque<bool>>,
    }

    impl TransportFactory for FixtureFactory {
        fn spawn(&self) -> Result<TransportSession> {
            let (responder, events) = mpsc::channel(16);
            Ok(TransportSession {
                child: Box::new(FixtureChild {
                    writes: self.writes.clone(),
                    responder,
                    stale_write: self
                        .stale_writes
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or(false),
                }),
                events,
            })
        }
    }

    fn fixture_response(request: &Value, stale_write: bool) -> Value {
        let id = request["id"].clone();
        let result = match request["method"].as_str().unwrap() {
            "manager.info" => json!({
                "version": env!("MTLS_MANAGER_VERSION"),
                "commit": "fixture",
                "build_date": "fixture",
                "target": env!("MTLS_MANAGER_TARGET"),
                "deployment_id": env!("MTLS_DEPLOYMENT_ID"),
                "management_protocol_version": env!("MTLS_MANAGEMENT_PROTOCOL_VERSION")
            }),
            "agent.detect" => json!({ "agents": [{
                "agent": "opencode", "name": "opencode", "detected": true,
                "path": "/fixture/opencode.jsonc", "format": "jsonc",
                "exists": true, "writable": true, "configured": false, "invalid": false
            }] }),
            "agent.preview" => json!({
                "revision_token": "fixture-revision",
                "agents": [{
                    "agent": "codex", "name": "Codex", "files": [
                        {
                            "path": "/fixture/config.toml", "format": "toml",
                            "operation": "replace", "operations": ["replace", "preserve"],
                            "contains_api_key": false,
                            "backup": { "required": true, "sensitive": true }
                        },
                        {
                            "path": "/fixture/auth.json", "format": "json",
                            "operation": "create", "operations": ["create"],
                            "contains_api_key": true,
                            "backup": { "required": false, "sensitive": false }
                        }
                    ]
                }]
            }),
            "agent.write" if stale_write => {
                return json!({
                    "id": id,
                    "error": { "code": "PREVIEW_STALE", "message": "preview is stale" }
                });
            }
            "agent.write" => json!({
                "transaction_id": "fixture-transaction",
                "agents": [{
                    "agent": "codex", "success": true,
                    "files": [{
                        "path": "/fixture/auth.json", "replaced": true,
                        "backup_path": "/fixture/auth.json.bak-safe"
                    }],
                    "changed": ["/fixture/auth.json"],
                    "backups": ["/fixture/auth.json.bak-safe"]
                }],
                "sensitive_files": true,
                "warning": "backups are sensitive"
            }),
            "router.force_terminate_occupant" => json!({ "state": "absent" }),
            "router.status" => json!({ "state": "absent" }),
            method => panic!("unexpected fixture method {method}"),
        };
        json!({ "id": id, "result": result })
    }

    fn runtime() -> &'static tokio::runtime::Runtime {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().unwrap())
    }

    fn fixture_client(stale_write: bool) -> (ManagerClient, Arc<Mutex<Vec<Value>>>) {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let factory = Arc::new(FixtureFactory {
            writes: writes.clone(),
            stale_writes: Mutex::new([stale_write].into()),
        });
        (ManagerClient::new(factory), writes)
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
    fn agent_commands_preserve_structured_boundary_results() {
        runtime().block_on(async {
            let (manager, writes) = fixture_client(false);
            let detected = agent_detect_command(&manager).await.unwrap();
            assert_eq!(detected.agents[0].format, "jsonc");

            let preview = agent_preview_command(
                AgentSelection {
                    agents: vec!["codex".to_owned()],
                },
                &manager,
            )
            .await
            .unwrap();
            assert_eq!(preview.agents[0].files.len(), 2);
            assert!(preview.agents[0].files[1].contains_api_key);

            let key = "command-boundary-secret-fixture";
            let result = agent_write_command(
                AgentWriteRequest {
                    agents: vec!["codex".to_owned()],
                    revision_token: preview.revision_token,
                    api_key: key.to_owned(),
                },
                &manager,
            )
            .await
            .unwrap();
            assert!(result.agents[0].success);
            assert_eq!(result.agents[0].changed, ["/fixture/auth.json"]);
            assert!(!serde_json::to_string(&result).unwrap().contains(key));

            let writes = writes.lock().unwrap();
            let agent_writes: Vec<_> = writes
                .iter()
                .filter(|request| request["method"] == "agent.write")
                .collect();
            assert_eq!(agent_writes.len(), 1);
            assert_eq!(agent_writes[0]["params"]["api_key"], key);
        });
    }

    #[test]
    fn stale_preview_code_crosses_the_command_boundary_without_sensitive_data() {
        runtime().block_on(async {
            let (manager, _) = fixture_client(true);
            let key = "stale-command-secret-fixture";
            let error = agent_write_command(
                AgentWriteRequest {
                    agents: vec!["claude".to_owned()],
                    revision_token: "stale-revision".to_owned(),
                    api_key: key.to_owned(),
                },
                &manager,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, "PREVIEW_STALE");
            assert!(!error.message.contains(key));
        });
    }

    #[test]
    fn force_termination_command_submits_only_the_token_and_reconciles() {
        runtime().block_on(async {
            let (manager, writes) = fixture_client(false);
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
