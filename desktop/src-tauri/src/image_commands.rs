//! Tauri IPC commands for image conversations, the read-only `image-asset`
//! custom URI handler, and the exactly-one global operation guard.

use crate::commands::AppState;
use crate::error::{CommandError, Result};
use crate::image_client::{self, GenerationError, GenerationRequest};
use crate::image_models;
use crate::image_store::{self, AssetSource, ImageStore, MessageRole, MessageStatus, Snapshot};
use crate::trusted_channel::{TrustedChannel, TrustedChannelConfig};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::{Mutex, MutexGuard};

/// Global image operation guard: at most one generation at a time.
pub struct ImageOperationGuard {
    pub operation_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub cancel_flag: Arc<AtomicBool>,
}

/// Image readiness state (session-only, not persisted).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ImageReadiness {
    pub ready: bool,
    pub available_models: Vec<PresetModelSummary>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PresetModelSummary {
    pub id: String,
    pub display_name: String,
    pub available: bool,
}

impl Default for ImageReadiness {
    fn default() -> Self {
        Self {
            ready: false,
            available_models: image_models::PRESET_MODELS
                .iter()
                .map(|p| PresetModelSummary {
                    id: p.id.to_owned(),
                    display_name: p.display_name.to_owned(),
                    available: false,
                })
                .collect(),
            reason: "not_checked".into(),
        }
    }
}

// --- IPC DTOs (strict, deny_unknown_fields) ---

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateConversationRequest {
    pub model: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectConversationRequest {
    pub conversation_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetConversationModelRequest {
    pub conversation_id: String,
    pub model: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteConversationRequest {
    pub conversation_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartGenerationRequest {
    pub conversation_id: String,
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub reference_asset_id: String,
}

// --- IPC response types ---

#[derive(Clone, Debug, Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub selected: bool,
    pub title: String,
    pub selected_model: String,
    pub message_count: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MessageSummary {
    pub id: String,
    pub role: String,
    pub prompt: String,
    pub reference_asset_id: String,
    pub model_id: String,
    pub status: String,
    pub output_asset_id: String,
    pub error_category: String,
    pub created_at: String,
    pub completed_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportResult {
    pub asset_id: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct GenerationStartResult {
    pub operation_id: String,
    pub message_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CurrentOperationSummary {
    pub operation_id: String,
    pub conversation_id: String,
    pub message_id: String,
}

// --- Commands ---

#[tauri::command]
pub async fn image_readiness(state: State<'_, AppState>) -> Result<ImageReadiness> {
    refresh_image_readiness(&state.manager, &state.credentials, &state.image_readiness).await;
    Ok(state.image_readiness.lock().await.clone())
}

#[tauri::command]
pub async fn image_current_operation(
    state: State<'_, AppState>,
) -> Result<Option<CurrentOperationSummary>> {
    Ok(state
        .image_operation
        .lock()
        .await
        .as_ref()
        .map(|operation| CurrentOperationSummary {
            operation_id: operation.operation_id.clone(),
            conversation_id: operation.conversation_id.clone(),
            message_id: operation.message_id.clone(),
        }))
}

#[tauri::command]
pub async fn image_conversations(state: State<'_, AppState>) -> Result<Vec<ConversationSummary>> {
    let _store_guard = state.image_store_lock.lock().await;
    let store = &state.image_store;
    let snapshot = store.load().map_err(map_store_error)?;
    Ok(snapshot
        .conversations
        .iter()
        .map(|c| ConversationSummary {
            id: c.id.clone(),
            selected: c.id == snapshot.selected_conversation_id,
            title: c.title.clone(),
            selected_model: c.selected_model.clone(),
            message_count: c.message_ids.len(),
            created_at: c.created_at.clone(),
            updated_at: c.updated_at.clone(),
        })
        .collect())
}

#[tauri::command]
pub async fn image_create_conversation(
    state: State<'_, AppState>,
    request: CreateConversationRequest,
) -> Result<ConversationSummary> {
    if image_models::find_preset(&request.model).is_none() {
        return Err(CommandError::new(
            "IMAGE_INVALID_MODEL",
            "model is not a valid preset",
        ));
    }
    let _store_guard = state.image_store_lock.lock().await;
    let store = &state.image_store;
    let mut snapshot = store.load().map_err(map_store_error)?;
    let conv = store.create_conversation(&mut snapshot, &request.model);
    store.save(&snapshot).map_err(map_store_error)?;
    Ok(ConversationSummary {
        id: conv.id,
        selected: true,
        title: conv.title,
        selected_model: conv.selected_model,
        message_count: 0,
        created_at: conv.created_at,
        updated_at: conv.updated_at,
    })
}

#[tauri::command]
pub async fn image_select_conversation(
    state: State<'_, AppState>,
    request: SelectConversationRequest,
) -> Result<()> {
    let _store_guard = state.image_store_lock.lock().await;
    let store = &state.image_store;
    let mut snapshot = store.load().map_err(map_store_error)?;
    ImageStore::select_conversation(&mut snapshot, &request.conversation_id)
        .map_err(map_store_error)?;
    store.save(&snapshot).map_err(map_store_error)?;
    Ok(())
}

#[tauri::command]
pub async fn image_set_conversation_model(
    state: State<'_, AppState>,
    request: SetConversationModelRequest,
) -> Result<()> {
    if image_models::find_preset(&request.model).is_none() {
        return Err(CommandError::new(
            "IMAGE_INVALID_MODEL",
            "model is not a valid preset",
        ));
    }
    let (operation_guard, _store_guard) = lock_image_mutation(
        &state.image_operation,
        &state.image_store_lock,
        Some(&request.conversation_id),
    )
    .await?;
    let store = &state.image_store;
    let mut snapshot = store.load().map_err(map_store_error)?;
    ImageStore::set_conversation_model(&mut snapshot, &request.conversation_id, &request.model)
        .map_err(map_store_error)?;
    let result = store.save(&snapshot).map_err(map_store_error);
    drop(operation_guard);
    result
}

#[tauri::command]
pub async fn image_delete_conversation(
    state: State<'_, AppState>,
    request: DeleteConversationRequest,
) -> Result<()> {
    let (operation_guard, _store_guard) = lock_image_mutation(
        &state.image_operation,
        &state.image_store_lock,
        Some(&request.conversation_id),
    )
    .await?;
    let store = &state.image_store;
    let mut snapshot = store.load().map_err(map_store_error)?;
    let removed_assets = store
        .delete_conversation(&mut snapshot, &request.conversation_id)
        .map_err(map_store_error)?;
    store.save(&snapshot).map_err(map_store_error)?;
    // The snapshot commit is authoritative. Cleanup failure leaves recoverable
    // orphans for the next startup and must not report that deletion failed.
    let _ = store.remove_asset_files(&removed_assets);
    drop(operation_guard);
    Ok(())
}

#[tauri::command]
pub async fn image_reset_store(state: State<'_, AppState>) -> Result<()> {
    let (operation_guard, _store_guard) =
        lock_image_mutation(&state.image_operation, &state.image_store_lock, None).await?;
    let result = state.image_store.reset().map_err(map_store_error);
    drop(operation_guard);
    result
}

#[tauri::command]
pub async fn image_messages(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<MessageSummary>> {
    let _store_guard = state.image_store_lock.lock().await;
    let store = &state.image_store;
    let snapshot = store.load().map_err(map_store_error)?;
    Ok(snapshot
        .messages
        .iter()
        .filter(|m| m.conversation_id == conversation_id)
        .map(|m| MessageSummary {
            id: m.id.clone(),
            role: match m.role {
                MessageRole::User => "user".into(),
                MessageRole::Assistant => "assistant".into(),
            },
            prompt: m.prompt.clone(),
            reference_asset_id: m.reference_asset_id.clone(),
            model_id: m.model_id.clone(),
            status: match m.status {
                MessageStatus::Running => "running".into(),
                MessageStatus::Succeeded => "succeeded".into(),
                MessageStatus::Failed => "failed".into(),
                MessageStatus::Cancelled => "cancelled".into(),
                MessageStatus::Interrupted => "interrupted".into(),
            },
            output_asset_id: m.output_asset_id.clone(),
            error_category: m.error_category.clone(),
            created_at: m.created_at.clone(),
            completed_at: m.completed_at.clone(),
        })
        .collect())
}

#[tauri::command]
pub async fn image_select_reference(state: State<'_, AppState>) -> Result<ImportResult> {
    let file_path = rfd::AsyncFileDialog::new()
        .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
        .set_title("Select a reference image")
        .pick_file()
        .await
        .ok_or_else(|| CommandError::new("IMAGE_FILE_CANCELLED", "file selection cancelled"))?;
    let path = file_path.path().to_owned();
    let _store_guard = state.image_store_lock.lock().await;
    let store = &state.image_store;
    let mut snapshot = store.load().map_err(map_store_error)?;
    let asset = store
        .import_file(&mut snapshot, &path)
        .map_err(map_store_error)?;
    store.save(&snapshot).map_err(map_store_error)?;
    Ok(ImportResult {
        asset_id: asset.id,
        format: asset.format,
        width: asset.width,
        height: asset.height,
    })
}

#[tauri::command]
pub async fn image_start_generation(
    state: State<'_, AppState>,
    app: AppHandle,
    request: StartGenerationRequest,
) -> Result<GenerationStartResult> {
    if image_models::find_preset(&request.model).is_none() {
        return Err(CommandError::new(
            "IMAGE_INVALID_MODEL",
            "model is not a valid preset",
        ));
    }

    // Validate prompt
    let prompt_bytes = request.prompt.as_bytes();
    if request.prompt.trim().is_empty()
        || prompt_bytes.len() > crate::image_limits::MAX_PROMPT_BYTES
    {
        return Err(CommandError::new(
            "IMAGE_INVALID_PROMPT",
            "prompt is empty or exceeds size limit",
        ));
    }

    let readiness = state.image_readiness.lock().await.clone();
    if !readiness.ready
        || !readiness
            .available_models
            .iter()
            .any(|model| model.id == request.model && model.available)
    {
        return Err(CommandError::new(
            "IMAGE_NOT_READY",
            "selected image model is unavailable",
        ));
    }

    // Reserve the global slot before mutating storage so two concurrent invokes
    // cannot both persist running operations.
    let mut operation = state.image_operation.lock().await;
    if operation.is_some() {
        return Err(CommandError::new(
            "IMAGE_BUSY",
            "a generation is already in progress",
        ));
    }

    let _store_guard = state.image_store_lock.lock().await;
    let store = &state.image_store;
    let mut snapshot = store.load().map_err(map_store_error)?;
    let conversation = snapshot
        .conversations
        .iter()
        .find(|conversation| conversation.id == request.conversation_id)
        .ok_or_else(|| CommandError::new("IMAGE_STORE_ERROR", "conversation was not found"))?;
    if conversation.selected_model != request.model {
        return Err(CommandError::new(
            "IMAGE_INVALID_MODEL",
            "model does not match the conversation selection",
        ));
    }

    let reference_data_uri = if request.reference_asset_id.is_empty() {
        None
    } else {
        let mime = ImageStore::asset_mime(&snapshot, &request.reference_asset_id)
            .map_err(map_store_error)?;
        let asset_data = store
            .read_asset(&snapshot, &request.reference_asset_id)
            .map_err(map_store_error)?;
        Some(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&asset_data)
        ))
    };

    // Add user message
    let reference = if request.reference_asset_id.is_empty() {
        None
    } else {
        Some(request.reference_asset_id.as_str())
    };
    store
        .add_user_message(
            &mut snapshot,
            &request.conversation_id,
            &request.prompt,
            reference,
        )
        .map_err(map_store_error)?;

    // Add running assistant placeholder
    let asst_msg = store
        .add_running_assistant(&mut snapshot, &request.conversation_id, &request.model)
        .map_err(map_store_error)?;

    // Persist before request
    store.save(&snapshot).map_err(map_store_error)?;

    let operation_id = uuid::Uuid::new_v4().to_string();
    let cancel_flag = Arc::new(AtomicBool::new(false));

    *operation = Some(ImageOperationGuard {
        operation_id: operation_id.clone(),
        conversation_id: request.conversation_id.clone(),
        message_id: asst_msg.id.clone(),
        cancel_flag: cancel_flag.clone(),
    });
    drop(_store_guard);
    drop(operation);

    // Spawn generation task
    let operation_id_clone = operation_id.clone();
    let message_id = asst_msg.id.clone();
    let conversation_id = request.conversation_id.clone();
    let model = request.model.clone();
    let prompt = request.prompt.clone();
    let data_dir = state.paths.data_dir.clone();
    let app_handle = app.clone();
    let manager = state.manager.clone();
    let credentials = state.credentials.clone();
    let operation_guard = state.image_operation.clone();
    let store_lock = state.image_store_lock.clone();

    tokio::spawn(async move {
        let result = run_generation(
            &manager,
            &credentials,
            &model,
            &prompt,
            reference_data_uri.as_deref(),
            &cancel_flag,
        )
        .await;

        let store = ImageStore::from_data_dir(&data_dir);
        let _store_guard = store_lock.lock().await;
        let mut event_status = "failed";
        if let Ok(mut snapshot) = store.load() {
            let (status, output_asset_id, error_category) = match result {
                Ok(gen_result) => {
                    let validated = gen_result.validated;
                    let source = AssetSource::Generation;
                    match store.save_asset(
                        &mut snapshot,
                        &gen_result.image_bytes,
                        &validated,
                        source,
                    ) {
                        Ok(asset) => (MessageStatus::Succeeded, asset.id, String::new()),
                        Err(_) => (MessageStatus::Failed, String::new(), "store_error".into()),
                    }
                }
                Err(GenerationError::Cancelled) => {
                    (MessageStatus::Cancelled, String::new(), "cancelled".into())
                }
                Err(GenerationError::Timeout) => {
                    (MessageStatus::Failed, String::new(), "timeout".into())
                }
                Err(e) => (MessageStatus::Failed, String::new(), sanitize_error(&e)),
            };

            if store
                .complete_assistant_message(
                    &mut snapshot,
                    &message_id,
                    status.clone(),
                    if output_asset_id.is_empty() {
                        None
                    } else {
                        Some(&output_asset_id)
                    },
                    if error_category.is_empty() {
                        None
                    } else {
                        Some(&error_category)
                    },
                )
                .is_ok()
                && store.save(&snapshot).is_ok()
            {
                event_status = match status {
                    MessageStatus::Succeeded => "succeeded",
                    MessageStatus::Cancelled => "cancelled",
                    _ => "failed",
                };
            }
        }

        drop(_store_guard);
        clear_operation(&operation_guard, &operation_id_clone).await;

        let _ = app_handle.emit(
            "image-operation",
            serde_json::json!({
                "operation_id": operation_id_clone,
                "conversation_id": conversation_id,
                "message_id": message_id,
                "status": event_status,
            }),
        );
    });

    Ok(GenerationStartResult {
        operation_id,
        message_id: asst_msg.id,
    })
}

#[tauri::command]
pub async fn image_cancel_generation(state: State<'_, AppState>) -> Result<()> {
    let guard = state.image_operation.lock().await;
    if let Some(op) = guard.as_ref() {
        op.cancel_flag.store(true, Ordering::SeqCst);
    }
    Ok(())
}

// --- Custom URI handler ---

/// Validates that an asset ID is a canonical SHA-256 hex string.
pub fn is_valid_asset_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Resolves an asset ID to its file path, rejecting path traversal.
pub fn resolve_asset_path(
    store: &ImageStore,
    snapshot: &Snapshot,
    asset_id: &str,
) -> Result<std::path::PathBuf> {
    if !is_valid_asset_id(asset_id) {
        return Err(CommandError::new(
            "IMAGE_INVALID_ASSET_ID",
            "invalid asset ID",
        ));
    }
    if asset_id.contains('/') || asset_id.contains('\\') || asset_id.contains("..") {
        return Err(CommandError::new(
            "IMAGE_INVALID_ASSET_ID",
            "path traversal rejected",
        ));
    }
    store
        .asset_path(snapshot, asset_id)
        .map_err(map_store_error)
}

// --- Helpers ---

fn map_store_error(_error: image_store::StoreError) -> CommandError {
    CommandError::new(
        "IMAGE_STORE_ERROR",
        "image conversation storage operation failed",
    )
}

async fn lock_image_mutation<'a>(
    operation: &'a Mutex<Option<ImageOperationGuard>>,
    store: &'a Mutex<()>,
    conversation_id: Option<&str>,
) -> Result<(
    MutexGuard<'a, Option<ImageOperationGuard>>,
    MutexGuard<'a, ()>,
)> {
    let operation_guard = operation.lock().await;
    let conflicts = operation_guard.as_ref().is_some_and(|current| {
        conversation_id
            .map(|conversation_id| current.conversation_id == conversation_id)
            .unwrap_or(true)
    });
    if conflicts {
        return Err(CommandError::new(
            "IMAGE_BUSY",
            "an image operation conflicts with this change",
        ));
    }
    let store_guard = store.lock().await;
    Ok((operation_guard, store_guard))
}

fn sanitize_error(e: &GenerationError) -> String {
    match e {
        GenerationError::NotReady => "not_ready".into(),
        GenerationError::InvalidPrompt => "invalid_prompt".into(),
        GenerationError::InvalidModel => "invalid_model".into(),
        GenerationError::InvalidReferenceImage => "invalid_reference_image".into(),
        GenerationError::ChannelError(_) => "channel_error".into(),
        GenerationError::ResponseParseFailed => "response_parse_failed".into(),
        GenerationError::ResponseUrlOnly => "response_url_only".into(),
        GenerationError::ResponseEmpty => "response_empty".into(),
        GenerationError::ResponseMultipleImages => "response_multiple_images".into(),
        GenerationError::ResponseUnknownFormat => "response_unknown_format".into(),
        GenerationError::ResponseExceedsLimits => "response_exceeds_limits".into(),
        GenerationError::ResponseDimensionsUnreadable => "response_dimensions_unreadable".into(),
        GenerationError::Cancelled => "cancelled".into(),
        GenerationError::Timeout => "timeout".into(),
    }
}

pub(crate) async fn refresh_image_readiness(
    manager: &crate::manager::ManagerClient,
    credentials: &crate::credential::CredentialStore,
    readiness: &Mutex<ImageReadiness>,
) {
    let mut readiness = readiness.lock().await;
    let next = match authenticated_channel(manager, credentials).await {
        Ok((mut channel, api_key)) => match channel
            .fetch_catalog(&api_key, crate::image_limits::CATALOG_BODY_LIMIT)
            .await
            .ok()
            .and_then(|body| image_models::parse_catalog(&body).ok())
        {
            Some(catalog) => {
                let available = image_models::available_presets(&catalog);
                let ready = !available.is_empty();
                ImageReadiness {
                    ready,
                    available_models: image_models::PRESET_MODELS
                        .iter()
                        .map(|preset| PresetModelSummary {
                            id: preset.id.to_owned(),
                            display_name: preset.display_name.to_owned(),
                            available: available.iter().any(|item| item.id == preset.id),
                        })
                        .collect(),
                    reason: if ready {
                        String::new()
                    } else {
                        "models_unavailable".into()
                    },
                }
            }
            None => ImageReadiness {
                reason: "catalog_unavailable".into(),
                ..ImageReadiness::default()
            },
        },
        Err(error) => ImageReadiness {
            reason: sanitize_error(&error),
            ..ImageReadiness::default()
        },
    };
    *readiness = next;
}

async fn authenticated_channel(
    manager: &crate::manager::ManagerClient,
    credentials: &crate::credential::CredentialStore,
) -> std::result::Result<(TrustedChannel, zeroize::Zeroizing<String>), GenerationError> {
    let config: TrustedChannelConfig = manager
        .call("router.trusted_channel", serde_json::json!({}))
        .await
        .map_err(|_| GenerationError::NotReady)?;
    let mut channel = TrustedChannel::connect(&config)
        .await
        .map_err(GenerationError::from)?;
    channel
        .verify_trust(&config)
        .await
        .map_err(GenerationError::from)?;
    let api_key = credentials
        .use_()
        .await
        .map_err(|_| GenerationError::NotReady)?;
    Ok((channel, api_key))
}

async fn run_generation(
    manager: &crate::manager::ManagerClient,
    credentials: &crate::credential::CredentialStore,
    model: &str,
    prompt: &str,
    reference_data_uri: Option<&str>,
    cancel_flag: &Arc<AtomicBool>,
) -> std::result::Result<image_client::GenerationResult, GenerationError> {
    cancelable_generation(
        run_generation_inner(manager, credentials, model, prompt, reference_data_uri),
        cancel_flag,
    )
    .await
}

async fn cancelable_generation<F, T>(
    operation: F,
    cancel_flag: &AtomicBool,
) -> std::result::Result<T, GenerationError>
where
    F: std::future::Future<Output = std::result::Result<T, GenerationError>>,
{
    tokio::select! {
        result = operation => result,
        _ = wait_for_cancel(cancel_flag) => Err(GenerationError::Cancelled),
    }
}

async fn run_generation_inner(
    manager: &crate::manager::ManagerClient,
    credentials: &crate::credential::CredentialStore,
    model: &str,
    prompt: &str,
    reference_data_uri: Option<&str>,
) -> std::result::Result<image_client::GenerationResult, GenerationError> {
    let req = GenerationRequest {
        model: model.to_owned(),
        prompt: prompt.to_owned(),
        reference_image_data_uri: reference_data_uri.map(|value| value.to_owned()),
    };
    image_client::validate_request(&req)?;
    let (mut channel, api_key) = authenticated_channel(manager, credentials).await?;
    let catalog_body = channel
        .fetch_catalog(&api_key, crate::image_limits::CATALOG_BODY_LIMIT)
        .await
        .map_err(GenerationError::from)?;
    let catalog =
        image_models::parse_catalog(&catalog_body).map_err(|_| GenerationError::NotReady)?;
    if !image_models::is_preset_available(&catalog, model) {
        return Err(GenerationError::InvalidModel);
    }
    image_client::execute(&mut channel, &api_key, &req).await
}

async fn wait_for_cancel(cancel_flag: &AtomicBool) {
    while !cancel_flag.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn clear_operation(operation_guard: &Mutex<Option<ImageOperationGuard>>, operation_id: &str) {
    let mut guard = operation_guard.lock().await;
    if guard
        .as_ref()
        .is_some_and(|operation| operation.operation_id == operation_id)
    {
        guard.take();
    }
}

use base64::Engine;
use tauri::Emitter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_asset_id_accepts_sha256_hex() {
        assert!(is_valid_asset_id("a".repeat(64).as_str()));
        assert!(is_valid_asset_id("0123456789abcdef".repeat(4).as_str()));
    }

    #[test]
    fn invalid_asset_id_rejects_non_hex() {
        assert!(!is_valid_asset_id("g".repeat(64).as_str()));
        assert!(!is_valid_asset_id("A".repeat(64).as_str()));
        assert!(!is_valid_asset_id("short"));
        assert!(!is_valid_asset_id("a".repeat(63).as_str()));
        assert!(!is_valid_asset_id("a".repeat(65).as_str()));
    }

    #[test]
    fn path_traversal_rejected() {
        assert!(!is_valid_asset_id("../../../etc/passwd"));
        assert!(!is_valid_asset_id(&format!("{}../../", "a".repeat(60))));
    }

    #[test]
    fn sanitize_error_never_leaks_details() {
        let errors = vec![
            GenerationError::NotReady,
            GenerationError::InvalidPrompt,
            GenerationError::ChannelError(
                crate::trusted_channel::TrustedChannelError::VersionMismatch,
            ),
            GenerationError::ResponseParseFailed,
            GenerationError::Cancelled,
            GenerationError::Timeout,
        ];
        for e in &errors {
            let sanitized = sanitize_error(e);
            assert!(!sanitized.contains("Bearer"));
            assert!(!sanitized.contains("base64"));
            assert!(!sanitized.contains("sk-"));
            assert!(!sanitized.contains("data:image"));
        }
    }

    #[test]
    fn readiness_default_has_both_presets_unavailable() {
        let r = ImageReadiness::default();
        assert!(!r.ready);
        assert_eq!(r.available_models.len(), 2);
        assert!(r.available_models.iter().all(|m| !m.available));
    }

    #[tokio::test]
    async fn cancellation_drops_an_in_progress_operation() {
        let cancel = Arc::new(AtomicBool::new(false));
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            trigger.store(true, Ordering::SeqCst);
        });
        let pending = std::future::pending::<std::result::Result<(), GenerationError>>();
        assert_eq!(
            cancelable_generation(pending, &cancel).await,
            Err(GenerationError::Cancelled)
        );
    }

    #[tokio::test]
    async fn destructive_mutation_cannot_pass_generation_reservation() {
        let operation = Arc::new(Mutex::new(None));
        let store = Arc::new(Mutex::new(()));
        let starter_operation = operation.clone();
        let starter_store = store.clone();
        let starter = tokio::spawn(async move {
            let mut operation_guard = starter_operation.lock().await;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let _store_guard = starter_store.lock().await;
            *operation_guard = Some(ImageOperationGuard {
                operation_id: "operation-1".into(),
                conversation_id: "conversation-1".into(),
                message_id: "message-1".into(),
                cancel_flag: Arc::new(AtomicBool::new(false)),
            });
        });
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        let error = match lock_image_mutation(&operation, &store, Some("conversation-1")).await {
            Ok(_) => panic!("conflicting mutation acquired the store lock"),
            Err(error) => error,
        };
        assert_eq!(error.code, "IMAGE_BUSY");
        starter.await.unwrap();
    }
}
