use serde::Deserialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::commands::AppState;
use crate::error::{CommandError, Result};
use crate::image_workbench::channel::TrustTarget;
use crate::image_workbench::service::{EventSink, Readiness};
use crate::image_workbench::{SafeKind, WorkbenchError};
use crate::manager_core::process;
use crate::types::RouterStatus;

pub const CHAT_DELTA_EVENT: &str = "workbench-chat-delta";
pub const PHASE_EVENT: &str = "workbench-phase";
pub const IMAGE_STATUS_EVENT: &str = "workbench-image-status";
pub const OPERATION_DONE_EVENT: &str = "workbench-operation-done";

pub struct WorkbenchCommands;

struct AppSink(AppHandle);

impl EventSink for AppSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.0.emit(event, payload);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationIdRequest {
    pub conversation_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetOptionsRequest {
    pub conversation_id: String,
    pub chat_model: Option<String>,
    pub image_model: Option<String>,
    pub size: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportBytesRequest {
    pub bytes: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteAssetRequest {
    pub asset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendRequest {
    pub conversation_id: String,
    pub text: String,
    pub force_image: bool,
    pub reference_asset_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegenerateRequest {
    pub conversation_id: String,
    pub job_id: String,
    pub reference_asset_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetIdRequest {
    pub asset_id: String,
}

#[tauri::command]
pub async fn workbench_readiness(state: State<'_, AppState>) -> Result<Readiness> {
    refresh_if_possible(&state).await
}

#[tauri::command]
pub async fn workbench_refresh_catalogs(state: State<'_, AppState>) -> Result<Readiness> {
    refresh_if_possible(&state).await
}

#[tauri::command]
pub async fn workbench_list(state: State<'_, AppState>) -> Result<serde_json::Value> {
    let snapshot = match state.workbench.store().snapshot() {
        Ok(snapshot) => snapshot,
        Err(_) => {
            return Ok(json!({
                "selected_conversation_id": null,
                "conversations": [],
                "messages": [],
                "jobs": [],
                "assets": [],
            }))
        }
    };
    Ok(json!({
        "selected_conversation_id": snapshot.selected_conversation_id,
        "conversations": snapshot.conversations,
        "messages": snapshot.messages,
        "jobs": snapshot.jobs,
        "assets": snapshot.assets,
    }))
}

#[tauri::command]
pub async fn workbench_create(state: State<'_, AppState>) -> Result<serde_json::Value> {
    let readiness = state.workbench.readiness_snapshot(true, true);
    let chat = readiness
        .chat_models
        .iter()
        .find(|id| *id == crate::image_workbench::DEFAULT_CHAT_MODEL)
        .cloned();
    let image = readiness
        .image_models
        .iter()
        .find(|item| item.id == crate::image_workbench::DEFAULT_IMAGE_MODEL)
        .map(|item| item.id.clone());
    let conversation =
        state
            .workbench
            .store()
            .create_conversation(chat, image, "1024x1024".into())?;
    Ok(json!(conversation))
}

#[tauri::command]
pub async fn workbench_select(
    request: ConversationIdRequest,
    state: State<'_, AppState>,
) -> Result<serde_json::Value> {
    let snapshot = state
        .workbench
        .store()
        .select_conversation(&request.conversation_id)?;
    Ok(json!(snapshot.selected_conversation_id))
}

#[tauri::command]
pub async fn workbench_delete(
    request: ConversationIdRequest,
    state: State<'_, AppState>,
) -> Result<serde_json::Value> {
    let snapshot = state
        .workbench
        .store()
        .delete_conversation(&request.conversation_id)?;
    Ok(json!({
        "selected_conversation_id": snapshot.selected_conversation_id,
        "conversations": snapshot.conversations,
    }))
}

#[tauri::command]
pub async fn workbench_set_options(
    request: SetOptionsRequest,
    state: State<'_, AppState>,
) -> Result<()> {
    state.workbench.store().set_options(
        &request.conversation_id,
        request.chat_model.map(Some),
        request.image_model.map(Some),
        request.size,
    )?;
    Ok(())
}

#[tauri::command]
pub async fn workbench_pick_reference(state: State<'_, AppState>) -> Result<serde_json::Value> {
    let workbench = state.workbench.clone();
    let asset = tauri::async_runtime::spawn_blocking(move || workbench.pick_reference())
        .await
        .map_err(|_| CommandError::from(WorkbenchError::new(SafeKind::Unknown)))??;
    Ok(json!(asset))
}

#[tauri::command]
pub async fn workbench_import_bytes(
    request: ImportBytesRequest,
    state: State<'_, AppState>,
) -> Result<serde_json::Value> {
    if request.bytes.len() > crate::image_workbench::image::MAX_IMAGE_BYTES {
        return Err(WorkbenchError::new(SafeKind::ImageTooLarge).into());
    }
    let asset = state.workbench.import_bytes(&request.bytes)?;
    Ok(json!(asset))
}

#[tauri::command]
pub async fn workbench_quote_asset(
    request: QuoteAssetRequest,
    state: State<'_, AppState>,
) -> Result<serde_json::Value> {
    let (meta, _) = state.workbench.store().read_asset(&request.asset_id)?;
    Ok(json!(meta))
}

#[tauri::command]
pub async fn workbench_send(
    request: SendRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value> {
    let target = bind_target(&state).await?;
    let key = state.credentials.use_().await?;
    let workbench = state.workbench.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        workbench.send(
            &request.conversation_id,
            request.text,
            request.force_image,
            request.reference_asset_id,
            &target,
            &key,
            &AppSink(app),
        )
    })
    .await
    .map_err(|_| CommandError::from(WorkbenchError::new(SafeKind::Unknown)))??;
    Ok(json!({
        "selected_conversation_id": snapshot.selected_conversation_id,
        "conversations": snapshot.conversations,
        "messages": snapshot.messages,
        "jobs": snapshot.jobs,
        "assets": snapshot.assets,
    }))
}

#[tauri::command]
pub async fn workbench_regenerate(
    request: RegenerateRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value> {
    let target = bind_target(&state).await?;
    let key = state.credentials.use_().await?;
    let workbench = state.workbench.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        workbench.regenerate(
            &request.conversation_id,
            &request.job_id,
            request.reference_asset_id,
            &target,
            &key,
            &AppSink(app),
        )
    })
    .await
    .map_err(|_| CommandError::from(WorkbenchError::new(SafeKind::Unknown)))??;
    Ok(json!({
        "messages": snapshot.messages,
        "jobs": snapshot.jobs,
        "assets": snapshot.assets,
    }))
}

#[tauri::command]
pub async fn workbench_cancel(state: State<'_, AppState>) -> Result<()> {
    state.workbench.cancel();
    Ok(())
}

#[tauri::command]
pub async fn workbench_save_asset(
    request: AssetIdRequest,
    state: State<'_, AppState>,
) -> Result<()> {
    let workbench = state.workbench.clone();
    tauri::async_runtime::spawn_blocking(move || workbench.save_asset(&request.asset_id))
        .await
        .map_err(|_| CommandError::from(WorkbenchError::new(SafeKind::Unknown)))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn workbench_rebuild(state: State<'_, AppState>) -> Result<serde_json::Value> {
    let snapshot = state.workbench.store().rebuild()?;
    Ok(json!({ "version": snapshot.version }))
}

async fn refresh_if_possible(state: &AppState) -> Result<Readiness> {
    let has_key = state
        .credentials
        .read_summary()
        .await
        .map(|s| s.present)
        .unwrap_or(false);
    match bind_target(state).await {
        Ok(target) if has_key => {
            let key = state.credentials.use_().await?;
            let workbench = state.workbench.clone();
            match tauri::async_runtime::spawn_blocking(move || {
                workbench.refresh_catalogs(&target, &key)
            })
            .await
            {
                Ok(Ok(readiness)) => Ok(readiness),
                _ => Ok(state.workbench.readiness_snapshot(has_key, true)),
            }
        }
        Ok(_) => Ok(state.workbench.readiness_snapshot(has_key, true)),
        Err(_) => Ok(state.workbench.readiness_snapshot(has_key, false)),
    }
}

pub async fn refresh_after_credential_change(state: &AppState) {
    let _ = refresh_if_possible(state).await;
}

async fn bind_target(state: &AppState) -> Result<TrustTarget> {
    let status: RouterStatus = state.manager.call("router.status", json!({})).await?;
    if !status.available() {
        return Err(CommandError::from(WorkbenchError::new(SafeKind::NotReady)));
    }
    let listen = status
        .listen_addr
        .ok_or_else(|| CommandError::from(WorkbenchError::new(SafeKind::NotReady)))?;
    let authority = listen
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_owned();
    let pid = status
        .pid
        .ok_or_else(|| CommandError::from(WorkbenchError::new(SafeKind::NotReady)))?
        as i32;
    let identity = process::inspect(pid)
        .map_err(|_| CommandError::from(WorkbenchError::new(SafeKind::Identity)))?;
    Ok(TrustTarget {
        authority,
        pid,
        deployment_id: env!("MTLS_DEPLOYMENT_ID").to_owned(),
        protocol_version: env!("MTLS_MANAGEMENT_PROTOCOL_VERSION").to_owned(),
        started_at: identity.started_at,
        executable: identity.executable.clone(),
        binary_path: identity.executable,
    })
}
