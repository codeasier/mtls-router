mod autostart;
mod commands;
mod credential;
mod error;
mod image_client;
mod image_commands;
mod image_limits;
mod image_models;
mod image_store;
mod image_validation;
mod lifecycle;
mod manager;
mod model_config;
mod orchestration;
mod paths;
mod port_recovery;
mod process_identity;
mod router_process;
mod scheduler;
mod sidecar;
mod tray;
mod trusted_channel;
mod types;
mod updater;

use commands::AppState;
use credential::{CredentialError, CredentialStore};
use manager::{ManagerClient, TauriTransportFactory};
use scheduler::PollScheduler;
use sidecar::SidecarPaths;
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, WindowEvent};

const POLL_SNAPSHOT_EVENT: &str = "router-poll-snapshot";

fn load_credentials(path: std::path::PathBuf) -> Arc<CredentialStore> {
    let credentials = Arc::new(CredentialStore::new(path));
    if let Err(CredentialError::InvalidFormat(_)) =
        tauri::async_runtime::block_on(credentials.read_summary())
    {
        eprintln!("CodeasierRouter: removing malformed credential file");
        let _ = tauri::async_runtime::block_on(credentials.delete());
    }
    credentials
}

pub fn verify_manager_handshake() -> Result<(), String> {
    let sidecars = SidecarPaths::resolve().map_err(|error| error.to_string())?;
    sidecars.validate().map_err(|error| error.to_string())?;

    let data_dir =
        std::env::temp_dir().join(format!("mtls-router-handshake-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&data_dir).map_err(|_| "cannot create verification directory".to_owned())?;

    let result = (|| {
        let mut child = Command::new(&sidecars.manager)
            .arg("serve")
            .arg("--router-sidecar")
            .arg(&sidecars.router)
            .env("MTLS_ROUTER_DESKTOP_DATA_DIR", &data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "packaged manager cannot execute".to_owned())?;

        child
            .stdin
            .take()
            .ok_or_else(|| "cannot open manager input".to_owned())?
            .write_all(b"{\"id\":\"desktop-package\",\"method\":\"manager.info\",\"params\":{}}\n")
            .map_err(|_| "cannot write manager.info request".to_owned())?;

        let deadline = Instant::now() + Duration::from_secs(3);
        while child
            .try_wait()
            .map_err(|_| "cannot wait for packaged manager".to_owned())?
            .is_none()
        {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("packaged manager handshake timed out".to_owned());
            }
            thread::sleep(Duration::from_millis(10));
        }
        let output = child
            .wait_with_output()
            .map_err(|_| "cannot read manager.info response".to_owned())?;
        if !output.status.success() {
            return Err("packaged manager exited unsuccessfully".to_owned());
        }
        let response: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| "manager.info response is malformed".to_owned())?;
        if response.get("id").and_then(serde_json::Value::as_str) != Some("desktop-package")
            || response.get("error").is_some_and(|error| !error.is_null())
        {
            return Err("manager.info response is invalid".to_owned());
        }
        let info: types::ManagerInfo = serde_json::from_value(
            response
                .get("result")
                .cloned()
                .ok_or_else(|| "manager.info result is missing".to_owned())?,
        )
        .map_err(|_| "manager.info result is malformed".to_owned())?;
        manager::validate_handshake(&info).map_err(|error| error.to_string())
    })();

    let _ = fs::remove_dir_all(data_dir);
    result
}

fn build_app() -> tauri::Result<tauri::App<tauri::Wry>> {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(autostart::plugin());

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }));
    }

    builder
        .register_uri_scheme_protocol("image-asset", |_ctx, request| {
            use std::borrow::Cow;
            use tauri::http::{Response, StatusCode};
            let uri = request.uri().to_string();
            let asset_id = uri
                .strip_prefix("image-asset://localhost/")
                .or_else(|| uri.strip_prefix("http://image-asset.localhost/"))
                .unwrap_or("");
            let asset_id = asset_id.trim_end_matches('/');
            if !image_commands::is_valid_asset_id(asset_id) {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Cow::from(Vec::new()))
                    .unwrap();
            }
            let data_dir = crate::paths::resolve()
                .map(|p| p.data_dir)
                .unwrap_or_default();
            if data_dir.is_empty() {
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Cow::from(Vec::new()))
                    .unwrap();
            }
            let store = crate::image_store::ImageStore::from_data_dir(&data_dir);
            let snapshot = match store.load() {
                Ok(s) => s,
                Err(_) => {
                    return Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Cow::from(Vec::new()))
                        .unwrap();
                }
            };
            let path = match image_commands::resolve_asset_path(&store, &snapshot, asset_id) {
                Ok(p) => p,
                Err(_) => {
                    return Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(Cow::from(Vec::new()))
                        .unwrap();
                }
            };
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(_) => {
                    return Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(Cow::from(Vec::new()))
                        .unwrap();
                }
            };
            let mime = crate::image_store::ImageStore::asset_mime(&snapshot, asset_id)
                .unwrap_or("application/octet-stream");
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", mime)
                .header("X-Content-Type-Options", "nosniff")
                .header("Cache-Control", "private, no-store")
                .body(Cow::from(data))
                .unwrap()
        })
        .setup(|app| {
            let sidecars = SidecarPaths::resolve();
            let parent = process_identity::current();
            let paths = paths::resolve()?;
            let credentials = load_credentials(std::path::PathBuf::from(&paths.credentials_path));
            let image_store = Arc::new(crate::image_store::ImageStore::from_data_dir(
                &paths.data_dir,
            ));
            if let Ok(mut snapshot) = image_store.load() {
                let changed = image_store.finalize_leftover_running(&mut snapshot)
                    | image_store.prune_unreferenced_assets(&mut snapshot);
                let saved = !changed || image_store.save(&snapshot).is_ok();
                if changed && !saved {
                    eprintln!("CodeasierRouter: image snapshot save failed");
                } else if image_store.cleanup_orphans(&snapshot).is_err() {
                    eprintln!("CodeasierRouter: image orphan cleanup failed");
                }
            }
            let manager = match (sidecars, parent) {
                (Ok(sidecars), Ok(parent)) => match sidecars.validate() {
                    Ok(()) => ManagerClient::new(Arc::new(TauriTransportFactory::new(
                        app.handle().clone(),
                        sidecars,
                        parent,
                        uuid::Uuid::new_v4().to_string(),
                    ))),
                    Err(error) => ManagerClient::failed(error),
                },
                (Err(error), _) | (_, Err(error)) => ManagerClient::failed(error),
            };
            let image_readiness = Arc::new(tokio::sync::Mutex::new(
                crate::image_commands::ImageReadiness::default(),
            ));
            let previous_image_health = Arc::new(std::sync::Mutex::new(false));
            let readiness_manager = manager.clone();
            let readiness_credentials = credentials.clone();
            let readiness_state = image_readiness.clone();
            let readiness_health = previous_image_health.clone();
            let observer_app = app.handle().clone();
            let scheduler = PollScheduler::with_observer(manager.clone(), move |snapshot| {
                if snapshot.status.is_some() || snapshot.status_error.is_some() {
                    tray::update_poll_snapshot(&observer_app, &snapshot);
                }
                let healthy = snapshot
                    .health
                    .as_ref()
                    .is_some_and(|health| health.status == "ok")
                    && !snapshot.health_stale;
                let _ = observer_app.emit(POLL_SNAPSHOT_EVENT, snapshot);
                let changed = readiness_health
                    .lock()
                    .map(|mut previous| {
                        let changed = *previous != healthy;
                        *previous = healthy;
                        changed
                    })
                    .unwrap_or(false);
                if changed {
                    let manager = readiness_manager.clone();
                    let credentials = readiness_credentials.clone();
                    let readiness = readiness_state.clone();
                    tauri::async_runtime::spawn(async move {
                        if healthy {
                            crate::image_commands::refresh_image_readiness(
                                &manager,
                                &credentials,
                                &readiness,
                            )
                            .await;
                        } else {
                            *readiness.lock().await =
                                crate::image_commands::ImageReadiness::default();
                        }
                    });
                }
            });
            let lifecycle = Arc::new(lifecycle::LifecycleState::default());
            autostart::initialize_default(app)?;
            tray::setup(
                app,
                manager.clone(),
                scheduler.clone(),
                &paths.log_file,
                lifecycle.clone(),
            )?;
            app.manage(AppState {
                manager: manager.clone(),
                scheduler: scheduler.clone(),
                paths: paths.clone(),
                model_flows: Default::default(),
                pending_occupant: Default::default(),
                credentials,
                lifecycle: lifecycle.clone(),
                image_store,
                image_store_lock: Default::default(),
                image_operation: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                image_readiness,
            });
            scheduler.start();
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let Some(output) = lifecycle
                    .run_operation(orchestration::first_launch(&manager, &scheduler))
                    .await
                else {
                    return;
                };
                if let Ok(status) = output.value {
                    tray::update_status(&app_handle, &status.into());
                }
                if output.quit_action == lifecycle::QuitAction::ExecuteQuit {
                    tray::execute_quit(app_handle);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            tray::handle_window_event(window, event);
            match event {
                WindowEvent::Focused(true) => {
                    window.state::<AppState>().scheduler.set_visible(true);
                    if window.label() == "main" {
                        let _ = tray::emit_main_window_event(
                            window.app_handle(),
                            tray::MainWindowEvent::Focused,
                        );
                    }
                }
                WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed => {
                    window.state::<AppState>().scheduler.set_visible(false);
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::router_status,
            commands::router_start,
            commands::router_stop,
            commands::router_inspect_occupant,
            commands::router_force_terminate_occupant,
            commands::router_cancel_release_observation,
            commands::router_health,
            commands::poll_snapshot,
            commands::router_logs,
            commands::component_versions,
            updater::update_check,
            updater::update_install,
            commands::diagnostics_collect,
            commands::open_log_location,
            commands::agent_detect,
            commands::agent_models,
            commands::agent_render,
            commands::agent_preview,
            commands::agent_write,
            commands::agent_cleanup_preview,
            commands::agent_cleanup_write,
            commands::agent_model_flow_destroy,
            commands::agent_model_config_import,
            commands::agent_model_config_export,
            commands::get_credential,
            commands::save_credential,
            commands::delete_credential,
            autostart::autostart_get,
            autostart::autostart_set_immediate,
            autostart::prepare_for_uninstall,
            commands::desktop_paths,
            commands::window_visibility,
            commands::set_native_language,
            commands::set_agent_draft_dirty,
            commands::resolve_app_quit,
            image_commands::image_readiness,
            image_commands::image_current_operation,
            image_commands::image_conversations,
            image_commands::image_create_conversation,
            image_commands::image_select_conversation,
            image_commands::image_set_conversation_model,
            image_commands::image_delete_conversation,
            image_commands::image_reset_store,
            image_commands::image_messages,
            image_commands::image_select_reference,
            image_commands::image_start_generation,
            image_commands::image_cancel_generation,
        ])
        .build(tauri::generate_context!())
}

pub fn verify_app_startup() -> Result<(), String> {
    build_app()
        .map(drop)
        .map_err(|error| format!("application initialization failed: {error}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    build_app()
        .expect("error while building CodeasierRouter desktop")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let state = app.state::<AppState>();
                if let Ok(operation) = state.image_operation.try_lock() {
                    if let Some(operation) = operation.as_ref() {
                        operation
                            .cancel_flag
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                let lifecycle = &state.lifecycle;
                if tray::should_prevent_exit(lifecycle) {
                    api.prevent_exit();
                    tray::request_quit(app.clone());
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_capability_has_no_arbitrary_shell_file_or_opener_permission() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert_eq!(
            capability["permissions"],
            serde_json::json!(["core:default"])
        );
        let text = capability.to_string();
        assert!(!text.contains("shell:"));
        assert!(!text.contains("opener:"));
        assert!(!text.contains("fs:"));
        assert!(!text.contains("http:"));
    }

    #[test]
    fn malformed_credential_file_is_removed_during_startup() {
        let directory = std::env::temp_dir().join(format!(
            "mtls-router-startup-credential-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("credentials.json");
        std::fs::write(&path, b"{not-json").unwrap();

        let credentials = load_credentials(path.clone());

        assert!(!path.exists());
        assert!(matches!(
            tauri::async_runtime::block_on(credentials.read_summary()),
            Err(CredentialError::NotFound)
        ));
        let _ = std::fs::remove_dir_all(directory);
    }
}
