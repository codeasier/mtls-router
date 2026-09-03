mod autostart;
mod commands;
mod credential;
mod diagnostic_snapshot;
mod error;
mod installation;
mod lifecycle;
mod manager;
mod manager_diagnostics;
mod model_config;
mod orchestration;
mod paths;
mod port_recovery;
mod process_identity;
mod scheduler;
mod sidecar;
mod tray;
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
        .setup(|app| {
            let sidecars = SidecarPaths::resolve();
            let parent = process_identity::current();
            let paths = paths::resolve()?;
            let credentials = load_credentials(std::path::PathBuf::from(&paths.credentials_path));
            let manager = match (sidecars, parent) {
                (Ok(sidecars), Ok(parent)) => match sidecars.validate() {
                    Ok(()) => {
                        match installation::load_or_create(
                            &paths.data_dir,
                            (env!("MTLS_MANAGER_SHA256"), env!("MTLS_ROUTER_SHA256")),
                        ) {
                            Ok(ownership) => {
                                ManagerClient::new(Arc::new(TauriTransportFactory::new(
                                    app.handle().clone(),
                                    sidecars,
                                    parent,
                                    uuid::Uuid::new_v4().to_string(),
                                    ownership,
                                )))
                            }
                            Err(error) => ManagerClient::failed(error),
                        }
                    }
                    Err(error) => ManagerClient::failed(error),
                },
                (Err(error), _) | (_, Err(error)) => ManagerClient::failed(error),
            };
            let observer_app = app.handle().clone();
            let scheduler = PollScheduler::with_observer(manager.clone(), move |snapshot| {
                if snapshot.status.is_some() || snapshot.status_error.is_some() {
                    tray::update_poll_snapshot(&observer_app, &snapshot);
                }
                let _ = observer_app.emit(POLL_SNAPSHOT_EVENT, snapshot);
            });
            let lifecycle = Arc::new(lifecycle::LifecycleState::default());
            autostart::initialize_default(app)?;
            tray::setup(
                app,
                manager.clone(),
                scheduler.clone(),
                &paths.log_directory,
                lifecycle.clone(),
            )?;
            app.manage(AppState {
                manager: manager.clone(),
                scheduler: scheduler.clone(),
                paths,
                model_flows: Default::default(),
                pending_occupant: Default::default(),
                credentials,
                lifecycle: lifecycle.clone(),
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
            commands::apikey_usage,
            autostart::autostart_get,
            autostart::autostart_set_immediate,
            autostart::prepare_for_uninstall,
            commands::desktop_paths,
            commands::window_visibility,
            commands::set_native_language,
            commands::set_agent_draft_dirty,
            commands::resolve_app_quit,
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
                let lifecycle = &app.state::<AppState>().lifecycle;
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
