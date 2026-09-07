mod autostart;
mod commands;
mod credential;
mod diagnostic_snapshot;
mod error;
mod installation;
mod lifecycle;
mod manager;
mod manager_core;
mod manager_diagnostics;
mod model_config;
mod orchestration;
mod paths;
mod port_recovery;
mod protocol;
mod redaction;
mod router_core;
mod runtime;
mod scheduler;
mod support_bundle;
mod tray;
mod types;
mod updater;

use commands::AppState;
use credential::{CredentialError, CredentialStore};
use manager::ManagerClient;
use manager_core::production_factory;
use scheduler::PollScheduler;
use std::sync::Arc;
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

/// Builds the in-process manager client for this desktop session. There is
/// no sidecar to resolve, hash, or spawn; the only startup failures are local
/// paths, installation metadata, and process identity.
fn embedded_manager(data_dir: &str) -> ManagerClient {
    let ownership = match installation::load_or_create(data_dir) {
        Ok(ownership) => ownership,
        Err(error) => return ManagerClient::failed(error),
    };
    match runtime::production_runtime(
        uuid::Uuid::new_v4().to_string(),
        &ownership,
        std::path::Path::new(data_dir),
    ) {
        Ok(runtime) => ManagerClient::new(Arc::new(production_factory(runtime))),
        Err(error) => ManagerClient::failed(error),
    }
}

/// Packaging check: the embedded manager must answer `manager.info` with the
/// identity this desktop build was compiled with. Uses a throwaway data
/// directory so no user state is touched.
pub fn verify_manager_handshake() -> Result<(), String> {
    let data_dir =
        std::env::temp_dir().join(format!("mtls-router-handshake-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&data_dir)
        .map_err(|_| "cannot create verification directory".to_owned())?;
    let result = (|| {
        let manager = embedded_manager(&data_dir.to_string_lossy());
        let info: types::ManagerInfo = tauri::async_runtime::block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_secs(3),
                manager.call("manager.info", serde_json::json!({})),
            )
            .await
            .map_err(|_| "embedded manager handshake timed out".to_owned())?
            .map_err(|error| error.to_string())
        })?;
        manager::validate_handshake(&info).map_err(|error| error.to_string())
    })();
    let _ = std::fs::remove_dir_all(data_dir);
    result
}

fn build_app() -> tauri::Result<tauri::App<tauri::Wry>> {
    let mut builder = tauri::Builder::default()
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
            let paths = paths::resolve()?;
            let credentials = load_credentials(std::path::PathBuf::from(&paths.credentials_path));
            let manager = embedded_manager(&paths.data_dir);
            let diagnostics = diagnostic_snapshot::DiagnosticStore::new(
                paths::last_diagnostics_path(&paths.data_dir),
            );
            let observer_app = app.handle().clone();
            let observer_diagnostics = diagnostics.clone();
            let observer_manager = manager.clone();
            let scheduler = PollScheduler::with_observer(manager.clone(), move |snapshot| {
                let failure = observer_manager.last_diagnostic();
                observer_diagnostics.capture_and_persist(
                    &snapshot,
                    failure
                        .as_ref()
                        .map(|value| (value.stage.as_str(), value.code.as_str())),
                );
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
                diagnostics,
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
            commands::diagnostics_snapshot,
            commands::export_support_bundle,
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
    fn bundle_declares_no_external_binaries_and_no_shell_plugin() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert!(
            config["bundle"].get("externalBin").is_none(),
            "desktop bundles must not ship Go sidecars"
        );
        let shell_plugin = ["tauri-plugin", "-shell"].concat();
        assert!(!include_str!("../Cargo.toml").contains(&shell_plugin));
        let spawn_markers = [
            ["tauri_plugin", "_shell"].concat(),
            [".side", "car("].concat(),
            ["std::process::", "Command"].concat(),
        ];
        for (name, source) in [
            ("manager.rs", include_str!("manager.rs")),
            ("runtime.rs", include_str!("runtime.rs")),
            ("session.rs", include_str!("manager_core/session.rs")),
            ("embedded.rs", include_str!("manager_core/embedded.rs")),
        ] {
            for marker in &spawn_markers {
                assert!(!source.contains(marker), "{name} contains {marker}");
            }
        }
    }

    #[test]
    fn embedded_manager_handshake_needs_no_sidecar_binaries() {
        let exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        for name in ["mtls-router-manager", "mtls-router"] {
            assert!(
                !exe_dir.join(name).exists() && !exe_dir.join(format!("{name}.exe")).exists(),
                "test binary directory must not contain a Go sidecar"
            );
        }
        verify_manager_handshake().expect("in-process handshake");
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
